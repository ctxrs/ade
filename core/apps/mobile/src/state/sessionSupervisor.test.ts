/// <reference types="vitest" />
import { afterEach, describe, expect, it, vi } from "vitest";

import { buildDummySessionSnapshot } from "../test/fixtures/dummyWorkspace";

vi.mock("../api/client", () => ({
  getSessionSnapshot: vi.fn(),
  getSessionHistory: vi.fn(),
  listTurnTools: vi.fn(async () => []),
  getSessionDiff: vi.fn(async () => ({ diff: "" })),
  idToString: (value) => {
    if (typeof value === "string") return value;
    if (value && typeof value === "object" && "0" in value) {
      return String(value["0"]);
    }
    return value ? String(value) : "";
  },
}));

vi.mock("./uiStateStore", () => ({
  loadSessionHeadV1: vi.fn(async () => null),
  saveSessionHeadV1: vi.fn(async () => {}),
}));

import { getSessionSnapshot } from "../api/client";
import { loadSessionHeadV1 } from "./uiStateStore";
import { SessionSupervisor } from "./sessionSupervisor";

const conn = { baseUrl: "https://example.com", token: "test-token" };

const mkSession = (sessionId) => ({
  id: sessionId,
  task_id: "task-1",
  workspace_id: "ws-1",
  worktree_id: "wt-1",
  provider_id: "fake",
  model_id: "fake-model",
  title: "New Task",
  agent_role: "assistant",
  status: "active",
});

async function waitForCondition(cond, timeoutMs = 1000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (cond()) return;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("Timed out waiting for condition");
}

describe("SessionSupervisor", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it("hydrates session head and derives queue from queued messages", async () => {
    const sessionId = "session-1";

    const headMessages = [
      {
        id: "m1",
        session_id: sessionId,
        role: "user",
        content: "queued",
        delivery: "queued",
        created_at: new Date().toISOString(),
      },
    ];

    const head = {
      session: mkSession(sessionId),
      turns: [],
      events: [],
      messages: headMessages,
      last_event_seq: 1,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    };
    vi.mocked(getSessionSnapshot).mockResolvedValue({
      summary: {
        session: head.session,
        last_message_at: headMessages[0]?.created_at ?? null,
        last_message_preview: headMessages[0]?.content ?? null,
        last_event_seq: head.last_event_seq,
        activity: { is_working: false, last_turn_status: null },
        unread: false,
      },
      head,
    });

    const sup = new SessionSupervisor(conn);
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && !entry.loading);
    });

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.messages.length).toBe(1);
    expect(entry?.queue.length).toBe(1);
  });

  it("uses cached head to avoid refetch", async () => {
    const sessionId = "session-cache";

    const cachedSnapshot = buildDummySessionSnapshot({
      sessionId,
      taskId: "task-cache",
      workspaceId: "ws-cache",
      turnCount: 5,
    });

    vi.mocked(loadSessionHeadV1).mockResolvedValueOnce({
      v: 1,
      sessionId,
      head: cachedSnapshot.head,
      updatedAtMs: Date.now(),
    });

    const sup = new SessionSupervisor(conn);
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && entry.messages.length === cachedSnapshot.head.messages.length);
    });

    expect(getSessionSnapshot).not.toHaveBeenCalled();
    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.messages.length).toBe(cachedSnapshot.head.messages.length);
  });

  it("applies session head deltas from workspace stream", async () => {
    const sessionId = "session-2";

    const head = {
      session: mkSession(sessionId),
      turns: [],
      events: [],
      messages: [],
      last_event_seq: 0,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    };
    vi.mocked(getSessionSnapshot).mockResolvedValue({
      summary: {
        session: head.session,
        last_message_at: null,
        last_message_preview: null,
        last_event_seq: head.last_event_seq,
        activity: { is_working: false, last_turn_status: null },
        unread: false,
      },
      head,
    });

    const sup = new SessionSupervisor(conn);

    const listeners = new Set();
    const store = {
      subscribe: () => () => {},
      subscribeEvents: (listener) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      setSubscriptions: () => {},
      getSnapshot: () => ({
        workspaceId: "ws-1",
        initialized: true,
        connection: "connected",
        tasksById: {},
        activeIds: [],
        archivedIds: [],
        totalActive: 0,
        totalArchived: 0,
        fetchState: { active: "idle", archived: "idle" },
        hasMoreActive: false,
        hasMoreArchived: false,
        archivedLoaded: false,
      }),
    };

    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && !entry.loading);
    });

    const beforeEntry = sup.getSnapshot().sessions[sessionId];
    const now = new Date().toISOString();
    const event = {
      seq: 2,
      id: "e1",
      session_id: sessionId,
      turn_id: "turn-1",
      event_type: "assistant_chunk",
      payload_json: { content_fragment: "hello" },
      created_at: now,
    };
    const message = {
      id: "m2",
      session_id: sessionId,
      turn_id: "turn-1",
      role: "assistant",
      content: "hello",
      delivery: "immediate",
      created_at: now,
    };

    const deltaEvent = {
      type: "session_head_delta",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      delta: {
        session_id: sessionId,
        last_event_seq: 2,
        event,
        message,
      },
    };

    listeners.forEach((listener) => listener(deltaEvent));

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.events.length).toBe(1);
    expect(entry?.messages.length).toBe(1);
    expect(entry?.turns.length).toBe(1);
    expect(entry?.lastEventSeq).toBe(2);
  });
});
