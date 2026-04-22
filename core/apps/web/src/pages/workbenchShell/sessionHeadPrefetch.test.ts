import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionHeadSnapshot } from "@ctx/types";
import { SessionHeadBootstrapCache } from "../../state/sessionHeadBootstrapCache";
import type { WorkspaceActiveSnapshotState } from "../../state/workspaceActiveSnapshotStore";
import {
  collectSessionHeadsForSupervisor,
  planSessionHeadPrefetchTargets,
  primeAuthoritativeSessionHeads,
  primePersistedSessionHeads,
  SESSION_HEAD_PREFETCH_CONCURRENCY,
  SESSION_HEAD_PREFETCH_TARGET_LIMIT,
} from "./sessionHeadPrefetch";

const loadSessionHeadV1Mock = vi.fn();
const getSessionHeadMock = vi.fn();

vi.mock("../../state/uiStateStore", () => ({
  loadSessionHeadV1: (...args: unknown[]) => loadSessionHeadV1Mock(...args),
}));

vi.mock("../../api/clientSessions", () => ({
  getSessionHead: (...args: unknown[]) => getSessionHeadMock(...args),
}));

const now = "2026-03-18T00:00:00.000Z";

const makeHead = (sessionId: string, opts?: { turnCount?: number; lastEventSeq?: number }): SessionHeadSnapshot => {
  const turnCount = opts?.turnCount ?? 1;
  const lastEventSeq = opts?.lastEventSeq ?? turnCount;
  return {
    session: {
      id: sessionId,
      task_id: "task-1",
      workspace_id: "workspace-1",
      worktree_id: "worktree-1",
      provider_id: "codex",
      model_id: "gpt-5",
      title: sessionId,
      agent_role: "assistant",
      status: "active",
      created_at: now,
      updated_at: now,
    },
    turns: Array.from({ length: turnCount }, (_, index) => ({
      turn_id: `turn-${index + 1}`,
      session_id: sessionId,
      run_id: null,
      user_message_id: `msg-${index + 1}`,
      status: "completed",
      start_seq: index + 1,
      end_seq: index + 2,
      started_at: now,
      updated_at: now,
      assistant_partial: null,
      thought_partial: null,
      metrics_json: null,
      tool_total: 0,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 0,
      tool_failed: 0,
    })),
    messages: Array.from({ length: turnCount }, (_, index) => ({
      id: `message-${index + 1}`,
      session_id: sessionId,
      task_id: "task-1",
      turn_id: `turn-${index + 1}`,
      role: index % 2 === 0 ? "user" : "assistant",
      content: `message-${index + 1}`,
      delivery: "immediate",
      created_at: now,
      updated_at: now,
    })),
    events: [],
    last_event_seq: lastEventSeq,
    has_more_turns: turnCount < 3,
    has_more_history: turnCount < 3,
    history_cursor: turnCount < 3 ? 1 : null,
  };
};

const makeSnapshot = (
  sessionId: string,
  opts?: { lastEventSeq?: number; projectionRev?: number; stateRev?: number },
): WorkspaceActiveSnapshotState => ({
  workspaceId: "workspace-1",
  initialized: true,
  liveSnapshotApplied: true,
  connection: "connected",
  tasksById: {
    "task-1": {
      id: "task-1",
      task: {
        id: "task-1",
        workspace_id: "workspace-1",
        title: "Task 1",
        status: "running",
        created_at: now,
        updated_at: now,
        last_activity_at: now,
        archived_at: null,
        assistant_seen_at: null,
        last_assistant_message_at: now,
        primary_session_id: sessionId,
      },
      sessions: [
        {
          session: makeHead(sessionId).session,
          last_message_at: now,
          last_message_preview: "preview",
          last_event_seq: opts?.lastEventSeq ?? 1,
          projection_rev: opts?.projectionRev,
          state_rev: opts?.stateRev,
          activity: { is_working: false, last_turn_status: null },
          unread: false,
        },
      ],
      primarySessionId: sessionId,
      primarySessionHead: null,
      sort_at: now,
      sortAtMs: Date.parse(now),
    },
  },
  activeIds: ["task-1"],
  archivedIds: [],
  totalActive: 1,
  totalArchived: 0,
  archivedRev: 0,
  worktreeVcsById: {},
  fetchState: { active: "idle", archived: "idle" },
  hasMoreActive: false,
  hasMoreArchived: false,
  archivedLoaded: false,
});

const makeSnapshotWithSessions = (
  sessionIds: readonly string[],
  opts?: { lastEventSeq?: number; projectionRev?: number; stateRev?: number },
): WorkspaceActiveSnapshotState => {
  const primarySessionId = sessionIds[0] ?? "session-1";
  return {
    ...makeSnapshot(primarySessionId, opts),
    tasksById: {
      "task-1": {
        ...makeSnapshot(primarySessionId, opts).tasksById["task-1"],
        sessions: sessionIds.map((sessionId) => ({
          session: makeHead(sessionId).session,
          last_message_at: now,
          last_message_preview: "preview",
          last_event_seq: opts?.lastEventSeq ?? 1,
          projection_rev: opts?.projectionRev,
          state_rev: opts?.stateRev,
          activity: { is_working: false, last_turn_status: null },
          unread: false,
        })),
      },
    },
  };
};

