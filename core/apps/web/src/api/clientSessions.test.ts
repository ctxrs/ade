import { beforeEach, describe, expect, it, vi } from "vitest";

const { apiAnyMock, trackFirstTurnSubmittedMock, trackSessionCreatedMock, getDaemonConnectionMock } = vi.hoisted(() => ({
  apiAnyMock: vi.fn(),
  trackFirstTurnSubmittedMock: vi.fn(),
  trackSessionCreatedMock: vi.fn(),
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
  };
});

import { createSession } from "./clientSessions";

describe("createSession analytics", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apiAnyMock.mockResolvedValue({ id: "session-123" });
  });

  it("posts canonical execution_environment and split analytics fields", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", {
      execution_environment: "container_disk_isolated",
    });

    const [, options] = apiAnyMock.mock.calls[0] ?? [];
    expect(JSON.parse(String(options?.body ?? "{}"))).toEqual({
      provider_id: "codex",
      model_id: "gpt-5-codex",
      execution_environment: "container_disk_isolated",
    });
    expect(trackSessionCreatedMock).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5-codex",
      executionEnvironment: "container_disk_isolated",
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
  });

  it("does not emit first_turn_submitted when initial_prompt is absent", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", { execution_environment: "host" });
    expect(trackFirstTurnSubmittedMock).not.toHaveBeenCalled();
  });
});
