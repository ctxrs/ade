import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
import type { SessionSubscriptionCursor } from "./sessionSubscription";
import type { WorkspaceActiveSnapshotEventSource, WorkspaceActiveSnapshotState } from "./workspaceActiveSnapshotStore";
import { loadSessionHistoryPageV1, loadTaskThoughtsV1, saveSessionHistoryPageV1, saveTaskThoughtsV1 } from "./uiStateStore";

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
    getDaemonConnection: vi.fn(() => ({
      baseUrl: "http://daemon.test",
      wsBaseUrl: "ws://daemon.test",
      authToken: null,
      runId: null,
      source: "test",
      targetScope: { kind: "browser", baseUrl: "http://daemon.test" },
    })),
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
  clearSessionHeadV1: vi.fn(async () => {}),
  clearSessionHistoryPagesV1: vi.fn(async () => {}),
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
  getSessionHistory,
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

const mkTurn = ({
  sessionId,
  turnId,
  status,
  startSeq,
}: {
  sessionId: string;
  turnId: string;
  status: SessionTurn["status"];
  startSeq: number;
}): SessionTurn => {
  const startedAt = new Date(Date.UTC(2026, 2, 9, 0, 0, startSeq)).toISOString();
  return {
    turn_id: turnId,
    session_id: sessionId,
    run_id: null,
    user_message_id: `user-${turnId}`,
    status,
    start_seq: startSeq,
    end_seq: status === "completed" ? startSeq + 1 : null,
    started_at: startedAt,
    updated_at: startedAt,
    assistant_partial: "",
    thought_partial: "",
    metrics_json: null,
    tool_total: 0,
    tool_pending: 0,
    tool_running: 0,
    tool_completed: 0,
    tool_failed: 0,
  };
};

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

type TestInternalEntry = {
  turnsHydrated: boolean;
  turns: SessionTurn[];
  turnsRev: number;
  freshness?: "bootstrap" | "authoritative" | "recovering";
  messages: Message[];
  messagesRev: number;
  events: SessionEvent[];
  eventsRev: number;
  queue: Message[];
  hasMoreTurns: boolean;
  lastEventSeq?: number;
  oldestTurnSeq?: number;
  stateLoaded?: boolean;
  stateRev?: number;
  stateAppliedRev?: number;
  subagentInvocationsLoaded?: boolean;
  subagentInvocationsAppliedRev?: number;
  loadState: "pending_hydration" | "live" | "recovering" | "fatal";
};

type SessionSupervisorInternals = {
  entries: Map<string, TestInternalEntry>;
  stateCacheBySessionId: Map<string, { state: { git_status: unknown }; stateRev?: number }>;
  ensureEntry: (sessionId: string) => TestInternalEntry;
  handleReplicaPatches: (patches: SessionReplicaPatch[]) => void;
};

const asSupervisorInternals = (value: unknown): SessionSupervisorInternals => value as SessionSupervisorInternals;

const getSessionHistoryMock = vi.mocked(getSessionHistory);
const getSessionHeadMock = vi.mocked(getSessionHead);
const getSessionSnapshotMock = vi.mocked(getSessionSnapshot);
const getSessionStateMock = vi.mocked(getSessionState);
const listSessionArtifactsMock = vi.mocked(listSessionArtifacts);
const listSessionSubagentInvocationsMock = vi.mocked(listSessionSubagentInvocations);
const loadSessionHistoryPageV1Mock = vi.mocked(loadSessionHistoryPageV1);
const saveSessionHistoryPageV1Mock = vi.mocked(saveSessionHistoryPageV1);
const loadTaskThoughtsV1Mock = vi.mocked(loadTaskThoughtsV1);
const saveTaskThoughtsV1Mock = vi.mocked(saveTaskThoughtsV1);

beforeEach(() => {
  getSessionHeadMock.mockReset();
  getSessionHeadMock.mockImplementation(async () => {
    throw new Error("getSessionHead must be mocked per test");
  });
  getSessionSnapshotMock.mockReset();
  getSessionSnapshotMock.mockImplementation(async () => {
    throw new Error("getSessionSnapshot must be mocked per test");
  });
  getSessionStateMock.mockReset();
  getSessionStateMock.mockResolvedValue({ artifacts: [], git_status: null });
  getSessionHistoryMock.mockReset();
  listSessionArtifactsMock.mockReset();
  listSessionArtifactsMock.mockResolvedValue([]);
  listSessionSubagentInvocationsMock.mockReset();
  listSessionSubagentInvocationsMock.mockResolvedValue([]);
  loadSessionHistoryPageV1Mock.mockReset();
  loadSessionHistoryPageV1Mock.mockResolvedValue(null);
  saveSessionHistoryPageV1Mock.mockReset();
  saveSessionHistoryPageV1Mock.mockResolvedValue();
  loadTaskThoughtsV1Mock.mockReset();
  loadTaskThoughtsV1Mock.mockResolvedValue(null);
  saveTaskThoughtsV1Mock.mockReset();
  saveTaskThoughtsV1Mock.mockResolvedValue();
});

