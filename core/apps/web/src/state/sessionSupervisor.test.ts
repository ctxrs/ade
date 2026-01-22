import { afterEach, describe, expect, it, vi } from "vitest";
import { waitForCondition } from "../testUtils/waitForCondition";

import type { Message, Session, SessionEvent, SessionTurn, WorkspaceActiveSnapshotEvent } from "../api/client";
import type { WorkspaceActiveSnapshotEventSource } from "./workspaceActiveSnapshotStore";

vi.mock("../api/client", () => {
  const idToString = (id: any): string => (typeof id === "string" ? id : id?.["0"]);
  return {
    idToString,
    getProviderOptions: vi.fn(async () => undefined),
    getSessionHead: vi.fn(),
    getSessionSnapshot: vi.fn(),
    getSessionState: vi.fn(async () => ({ artifacts: [], git_status: null })),
    getSessionHistory: vi.fn(),
    listSessionArtifacts: vi.fn(async () => []),
    listTurnTools: vi.fn(async () => []),
  };
});

vi.mock("./uiStateStore", () => ({
  loadSessionAcpMetaV1: vi.fn(async () => null),
  loadSessionHeadV1: vi.fn(async () => null),
  loadSessionHistoryPageV1: vi.fn(async () => null),
  saveSessionAcpMetaV1: vi.fn(async () => {}),
  saveSessionHeadV1: vi.fn(async () => {}),
  saveSessionHistoryPageV1: vi.fn(async () => {}),
}));

import { getSessionHead, getSessionSnapshot } from "../api/client";
import { saveSessionHeadV1 } from "./uiStateStore";

const mkSession = (sessionId: string): Session => ({
  id: { 0: sessionId },
  task_id: { 0: "task-1" },
  workspace_id: { 0: "ws-1" },
  worktree_id: { 0: "wt-1" },
  provider_id: "fake",
  model_id: "fake-model",
  title: "New Task",
  agent_role: "assistant",
  status: "active",
});

