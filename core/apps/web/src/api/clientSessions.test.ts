import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  apiAnyMock,
  trackFirstTurnSubmittedMock,
  trackSessionCreatedMock,
  trackUserMessageSentMock,
  getDaemonConnectionMock,
} = vi.hoisted(() => ({
  apiAnyMock: vi.fn(),
  trackFirstTurnSubmittedMock: vi.fn(),
  trackSessionCreatedMock: vi.fn(),
  trackUserMessageSentMock: vi.fn(),
  getDaemonConnectionMock: vi.fn(() => ({
    baseUrl: "http://127.0.0.1:4399",
    targetScope: { kind: "desktop_local" },
    authToken: null,
  })),
}));

vi.mock("./clientBase", () => ({
  apiAny: apiAnyMock,
  authToken: vi.fn(() => null),
}));

vi.mock("./daemonConnection", () => ({
  getDaemonConnection: getDaemonConnectionMock,
  getDaemonHttpUrl: vi.fn(),
}));

vi.mock("../utils/analytics", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../utils/analytics")>();
  return {
    ...actual,
    trackFirstTurnSubmitted: trackFirstTurnSubmittedMock,
    trackSessionCreated: trackSessionCreatedMock,
    trackUserMessageSent: trackUserMessageSentMock,
  };
});

import { createSession, postMessage } from "./clientSessions";

describe("createSession analytics", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apiAnyMock.mockResolvedValue({ id: "session-123" });
  });

  it("posts canonical execution_environment and split analytics fields", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", {
      execution_environment: "sandbox",
    });

    const [, options] = apiAnyMock.mock.calls[0] ?? [];
    expect(JSON.parse(String(options?.body ?? "{}"))).toEqual({
      provider_id: "codex",
      model_id: "gpt-5-codex",
      execution_environment: "sandbox",
    });
    expect(trackSessionCreatedMock).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5-codex",
      executionEnvironment: "sandbox",
      sessionRootKind: "worktree",
      sessionLocation: "local",
    });
  });

  it("emits first_turn_submitted when session starts with initial_prompt", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", {
      execution_environment: "host",
      initial_prompt: "hello",
      initial_message_id: "message-1",
      initial_turn_id: "turn-1",
    });

    expect(trackFirstTurnSubmittedMock).toHaveBeenCalledWith({
      sessionId: "session-123",
      providerId: "codex",
      modelId: "gpt-5-codex",
    });
    expect(trackUserMessageSentMock).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5-codex",
      reasoningEffort: null,
      executionEnvironment: "host",
      sessionKind: "primary",
      isFirstTurn: true,
    });
  });

  it("does not emit first_turn_submitted when initial_prompt is absent", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", { execution_environment: "host" });
    expect(trackFirstTurnSubmittedMock).not.toHaveBeenCalled();
  });

  it("posts split reasoning_effort while preserving analytics on the effective full model id", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", {
      execution_environment: "host",
      reasoning_effort: "xhigh",
    });

    const [, options] = apiAnyMock.mock.calls[0] ?? [];
    expect(JSON.parse(String(options?.body ?? "{}"))).toEqual({
      provider_id: "codex",
      model_id: "gpt-5-codex",
      reasoning_effort: "xhigh",
      execution_environment: "host",
    });
    expect(trackSessionCreatedMock).toHaveBeenCalledWith(expect.objectContaining({
      providerId: "codex",
      modelId: "gpt-5-codex/xhigh",
    }));
  });
});

describe("postMessage analytics", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apiAnyMock.mockResolvedValue({ id: "message-123" });
  });

  it("emits user_message_sent with bounded analytics metadata", async () => {
    await postMessage("session-1", "hello", "immediate", [], {
      id: "message-1",
      turn_id: "turn-1",
      analytics: {
        providerId: "codex",
        modelId: "gpt-5-codex/xhigh",
        reasoningEffort: "xhigh",
        executionEnvironment: "sandbox",
        sessionKind: "primary",
        isFirstTurn: false,
      },
    });

    expect(trackUserMessageSentMock).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5-codex/xhigh",
      reasoningEffort: "xhigh",
      executionEnvironment: "sandbox",
      sessionKind: "primary",
      isFirstTurn: false,
    });
    expect(trackFirstTurnSubmittedMock).toHaveBeenCalledWith({
      sessionId: "session-1",
      providerId: "codex",
      modelId: "gpt-5-codex/xhigh",
    });
  });
});