const mkWorkspaceSnapshotState = (): WorkspaceActiveSnapshotState => ({
  workspaceId: "ws-1",
  initialized: true,
  liveSnapshotApplied: true,
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

const mkWorkspaceTaskSummary = ({
  taskId,
  primarySessionId,
  sessionIds,
}: {
  taskId: string;
  primarySessionId: string;
  sessionIds: string[];
}) => ({
  id: taskId,
  task: {
    id: taskId,
    workspace_id: "ws-1",
    title: `Task ${taskId}`,
    status: "running",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    last_activity_at: new Date().toISOString(),
    archived_at: null,
    primary_session_id: primarySessionId,
  },
  sessions: sessionIds.map((sessionId) => ({
    session: mkSession(sessionId),
    last_message_at: null,
    last_message_preview: null,
    last_event_seq: null,
    state_rev: undefined,
    activity: { is_working: false, last_turn_status: null },
    unread: false,
  })),
  primarySessionId,
  primarySessionHead: null,
  sort_at: new Date().toISOString(),
  sortAtMs: Date.now(),
});

const attachWorkspaceStore = (
  sup: {
    setSubscribedSessionIdsSink: (sink: ((sessions: SessionSubscriptionCursor[]) => void) | null) => void;
    setWorkspaceSnapshotState: (state: WorkspaceActiveSnapshotState | null) => void;
    setWorkspaceSessionHeads: (heads: Record<string, SessionHeadSnapshot>) => void;
    handleWorkspaceEvent: (evt: WorkspaceActiveSnapshotEvent) => void;
  },
  store: WorkspaceActiveSnapshotEventSource & { getSessionHeadsSnapshot?: () => Record<string, SessionHeadSnapshot> },
) => {
  const sync = () => {
    sup.setWorkspaceSnapshotState(store.getSnapshot());
    sup.setWorkspaceSessionHeads(store.getSessionHeadsSnapshot?.() ?? {});
  };
  sup.setSubscribedSessionIdsSink((sessions) => store.setSubscribedSessions?.(sessions));
  sync();
  const unsubState = store.subscribe(sync);
  const unsubEvents = store.subscribeEvents((evt) => sup.handleWorkspaceEvent(evt));
  return () => {
    unsubEvents();
    unsubState();
    sup.setSubscribedSessionIdsSink(null);
    sup.setWorkspaceSessionHeads({});
    sup.setWorkspaceSnapshotState(null);
  };
};

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

  it("bumps messagesRev when streamed queue events flip delivery in place", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-queue-rev";
    const createdAt = new Date().toISOString();
    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);
    const entry = internals.ensureEntry(sessionId);

    entry.messages = [
      {
        id: "message-1",
        session_id: sessionId,
        task_id: "task-1",
        role: "user",
        content: "queue me",
        attachments: [],
        delivery: "immediate",
        created_at: createdAt,
        turn_id: "turn-1",
        order_seq: 1,
      } as Message,
    ];
    entry.queue = [];
    const beforeMessagesRev = entry.messagesRev;

    internals.handleReplicaPatches([
      {
        op: "append",
        sessionId,
        data: {
          events: [
            {
              seq: 1,
              id: "event-queue-added",
              session_id: sessionId,
              run_id: "run-1",
              turn_id: "turn-1",
              event_type: "message_queue_added",
              payload_json: { message_id: "message-1" },
              created_at: createdAt,
            },
          ],
        },
      },
    ]);

    expect(entry.messages).toHaveLength(1);
    expect(entry.messages[0]?.delivery).toBe("queued");
    expect(entry.queue.map((message) => String(message.id))).toEqual(["message-1"]);
    expect(entry.messagesRev).toBeGreaterThan(beforeMessagesRev);
  });

  it("bumps turnsRev when streamed turn events mutate an existing turn in place", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-turn-rev";
    const createdAt = new Date().toISOString();
    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);
    const entry = internals.ensureEntry(sessionId);

    entry.turns = [
      {
        turn_id: "turn-1",
        session_id: sessionId,
        run_id: "run-1",
        user_message_id: "message-1",
        status: "running",
        start_seq: 1,
        end_seq: null,
        started_at: createdAt,
        updated_at: createdAt,
        assistant_partial: "",
        thought_partial: "",
        metrics_json: null,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
      } as SessionTurn,
    ];
    const beforeTurnsRev = entry.turnsRev;

    internals.handleReplicaPatches([
      {
        op: "append",
        sessionId,
        data: {
          events: [
            {
              seq: 2,
              id: "event-turn-done",
              session_id: sessionId,
              run_id: "run-1",
              turn_id: "turn-1",
              event_type: "done",
              payload_json: {},
              created_at: createdAt,
            },
          ],
        },
      },
    ]);

    expect(entry.turns).toHaveLength(1);
    expect(entry.turns[0]?.status).toBe("completed");
    expect(entry.turnsRev).toBeGreaterThan(beforeTurnsRev);
  });

  it("re-emits subscribed session ids when active-task membership changes under an open session", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-reemit-active-membership";
    const sink = vi.fn();
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
      has_more_history: false,
      history_cursor: null,
    });

    const sup = new SessionSupervisor();
    sup.setSubscribedSessionIdsSink(sink);
    sink.mockClear();

    sup.openSession(sessionId, { mode: "active" });

    expect(sink).toHaveBeenCalledWith([{ sessionId, afterSeq: null }]);
    sink.mockClear();

    sup.setActiveTaskSessionIds([sessionId]);

    expect(sink).toHaveBeenCalledWith([{ sessionId, afterSeq: null }]);
  });

  it("re-emits subscribed session ids when workspace active-primary membership flips under an identical plan", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sink = vi.fn();
    const sup = new SessionSupervisor();
    sup.setSubscribedSessionIdsSink(sink);
    sink.mockClear();

    sup.setWarmSessionIds(["session-1", "session-2"]);
    expect(sink).toHaveBeenCalledWith([
      { sessionId: "session-1", afterSeq: null },
      { sessionId: "session-2", afterSeq: null },
    ]);
    sink.mockClear();

    const stateWithPrimaryOne: WorkspaceActiveSnapshotState = {
      ...mkWorkspaceSnapshotState(),
      activeIds: ["task-1"],
      tasksById: {
        "task-1": mkWorkspaceTaskSummary({
          taskId: "task-1",
          primarySessionId: "session-1",
          sessionIds: ["session-1", "session-2"],
        }),
      },
      totalActive: 1,
    };
    sup.setWorkspaceSnapshotState(stateWithPrimaryOne);
    expect(sink).toHaveBeenCalledWith([
      { sessionId: "session-1", afterSeq: null },
      { sessionId: "session-2", afterSeq: null },
    ]);
    sink.mockClear();

    const stateWithPrimaryTwo: WorkspaceActiveSnapshotState = {
      ...stateWithPrimaryOne,
      tasksById: {
        "task-1": mkWorkspaceTaskSummary({
          taskId: "task-1",
          primarySessionId: "session-2",
          sessionIds: ["session-1", "session-2"],
        }),
      },
    };
    sup.setWorkspaceSnapshotState(stateWithPrimaryTwo);

    expect(sink).toHaveBeenCalledWith([
      { sessionId: "session-1", afterSeq: null },
      { sessionId: "session-2", afterSeq: null },
    ]);
  });

  it("drops replay cursors for subscribed sessions that enter recovering on session_gap", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-gap-cursor";
    const sink = vi.fn();
    const head: SessionHeadSnapshot = {
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 7,
      state_rev: 7,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    };
    const activeState: WorkspaceActiveSnapshotState = {
      ...mkWorkspaceSnapshotState(),
      activeIds: ["task-gap-cursor"],
      tasksById: {
        "task-gap-cursor": {
          ...mkWorkspaceTaskSummary({
            taskId: "task-gap-cursor",
            primarySessionId: sessionId,
            sessionIds: [sessionId],
          }),
          primarySessionHead: head,
        },
      },
      totalActive: 1,
    };

    const sup = new SessionSupervisor();
    sup.setSubscribedSessionIdsSink(sink);
    sup.setWorkspaceSessionHeads({ [sessionId]: head });
    sup.setWorkspaceSnapshotState(activeState);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() =>
      sink.mock.calls.some(
        (call) => call[0]?.[0]?.sessionId === sessionId && call[0]?.[0]?.afterSeq === 7,
      ),
    );
    sink.mockClear();

    sup.handleWorkspaceEvent({
      type: "session_gap",
      workspace_id: "ws-1",
      snapshot_rev: 8,
      session_id: sessionId,
      after_seq: 7,
    });

    expect(sink).toHaveBeenCalledWith([{ sessionId, afterSeq: null }]);
    expect(sup.getSnapshot().sessions[sessionId]?.freshness).toBe("recovering");
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

  it("pushes ACP model catalogs back into the shared provider bootstrap store", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");
    const providersBootstrapStore = await import("./providersBootstrapStore");

    const sessionId = "session-shared-provider-models";
    const workspaceId = "ws-shared-provider-models";
    const now = new Date().toISOString();

    providersBootstrapStore.updateProvidersBootstrap(workspaceId, (current) => ({
      ...current,
      provider_options: {
        ...current.provider_options,
        codex: {
          provider_id: "codex",
          workspace_id: workspaceId,
          supports_load: false,
          auth_required: false,
          probed_at: now,
          models: {
            models: [{ id: "gpt-5.4" }, { id: "gpt-5.3-codex" }],
            current_model_id: "gpt-5.4",
            meta: {
              source_kind: "subscription",
              catalog_source: "runtime_probe_live",
              refresh_pending: false,
            },
          },
        },
      },
    }));

    const session = {
      ...mkSession(sessionId),
      workspace_id: workspaceId,
      provider_id: "codex",
      model_id: "gpt-5.3-codex",
    };
    getSessionSnapshotMock.mockResolvedValue({
      summary: {
        session,
      },
    });
    getSessionHeadMock.mockResolvedValue({
      session,
      turns: [] as SessionTurn[],
      events: [
        {
          seq: 1,
          id: "init-models-1",
          session_id: sessionId,
          event_type: "init",
          payload_json: {
            current_model_id: "gpt-5.3-codex",
            models: {
              models: [{ id: "gpt-5.4" }, { id: "gpt-5.3-codex" }, { id: "gpt-5.3-codex-spark" }],
              current_model_id: "gpt-5.3-codex",
            },
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
      const models = providersBootstrapStore
        .getProvidersBootstrapSnapshot(workspaceId)
        .provider_options
        .codex
        ?.models as Record<string, unknown> | undefined;
      const list = Array.isArray(models?.models) ? models.models : [];
      return list.some((entry) => asRecord(entry).id === "gpt-5.3-codex-spark");
    });

    const models = providersBootstrapStore
      .getProvidersBootstrapSnapshot(workspaceId)
      .provider_options
      .codex
      ?.models as Record<string, unknown> | undefined;
    const list = Array.isArray(models?.models) ? models.models : [];
    const meta = asRecord(models?.meta);

    expect(list.map((entry) => String(asRecord(entry).id))).toContain("gpt-5.3-codex-spark");
    expect(models?.current_model_id).toBe("gpt-5.4");
    expect(meta.catalog_source).toBe("session_acp_live");
    expect(meta.refresh_pending).toBe(false);
  });

  it("updates the live session current model from init events without rewriting shared provider defaults", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");
    const providersBootstrapStore = await import("./providersBootstrapStore");

    const sessionId = "session-shared-provider-current-model";
    const workspaceId = "ws-shared-provider-current-model";
    const now = new Date().toISOString();

    providersBootstrapStore.updateProvidersBootstrap(workspaceId, (current) => ({
      ...current,
      provider_options: {
        ...current.provider_options,
        codex: {
          provider_id: "codex",
          workspace_id: workspaceId,
          supports_load: false,
          auth_required: false,
          probed_at: now,
          models: {
            models: [{ id: "gpt-5.4" }, { id: "gpt-5.3-codex" }, { id: "gpt-5.3-codex-spark" }],
            current_model_id: "gpt-5.4",
            meta: {
              source_kind: "subscription",
              catalog_source: "runtime_probe_live",
              refresh_pending: false,
            },
          },
        },
      },
    }));

    const session = {
      ...mkSession(sessionId),
      workspace_id: workspaceId,
      provider_id: "codex",
      model_id: "gpt-5.3-codex",
    };
    getSessionSnapshotMock.mockResolvedValue({
      summary: {
        session,
      },
    });
    getSessionHeadMock.mockResolvedValue({
      session,
      turns: [] as SessionTurn[],
      events: [
        {
          seq: 1,
          id: "init-models-1",
          session_id: sessionId,
          event_type: "init",
          payload_json: {
            current_model_id: "gpt-5.3-codex",
            models: {
              models: [{ id: "gpt-5.4" }, { id: "gpt-5.3-codex" }, { id: "gpt-5.3-codex-spark" }],
              current_model_id: "gpt-5.3-codex",
            },
          },
          created_at: now,
        },
        {
          seq: 2,
          id: "init-models-2",
          session_id: sessionId,
          event_type: "init",
          payload_json: {
            current_model_id: "gpt-5.3-codex-spark",
          },
          created_at: now,
        },
      ] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 2,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    });

    const sup = new SessionSupervisor();
    sup.openSession(sessionId, { mode: "archived" });

    await waitForCondition(
      () => sup.getSnapshot().sessions[sessionId]?.acpCurrentModelId === "gpt-5.3-codex-spark",
    );

    expect(sup.getSnapshot().sessions[sessionId]?.acpCurrentModelId).toBe("gpt-5.3-codex-spark");
    const models = providersBootstrapStore
      .getProvidersBootstrapSnapshot(workspaceId)
      .provider_options
      .codex
      ?.models as Record<string, unknown> | undefined;
    expect(models?.current_model_id).toBe("gpt-5.4");
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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    attachWorkspaceStore(sup, store);
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
      setSubscribedSessions: () => {},
      getSnapshot: () => activeState,
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => Boolean(sup.getSnapshot().sessions[sessionId]));
    expect(sup.getSnapshot().sessions[sessionId]?.loadState).toBe("pending_hydration");
    await waitForCondition(() => getSessionHeadMock.mock.calls.length === 1);
    getSessionHeadMock.mockClear();
    getSessionHeadMock.mockImplementation(() => new Promise(() => {}));

    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    const gapEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_gap",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      session_id: sessionId,
      after_seq: 100,
    };
    listeners.forEach((listener) => listener(gapEvent));
    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return entry?.freshness === "recovering" && entry.loadState === "pending_hydration";
    });
    expect(sup.getSnapshot().sessions[sessionId]?.error).toBeUndefined();
    expect(getSessionHead).toHaveBeenCalledTimes(1);

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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    attachWorkspaceStore(sup, store);
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

  it("synthesizes assistant messages from assistant_message_inserted deltas", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-inserted-message";
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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    attachWorkspaceStore(sup, store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId] != null);

    const now = new Date().toISOString();
    const turnId = "turn-1";
    const sendDelta = (seq: number, event_type: SessionEvent["event_type"], payload_json: unknown = {}) => {
      const event: SessionEvent = {
        seq,
        id: `e${seq}`,
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

    sendDelta(1, "turn_started", { message_id: "user-msg-1" });
    sendDelta(2, "assistant_complete", {
      full_content: "Hello. What do you want to work on?",
      message_id: "provider-msg-1",
      order_seq: 2,
    });
    sendDelta(3, "assistant_message_inserted", {
      message_id: "assistant-msg-1",
      content: "Hello. What do you want to work on?",
      delivery: "immediate",
      order_seq: 2,
      turn_sequence: 1,
      provider_message_id: "provider-msg-1",
    });

    await waitForCondition(
      () => (sup.getSnapshot().sessions[sessionId]?.messages.length ?? 0) === 1,
    );

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.messages[0]).toMatchObject({
      id: "assistant-msg-1",
      session_id: sessionId,
      turn_id: turnId,
      turn_sequence: 1,
      order_seq: 2,
      role: "assistant",
      content: "Hello. What do you want to work on?",
      delivery: "immediate",
    });
    expect(entry?.turns[0]?.assistant_partial ?? "").toBe("");
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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    attachWorkspaceStore(sup, store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId] != null);
    await waitForCondition(() => getSessionHeadMock.mock.calls.length === 1);
    getSessionHeadMock.mockClear();

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

  it("marks entry stale on session gap while forcing a new /head hydrate", async () => {
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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
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
    getSessionHeadMock.mockClear();
    getSessionHeadMock.mockImplementation(() => new Promise(() => {}));

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
    expect(internalEntry.loadState).toBe("pending_hydration");
    expect(internalEntry.freshness).toBe("recovering");
    expect(getSessionHead).toHaveBeenCalledTimes(1);

    alertSpy.mockRestore();
  });

  it("recovers active session gap from stream seed while a forced /head hydrate is pending and preserves local queued drafts", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-gap-queue-recovery";
    const now = new Date().toISOString();
    const activeState = mkWorkspaceSnapshotState();
    activeState.activeIds = ["task-gap-queue"];
    activeState.tasksById = {
      "task-gap-queue": {
        id: "task-gap-queue",
        task: {
          id: "task-gap-queue",
          workspace_id: "ws-1",
          title: "Gap queue recovery",
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
      setSubscribedSessions: () => {},
      getSnapshot: () => activeState,
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
    sup.openSession(sessionId, { mode: "active" });

    const serverMessage: Message = {
      id: "m-server",
      session_id: sessionId,
      task_id: "task-gap-queue",
      turn_id: "turn-server",
      role: "assistant",
      content: "baseline server copy",
      delivery: "immediate",
      created_at: now,
    };
    listeners.forEach((listener) =>
      listener({
        type: "session_head_seed",
        workspace_id: "ws-1",
        snapshot_rev: 1,
        head: {
          session: mkSession(sessionId),
          turns: [] as SessionTurn[],
          events: [] as SessionEvent[],
          messages: [serverMessage],
          last_event_seq: 1,
          state_rev: 1,
          has_more_turns: false,
          has_more_history: false,
          history_cursor: null,
        },
      }),
    );

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.loadState === "live");
    getSessionHeadMock.mockClear();
    getSessionHeadMock.mockImplementation(() => new Promise(() => {}));

    const internals = asSupervisorInternals(sup);
    const internalEntry = internals.entries.get(sessionId);
    expect(internalEntry).toBeDefined();
    if (!internalEntry) throw new Error("Expected internal entry to exist");

    const queuedLocalMessage: Message = {
      id: "m-local",
      session_id: sessionId,
      task_id: "task-gap-queue",
      turn_id: "turn-local",
      role: "user",
      content: "queued local draft",
      delivery: "queued",
      created_at: new Date(Date.parse(now) + 1).toISOString(),
    };
    internalEntry.messages = [serverMessage, queuedLocalMessage];
    internalEntry.queue = [queuedLocalMessage];

    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    listeners.forEach((listener) =>
      listener({
        type: "session_gap",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        session_id: sessionId,
        after_seq: 50,
      }),
    );
    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return entry?.freshness === "recovering" && entry.loadState === "pending_hydration";
    });
    expect(getSessionHead).toHaveBeenCalledTimes(1);

    listeners.forEach((listener) =>
      listener({
        type: "session_head_seed",
        workspace_id: "ws-1",
        snapshot_rev: 3,
        head: {
          session: mkSession(sessionId),
          turns: [] as SessionTurn[],
          events: [] as SessionEvent[],
          messages: [
            {
              ...serverMessage,
              content: "fresh server copy",
            },
          ],
          last_event_seq: 2,
          state_rev: 2,
          has_more_turns: false,
          has_more_history: false,
          history_cursor: null,
        },
      }),
    );

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return (
        entry?.loadState === "live" &&
        entry.messages.some((message) => message.id === "m-server" && message.content === "fresh server copy") &&
        entry.messages.some((message) => message.id === "m-local" && message.content === "queued local draft") &&
        entry.queue.some((message) => message.id === "m-local")
      );
    });

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.messages.map((message) => message.id)).toEqual(["m-server", "m-local"]);
    expect(entry?.queue.map((message) => message.id)).toEqual(["m-local"]);
    expect(entry?.loadState).toBe("live");
    expect(entry?.error).toBeUndefined();
    expect(getSessionHead).toHaveBeenCalledTimes(1);

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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);

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
          provider_tool_name: "Bash",
          title: "Run",
          subtitle: "pwd",
          status: "completed",
          input_preview: { command: "pwd" },
          first_event_seq: 2,
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
    expect((entry?.turnToolsByTurnId[turnId]?.[0] as { first_event_seq?: number | null } | undefined)?.first_event_seq).toBe(2);
    expect(entry?.turnToolsByTurnId[turnId]?.[0]?.provider_tool_name).toBe("Bash");
    expect(entry?.turnToolsByTurnId[turnId]?.[0]?.subtitle).toBe("pwd");

    await sup.loadTurnTools(sessionId, turnId);
    expect(listTurnTools).toHaveBeenCalledWith(sessionId, turnId);
  });

  it("treats live active snapshot heads as authoritative on first open", async () => {
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
    const store: WorkspaceActiveSnapshotEventSource & {
      getSessionHeadsSnapshot: () => Record<string, SessionHeadSnapshot>;
    } = {
      subscribe: () => () => {},
      subscribeEvents: (_listener: (evt: WorkspaceActiveSnapshotEvent) => void) => () => {},
      getSessionHeadSnapshot: () => null,
      getSessionHeadsSnapshot: () => ({ [sessionId]: head }),
      getWorktreeRoot: () => null,
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return entry?.freshness === "authoritative" && entry.lastEventSeq === 0;
    });
    expect(getSessionHead).not.toHaveBeenCalled();

    expect(getSessionSnapshot).not.toHaveBeenCalled();
  });

  it("rehydrates from /head when active heads came only from bootstrap cache", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-5-bootstrap";
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
    let resolveHead!: (value: SessionHeadSnapshot) => void;
    const headPromise = new Promise<SessionHeadSnapshot>((resolve) => {
      resolveHead = resolve;
    });
    getSessionHeadMock.mockImplementationOnce(() => headPromise);

    const store: WorkspaceActiveSnapshotEventSource & {
      getSessionHeadsSnapshot: () => Record<string, SessionHeadSnapshot>;
    } = {
      subscribe: () => () => {},
      subscribeEvents: (_listener: (evt: WorkspaceActiveSnapshotEvent) => void) => () => {},
      getSessionHeadSnapshot: () => null,
      getSessionHeadsSnapshot: () => ({ [sessionId]: head }),
      getWorktreeRoot: () => null,
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessions: () => {},
      getSnapshot: () => ({ ...mkWorkspaceSnapshotState(), liveSnapshotApplied: false }),
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return entry?.freshness === "bootstrap" && entry.lastEventSeq === 0;
    });
    expect(getSessionHead).toHaveBeenCalledTimes(1);

    resolveHead(head);
    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.freshness === "authoritative");
  });

  it("forces /head on warm reopen after disconnect clears authority", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-disconnect-reopen";
    const head: SessionHeadSnapshot = {
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 3,
      state_rev: 3,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    };
    const activeState: WorkspaceActiveSnapshotState = {
      ...mkWorkspaceSnapshotState(),
      activeIds: ["task-disconnect-reopen"],
      tasksById: {
        "task-disconnect-reopen": {
          ...mkWorkspaceTaskSummary({
            taskId: "task-disconnect-reopen",
            primarySessionId: sessionId,
            sessionIds: [sessionId],
          }),
          primarySessionHead: head,
        },
      },
      totalActive: 1,
    };
    let resolveHead!: (value: SessionHeadSnapshot) => void;
    const headPromise = new Promise<SessionHeadSnapshot>((resolve) => {
      resolveHead = resolve;
    });

    const sup = new SessionSupervisor();
    sup.setWorkspaceSessionHeads({ [sessionId]: head });
    sup.setWorkspaceSnapshotState(activeState);
    const close = sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.freshness === "authoritative");
    expect(getSessionHeadMock).not.toHaveBeenCalled();

    sup.setWorkspaceSnapshotState({ ...activeState, connection: "disconnected" });
    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.freshness === "recovering");

    close();
    getSessionHeadMock.mockImplementationOnce(() => headPromise);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => getSessionHeadMock.mock.calls.length === 1);
    expect(sup.getSnapshot().sessions[sessionId]?.freshness).toBe("recovering");

    resolveHead(head);
    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.freshness === "authoritative");
  });

  it("evicts omitted stale running turns from bounded active heads", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-stale-running";
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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId] != null);

    listeners.forEach((listener) =>
      listener({
        type: "session_head_seed",
        workspace_id: "ws-1",
        snapshot_rev: 1,
        head: {
          session: mkSession(sessionId),
          turns: [mkTurn({ sessionId, turnId: "turn-stale", status: "running", startSeq: 1 })],
          events: [] as SessionEvent[],
          messages: [] as Message[],
          last_event_seq: 1,
          state_rev: 1,
          has_more_turns: false,
          has_more_history: false,
          history_cursor: null,
          head_window: {
            turn_limit: 5,
            message_limit: 200,
            event_limit: 0,
            byte_limit: 1500000,
            turn_count: 1,
            message_count: 0,
            event_count: 0,
            bytes: 0,
            truncated: false,
          },
        },
      }),
    );

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.turns.length === 1);
    expect(sup.getSnapshot().sessions[sessionId]?.turns[0]?.status).toBe("running");

    const freshTurns = Array.from({ length: 5 }, (_, index) =>
      mkTurn({
        sessionId,
        turnId: `turn-fresh-${index + 1}`,
        status: "completed",
        startSeq: index + 10,
      }),
    );

    listeners.forEach((listener) =>
      listener({
        type: "session_head_seed",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        head: {
          session: mkSession(sessionId),
          turns: freshTurns,
          events: [] as SessionEvent[],
          messages: [] as Message[],
          last_event_seq: 20,
          state_rev: 2,
          has_more_turns: true,
          has_more_history: false,
          history_cursor: null,
          head_window: {
            turn_limit: 5,
            message_limit: 200,
            event_limit: 0,
            byte_limit: 1500000,
            turn_count: 5,
            message_count: 0,
            event_count: 0,
            bytes: 0,
            truncated: true,
          },
        },
      }),
    );

    await waitForCondition(() => sup.getSnapshot().sessions[sessionId]?.lastEventSeq === 20);

    const entry = sup.getSnapshot().sessions[sessionId];
    expect(entry?.turns.map((turn) => turn.turn_id)).toEqual(freshTurns.map((turn) => turn.turn_id));
    expect(entry?.turns.some((turn) => turn.status === "running" || turn.status === "queued")).toBe(
      false,
    );
  });

  it("auto-loads support for open sessions when an authoritative head revision is known", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-support-race";
    const head: SessionHeadSnapshot = {
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [] as Message[],
      last_event_seq: 3,
      state_rev: 7,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    };

    const activeState = mkWorkspaceSnapshotState();
    activeState.activeIds = ["task-1"];
    activeState.tasksById = {
      "task-1": mkWorkspaceTaskSummary({
        taskId: "task-1",
        primarySessionId: sessionId,
        sessionIds: [sessionId],
      }),
    };

    const store: WorkspaceActiveSnapshotEventSource & {
      getSessionHeadsSnapshot: () => Record<string, SessionHeadSnapshot>;
    } = {
      subscribe: () => () => {},
      subscribeEvents: (_listener: (evt: WorkspaceActiveSnapshotEvent) => void) => () => {},
      getSessionHeadSnapshot: (id: string) => (id === sessionId ? head : null),
      getSessionHeadsSnapshot: () => ({ [sessionId]: head }),
      getWorktreeRoot: () => null,
      getWorktreeVcsSnapshot: () => null,
      setSubscribedSessions: () => {},
      getSnapshot: () => activeState,
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
    const internals = asSupervisorInternals(sup);
    sup.openSession(sessionId);

    await waitForCondition(() => {
      const entry = internals.entries.get(sessionId);
      return entry?.stateRev === 7
        && entry?.stateAppliedRev === 7
        && entry?.subagentInvocationsAppliedRev === 7
        && entry?.stateLoaded === true
        && entry?.subagentInvocationsLoaded === true;
    });

    expect(getSessionState).toHaveBeenCalledTimes(1);
    expect(listSessionSubagentInvocations).toHaveBeenCalledTimes(1);
    expect(getSessionHead).not.toHaveBeenCalled();
  });

  it("reloads support after reopen when no authoritative revision is known", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-support-unknown-revision-reopen";
    const createdAt = new Date().toISOString();
    getSessionStateMock
      .mockResolvedValueOnce({
        artifacts: [
          {
            id: "artifact-a",
            session_id: sessionId,
            task_id: "task-1",
            worktree_id: "wt-1",
            absolute_path: "/tmp/a",
            mime_type: "text/plain",
            bytes: 1,
            created_at: createdAt,
          },
        ],
        git_status: null,
      } as never)
      .mockResolvedValueOnce({
        artifacts: [
          {
            id: "artifact-b",
            session_id: sessionId,
            task_id: "task-1",
            worktree_id: "wt-1",
            absolute_path: "/tmp/b",
            mime_type: "text/plain",
            bytes: 1,
            created_at: createdAt,
          },
        ],
        git_status: null,
      } as never);
    listSessionSubagentInvocationsMock
      .mockResolvedValueOnce([{ id: "subagent-a" }] as never)
      .mockResolvedValueOnce([{ id: "subagent-b" }] as never);

    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);

    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => {
      const entry = internals.entries.get(sessionId);
      return Boolean(
        entry?.stateLoaded
          && entry?.subagentInvocationsLoaded
          && sup.getSnapshot().sessions[sessionId]?.artifacts[0]?.absolute_path === "/tmp/a"
          && sup.getSnapshot().sessions[sessionId]?.subagentInvocations[0]?.id === "subagent-a",
      );
    });

    sup.closeSession(sessionId);
    sup.openSession(sessionId, { mode: "active" });

    const reopened = internals.entries.get(sessionId);
    expect(reopened?.stateRev).toBeUndefined();
    expect(reopened?.stateLoaded).toBe(false);
    expect(reopened?.subagentInvocationsLoaded).toBe(false);

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions[sessionId];
      return entry?.stateLoaded === true
        && entry?.subagentInvocationsLoaded === true
        && entry?.artifacts[0]?.absolute_path === "/tmp/b"
        && entry?.subagentInvocations[0]?.id === "subagent-b";
    });

    expect(getSessionState).toHaveBeenCalledTimes(2);
    expect(listSessionSubagentInvocations).toHaveBeenCalledTimes(2);
  });

  it("retries failed support auto-loads after reopen when the revision is unchanged", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-support-known-revision-reopen";
    const createdAt = new Date().toISOString();
    getSessionStateMock
      .mockRejectedValueOnce(new Error("daemon offline"))
      .mockResolvedValueOnce({
        artifacts: [
          {
            id: "artifact-b",
            session_id: sessionId,
            task_id: "task-1",
            worktree_id: "wt-1",
            absolute_path: "/tmp/b",
            mime_type: "text/plain",
            bytes: 1,
            created_at: createdAt,
          },
        ],
        git_status: null,
      } as never);
    listSessionSubagentInvocationsMock
      .mockRejectedValueOnce(new Error("subagent query failed"))
      .mockResolvedValueOnce([{ id: "subagent-b" }] as never);

    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);
    const entry = internals.ensureEntry(sessionId);
    entry.stateRev = 7;

    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => {
      const current = sup.getSnapshot().sessions[sessionId];
      return Boolean(current?.loadErrors?.state && current?.loadErrors?.subagentInvocations);
    });

    sup.closeSession(sessionId);
    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => {
      const current = internals.entries.get(sessionId);
      return current?.stateAppliedRev === 7
        && current?.subagentInvocationsAppliedRev === 7
        && sup.getSnapshot().sessions[sessionId]?.artifacts[0]?.absolute_path === "/tmp/b"
        && sup.getSnapshot().sessions[sessionId]?.subagentInvocations[0]?.id === "subagent-b";
    });

    expect(getSessionState).toHaveBeenCalledTimes(2);
    expect(listSessionSubagentInvocations).toHaveBeenCalledTimes(2);
  });

  it("preserves support-load caches across replica replace patches", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-replace-support-cache";
    const now = new Date().toISOString();
    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);
    const entry = internals.ensureEntry(sessionId);
    entry.stateRev = 7;
    entry.stateAppliedRev = 7;

    sup.loadSessionState(sessionId);
    sup.loadSubagentInvocations(sessionId);

    await waitForCondition(() => {
      const current = internals.entries.get(sessionId);
      return Boolean(current?.stateLoaded && current?.subagentInvocationsLoaded);
    });

    internals.handleReplicaPatches([
      {
        op: "replace",
        sessionId,
        data: {
          session: mkSession(sessionId),
          turns: [] as SessionTurn[],
          events: [] as SessionEvent[],
          messages: [
            {
              id: "m-replace-support-cache",
              session_id: sessionId,
              task_id: "task-1",
              role: "assistant",
              content: "replacement head",
              delivery: "immediate",
              created_at: now,
            } as Message,
          ],
          lastEventSeq: 7,
          stateRev: 7,
          hasMoreTurns: false,
        },
      },
    ]);

    const replaced = internals.entries.get(sessionId);
    expect(replaced?.stateLoaded).toBe(true);
    expect(replaced?.stateAppliedRev).toBe(7);
    expect(replaced?.subagentInvocationsLoaded).toBe(true);

    sup.loadSessionState(sessionId);
    sup.loadSubagentInvocations(sessionId);

    await Promise.resolve();

    expect(getSessionState).toHaveBeenCalledTimes(1);
    expect(listSessionSubagentInvocations).toHaveBeenCalledTimes(1);
  });

  it("refetches session state instead of reusing cache when no revision is known", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-state-unknown-revision";
    const createdAt = new Date().toISOString();
    getSessionStateMock
      .mockResolvedValueOnce({
        artifacts: [
          {
            id: "artifact-a",
            session_id: sessionId,
            task_id: "task-1",
            worktree_id: "wt-1",
            absolute_path: "/tmp/a",
            mime_type: "text/plain",
            bytes: 1,
            created_at: createdAt,
          },
        ],
        git_status: null,
      } as never)
      .mockResolvedValueOnce({
        artifacts: [
          {
            id: "artifact-b",
            session_id: sessionId,
            task_id: "task-1",
            worktree_id: "wt-1",
            absolute_path: "/tmp/b",
            mime_type: "text/plain",
            bytes: 1,
            created_at: createdAt,
          },
        ],
        git_status: null,
      } as never);

    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);

    sup.loadSessionState(sessionId);
    await waitForCondition(() => internals.entries.get(sessionId)?.stateLoaded === true);

    internals.entries.delete(sessionId);

    sup.loadSessionState(sessionId);
    await waitForCondition(() => {
      const entry = internals.entries.get(sessionId);
      return entry?.stateLoaded === true
        && sup.getSnapshot().sessions[sessionId]?.artifacts[0]?.absolute_path === "/tmp/b";
    });

    expect(getSessionState).toHaveBeenCalledTimes(2);
  });

  it("refetches session state when streamed head revisions advance after a warm load", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-state-rev-warm-cache";
    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);

    sup.loadSessionState(sessionId);

    await waitForCondition(() => internals.entries.get(sessionId)?.stateLoaded === true);

    internals.handleReplicaPatches([
      {
        op: "append",
        sessionId,
        data: {
          lastEventSeq: 7,
          stateRev: 7,
        },
      },
    ]);

    await waitForCondition(() => internals.entries.get(sessionId)?.stateRev === 7);

    sup.loadSessionState(sessionId);
    await Promise.resolve();

    internals.handleReplicaPatches([
      {
        op: "append",
        sessionId,
        data: {
          lastEventSeq: 9,
          stateRev: 9,
        },
      },
    ]);

    await waitForCondition(() => internals.entries.get(sessionId)?.stateRev === 9);

    sup.loadSessionState(sessionId);
    await Promise.resolve();

    await waitForCondition(() => {
      const entry = internals.entries.get(sessionId);
      return entry?.stateAppliedRev === 9;
    });

    expect(getSessionState).toHaveBeenCalledTimes(2);
  });

  it("auto-refreshes open-session support when streamed state revisions advance", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-support-rev-advance";
    const createdAt = new Date().toISOString();
    getSessionStateMock
      .mockResolvedValueOnce({
        artifacts: [
          {
            id: "artifact-a",
            session_id: sessionId,
            task_id: "task-1",
            worktree_id: "wt-1",
            absolute_path: "/tmp/a",
            mime_type: "text/plain",
            bytes: 1,
            created_at: createdAt,
          },
        ],
        git_status: null,
      } as never)
      .mockResolvedValueOnce({
        artifacts: [
          {
            id: "artifact-b",
            session_id: sessionId,
            task_id: "task-1",
            worktree_id: "wt-1",
            absolute_path: "/tmp/b",
            mime_type: "text/plain",
            bytes: 1,
            created_at: createdAt,
          },
        ],
        git_status: null,
      } as never);
    listSessionSubagentInvocationsMock
      .mockResolvedValueOnce([{ id: "subagent-a" }] as never)
      .mockResolvedValueOnce([{ id: "subagent-b" }] as never);

    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);
    const entry = internals.ensureEntry(sessionId);
    entry.stateRev = 7;

    sup.openSession(sessionId, { mode: "active" });

    await waitForCondition(() => {
      const current = internals.entries.get(sessionId);
      return current?.stateAppliedRev === 7
        && current?.subagentInvocationsAppliedRev === 7
        && sup.getSnapshot().sessions[sessionId]?.artifacts[0]?.absolute_path === "/tmp/a"
        && sup.getSnapshot().sessions[sessionId]?.subagentInvocations[0]?.id === "subagent-a";
    });

    internals.handleReplicaPatches([
      {
        op: "append",
        sessionId,
        data: {
          lastEventSeq: 9,
          stateRev: 9,
        },
      },
    ]);

    await waitForCondition(() => {
      const current = internals.entries.get(sessionId);
      return current?.stateAppliedRev === 9
        && current?.subagentInvocationsAppliedRev === 9
        && sup.getSnapshot().sessions[sessionId]?.artifacts[0]?.absolute_path === "/tmp/b"
        && sup.getSnapshot().sessions[sessionId]?.subagentInvocations[0]?.id === "subagent-b";
    });

    expect(getSessionState).toHaveBeenCalledTimes(2);
    expect(listSessionSubagentInvocations).toHaveBeenCalledTimes(2);
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
      setSubscribedSessions: () => {},
      getSnapshot: () => activeState,
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
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
      setSubscribedSessions: () => {},
      getSnapshot: () => archivedState,
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
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
      setSubscribedSessions: () => {},
      getSnapshot: () => archivedState,
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
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
      setSubscribedSessions: () => {},
      getSnapshot: () => mkWorkspaceSnapshotState(),
    };

    const sup = new SessionSupervisor();
    attachWorkspaceStore(sup, store);
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

  it("loads and saves session history pages with workspace owner scope", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-history-scope";
    loadSessionHistoryPageV1Mock.mockResolvedValueOnce(null);
    getSessionHistoryMock.mockResolvedValueOnce({
      turns: [],
      messages: [],
      has_more: false,
      next_cursor: null,
    } as never);

    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);
    sup.setSession(mkSession(sessionId));
    const entry = internals.ensureEntry(sessionId);
    entry.hasMoreTurns = true;
    entry.oldestTurnSeq = 10;

    await sup.loadMoreTurns(sessionId);

    expect(loadSessionHistoryPageV1Mock).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "workspace",
        workspaceId: "ws-1",
        daemon: { kind: "browser", baseUrl: "http://daemon.test" },
      }),
      sessionId,
      10,
      60,
    );
    expect(saveSessionHistoryPageV1Mock).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "workspace",
        workspaceId: "ws-1",
        daemon: { kind: "browser", baseUrl: "http://daemon.test" },
      }),
      sessionId,
      10,
      60,
      expect.objectContaining({ has_more: false }),
    );
  });

  it("loads and saves task thought caches with workspace owner scope", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-thought-scope";
    const createdAt = new Date().toISOString();
    loadTaskThoughtsV1Mock.mockResolvedValueOnce({
      v: 1,
      taskId: "task-1",
      sessions: {},
      updatedAtMs: Date.now(),
    });

    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);
    sup.setSession(mkSession(sessionId));

    internals.handleReplicaPatches([
      {
        op: "append",
        sessionId,
        data: {
          session: mkSession(sessionId),
          events: [
            {
              id: "evt-thought-1",
              session_id: sessionId,
              turn_id: "turn-1",
              event_type: "thought_chunk",
              payload_json: {
                item_id: "item-1",
                full_content: "final thought",
                is_final: true,
              },
              created_at: createdAt,
              seq: 1,
            },
          ],
          lastEventSeq: 1,
        },
      },
    ]);

    await waitForCondition(() => saveTaskThoughtsV1Mock.mock.calls.length > 0);

    expect(loadTaskThoughtsV1Mock).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "workspace",
        workspaceId: "ws-1",
        daemon: { kind: "browser", baseUrl: "http://daemon.test" },
      }),
      "task-1",
    );
    expect(saveTaskThoughtsV1Mock).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "workspace",
        workspaceId: "ws-1",
        daemon: { kind: "browser", baseUrl: "http://daemon.test" },
      }),
      "task-1",
      expect.objectContaining({
        sessions: expect.objectContaining({
          [sessionId]: expect.objectContaining({
            sessionId,
          }),
        }),
      }),
    );
  });

  it("clears cached git status when a fresh state response omits it", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-state-clears-git-status";
    getSessionStateMock
      .mockResolvedValueOnce({
        artifacts: [],
        git_status: {
          summary_line: "main +1",
          branch: "main",
          upstream: "origin/main",
          ahead: 1,
          behind: 0,
          detached: false,
          staged: 0,
          unstaged: 1,
          untracked: 0,
        },
      } as never)
      .mockResolvedValueOnce({
        artifacts: [],
        git_status: null,
      } as never);

    const sup = new SessionSupervisor();
    const internals = asSupervisorInternals(sup);

    sup.loadSessionState(sessionId);
    await waitForCondition(() => internals.entries.get(sessionId)?.stateLoaded === true);

    sup.loadSessionState(sessionId, { force: true });
    await waitForCondition(() => internals.stateCacheBySessionId.get(sessionId)?.state.git_status === null);

    expect(sup.getSnapshot().sessions[sessionId]?.gitStatusSummary).toBeNull();
    expect(internals.stateCacheBySessionId.get(sessionId)?.state.git_status).toBeNull();
  });
});