describe("sessionHeadPrefetch", () => {
  beforeEach(() => {
    loadSessionHeadV1Mock.mockReset();
    loadSessionHeadV1Mock.mockResolvedValue(null);
    getSessionHeadMock.mockReset();
    getSessionHeadMock.mockResolvedValue(null);
  });

  it("loads persisted heads into the bootstrap cache once for active primary sessions", async () => {
    const sessionId = "session-1";
    const snapshot = makeSnapshot(sessionId);
    const persistedHead = makeHead(sessionId, { turnCount: 2, lastEventSeq: 2 });
    loadSessionHeadV1Mock.mockResolvedValue({
      v: 1,
      sessionId,
      head: {
        ...persistedHead,
        has_more_history: undefined,
        history_cursor: undefined,
      },
      updatedAtMs: Date.now(),
    });
    const bootstrapCache = new SessionHeadBootstrapCache();
    const store = {
      getSessionHeadSnapshot: vi.fn(() => null),
      getSessionHeadsSnapshot: vi.fn(() => ({})),
    };

    const firstChanged = await primePersistedSessionHeads(snapshot, store, bootstrapCache);
    const secondChanged = await primePersistedSessionHeads(snapshot, store, bootstrapCache);

    expect(firstChanged).toBe(true);
    expect(secondChanged).toBe(false);
    expect(loadSessionHeadV1Mock).toHaveBeenCalledTimes(1);
    expect(bootstrapCache.snapshot()[sessionId]?.turns).toHaveLength(2);
  });

  it("prefers the richer bootstrap cached head over a narrower direct store head", () => {
    const sessionId = "session-1";
    const snapshot = makeSnapshot(sessionId);
    const directHead = makeHead(sessionId, { turnCount: 1, lastEventSeq: 2 });
    const persistedHead = makeHead(sessionId, { turnCount: 2, lastEventSeq: 2 });
    const bootstrapCache = new SessionHeadBootstrapCache();
    bootstrapCache.upsert(persistedHead);
    const store = {
      getSessionHeadSnapshot: vi.fn(() => directHead),
      getSessionHeadsSnapshot: vi.fn(() => ({ [sessionId]: directHead })),
    };

    const heads = collectSessionHeadsForSupervisor(snapshot, store, bootstrapCache);

    expect(heads[sessionId]?.turns).toHaveLength(2);
    expect(heads[sessionId]?.messages).toHaveLength(2);
  });

  it("collects only the requested session heads when an explicit target list is provided", () => {
    const sessionId = "session-1";
    const otherSessionId = "session-2";
    const snapshot = makeSnapshot(sessionId);
    const bootstrapCache = new SessionHeadBootstrapCache();
    const store = {
      getSessionHeadSnapshot: vi.fn((id: string) => {
        if (id === sessionId) return makeHead(sessionId, { turnCount: 2, lastEventSeq: 2 });
        if (id === otherSessionId) return makeHead(otherSessionId, { turnCount: 3, lastEventSeq: 3 });
        return null;
      }),
      getSessionHeadsSnapshot: vi.fn(() => ({
        [sessionId]: makeHead(sessionId, { turnCount: 2, lastEventSeq: 2 }),
        [otherSessionId]: makeHead(otherSessionId, { turnCount: 3, lastEventSeq: 3 }),
      })),
    };

    const heads = collectSessionHeadsForSupervisor(snapshot, store, bootstrapCache, [sessionId]);

    expect(Object.keys(heads)).toEqual([sessionId]);
    expect(heads[sessionId]?.turns).toHaveLength(2);
  });

  it("prefetches an authoritative head when the summary is newer than known heads", async () => {
    const sessionId = "session-1";
    const snapshot = makeSnapshot(sessionId, { lastEventSeq: 5 });
    const authoritativeHead = makeHead(sessionId, { turnCount: 3, lastEventSeq: 5 });
    getSessionHeadMock.mockResolvedValue(authoritativeHead);
    const bootstrapCache = new SessionHeadBootstrapCache();
    const store = {
      getSessionHeadSnapshot: vi.fn(() => null),
      getSessionHeadsSnapshot: vi.fn(() => ({})),
    };

    const firstChanged = await primeAuthoritativeSessionHeads(snapshot, store, bootstrapCache, [sessionId]);
    const secondChanged = await primeAuthoritativeSessionHeads(snapshot, store, bootstrapCache, [sessionId]);

    expect(firstChanged).toBe(true);
    expect(secondChanged).toBe(false);
    expect(getSessionHeadMock).toHaveBeenCalledTimes(1);
    expect(bootstrapCache.snapshot()[sessionId]?.last_event_seq).toBe(5);
  });

  it("does not prefetch an authoritative head when the direct head already satisfies the summary", async () => {
    const sessionId = "session-1";
    const snapshot = makeSnapshot(sessionId, { lastEventSeq: 2 });
    const directHead = makeHead(sessionId, { turnCount: 2, lastEventSeq: 2 });
    const bootstrapCache = new SessionHeadBootstrapCache();
    const store = {
      getSessionHeadSnapshot: vi.fn(() => directHead),
      getSessionHeadsSnapshot: vi.fn(() => ({ [sessionId]: directHead })),
    };

    const changed = await primeAuthoritativeSessionHeads(snapshot, store, bootstrapCache, [sessionId]);

    expect(changed).toBe(false);
    expect(getSessionHeadMock).not.toHaveBeenCalled();
  });

  it("plans foreground targets before bounded warm targets", () => {
    const plan = planSessionHeadPrefetchTargets({
      foregroundSessionIds: ["foreground", "warm-1"],
      warmSessionIds: ["warm-1", "warm-2", "warm-3"],
      maxTargets: 3,
    });

    expect(plan.targetSessionIds).toEqual(["foreground", "warm-1", "warm-2"]);
    expect(plan.foregroundSessionIds).toEqual(["foreground", "warm-1"]);
    expect(plan.warmSessionIds).toEqual(["warm-2"]);
  });

  it("caps authoritative head prefetch when no explicit target list is provided", async () => {
    const sessionIds = Array.from({ length: SESSION_HEAD_PREFETCH_TARGET_LIMIT + 25 }, (_, index) => `session-${index + 1}`);
    const snapshot = makeSnapshotWithSessions(sessionIds, { lastEventSeq: 5 });
    getSessionHeadMock.mockImplementation(async (sessionId: string) =>
      makeHead(sessionId, { turnCount: 3, lastEventSeq: 5 }),
    );
    const bootstrapCache = new SessionHeadBootstrapCache();
    const store = {
      getSessionHeadSnapshot: vi.fn(() => null),
      getSessionHeadsSnapshot: vi.fn(() => ({})),
    };

    await primeAuthoritativeSessionHeads(snapshot, store, bootstrapCache);

    expect(getSessionHeadMock).toHaveBeenCalledTimes(SESSION_HEAD_PREFETCH_TARGET_LIMIT);
    expect(getSessionHeadMock).toHaveBeenCalledWith("session-1", expect.any(Number), true);
    expect(getSessionHeadMock).not.toHaveBeenCalledWith(
      `session-${SESSION_HEAD_PREFETCH_TARGET_LIMIT + 1}`,
      expect.any(Number),
      true,
    );
  });

  it("caps persisted bootstrap head priming with the same target budget", async () => {
    const sessionIds = Array.from({ length: 20 }, (_, index) => `session-${index + 1}`);
    const snapshot = makeSnapshotWithSessions(sessionIds);
    loadSessionHeadV1Mock.mockImplementation(async (sessionId: string) => ({
      v: 1,
      sessionId,
      head: makeHead(sessionId),
      updatedAtMs: Date.now(),
    }));
    const bootstrapCache = new SessionHeadBootstrapCache();
    const store = {
      getSessionHeadSnapshot: vi.fn(() => null),
      getSessionHeadsSnapshot: vi.fn(() => ({})),
    };

    await primePersistedSessionHeads(snapshot, store, bootstrapCache, undefined, { maxTargets: 5 });

    expect(loadSessionHeadV1Mock).toHaveBeenCalledTimes(5);
    expect(loadSessionHeadV1Mock).toHaveBeenCalledWith("session-1");
    expect(loadSessionHeadV1Mock).not.toHaveBeenCalledWith("session-6");
  });

  it("limits concurrent authoritative head requests", async () => {
    const sessionIds = Array.from({ length: 6 }, (_, index) => `session-${index + 1}`);
    const snapshot = makeSnapshotWithSessions(sessionIds, { lastEventSeq: 5 });
    let inFlight = 0;
    let maxInFlight = 0;
    getSessionHeadMock.mockImplementation(async (sessionId: string) => {
      inFlight += 1;
      maxInFlight = Math.max(maxInFlight, inFlight);
      await new Promise((resolve) => setTimeout(resolve, 0));
      inFlight -= 1;
      return makeHead(sessionId, { turnCount: 3, lastEventSeq: 5 });
    });
    const bootstrapCache = new SessionHeadBootstrapCache();
    const store = {
      getSessionHeadSnapshot: vi.fn(() => null),
      getSessionHeadsSnapshot: vi.fn(() => ({})),
    };

    await primeAuthoritativeSessionHeads(snapshot, store, bootstrapCache, sessionIds, {
      concurrency: SESSION_HEAD_PREFETCH_CONCURRENCY,
      maxTargets: sessionIds.length,
    });

    expect(maxInFlight).toBeLessThanOrEqual(SESSION_HEAD_PREFETCH_CONCURRENCY);
    expect(getSessionHeadMock).toHaveBeenCalledTimes(sessionIds.length);
  });
});
