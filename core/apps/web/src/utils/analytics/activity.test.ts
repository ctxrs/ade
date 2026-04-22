import { beforeEach, describe, expect, it, vi } from "vitest";

const { captureIncidentEventMock, captureProductEventMock } = vi.hoisted(() => ({
  captureIncidentEventMock: vi.fn(),
  captureProductEventMock: vi.fn(),
}));

vi.mock("./client", () => ({
  captureIncidentEvent: captureIncidentEventMock,
  captureProductEvent: captureProductEventMock,
}));

import {
  trackDesktopWebviewRecoveryObserved,
  trackTaskCreated,
  trackTurnCompleted,
  trackTurnStarted,
  trackUnknownEventBurst,
  trackUserMessageSent,
} from "./activity";

describe("usage analytics activity helpers", () => {
  beforeEach(() => {
    captureIncidentEventMock.mockReset();
    captureProductEventMock.mockReset();
    window.localStorage.clear();
    window.sessionStorage.clear();
  });

  it("normalizes task, message, and turn events onto base model ids plus reasoning effort", () => {
    trackTaskCreated({
      providerId: "codex",
      modelId: "gpt-5.4/high",
      executionEnvironment: "sandbox",
    });
    trackUserMessageSent({
      providerId: "codex",
      modelId: "gpt-5.4/high",
      executionEnvironment: "sandbox",
      sessionKind: "primary",
      isFirstTurn: true,
    });
    trackTurnStarted({
      providerId: "codex",
      modelId: "gpt-5.4/high",
      executionEnvironment: "sandbox",
      sessionKind: "subagent",
    });

    expect(captureProductEventMock).toHaveBeenCalledWith(
      "task_created",
      1,
      {
        provider_id: "codex",
        model_id: "gpt-5.4",
        reasoning_effort: "high",
        execution_environment: "sandbox",
        session_kind: "primary",
      },
    );
    expect(captureProductEventMock).toHaveBeenCalledWith(
      "user_message_sent",
      1,
      {
        provider_id: "codex",
        model_id: "gpt-5.4",
        reasoning_effort: "high",
        execution_environment: "sandbox",
        session_kind: "primary",
        is_first_turn: true,
      },
    );
    expect(captureProductEventMock).toHaveBeenCalledWith(
      "turn_started",
      1,
      {
        provider_id: "codex",
        model_id: "gpt-5.4",
        reasoning_effort: "high",
        execution_environment: "sandbox",
        session_kind: "subagent",
      },
    );
  });

  it("flattens bounded token metrics onto turn_completed", () => {
    trackTurnCompleted({
      providerId: "codex",
      modelId: "gpt-5.4/high",
      executionEnvironment: "sandbox",
      sessionKind: "subagent",
      status: "completed",
      durationMs: 2_000,
      metrics: {
        context_tokens_estimate: 120,
        total_input_tokens: 80,
        total_output_tokens: 40,
        context_window_tokens: 200,
        remaining_tokens_estimate: 80,
        remaining_fraction: 0.4,
      },
    });

    expect(captureProductEventMock).toHaveBeenCalledWith(
      "turn_completed",
      1,
      {
        provider_id: "codex",
        model_id: "gpt-5.4",
        reasoning_effort: "high",
        execution_environment: "sandbox",
        status: "completed",
        duration_bucket: "under_15s",
        session_kind: "subagent",
        total_tokens_estimate: 120,
        input_tokens: 80,
        output_tokens: 40,
        context_window_tokens: 200,
        remaining_tokens_estimate: 80,
        remaining_fraction: 0.4,
      },
    );
  });

  it("buckets unknown event original types before remote incident capture", () => {
    trackUnknownEventBurst({
      source: "session_replica_ingest",
      sessionId: "session-123",
      taskId: "task-123",
      workspaceId: "workspace-123",
      originalType: "vendor.unique.event.name.with.unbounded.detail",
      count: 25,
      windowMs: 5_000,
    });

    expect(captureIncidentEventMock).toHaveBeenCalledWith(
      "unknown_event_burst",
      1,
      {
        source: "session_replica_ingest",
        has_session_scope: true,
        has_task_scope: true,
        has_workspace_scope: true,
        original_type_class: "other",
        count: 25,
        window_ms: 5_000,
      },
      { source: "session_replica_ingest" },
    );
  });

  it("captures metadata-only desktop webview recovery events", () => {
    trackDesktopWebviewRecoveryObserved({
      trigger: "heartbeat_timeout",
      action: "recreate",
      surface: "workbench",
      daemonHealth: "ok",
      suppressionReason: "window_not_focused",
    });

    expect(captureProductEventMock).toHaveBeenCalledWith(
      "desktop_webview_recovery_observed",
      1,
      {
        trigger: "heartbeat_timeout",
        action: "recreate",
        surface: "workbench",
        daemon_health: "ok",
        suppression_reason: "window_not_focused",
      },
    );
  });
});
