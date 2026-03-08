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
import type { SessionReplicaPatch } from "./sessionReplicaProtocol";
import type { WorkspaceActiveSnapshotEventSource, WorkspaceActiveSnapshotState } from "./workspaceActiveSnapshotStore";

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

import {
  getSessionHead,
  getSessionSnapshot,
  getSessionState,
  listSessionArtifacts,
  listSessionSubagentInvocations,
} from "../api/client";

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

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

type TestInternalEntry = {
  turnsHydrated: boolean;
  messages: Message[];
  queue: Message[];
  lastEventSeq?: number;
  loadState: "pending_hydration" | "live" | "recovering" | "fatal";
};

type SessionSupervisorInternals = {
  entries: Map<string, TestInternalEntry>;
  ensureEntry: (sessionId: string) => TestInternalEntry;
  handleReplicaPatches: (patches: SessionReplicaPatch[]) => void;
};

const asSupervisorInternals = (value: unknown): SessionSupervisorInternals => value as SessionSupervisorInternals;

const getSessionHeadMock = vi.mocked(getSessionHead);
const getSessionSnapshotMock = vi.mocked(getSessionSnapshot);
const getSessionStateMock = vi.mocked(getSessionState);
const listSessionArtifactsMock = vi.mocked(listSessionArtifacts);
const listSessionSubagentInvocationsMock = vi.mocked(listSessionSubagentInvocations);

