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
  }, 10000);

  it("interleaves tool + thought activity and appends a status row", async () => {
    const { buildWorkbenchThreadViewModel } = await import("./SessionPage");

    const turns = [
      {
        turn_id: "t1",
        session_id: "s1",
        status: "running",
        started_at: "2025-12-15T00:00:00.000Z",
        updated_at: "2025-12-15T00:00:05.000Z",
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
      },
    ];

    const messages = [
      {
        id: "m1",
        session_id: "s1",
        role: "user",
        content: "hello",
        attachments: [],
        delivery: "immediate",
        created_at: "2025-12-15T00:00:00.000Z",
        turn_id: "t1",
      },
    ];

    const events = [
      {
        seq: 1,
        id: "e1",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: { tool_call_id: "tool-1", acp_update: { toolCallId: "tool-1", title: "ls" } },
        created_at: "2025-12-15T00:00:01.000Z",
      },
      {
        seq: 2,
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "thought_chunk",
        payload_json: { content_fragment: "thinking" },
        created_at: "2025-12-15T00:00:02.000Z",
      },
      {
        seq: 3,
        id: "e3",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: { tool_call_id: "tool-2", acp_update: { toolCallId: "tool-2", title: "pwd" } },
        created_at: "2025-12-15T00:00:03.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const items = out.groups[0]?.items ?? [];
    expect(items.map((it: any) => it.kind)).toEqual(["tool", "thought", "tool", "turn_status"]);
  }, 10000);

  it("uses the latest status update text per turn", async () => {
    const { buildWorkbenchThreadViewModel } = await import("./SessionPage");

    const turns = [
      {
        turn_id: "t1",
        session_id: "s1",
        status: "running",
        started_at: "2025-12-15T00:00:00.000Z",
        updated_at: "2025-12-15T00:00:05.000Z",
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
      },
    ];

    const messages = [
      {
        id: "m1",
        session_id: "s1",
        role: "user",
        content: "hello",
        attachments: [],
        delivery: "immediate",
        created_at: "2025-12-15T00:00:00.000Z",
        turn_id: "t1",
      },
    ];

    const events = [
      {
        seq: 1,
        id: "e1",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "notice",
        payload_json: { _meta: { statusText: "Preparing instructions" } },
        created_at: "2025-12-15T00:00:01.000Z",
      },
      {
        seq: 2,
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "notice",
        payload_json: { _meta: { statusText: "Preparing specs" } },
        created_at: "2025-12-15T00:00:02.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const statusItem = out.groups[0]?.items.find((it: any) => it.kind === "turn_status") as any;
    expect(statusItem?.status_text).toBe("Preparing specs");
  }, 10000);

  it("prefers tool harness status updates for turn status rows", async () => {
    const { buildWorkbenchThreadViewModel } = await import("./SessionPage");

    const turns = [
      {
        turn_id: "t1",
        session_id: "s1",
        status: "running",
        started_at: "2025-12-15T00:00:00.000Z",
        updated_at: "2025-12-15T00:00:02.000Z",
        tool_total: 1,
        tool_pending: 1,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
      },
    ];

    const messages = [
      {
        id: "m1",
        session_id: "s1",
        role: "user",
        content: "hello",
        attachments: [],
        delivery: "immediate",
        created_at: "2025-12-15T00:00:00.000Z",
        turn_id: "t1",
      },
    ];

    const events = [
      {
        seq: 1,
        id: "e1",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: {
          tool_call_id: "tool-1",
          acp_update: {
            toolCallId: "tool-1",
            kind: "search",
            status: "running",
            input: { query: "alpha" },
          },
        },
        created_at: "2025-12-15T00:00:01.000Z",
      },
      {
        seq: 2,
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "notice",
        payload_json: { _meta: { statusText: "Considering" } },
        created_at: "2025-12-15T00:00:02.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const statusItem = out.groups[0]?.items.find((it: any) => it.kind === "turn_status") as any;
    expect(statusItem?.status_text).toBe("Searching alpha");
  }, 10000);

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
  }, 10000);
});
