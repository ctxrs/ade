import { afterEach, describe, expect, it, vi } from "vitest";

import type { Message, Session, SessionEvent, SessionTurn, WorkspaceCatchupEvent } from "../api/client";

vi.mock("../api/client", () => {
  const idToString = (id: any): string => (typeof id === "string" ? id : id?.["0"]);
  return {
    idToString,
    getSessionHead: vi.fn(),
    getSessionHistory: vi.fn(),
    listTurnTools: vi.fn(async () => []),
    trackDiff: vi.fn(async () => ({ diff: "" })),
  };
});

vi.mock("./uiStateStore", () => ({
  loadSessionHeadV1: vi.fn(async () => null),
  saveSessionHeadV1: vi.fn(async () => {}),
  loadTrackDiffV1: vi.fn(async () => null),
  saveTrackDiffV1: vi.fn(async () => {}),
  deleteTrackDiffV1: vi.fn(async () => {}),
}));

import { getSessionHead } from "../api/client";

const mkSession = (sessionId: string, trackId: string): Session => ({
  id: { 0: sessionId },
  track_id: { 0: trackId },
  task_id: { 0: "task-1" },
  workspace_id: { 0: "ws-1" },
  worktree_id: { 0: "wt-1" },
  provider_id: "fake",
  model_id: "fake-model",
  agent_role: "assistant",
  status: "active",
});

async function waitForCondition(cond: () => boolean, timeoutMs = 1000) {
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
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-1";
    const trackId = "track-1";

    const headMessages: Message[] = [
      {
        id: { 0: "m1" },
        session_id: { 0: sessionId },
        role: "user",
        content: "queued",
        delivery: "queued",
        created_at: new Date().toISOString(),
      },
    ];

    (getSessionHead as any).mockResolvedValue({
      session: mkSession(sessionId, trackId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: headMessages,
      last_event_seq: 1,
      has_more_turns: false,
    });

    const sup = new SessionSupervisor();
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && !entry.loading);
    });

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.messages.length).toBe(1);
    expect(entry?.queue.length).toBe(1);
  });

  it("applies session head deltas from workspace stream", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-2";
    const trackId = "track-2";

    (getSessionHead as any).mockResolvedValue({
      session: mkSession(sessionId, trackId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 0,
      has_more_turns: false,
    });

    const sup = new SessionSupervisor();

    const listeners = new Set<(evt: WorkspaceCatchupEvent) => void>();
    const store = {
      subscribe: () => () => {},
      subscribeEvents: (listener: (evt: WorkspaceCatchupEvent) => void) => {
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

    sup.bindWorkspaceCatchupStore(store);
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && !entry.loading);
    });

    const now = new Date().toISOString();
    const event: SessionEvent = {
      seq: 2,
      id: { 0: "e1" },
      session_id: { 0: sessionId },
      turn_id: { 0: "turn-1" },
      event_type: "assistant_chunk",
      payload_json: { content_fragment: "hello" },
      created_at: now,
    };
    const message: Message = {
      id: { 0: "m2" },
      session_id: { 0: sessionId },
      turn_id: { 0: "turn-1" },
      role: "assistant",
      content: "hello",
      delivery: "immediate",
      created_at: now,
    };

    const deltaEvent: WorkspaceCatchupEvent = {
      type: "session_head_delta",
      workspace_id: { 0: "ws-1" },
      snapshot_rev: 1,
      delta: {
        session_id: { 0: sessionId },
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
