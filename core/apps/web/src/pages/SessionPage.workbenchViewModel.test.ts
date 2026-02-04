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
        payload_json: { message_id: "m1", content: "hello", attachments: [], order_seq: 1 },
        created_at: "2025-12-15T00:00:00.000Z",
      },
      {
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "assistant_chunk",
        payload_json: { content_fragment: "Hi", order_seq: 2 },
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
        order_seq: 1,
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
        payload_json: { tool_call_id: "tool-1", title: "ls", order_seq: 1 },
        created_at: "2025-12-15T00:00:01.000Z",
      },
      {
        seq: 2,
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "thought_chunk",
        payload_json: { content_fragment: "thinking", order_seq: 2 },
        created_at: "2025-12-15T00:00:02.000Z",
      },
      {
        seq: 3,
        id: "e3",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: { tool_call_id: "tool-2", title: "pwd", order_seq: 3 },
        created_at: "2025-12-15T00:00:03.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const items = out.groups[0]?.items ?? [];
    expect(items.map((it: any) => it.kind)).toEqual(["tool", "thought", "tool", "turn_status"]);
  }, 10000);

  it("uses CRP reasoning summaries for status and keeps trace chunks in thought rows", async () => {
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
        order_seq: 1,
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
        payload_json: { kind: "reasoning_summary", text: "Reading foo", crp_seq: 1, order_seq: 1 },
        created_at: "2025-12-15T00:00:01.000Z",
      },
      {
        seq: 2,
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "thought_chunk",
        payload_json: { content_fragment: "Thinking about bar", crp_seq: 2, crp_channel: "data", order_seq: 2 },
        created_at: "2025-12-15T00:00:02.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const items = out.groups[0]?.items ?? [];
    const thoughtItems = items.filter((it: any) => it.kind === "thought");
    expect(thoughtItems.length).toBe(1);
    expect(String(thoughtItems[0]?.content)).toContain("Thinking about bar");
    expect(String(thoughtItems[0]?.content)).not.toContain("Reading foo");

    const statusItem = items.find((it: any) => it.kind === "turn_status") as any;
    expect(statusItem?.custom_status).toBe("Reading foo");
  }, 10000);

  it("splits CRP thought chunks into blocks between control events", async () => {
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
        order_seq: 1,
      },
    ];

    const events = [
      {
        seq: 1,
        id: "e1",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "thought_chunk",
        payload_json: { content_fragment: "first", crp_seq: 1, crp_channel: "data", order_seq: 1 },
        created_at: "2025-12-15T00:00:01.000Z",
      },
      {
        seq: 2,
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: { tool_call_id: "tool-1", title: "ls", order_seq: 2 },
        created_at: "2025-12-15T00:00:02.000Z",
      },
      {
        seq: 3,
        id: "e3",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "thought_chunk",
        payload_json: { content_fragment: "second", crp_seq: 3, crp_channel: "data", order_seq: 3 },
        created_at: "2025-12-15T00:00:03.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const items = out.groups[0]?.items ?? [];
    const kinds = items.map((it: any) => it.kind);
    expect(kinds).toEqual(["thought", "tool", "thought", "turn_status"]);
  }, 10000);

  it("orders tool activity by order_seq", async () => {
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
        order_seq: 1,
      },
    ];

    const events = [
      {
        id: "e1",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: { tool_call_id: "tool-1", title: "first", order_seq: 1 },
        created_at: "2025-12-15T00:00:02.000Z",
      },
      {
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: { tool_call_id: "tool-2", title: "second", order_seq: 2 },
        created_at: "2025-12-15T00:00:01.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const tools = (out.groups[0]?.items ?? []).filter((it: any) => it.kind === "tool");
    expect(tools.map((it: any) => it.tool_call_id)).toEqual(["tool-1", "tool-2"]);
  }, 10000);

  it("keeps thought ordering stable when final chunks arrive", async () => {
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
        order_seq: 1,
      },
    ];

    const events = [
      {
        seq: 1,
        id: "e1",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "thought_chunk",
        payload_json: { content_fragment: "draft", item_id: "thought-1", order_seq: 1 },
        created_at: "2025-12-15T00:00:01.000Z",
      },
      {
        seq: 2,
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: { tool_call_id: "tool-1", title: "ls", order_seq: 2 },
        created_at: "2025-12-15T00:00:02.000Z",
      },
      {
        seq: 3,
        id: "e3",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "thought_chunk",
        payload_json: { full_content: "final", is_final: true, item_id: "thought-1", order_seq: 1 },
        created_at: "2025-12-15T00:00:03.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const items = out.groups[0]?.items ?? [];
    expect(items.map((it: any) => it.kind)).toEqual(["thought", "tool", "turn_status"]);
  }, 10000);

  it("keeps tool interleaving stable across tool updates", async () => {
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
        order_seq: 1,
      },
    ];

    const events = [
      {
        seq: 1,
        id: "e1",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "thought_chunk",
        payload_json: { content_fragment: "thinking", order_seq: 2 },
        created_at: "2025-12-15T00:00:01.000Z",
      },
      {
        seq: 2,
        id: "e2",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call",
        payload_json: { tool_call_id: "tool-1", title: "search", order_seq: 1 },
        created_at: "2025-12-15T00:00:03.000Z",
      },
      {
        seq: 3,
        id: "e3",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_call_update",
        payload_json: { tool_call_id: "tool-1", outputText: "partial", order_seq: 1 },
        created_at: "2025-12-15T00:00:04.000Z",
      },
      {
        seq: 4,
        id: "e4",
        session_id: "s1",
        run_id: "r1",
        turn_id: "t1",
        event_type: "tool_result",
        payload_json: { tool_call_id: "tool-1", outputText: "done", order_seq: 1 },
        created_at: "2025-12-15T00:00:05.000Z",
      },
    ];

    const out = buildWorkbenchThreadViewModel(turns as any, messages as any, {}, events as any);
    const items = out.groups[0]?.items ?? [];
    expect(items.map((it: any) => it.kind)).toEqual(["tool", "thought", "turn_status"]);
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
        order_seq: 1,
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
    expect(statusItem?.custom_status).toBe("Preparing specs");
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
        order_seq: 1,
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
          kind: "search",
          status: "running",
          input: { query: "alpha" },
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
    expect(statusItem?.custom_status).toBe("Searching alpha");
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

  it("merges optimistic queued messages into the queue panel list", async () => {
    const { mergeQueuedMessagesForPanel } = await import("./SessionPage.workbenchViewModel");

    const pending = [
      {
        clientId: "client-1",
        message: {
          id: "client-1",
          session_id: "s1",
          task_id: "t1",
          role: "user",
          content: "queued",
          delivery: "queued",
          created_at: "2025-12-15T00:00:00.000Z",
        },
      },
    ];

    const merged = mergeQueuedMessagesForPanel([] as any, pending as any);
    expect(merged).toHaveLength(1);
    expect(String(merged[0]?.id)).toBe("client-1");
  }, 10000);

  it("filters queued panel items once a turn starts running", async () => {
    const { filterQueuedMessagesForPanel } = await import("./SessionPage.workbenchViewModel");

    const queue = [
      {
        id: "m1",
        session_id: "s1",
        task_id: "t1",
        role: "user",
        content: "queued",
        delivery: "queued",
        created_at: "2025-12-15T00:00:00.000Z",
      },
    ];

    const turns = [
      {
        turn_id: "t1",
        session_id: "s1",
        user_message_id: "m1",
        status: "running",
        started_at: "2025-12-15T00:00:01.000Z",
        updated_at: "2025-12-15T00:00:02.000Z",
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
      },
    ];

    const filtered = filterQueuedMessagesForPanel(queue as any, turns as any);
    expect(filtered).toEqual([]);
  }, 10000);

  it("drops queued turns from the thread list when message ids are hidden", async () => {
    const { buildWorkbenchThreadViewModelFromTurns, filterTurnsForQueuedMessages } =
      await import("./SessionPage.workbenchViewModel");

    const turns = [
      {
        turn_id: "t-queued",
        session_id: "s1",
        user_message_id: "m-queued",
        status: "queued",
        started_at: "2025-12-15T00:00:00.000Z",
        updated_at: "2025-12-15T00:00:01.000Z",
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
      },
      {
        turn_id: "t-live",
        session_id: "s1",
        user_message_id: "m-live",
        status: "running",
        started_at: "2025-12-15T00:00:02.000Z",
        updated_at: "2025-12-15T00:00:03.000Z",
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
      },
    ];

    const messages = [
      {
        id: "m-live",
        session_id: "s1",
        role: "user",
        content: "hello",
        attachments: [],
        delivery: "immediate",
        created_at: "2025-12-15T00:00:02.000Z",
        turn_id: "t-live",
        order_seq: 1,
      },
    ];

    const filteredTurns = filterTurnsForQueuedMessages(turns as any, new Set(["m-queued"]));
    const out = buildWorkbenchThreadViewModelFromTurns(filteredTurns as any, messages as any, {}, [], new Map());
    expect(out.groups.length).toBe(1);
    expect(out.groups[0]?.header?.id).toBe("m-live");
  }, 10000);
});
