import { afterEach, describe, expect, it, vi } from "vitest";
import { waitForCondition } from "../testUtils/waitForCondition";

import type {
  Message,
  Session,
  SessionEvent,
  SessionHeadSnapshot,
  SessionTurn,
  WorkspaceActiveSnapshotEvent,
} from "../api/client";
import type { WorkspaceActiveSnapshotEventSource } from "./workspaceActiveSnapshotStore";

vi.mock("../api/client", () => {
  const idToString = (id: string | null | undefined): string => {
    if (id === null || id === undefined) return "";
    if (typeof id !== "string") {
      throw new Error("Expected id to be a string");
    }
    return id;
  };
  return {
    authToken: vi.fn(() => null),
    idToString,
    getDaemonClientConfig: vi.fn(() => ({
      baseUrl: "",
      wsBaseUrl: "",
      authToken: null,
      runId: null,
    })),
    subscribeDaemonConfig: vi.fn(() => () => {}),
    getProviderOptions: vi.fn(async () => undefined),
    getSessionHead: vi.fn(),
    getSessionSnapshot: vi.fn(),
    getSessionState: vi.fn(async () => ({ artifacts: [], git_status: null })),
    getSessionHistory: vi.fn(),
    listSessionArtifacts: vi.fn(async () => []),
    listSessionSubagentInvocations: vi.fn(async () => []),
    listTurnTools: vi.fn(async () => []),
    resolveDaemonBaseUrl: vi.fn(() => ""),
  };
});

vi.mock("./uiStateStore", () => ({
  loadSessionAcpMetaV1: vi.fn(async () => null),
  loadSessionHeadV1: vi.fn(async () => null),
  loadSessionHistoryPageV1: vi.fn(async () => null),
  loadTaskThoughtsV1: vi.fn(async () => null),
  saveTaskThoughtsV1: vi.fn(async () => {}),
  clearTaskThoughtsV1: vi.fn(async () => {}),
  saveSessionAcpMetaV1: vi.fn(async () => {}),
  saveSessionHeadV1: vi.fn(async () => {}),
  saveSessionHistoryPageV1: vi.fn(async () => {}),
}));

import { getSessionHead, getSessionSnapshot } from "../api/client";
import { saveSessionHeadV1 } from "./uiStateStore";

