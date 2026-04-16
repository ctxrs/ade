import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const clientMocks = vi.hoisted(() => ({
  recordClientCounterMetric: vi.fn(),
  recordClientGaugeMetric: vi.fn(),
  recordClientHistogramMetric: vi.fn(),
}));

const diagnosticMocks = vi.hoisted(() => ({
  emitUiDiagnostic: vi.fn(),
}));

const analyticsMocks = vi.hoisted(() => ({
  trackForegroundBacklogObserved: vi.fn(),
  trackForegroundFreshnessSlaMissed: vi.fn(),
  trackForegroundGapRecoveryObserved: vi.fn(),
}));

vi.mock("../api/client", () => ({
  recordClientCounterMetric: clientMocks.recordClientCounterMetric,
  recordClientGaugeMetric: clientMocks.recordClientGaugeMetric,
  recordClientHistogramMetric: clientMocks.recordClientHistogramMetric,
}));

vi.mock("./diagnosticsChannel", () => ({
  emitUiDiagnostic: diagnosticMocks.emitUiDiagnostic,
}));

vi.mock("../utils/analytics", () => ({
  trackForegroundBacklogObserved: analyticsMocks.trackForegroundBacklogObserved,
  trackForegroundFreshnessSlaMissed: analyticsMocks.trackForegroundFreshnessSlaMissed,
  trackForegroundGapRecoveryObserved: analyticsMocks.trackForegroundGapRecoveryObserved,
}));

import {
  noteGapRecoveryFinished,
  noteGapRecoveryStarted,
  noteGapRepairMismatch,
  noteFinalDeltaReceived,
  noteFinalVisible,
  noteInterruptClicked,
  noteInterruptPendingVisible,
  noteLateChunkAfterTerminal,
  noteNavThreadActivityMismatch,
  noteProjectionOrSeqRegression,
  noteSessionSwitchFirstPaint,
  noteSessionSwitchStarted,
  noteSwitchStaleVisible,
  resetForegroundFreshnessTelemetryForTests,
} from "./foregroundFreshnessTelemetry";