describe("SessionSupervisor", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it("hydrates session head and derives queue from queued messages", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-1";
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

    (getSessionSnapshot as any).mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    (getSessionHead as any).mockResolvedValue({
      session: mkSession(sessionId),
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
    (getSessionSnapshot as any).mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    (getSessionHead as any).mockResolvedValue({
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 0,
      has_more_turns: false,
    });

    const sup = new SessionSupervisor();

    const listeners = new Set<(evt: WorkspaceActiveSnapshotEvent) => void>();
    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (listener: (evt: WorkspaceActiveSnapshotEvent) => void) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      setSubscriptions: (_subs) => {},
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      getSnapshot: () => ({
        workspaceId: "ws-1",
        initialized: true,
        connection: "connected" as const,
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

    const deltaEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_delta",
      workspace_id: { 0: "ws-1" },
      snapshot_rev: 1,
      delta: {
        session_id: { 0: sessionId },
        last_event_seq: 2,
        state_rev: 2,
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

  it("persists delta heads without partial events", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-3";
    (getSessionSnapshot as any).mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
      head: {
        session: mkSession(sessionId),
        turns: [] as SessionTurn[],
        events: [] as SessionEvent[],
        messages: [] as Message[],
        last_event_seq: 0,
        has_more_turns: false,
      },
    });

    const sup = new SessionSupervisor();

    const listeners = new Set<(evt: WorkspaceActiveSnapshotEvent) => void>();
    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (listener: (evt: WorkspaceActiveSnapshotEvent) => void) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      setSubscriptions: (_subs) => {},
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      getSnapshot: () => ({
        workspaceId: "ws-1",
        initialized: true,
        connection: "connected" as const,
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

    const now = new Date().toISOString();
    const event: SessionEvent = {
      seq: 3,
      id: { 0: "e2" },
      session_id: { 0: sessionId },
      turn_id: { 0: "turn-2" },
      event_type: "assistant_chunk",
      payload_json: { content_fragment: "partial" },
      created_at: now,
    };
    const message: Message = {
      id: { 0: "m3" },
      session_id: { 0: sessionId },
      turn_id: { 0: "turn-2" },
      role: "assistant",
      content: "final",
      delivery: "immediate",
      created_at: now,
    };

    const deltaEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_delta",
      workspace_id: { 0: "ws-1" },
      snapshot_rev: 2,
      delta: {
        session_id: { 0: sessionId },
        last_event_seq: 3,
        state_rev: 3,
        event,
        message,
      },
    };

    listeners.forEach((listener) => listener(deltaEvent));

    await waitForCondition(() => (saveSessionHeadV1 as any).mock.calls.length > 0);

    const calls = (saveSessionHeadV1 as any).mock.calls;
    const call = calls[calls.length - 1];
    const persisted = call?.[1];
    expect(persisted?.events?.length ?? 0).toBe(0);
    expect(persisted?.turns?.[0]?.assistant_partial ?? null).toBeNull();
  });

  it("ignores active task upserts without head data", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-4";
    const now = new Date().toISOString();
    const task = {
      id: { 0: "task-4" },
      workspace_id: { 0: "ws-1" },
      title: "Active task",
      status: "running",
      created_at: now,
      updated_at: now,
    };
    const session = mkSession(sessionId);
    const message = {
      id: { 0: "m4" },
      session_id: { 0: sessionId },
      task_id: { 0: "task-4" },
      role: "assistant",
      content: "hello",
      delivery: "immediate",
      created_at: now,
    };

    const summary = {
      task,
      primary_session: {
        session,
        last_message_at: now,
        last_message_preview: "hello",
        last_event_seq: 1,
        state_rev: 1,
        activity: { is_working: false },
        unread: false,
      },
      primary_session_head: null,
      sessions: [],
      sort_at: now,
    };

    const listeners = new Set<(evt: WorkspaceActiveSnapshotEvent) => void>();
    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (listener: (evt: WorkspaceActiveSnapshotEvent) => void) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      setSubscriptions: (_subs) => {},
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      getSnapshot: () => ({
        workspaceId: "ws-1",
        initialized: true,
        connection: "connected" as const,
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

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);

    const upsertEvent: WorkspaceActiveSnapshotEvent = {
      type: "active_task_upsert",
      workspace_id: { 0: "ws-1" },
      snapshot_rev: 1,
      task: summary,
    };
    listeners.forEach((listener) => listener(upsertEvent));

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry).toBeUndefined();
  });

  it("hydrates tool summaries from head and still loads full tools on demand", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");
    const { listTurnTools } = await import("../api/client");

    const sessionId = "session-3";
    const turnId = "turn-1";

    (getSessionSnapshot as any).mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    (getSessionHead as any).mockResolvedValue({
      session: mkSession(sessionId),
      turns: [
        {
          turn_id: { 0: turnId },
          session_id: { 0: sessionId },
          run_id: null,
          user_message_id: null,
          status: "completed",
          start_seq: 1,
          end_seq: 2,
          started_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          assistant_partial: null,
          thought_partial: null,
          metrics_json: null,
          tool_total: 1,
          tool_pending: 0,
          tool_running: 0,
          tool_completed: 1,
          tool_failed: 0,
        } as SessionTurn,
      ],
      tool_summaries: [
        {
          session_id: { 0: sessionId },
          tool_call_id: "tool-1",
          turn_id: { 0: turnId },
          tool_kind: "execute",
          title: "Run",
          status: "completed",
          input_preview: { command: "pwd" },
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        },
      ],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 0,
      has_more_turns: false,
    });

    const sup = new SessionSupervisor();
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && !entry.loading);
    });

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.turnToolsByTurnId[turnId]?.length).toBe(1);

    await sup.loadTurnTools(sessionId, turnId);
    expect(listTurnTools).toHaveBeenCalledWith(sessionId, turnId);
  });
});
