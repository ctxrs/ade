import { beforeEach, describe, expect, it, vi } from "vitest";
import { handleSessionReplicaFreshnessEvent } from "./sessionReplicaBridge";

const getDaemonClientConfigMock = vi.hoisted(() => vi.fn(() => ({
  baseUrl: "http://127.0.0.1:4399",
  wsBaseUrl: "ws://127.0.0.1:4399",
  authToken: "browser-secret",
  runId: null,
})));
const subscribeDaemonConfigMock = vi.hoisted(() => vi.fn(() => () => {}));
const noteGapRecoveryStartedMock = vi.hoisted(() => vi.fn());
const noteFinalDeltaReceivedMock = vi.hoisted(() => vi.fn());

vi.mock("../utils/desktop", () => ({
  isDesktopApp: () => false,
}));

vi.mock("../api/client", () => ({
  getDaemonClientConfig: getDaemonClientConfigMock,
  subscribeDaemonConfig: subscribeDaemonConfigMock,
  getSessionHead: vi.fn(),
  getSessionSnapshot: vi.fn(),
  getSessionState: vi.fn(),
}));

vi.mock("./foregroundFreshnessTelemetry", () => ({
  noteFinalDeltaReceived: noteFinalDeltaReceivedMock,
  noteGapRecoveryFinished: vi.fn(),
  noteGapRecoveryStarted: noteGapRecoveryStartedMock,
  noteGapRepairMismatch: vi.fn(),
  noteProjectionOrSeqRegression: vi.fn(),
}));

describe("SessionReplicaBridge", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("records replica freshness telemetry through the main-thread handler", () => {
    handleSessionReplicaFreshnessEvent({
      type: "gap_recovery_started",
      sessionId: "session-1",
      reason: "session_gap",
    });
    handleSessionReplicaFreshnessEvent({
      type: "final_delta_received",
      sessionId: "session-1",
      turnId: "turn-1",
      emittedAtMs: 12,
      lastEventSeq: 3,
    });

    expect(noteGapRecoveryStartedMock).toHaveBeenCalledWith("session-1", "session_gap");
    expect(noteFinalDeltaReceivedMock).toHaveBeenCalledWith({
      sessionId: "session-1",
      turnId: "turn-1",
      emittedAtMs: 12,
      lastEventSeq: 3,
    });
  });
});
