import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionHeadSnapshot } from "@ctx/types";
import { SessionHeadBootstrapCache } from "../../state/sessionHeadBootstrapCache";
import type { WorkspaceActiveSnapshotState } from "../../state/workspaceActiveSnapshotStore";
import { collectSessionHeadsForSupervisor, primePersistedSessionHeads } from "./sessionHeadPrefetch";

const loadSessionHeadV1Mock = vi.fn();

vi.mock("../../state/uiStateStore", () => ({
  loadSessionHeadV1: (...args: unknown[]) => loadSessionHeadV1Mock(...args),
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

const makeSnapshot = (sessionId: string): WorkspaceActiveSnapshotState => ({
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
          last_event_seq: 1,
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

describe("sessionHeadPrefetch", () => {
  beforeEach(() => {
    loadSessionHeadV1Mock.mockReset();
    loadSessionHeadV1Mock.mockResolvedValue(null);
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
});