describe("foregroundFreshnessTelemetry", () => {
  beforeEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.spyOn(performance, "now").mockReturnValue(0);
    clientMocks.recordClientCounterMetric.mockReset();
    clientMocks.recordClientGaugeMetric.mockReset();
    clientMocks.recordClientHistogramMetric.mockReset();
    diagnosticMocks.emitUiDiagnostic.mockReset();
    analyticsMocks.trackForegroundBacklogObserved.mockReset();
    analyticsMocks.trackForegroundFreshnessSlaMissed.mockReset();
    analyticsMocks.trackForegroundGapRecoveryObserved.mockReset();
    resetForegroundFreshnessTelemetryForTests();
  });

  afterEach(() => {
    vi.useRealTimers();
    resetForegroundFreshnessTelemetryForTests();
  });

  it("records session switch first paint latency", () => {
    vi.spyOn(performance, "now").mockReturnValue(10);
    noteSessionSwitchStarted("session-old", "session-new");
    vi.spyOn(performance, "now").mockReturnValue(65);
    noteSessionSwitchFirstPaint("session-new");

    expect(clientMocks.recordClientHistogramMetric).toHaveBeenCalledWith(
      "workbench.switch_to_first_paint_ms",
      "ms",
      55,
      { phase: "first_paint" },
    );
    expect(diagnosticMocks.emitUiDiagnostic).not.toHaveBeenCalled();
    expect(analyticsMocks.trackForegroundFreshnessSlaMissed).not.toHaveBeenCalled();
  });

  it("records final visibility freshness and emits SLA miss diagnostics when breached", () => {
    const timeOrigin = performance.timeOrigin ?? 0;
    vi.spyOn(performance, "now").mockReturnValue(100);
    noteFinalDeltaReceived({
      sessionId: "session-1",
      turnId: "turn-1",
      emittedAtMs: timeOrigin + 60,
      lastEventSeq: 42,
    });

    vi.spyOn(performance, "now").mockReturnValue(220);
    noteFinalVisible("session-1", ["turn-1"]);

    expect(clientMocks.recordClientHistogramMetric).toHaveBeenCalledWith(
      "workbench.final_ws_to_dom_ms",
      "ms",
      120,
      undefined,
    );
    expect(clientMocks.recordClientHistogramMetric).toHaveBeenCalledWith(
      "workbench.final_ingress_to_dom_ms",
      "ms",
      160,
      undefined,
    );
    expect(diagnosticMocks.emitUiDiagnostic).toHaveBeenCalled();
    expect(analyticsMocks.trackForegroundFreshnessSlaMissed).toHaveBeenCalledWith({
      metric: "workbench.final_ws_to_dom_ms",
      surface: "final_delivery",
      bucket: "slight",
    });
    expect(analyticsMocks.trackForegroundFreshnessSlaMissed).toHaveBeenCalledWith({
      metric: "workbench.final_ingress_to_dom_ms",
      surface: "final_delivery",
      bucket: "slight",
    });
  });

  it("records interrupt click to pending latency", () => {
    vi.spyOn(performance, "now").mockReturnValue(50);
    noteInterruptClicked("session-1", "thread_header");
    vi.spyOn(performance, "now").mockReturnValue(70);
    noteInterruptPendingVisible("session-1");

    expect(clientMocks.recordClientHistogramMetric).toHaveBeenCalledWith(
      "workbench.interrupt_click_to_pending_ms",
      "ms",
      20,
      { source: "thread_header" },
    );
    expect(diagnosticMocks.emitUiDiagnostic).not.toHaveBeenCalled();
  });

  it("records gap recovery timeout once the recovery budget is exceeded", () => {
    vi.useFakeTimers();
    noteGapRecoveryStarted("session-1", "session_gap");

    vi.advanceTimersByTime(1000);

    expect(clientMocks.recordClientCounterMetric).toHaveBeenCalledWith(
      "workbench.foreground_gap_recovery_timeout_count",
    );
    expect(analyticsMocks.trackForegroundGapRecoveryObserved).toHaveBeenCalledWith({
      result: "timeout",
    });
    expect(analyticsMocks.trackForegroundFreshnessSlaMissed).toHaveBeenCalledWith({
      metric: "workbench.foreground_gap_recovery_timeout_count",
      surface: "gap_recovery",
      bucket: "severe",
    });
    expect(diagnosticMocks.emitUiDiagnostic).toHaveBeenCalledWith(
      expect.objectContaining({
        code: "foreground_gap_recovery.timeout",
        severity: "error",
      }),
    );

    noteGapRecoveryFinished("session-1");
  });

  it("records invariant counters once per dedupe key", () => {
    noteLateChunkAfterTerminal("turn-1");
    noteLateChunkAfterTerminal("turn-1");
    noteProjectionOrSeqRegression("session-1", "last_event_seq", 3, 7);
    noteGapRepairMismatch("session-1", 9, 5);
    noteSwitchStaleVisible("task-1", "session-old", "session-new");
    noteNavThreadActivityMismatch("task-1", "session-new", true, false);

    expect(clientMocks.recordClientCounterMetric).toHaveBeenCalledWith(
      "workbench.late_chunk_after_terminal_count",
      undefined,
    );
    expect(clientMocks.recordClientCounterMetric).toHaveBeenCalledWith(
      "workbench.projection_or_seq_regression_count",
      { dimension: "last_event_seq" },
    );
    expect(clientMocks.recordClientCounterMetric).toHaveBeenCalledWith(
      "workbench.gap_repair_mismatch_count",
      undefined,
    );
    expect(clientMocks.recordClientCounterMetric).toHaveBeenCalledWith(
      "workbench.switch_stale_visible_count",
      undefined,
    );
    expect(clientMocks.recordClientCounterMetric).toHaveBeenCalledWith(
      "workbench.nav_thread_activity_mismatch_count",
      undefined,
    );
    expect(clientMocks.recordClientCounterMetric).toHaveBeenCalledTimes(5);
  });
});
