import { beforeEach, describe, expect, it, vi } from "vitest";

const { apiAnyMock, trackFirstTurnSubmittedMock } = vi.hoisted(() => ({
  apiAnyMock: vi.fn(),
  trackFirstTurnSubmittedMock: vi.fn(),
}));

vi.mock("./clientBase", () => ({
  apiAny: apiAnyMock,
  authToken: vi.fn(() => null),
}));

vi.mock("../utils/analytics", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../utils/analytics")>();
  return {
    ...actual,
    trackFirstTurnSubmitted: trackFirstTurnSubmittedMock,
  };
});

import { createSession } from "./clientSessions";

describe("createSession analytics", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apiAnyMock.mockResolvedValue({ id: "session-123" });
  });

  it("emits first_turn_submitted when session starts with initial_prompt", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", {
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
    await createSession("task-1", "codex", "gpt-5-codex");
    expect(trackFirstTurnSubmittedMock).not.toHaveBeenCalled();
  });
});