const mkWorkspaceSnapshotState = (): WorkspaceActiveSnapshotState => ({
  workspaceId: "ws-1",
  initialized: true,
  connection: "connected" as const,
  tasksById: {},
  activeIds: [],
  archivedIds: [],
  totalActive: 0,
  totalArchived: 0,
  archivedRev: 0,
  worktreeVcsById: {},
  fetchState: { active: "idle" as const, archived: "idle" as const },
  hasMoreActive: false,
  hasMoreArchived: false,
  archivedLoaded: false,
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
        task_id: "task-1",
        role: "user",
        content: "queued",
        delivery: "queued",
        created_at: new Date().toISOString(),
      },
    ];

    getSessionSnapshotMock.mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    getSessionHeadMock.mockResolvedValue({
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: headMessages,
      last_event_seq: 1,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    });

    const sup = new SessionSupervisor();
    sup.openSession(sessionId, { mode: "archived" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.messages.length === 1);

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.messages.length).toBe(1);
    expect(entry?.queue.length).toBe(1);
  });

  it("hydrates protocol-derived slash command metadata from archived init events", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-slash-meta";
    const now = new Date().toISOString();
    getSessionSnapshotMock.mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    getSessionHeadMock.mockResolvedValue({
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [
        {
          seq: 1,
          id: "init-1",
          session_id: sessionId,
          event_type: "init",
          payload_json: {
            commands: [
              {
                name: "compact",
                description: "Summarize conversation to save context",
                argument_hint: "<focus>",
              },
            ],
            slash_commands: ["compact", "review"],
          },
          created_at: now,
        },
      ] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 1,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    });

    const sup = new SessionSupervisor();
    sup.openSession(sessionId, { mode: "archived" });

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Array.isArray(entry?.acpCommands) && Array.isArray(entry?.acpSlashCommands);
    });

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.acpCommands).toEqual([
      {
        name: "compact",
        description: "Summarize conversation to save context",
        argument_hint: "<focus>",
      },
    ]);
    expect(entry?.acpSlashCommands).toEqual(["compact", "review"]);
  });

  it("applies session head deltas from workspace stream", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-2";
    getSessionSnapshotMock.mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    getSessionHeadMock.mockResolvedValue({
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 0,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
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
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId] != null);
    const seedEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_seed",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      head: {
        session: mkSession(sessionId),
        turns: [] as SessionTurn[],
        events: [] as SessionEvent[],
        messages: [] as Message[],
        last_event_seq: 2,
        state_rev: 2,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
    };
    listeners.forEach((listener) => listener(seedEvent));

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
      task_id: "task-1",
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

  it("uses recovering->live transitions for active session gap recovery", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-recovery";
    const now = new Date().toISOString();
    const activeState = mkWorkspaceSnapshotState();
    activeState.activeIds = ["task-recovery"];
    activeState.tasksById = {
      "task-recovery": {
        id: "task-recovery",
        task: {
          id: "task-recovery",
          workspace_id: "ws-1",
          title: "Recovery",
          status: "running",
          primary_session_id: sessionId,
          created_at: now,
          updated_at: now,
          archived_at: null,
        },
        sessions: [{ session: mkSession(sessionId) }],
        primarySessionHead: null,
        sortAtMs: Date.parse(now),
      },
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
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => activeState,
    };

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => Boolean(sup.getSnapshot().sessions[sessionId]));
    expect(sup.getSnapshot().sessions[sessionId]?.loadState).toBe("pending_hydration");

    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    const gapEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_gap",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      session_id: sessionId,
      after_seq: 100,
    };
    listeners.forEach((listener) => listener(gapEvent));
    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.loadState === "recovering");
    expect(sup.getSnapshot().sessions[sessionId]?.error).toBeUndefined();

    const seedEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_seed",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      head: {
        session: mkSession(sessionId),
        turns: [] as SessionTurn[],
        events: [] as SessionEvent[],
        messages: [
          {
            id: "m-recovery",
            session_id: sessionId,
            task_id: "task-recovery",
            role: "assistant",
            content: "Recovered",
            delivery: "immediate",
            created_at: now,
          } as Message,
        ],
        last_event_seq: 1,
        state_rev: 1,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
    };
    listeners.forEach((listener) => listener(seedEvent));
    await waitForCondition(() => (sup.getSnapshot().sessions[sessionId]?.messages.length ?? 0) > 0);
    expect(sup.getSnapshot().sessions[sessionId]?.loadState).toBe("live");
    expect(sup.getSnapshot().sessions[sessionId]?.error).toBeUndefined();
    alertSpy.mockRestore();
  });

  it("preserves arrival order for transient assistant chunks (seq=null)", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-transient-order";
    getSessionSnapshotMock.mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    getSessionHeadMock.mockResolvedValue({
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 1,
      has_more_turns: false,
    } as SessionHeadSnapshot);

    const listeners = new Set<(evt: WorkspaceActiveSnapshotEvent) => void>();
    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (listener: (evt: WorkspaceActiveSnapshotEvent) => void) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId] != null);

    const now = Date.now();
    const sendChunk = (id: string, fragment: string, t: number) => {
      const ev = {
        seq: null,
        transient: true,
        id,
        session_id: sessionId,
        turn_id: "turn-1",
        event_type: "assistant_chunk",
        payload_json: { content_fragment: fragment },
        created_at: new Date(t).toISOString(),
      } as unknown as SessionEvent;
      const deltaEvent: WorkspaceActiveSnapshotEvent = {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 1,
        delta: {
          session_id: sessionId,
          last_event_seq: 1,
          state_rev: 1,
          event: ev,
        },
      };
      listeners.forEach((listener) => listener(deltaEvent));
    };

    sendChunk("e1", "a", now);
    sendChunk("e2", "b", now + 1);
    sendChunk("e3", "c", now + 2);

    const entry = sup.getSnapshot().sessions[sessionId];
    const fragments = (entry?.events ?? []).map((e) => String(asRecord(e.payload_json).content_fragment ?? ""));
    expect(fragments).toEqual(["a", "b", "c"]);
  });

  it("does not mark turn completed on assistant_complete before done", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-2b";
    getSessionSnapshotMock.mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    getSessionHeadMock.mockResolvedValue({
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 0,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
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
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId] != null);

    const now = new Date().toISOString();
    const turnId = "turn-1";
    const sendDelta = (seq: number, event_type: SessionEvent["event_type"], payload_json: unknown = {}) => {
      const event: SessionEvent = {
        seq,
        id: "e" + seq,
        session_id: sessionId,
        turn_id: turnId,
        event_type,
        payload_json: asRecord(payload_json),
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

  it("applies seeded+deltas without requiring /head hydration", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-3";
    getSessionSnapshotMock.mockResolvedValue({
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
        has_more_history: false,
        history_cursor: null,
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
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId] != null);

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
      task_id: "task-1",
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

    await waitForCondition(() => (sup.getSnapshot().sessions[sessionId]?.events.length ?? 0) > 0);
    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.messages.length).toBe(1);
    expect(entry?.events.length).toBe(1);
    expect(entry?.loadState).toBe("live");
    expect(getSessionHead).not.toHaveBeenCalled();
    expect(getSessionSnapshot).not.toHaveBeenCalled();
  });

  it("marks entry stale on session gap without forcing /head refetch", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-gap";
    const seededMessage: Message = {
      id: "msg-gap",
      session_id: sessionId,
      task_id: "task-1",
      turn_id: "turn-gap",
      role: "assistant",
      content: "hello",
      delivery: "immediate",
      created_at: new Date().toISOString(),
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
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId, { mode: "active" });
    const seedEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_seed",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      head: {
        session: mkSession(sessionId),
        turns: [] as SessionTurn[],
        events: [] as SessionEvent[],
        messages: [seededMessage],
        last_event_seq: 2,
        state_rev: 2,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
    };
    listeners.forEach((listener) => listener(seedEvent));

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.messages.length === 1);

    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    const priorSeq = sup.getSnapshot().sessions[sessionId]?.lastEventSeq;
    const gapEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_gap",
      workspace_id: "ws-1",
      snapshot_rev: 2,
      session_id: sessionId,
      after_seq: 5,
    };
    listeners.forEach((listener) => listener(gapEvent));

    const internalEntry = asSupervisorInternals(sup).entries.get(sessionId);
    expect(internalEntry).toBeDefined();
    if (!internalEntry) throw new Error("Expected internal entry to exist");
    expect(internalEntry.turnsHydrated).toBe(false);
    expect(internalEntry.messages.length).toBe(1);
    expect(internalEntry.lastEventSeq).toBe(priorSeq);
    expect(internalEntry.loadState).toBe("recovering");
    expect(getSessionHead).toHaveBeenCalledTimes(0);

    alertSpy.mockRestore();
  });

  it("preserves local-only queued messages across replica replace patches", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-replace-local-only";
    const sup = new SessionSupervisor();
    const internalEntry = asSupervisorInternals(sup).ensureEntry(sessionId);
    const now = Date.now();

    const serverMessage: Message = {
      id: "m-server",
      session_id: sessionId,
      task_id: "task-1",
      turn_id: "turn-1",
      role: "assistant",
      content: "old server copy",
      delivery: "immediate",
      created_at: new Date(now).toISOString(),
    };
    const queuedLocalMessage: Message = {
      id: "m-local",
      session_id: sessionId,
      task_id: "task-1",
      turn_id: "turn-2",
      role: "user",
      content: "queued local draft",
      delivery: "queued",
      created_at: new Date(now + 1).toISOString(),
    };

    internalEntry.messages = [serverMessage, queuedLocalMessage];
    internalEntry.queue = [queuedLocalMessage];

    asSupervisorInternals(sup).handleReplicaPatches([
      {
        op: "replace",
        sessionId,
        data: {
          messages: [
            {
              ...serverMessage,
              content: "fresh server copy",
            },
          ],
        },
      },
    ]);

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.messages.map((message) => message.id)).toEqual(["m-server", "m-local"]);
    expect(entry?.messages.find((message) => message.id === "m-server")?.content).toBe("fresh server copy");
    expect(entry?.messages.find((message) => message.id === "m-local")?.content).toBe("queued local draft");
    expect(entry?.queue.map((message) => message.id)).toEqual(["m-local"]);
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
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
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

    getSessionSnapshotMock.mockResolvedValue({
      summary: {
        session: mkSession(sessionId),
      },
    });
    getSessionHeadMock.mockResolvedValue({
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
      has_more_history: false,
      history_cursor: null,
    });

    const sup = new SessionSupervisor();
    sup.openSession(sessionId, { mode: "archived" });

    await waitForCondition(() => getSessionHeadMock.mock.calls.length > 0);
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
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && !entry.loading);
    });

    expect(getSessionHead).not.toHaveBeenCalled();
    expect(getSessionSnapshot).not.toHaveBeenCalled();
  });

  it("skips /head hydrate for active sessions when snapshot store is bound", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-active-no-head";
    getSessionHeadMock.mockRejectedValue(new Error("Load failed"));
    getSessionSnapshotMock.mockRejectedValue(new Error("Load failed"));

    const now = new Date().toISOString();
    const activeState = mkWorkspaceSnapshotState();
    activeState.activeIds = ["task-active"];
    activeState.tasksById = {
      "task-active": {
        id: "task-active",
        task: {
          id: "task-active",
          workspace_id: "ws-1",
          title: "Active",
          status: "running",
          primary_session_id: sessionId,
          created_at: now,
          updated_at: now,
          archived_at: null,
        },
        sessions: [
          {
            session: mkSession(sessionId),
          },
        ],
        primarySessionHead: null,
        sortAtMs: Date.parse(now),
      },
    };
    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (_listener: (evt: WorkspaceActiveSnapshotEvent) => void) => () => {},
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => activeState,
    };

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return Boolean(entry && !entry.loading);
    });

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.error).toBeUndefined();
    expect(getSessionHead).not.toHaveBeenCalled();
    expect(getSessionSnapshot).not.toHaveBeenCalled();
  });

  it("hydrates /head for archived sessions", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-archived-head";
    const now = new Date().toISOString();
    const headMessage: Message = {
      id: "m-archived",
      session_id: sessionId,
      task_id: "task-archived",
      role: "assistant",
      content: "archived",
      delivery: "immediate",
      created_at: now,
    };
    getSessionHeadMock.mockResolvedValue({
      session: { ...mkSession(sessionId), task_id: "task-archived", status: "completed" },
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [headMessage],
      last_event_seq: 1,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    });

    const archivedState = mkWorkspaceSnapshotState();
    archivedState.archivedIds = ["task-archived"];
    archivedState.tasksById = {
      "task-archived": {
        id: "task-archived",
        task: {
          id: "task-archived",
          workspace_id: "ws-1",
          title: "Archived",
          status: "completed",
          primary_session_id: sessionId,
          created_at: now,
          updated_at: now,
          archived_at: now,
        },
        sessions: [
          {
            session: { ...mkSession(sessionId), task_id: "task-archived", status: "completed" },
          },
        ],
        primarySessionHead: null,
        sortAtMs: Date.parse(now),
      },
    };
    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (_listener: (evt: WorkspaceActiveSnapshotEvent) => void) => () => {},
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => archivedState,
    };

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId);

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.messages.length === 1);

    expect(getSessionHead).toHaveBeenCalledTimes(1);
  });

  it("marks archived hydrate failures as fatal", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-archived-fatal";
    const now = new Date().toISOString();
    getSessionHeadMock.mockRejectedValue(new Error("Load failed"));

    const archivedState = mkWorkspaceSnapshotState();
    archivedState.archivedIds = ["task-archived-fatal"];
    archivedState.tasksById = {
      "task-archived-fatal": {
        id: "task-archived-fatal",
        task: {
          id: "task-archived-fatal",
          workspace_id: "ws-1",
          title: "Archived fatal",
          status: "completed",
          primary_session_id: sessionId,
          created_at: now,
          updated_at: now,
          archived_at: now,
        },
        sessions: [
          {
            session: { ...mkSession(sessionId), task_id: "task-archived-fatal", status: "completed" },
          },
        ],
        primarySessionHead: null,
        sortAtMs: Date.parse(now),
      },
    };

    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (_listener: (evt: WorkspaceActiveSnapshotEvent) => void) => () => {},
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => archivedState,
    };

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId);

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.loadState === "fatal");
    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.error).toContain("Load failed");
    expect(getSessionHead).toHaveBeenCalledTimes(1);
  });

  it("does not fallback to /head for unknown sessions and marks fatal after bounded resolution", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-unknown";
    const store: WorkspaceActiveSnapshotEventSource = {
      subscribe: () => () => {},
      subscribeEvents: (_listener: (evt: WorkspaceActiveSnapshotEvent) => void) => () => {},
      getSessionHeadSnapshot: () => null,
      getWorktreeRoot: () => null,
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessionIds: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    sup.bindWorkspaceActiveSnapshotStore(store);
    sup.openSession(sessionId);

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.loadState === "fatal");
    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.error).toContain("Session not found in workspace snapshot");
    expect(getSessionHead).not.toHaveBeenCalled();
  });

  it("records support load failures and clears them after a successful retry", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-support-load-errors";
    getSessionStateMock
      .mockRejectedValueOnce(new Error("daemon offline"))
      .mockResolvedValueOnce({ artifacts: [], git_status: null });
    listSessionArtifactsMock
      .mockRejectedValueOnce(new Error("artifacts endpoint unavailable"))
      .mockResolvedValueOnce([]);
    listSessionSubagentInvocationsMock
      .mockRejectedValueOnce(new Error("subagent query failed"))
      .mockResolvedValueOnce([]);

    const sup = new SessionSupervisor();
    sup.openSession(sessionId, { mode: "active" });

    sup.loadSessionState(sessionId);
    sup.loadArtifacts(sessionId);
    sup.loadSubagentInvocations(sessionId);

    await waitForCondition(() => {
      const loadErrors = sup.getSnapshot().sessions[sessionId]?.loadErrors;
      return Boolean(loadErrors?.state && loadErrors?.artifacts && loadErrors?.subagentInvocations);
    });

    const failedEntry = sup.getSnapshot().sessions[sessionId];
    expect(failedEntry?.stateLoading).toBe(false);
    expect(failedEntry?.artifactsLoading).toBe(false);
    expect(failedEntry?.subagentInvocationsLoading).toBe(false);
    expect(failedEntry?.loadErrors?.state).toBe("Failed to load session state: daemon offline");
    expect(failedEntry?.loadErrors?.artifacts).toBe(
      "Failed to load artifacts: artifacts endpoint unavailable",
    );
    expect(failedEntry?.loadErrors?.subagentInvocations).toBe(
      "Failed to load subagent invocations: subagent query failed",
    );

    sup.loadSessionState(sessionId, { force: true });
    sup.loadArtifacts(sessionId, { force: true });
    sup.loadSubagentInvocations(sessionId, { force: true });

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return (
        Boolean(entry?.stateLoaded) &&
        Boolean(entry?.loadErrors) &&
        !entry?.loadErrors?.state &&
        !entry?.loadErrors?.artifacts &&
        !entry?.loadErrors?.subagentInvocations &&
        entry?.artifactsLoading === false &&
        entry?.subagentInvocationsLoading === false
      );
    });

    const recoveredEntry = sup.getSnapshot().sessions[sessionId];
    expect(recoveredEntry?.artifacts).toEqual([]);
    expect(recoveredEntry?.subagentInvocations).toEqual([]);
  });
});
