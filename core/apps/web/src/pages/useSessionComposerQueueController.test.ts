import { describe, expect, it } from "vitest";
import type { Message } from "../api/client";
import type { PendingMessageEntry } from "./sessionView/pendingMessages";
import {
  reassignPendingMessagesToSession,
  shouldCarryPendingMessagesAcrossSessionChange,
} from "./useSessionComposerQueueController";

const buildPendingMessageEntry = (
  clientId: string,
  sessionId: string,
): PendingMessageEntry => ({
  clientId,
  message: {
    id: clientId,
    session_id: sessionId,
    task_id: "task-1",
    turn_id: `turn-${clientId}`,
    turn_sequence: 1,
    order_seq: 1,
    role: "user",
    content: `content-${clientId}`,
    attachments: [],
    delivery: "immediate",
    created_at: "2026-03-16T00:00:00.000Z",
  } satisfies Message,
});

describe("useSessionComposerQueueController handoff helpers", () => {
  it("carries pending first-turn messages into a newly assigned empty session", () => {
    const pendingMessages = [buildPendingMessageEntry("msg-1", "session-temp")];

    expect(
      shouldCarryPendingMessagesAcrossSessionChange({
        previousSessionId: "session-temp",
        nextSessionId: "session-real",
        handoff: {
          fromSessionId: "session-temp",
          messageIds: ["msg-1"],
        },
        pendingMessages,
        messageCount: 0,
        turnCount: 0,
      }),
    ).toBe(true);

    expect(
      reassignPendingMessagesToSession(pendingMessages, "session-real", {
        fromSessionId: "session-temp",
        messageIds: ["msg-1"],
      }),
    ).toEqual([buildPendingMessageEntry("msg-1", "session-real")]);
  });

  it("does not carry pending messages once the next session already has authoritative content", () => {
    const pendingMessages = [buildPendingMessageEntry("msg-1", "session-temp")];

    expect(
      shouldCarryPendingMessagesAcrossSessionChange({
        previousSessionId: "session-temp",
        nextSessionId: "session-real",
        handoff: {
          fromSessionId: "session-temp",
          messageIds: ["msg-1"],
        },
        pendingMessages,
        messageCount: 1,
        turnCount: 0,
      }),
    ).toBe(false);
  });
});
