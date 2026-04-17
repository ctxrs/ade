import type { Message, SessionTurn } from "../../api/client";
import { describe, expect, it } from "vitest";
import {
  buildAssistantStreamingStamp,
  buildMessagesStamp,
  buildTurnsStamp,
} from "./stamps";

describe("session thread projection stamps", () => {
  it("tracks messages by revision and cheap structural markers instead of full content", () => {
    const messages = [
      { id: "message-1", content: "hello" },
      { id: "message-2", content: "world" },
    ] as Message[];

    expect(buildMessagesStamp(messages, 7)).toBe("7:2:message-1:message-2");
    expect(
      buildMessagesStamp(
        [
          { ...messages[0], content: "changed a lot" },
          { ...messages[1], content: "changed too" },
        ] as Message[],
        7,
      ),
    ).toBe("7:2:message-1:message-2");
  });

  it("changes turns stamps when the structural edge changes or the revision changes", () => {
    const assistantStreamingStamp = buildAssistantStreamingStamp({}, 3);
    const turns = [
      { turn_id: "turn-1" },
      { turn_id: "turn-2" },
    ] as SessionTurn[];

    expect(buildTurnsStamp(turns, 5, assistantStreamingStamp)).toBe("5:2:turn-1:turn-2:3:0");
    expect(buildTurnsStamp([{ turn_id: "older" }, ...turns] as SessionTurn[], 6, assistantStreamingStamp)).toBe(
      "6:3:older:turn-2:3:0",
    );
  });

  it("tracks assistant streaming by explicit revision and entry count", () => {
    expect(buildAssistantStreamingStamp({}, 0)).toBe("0:0");
    expect(
      buildAssistantStreamingStamp(
        {
          "turn-1": { content: "partial", providerMessageId: "provider-1" },
        },
        4,
      ),
    ).toBe("4:1");
  });
});