const mkSession = (sessionId: string): Session => ({
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
        id: "m1",
        session_id: sessionId,
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

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.messages.length === 1);

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
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      setSubscribedSessionIds: () => {},
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

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.session != null);

    const now = new Date().toISOString();
    const event: SessionEvent = {
      seq: 2,
      id: "e1",
      session_id: sessionId,
      turn_id: "turn-1",
      event_type: "assistant_chunk",
      payload_json: { content_fragment: "hello" },
      created_at: now,
    };
    const message: Message = {
      id: "m2",
      session_id: sessionId,
      turn_id: "turn-1",
      role: "assistant",
      content: "hello",
      delivery: "immediate",
      created_at: now,
    };

    const deltaEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_delta",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      delta: {
        session_id: sessionId,
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

  it("does not mark turn completed on assistant_complete before done", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-2b";
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
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      setSubscribedSessionIds: () => {},
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

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.session != null);

    const now = new Date().toISOString();
    const turnId = "turn-1";
    const sendDelta = (seq: number, event_type: SessionEvent["event_type"], payload_json: any = {}) => {
      const event: SessionEvent = {
        seq,
        id: "e" + seq,
        session_id: sessionId,
        turn_id: turnId,
        event_type,
        payload_json,
        created_at: now,
      };
      const deltaEvent: WorkspaceActiveSnapshotEvent = {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 1,
        delta: {
          session_id: sessionId,
          last_event_seq: seq,
          state_rev: seq,
          event,
        },
      };
      listeners.forEach((listener) => listener(deltaEvent));
    };

    sendDelta(1, "turn_started");
    expect(sup.getSnapshot().sessions[sessionId]?.turns[0]?.status).toBe("running");

    sendDelta(2, "assistant_complete", { full_content: "hello" });
    expect(sup.getSnapshot().sessions[sessionId]?.turns[0]?.status).toBe("running");

    sendDelta(3, "done");
    expect(sup.getSnapshot().sessions[sessionId]?.turns[0]?.status).toBe("completed");
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
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      setSubscribedSessionIds: () => {},
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

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.session != null);

    const now = new Date().toISOString();
    const event: SessionEvent = {
      seq: 3,
      id: "e2",
      session_id: sessionId,
      turn_id: "turn-2",
      event_type: "assistant_chunk",
      payload_json: { content_fragment: "partial" },
      created_at: now,
    };
    const message: Message = {
      id: "m3",
      session_id: sessionId,
      turn_id: "turn-2",
      role: "assistant",
      content: "final",
      delivery: "immediate",
      created_at: now,
    };

    const deltaEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_delta",
      workspace_id: "ws-1",
      snapshot_rev: 2,
      delta: {
        session_id: sessionId,
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

  it("resets and reloads on session gap", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-gap";
    const headWithMessage = {
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [
        {
          id: "msg-gap",
          session_id: sessionId,
          turn_id: "turn-gap",
          role: "assistant",
          content: "hello",
          delivery: "immediate",
          created_at: new Date().toISOString(),
        } as Message,
      ],
      last_event_seq: 2,
      has_more_turns: false,
    };
    (getSessionHead as any).mockResolvedValue(headWithMessage);

    const listeners = new Set<(evt: WorkspaceActiveSnapshotEvent) => void>();
    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (listener: (evt: WorkspaceActiveSnapshotEvent) => void) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      setSubscribedSessionIds: () => {},
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
    sup.openSession(sessionId);

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.messages.length === 1);

    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    const gapEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_gap",
      workspace_id: "ws-1",
      snapshot_rev: 2,
      session_id: sessionId,
      after_seq: 5,
    };
    listeners.forEach((listener) => listener(gapEvent));

    const internalEntry = (sup as any).entries.get(sessionId);
    expect(internalEntry.turnsHydrated).toBe(false);
    expect(internalEntry.messages.length).toBe(1);
    expect(internalEntry.lastEventSeq).toBe(5);

    alertSpy.mockRestore();
  });

  it("ignores active task upserts without head data", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-4";
    const now = new Date().toISOString();
    const task = {
      id: "task-4",
      workspace_id: "ws-1",
      title: "Active task",
      status: "running",
      created_at: now,
      updated_at: now,
    };
    const session = mkSession(sessionId);
    const message = {
      id: "m4",
      session_id: sessionId,
      task_id: "task-4",
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
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      setSubscribedSessionIds: () => {},
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
      workspace_id: "ws-1",
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
          turn_id: turnId,
          session_id: sessionId,
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
          session_id: sessionId,
          tool_call_id: "tool-1",
          turn_id: turnId,
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

    await waitForCondition(() => (getSessionHead as any).mock.calls.length > 0);
    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.turnToolsByTurnId[turnId]?.length === 1);

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.turnToolsByTurnId[turnId]?.length).toBe(1);

    await sup.loadTurnTools(sessionId, turnId);
    expect(listTurnTools).toHaveBeenCalledWith(sessionId, turnId);
  });

  it("uses active snapshot heads to avoid HTTP on open", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-5";
    const head: SessionHeadSnapshot = {
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 0,
      state_rev: 0,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    };

    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (_listener: (evt: WorkspaceActiveSnapshotEvent) => void) => () => {},
      getSessionHeadSnapshot: (id: string) => (id === sessionId ? head : null),
      getWorktreeRoot: () => null,
      setSubscribedSessionIds: () => {},
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
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && !entry.loading);
    });

    expect(getSessionHead).not.toHaveBeenCalled();
    expect(getSessionSnapshot).not.toHaveBeenCalled();
  });
});
