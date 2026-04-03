import { beforeEach, describe, expect, it, vi } from "vitest";

const { captureProductEventMock } = vi.hoisted(() => ({
  captureProductEventMock: vi.fn(),
}));

vi.mock("./client", () => ({
  captureProductEvent: captureProductEventMock,
}));

import {
  trackTaskCreated,
  trackTurnCompleted,
  trackTurnStarted,
  trackUserMessageSent,
} from "./activity";

describe("usage analytics activity helpers", () => {
  beforeEach(() => {
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
});
