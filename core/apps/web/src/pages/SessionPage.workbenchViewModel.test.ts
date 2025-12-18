import { describe, expect, it, vi } from "vitest";

vi.mock("react-virtuoso", () => ({}));
vi.mock("react-syntax-highlighter", () => ({ Prism: () => null }));
vi.mock("react-syntax-highlighter/dist/esm/styles/prism", () => ({ oneDark: {} }));

describe("buildWorkbenchThreadViewModel", () => {
  it("renders assistant streaming even when messages list is empty", async () => {
    const { buildWorkbenchThreadViewModel } = await import("./SessionPage");

    const events = [
      {
        id: "e1",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "user_message",
        payload_json: { message_id: "m1", content: "hello", attachments: [] },
        created_at: "2025-12-15T00:00:00.000Z",
      },
      {
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "assistant_chunk",
        payload_json: { content_fragment: "Hi" },
        created_at: "2025-12-15T00:00:01.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel([], [] as any, {}, events as any);
    expect(out.groups.length).toBe(1);
    expect(out.groups[0]?.header?.content).toBe("hello");
    expect(out.groups[0]?.items.some((it: any) => it.kind === "assistant" && String(it.content).includes("Hi"))).toBe(true);
  });

  it("produces a messagesKey that changes when message content changes with same length", async () => {
    const { deriveMessagesKey } = await import("./SessionPage");

    const base = {
      id: "m1",
      session_id: "s1",
      role: "user",
      attachments: [],
      delivery: "immediate",
      created_at: "2025-12-15T00:00:00.000Z",
    };

    const k1 = deriveMessagesKey([{ ...base, content: "hello" }] as any);
    const k2 = deriveMessagesKey([{ ...base, content: "world" }] as any);
    expect(k1).not.toBe(k2);
  });
});
