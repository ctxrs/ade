import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  Session,
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  Task,
  WorkspaceActiveHeadBatch,
  WorkspaceActiveSnapshot,
  WorkspaceActiveTaskSummary,
} from "@ctx/types";
import { waitForCondition } from "../testUtils/waitForCondition";

vi.mock("../api/client", () => {
  const idToString = (id: string | null | undefined): string => {
    if (id === null || id === undefined) return "";
    if (typeof id !== "string") {
      throw new Error("Expected id to be a string");
    }
    return id;
  };
  return {
    idToString,
    authToken: vi.fn(() => null),
    getDaemonClientConfig: vi.fn(() => ({
      baseUrl: "",
      wsBaseUrl: "",
      authToken: null,
      runId: null,
    })),
    getDaemonBaseUrl: vi.fn(() => ""),
    resolveDaemonBaseUrl: vi.fn(() => ""),
    resolveDaemonWsBaseUrl: vi.fn(() => "ws://localhost"),
    subscribeDaemonConfig: vi.fn(() => () => {}),
    getHealth: vi.fn(async () => ({
      version: "0.0.0",
      daemon_version: "0.0.0",
      pid: 1,
      data_root: "/tmp/ctx",
      daemon_url: "",
      auth_required: false,
      compatibility: {
        desktop_exact_version: "0.0.0",
        mobile_api_min: 1,
        mobile_api_max: 1,
      },
    })),
    getSessionHead: vi.fn(async () => null),
    getWorkspaceActiveHeads: vi.fn(async () => ({
      workspace_id: "ws-1",
      snapshot_rev: 0,
      heads: [],
    })),
    getWorkspaceActiveSnapshot: vi.fn(),
    listWorkspaceArchivedTaskSummaries: vi.fn(async () => ({
      workspace_id: "ws-1",
      archived_rev: 0,
      tasks: [],
      total_archived: 0,
      next_cursor: null,
    })),
  };
});

vi.mock("./uiStateStore", () => ({
  loadWorkspaceActiveSnapshotV1: vi.fn(async () => null),
  saveWorkspaceActiveSnapshotV1: vi.fn(async () => {}),
}));

const mkTask = (taskId: string, workspaceId: string, now: string): Task => ({
  id: taskId,
  workspace_id: workspaceId,
  title: "Active task",
  status: "running",
  created_at: now,
  updated_at: now,
});

const mkSession = (sessionId: string, taskId: string, workspaceId: string, now: string): Session => ({
  id: sessionId,
  task_id: taskId,
  workspace_id: workspaceId,
  worktree_id: "wt-1",
  provider_id: "fake",
  model_id: "fake-model",
  title: "Session",
  agent_role: "assistant",
  status: "active",
  created_at: now,
  updated_at: now,
});

const mkSummary = (session: Session, now: string): SessionSnapshotSummary => ({
  session,
  last_message_at: now,
  last_message_preview: "hello",
  last_event_seq: 0,
  state_rev: 0,
  activity: { is_working: false },
  unread: false,
});

const mkHead = (session: Session): SessionHeadSnapshot => ({
  session,
  turns: [],
  events: [],
  messages: [],
  last_event_seq: 0,
  state_rev: 0,
  activity: { is_working: false },
  has_more_turns: false,
  history_cursor: null,
  has_more_history: false,
});

const mkActiveSummary = (
  task: Task,
  summary: SessionSnapshotSummary,
  head: SessionHeadSnapshot,
  now: string,
): WorkspaceActiveTaskSummary => ({
  task,
  primary_session: summary,
  primary_session_head: head,
  sessions: [summary],
  sort_at: now,
});

const openWsState = (globalThis.WebSocket as any)?.OPEN ?? 1;

const mkOpenWs = () => ({
  readyState: openWsState,
  send: vi.fn(),
});

describe("WorkspaceActiveSnapshotStore", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("hydrates from stream snapshot without HTTP", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getWorkspaceActiveHeads, getWorkspaceActiveSnapshot } = await import("../api/client");

    const now = new Date().toISOString();
    const task = mkTask("task-1", "ws-1", now);
    const session = mkSession("session-1", "task-1", "ws-1", now);
    const summary = mkSummary(session, now);
    const head = mkHead(session);

    const activeSummary = mkActiveSummary(task, summary, head, now);
    const activeSnapshot: WorkspaceActiveSnapshot = {
      workspace_id: "ws-1",
      snapshot_rev: 2,
      archived_rev: 0,
      active: { total_count: 1, tasks: [activeSummary] },
    };
    const activeHeads: WorkspaceActiveHeadBatch = {
      workspace_id: "ws-1",
      snapshot_rev: 2,
      heads: [head],
    };

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 2,
        active_snapshot: activeSnapshot,
        active_heads: activeHeads,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    expect(getWorkspaceActiveSnapshot).not.toHaveBeenCalled();
    expect(getWorkspaceActiveHeads).not.toHaveBeenCalled();

    const snapshot = store.getSnapshot();
    expect(snapshot.activeIds).toEqual(["task-1"]);
    expect(store.getSessionHeadSnapshot("session-1")?.last_event_seq).toBe(0);
  });

  it("requests snapshot on reset_required", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getWorkspaceActiveSnapshot } = await import("../api/client");

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    const ws = mkOpenWs();
    (store as any).ws = ws;
    await (store as any).handleStreamMessage(JSON.stringify({ type: "reset_required", latest_rev: 5 }));

    expect(getWorkspaceActiveSnapshot).not.toHaveBeenCalled();
    expect(ws.send).toHaveBeenCalled();
    store.destroy();
  });

  it("resubscribes on session_gap", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    const ws = mkOpenWs();
    (store as any).ws = ws;
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 1,
        event: {
          type: "session_gap",
          workspace_id: "ws-1",
          snapshot_rev: 1,
          session_id: "session-1",
          after_seq: 5,
        },
      }),
    );

    expect(ws.send).toHaveBeenCalledTimes(1);
    const payload = JSON.parse(String((ws.send as any).mock.calls[0]?.[0] ?? "{}"));
    expect(payload.type).toBe("subscribe");
    expect(payload.include_active_heads).toBe(false);
  });

  it("applies session_head_seed events", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const now = new Date().toISOString();
    const task = mkTask("task-seed", "ws-1", now);
    const session = mkSession("session-seed", "task-seed", "ws-1", now);
    const summary = mkSummary(session, now);
    const head = mkHead(session);
    const activeSnapshot: WorkspaceActiveSnapshot = {
      workspace_id: "ws-1",
      snapshot_rev: 1,
      archived_rev: 0,
      active: { total_count: 1, tasks: [mkActiveSummary(task, summary, head, now)] },
    };

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    const seededHead: SessionHeadSnapshot = {
      ...head,
      last_event_seq: 3,
      messages: [
        {
          id: "msg-seed",
          session_id: "session-seed",
          task_id: "task-seed",
          role: "assistant",
          content: "seeded",
          delivery: "immediate",
          created_at: now,
        },
      ],
    };

    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 2,
        event: {
          type: "session_head_seed",
          workspace_id: "ws-1",
          snapshot_rev: 1,
          head: seededHead,
        },
      }),
    );

    expect(store.getSessionHeadSnapshot("session-seed")?.last_event_seq).toBe(3);
    expect(store.getSessionHeadSnapshot("session-seed")?.messages?.[0]?.content).toBe("seeded");
  });

  it("requests snapshot on stream seq gap", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getWorkspaceActiveSnapshot } = await import("../api/client");

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    const ws = mkOpenWs();
    (store as any).ws = ws;
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 1,
        event: { type: "ready", workspace_id: "ws-1", snapshot_rev: 1, archived_rev: 0 },
      }),
    );
    expect(getWorkspaceActiveSnapshot).not.toHaveBeenCalled();
    expect(ws.send).not.toHaveBeenCalled();

    (getWorkspaceActiveSnapshot as any).mockClear();
    ws.send.mockClear();
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 3,
        event: { type: "ready", workspace_id: "ws-1", snapshot_rev: 3, archived_rev: 0 },
      }),
    );
    expect(getWorkspaceActiveSnapshot).not.toHaveBeenCalled();
    expect(ws.send).toHaveBeenCalled();
    store.destroy();
  });

  it("applies task_delta to remove archived tasks from the active list", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const now = "2024-01-01T00:00:00.000Z";
    const later = "2024-01-01T01:00:00.000Z";
    const archivedAt = "2024-01-02T00:00:00.000Z";

    const taskA = mkTask("task-1", "ws-1", now);
    const taskB = mkTask("task-2", "ws-1", later);
    const sessionA = mkSession("session-1", "task-1", "ws-1", now);
    const sessionB = mkSession("session-2", "task-2", "ws-1", later);
    const summaryA = mkSummary(sessionA, now);
    const summaryB = mkSummary(sessionB, later);
    const headA = mkHead(sessionA);
    const headB = mkHead(sessionB);

    const activeSnapshot: WorkspaceActiveSnapshot = {
      workspace_id: "ws-1",
      snapshot_rev: 1,
      archived_rev: 0,
      active: {
        total_count: 2,
        tasks: [mkActiveSummary(taskA, summaryA, headA, now), mkActiveSummary(taskB, summaryB, headB, later)],
      },
    };

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 2,
        event: {
          type: "task_delta",
          workspace_id: "ws-1",
          snapshot_rev: 2,
          delta: {
            kind: "archived",
            task: {
              ...taskA,
              archived_at: archivedAt,
              updated_at: archivedAt,
            },
          },
        },
      }),
    );

    const snapshot = store.getSnapshot();
    expect(snapshot.activeIds).toEqual(["task-2"]);
    expect(snapshot.archivedIds).toEqual([]);
    expect(snapshot.totalActive).toBe(1);
    expect(snapshot.totalArchived).toBe(0);
  });

  it("merges tool_summaries from session_head_delta events", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const now = "2024-01-01T00:00:00.000Z";
    const task = mkTask("task-1", "ws-1", now);
    const session = mkSession("session-1", "task-1", "ws-1", now);
    const summary = mkSummary(session, now);

    const turn = {
      turn_id: "turn-1",
      session_id: session.id,
      run_id: null,
      user_message_id: null,
      status: "completed",
      start_seq: 1,
      end_seq: 2,
      started_at: now,
      updated_at: now,
      assistant_partial: null,
      thought_partial: null,
      metrics_json: null,
      tool_total: 1,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 1,
      tool_failed: 0,
    };

    const head: SessionHeadSnapshot = {
      ...mkHead(session),
      turns: [turn as any],
      tool_summaries: [],
    };

    const activeSummary = mkActiveSummary(task, summary, head, now);
    const activeSnapshot: WorkspaceActiveSnapshot = {
      workspace_id: "ws-1",
      snapshot_rev: 1,
      archived_rev: 0,
      active: { total_count: 1, tasks: [activeSummary] },
    };
    const activeHeads: WorkspaceActiveHeadBatch = {
      workspace_id: "ws-1",
      snapshot_rev: 1,
      heads: [head],
    };

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
        active_heads: activeHeads,
      }),
    );
    await waitForCondition(() => store.getSnapshot().initialized);

    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 2,
        event: {
          type: "session_head_delta",
          workspace_id: "ws-1",
          snapshot_rev: 2,
          delta: {
            session_id: session.id,
            last_event_seq: 2,
            state_rev: 0,
            tool_summaries: [
              {
                session_id: session.id,
                tool_call_id: "call-1",
                turn_id: "turn-1",
                tool_kind: "tool",
                title: "Tool",
                status: "completed",
                input_preview: null,
                output_preview: null,
                created_at: now,
                updated_at: now,
              },
            ],
          },
        },
      }),
    );

    const updated = store.getSessionHeadSnapshot(session.id);
    expect(updated?.tool_summaries?.length ?? 0).toBe(1);
    expect(updated?.tool_summaries?.[0]?.tool_call_id).toBe("call-1");
  });

  it("applies session_summary_delta to update activity and previews", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const now = "2024-01-01T00:00:00.000Z";
    const later = "2024-01-01T02:00:00.000Z";
    const task = mkTask("task-1", "ws-1", now);
    const session = mkSession("session-1", "task-1", "ws-1", now);
    const summary = mkSummary(session, now);
    const head = mkHead(session);

    const activeSnapshot: WorkspaceActiveSnapshot = {
      workspace_id: "ws-1",
      snapshot_rev: 1,
      archived_rev: 0,
      active: {
        total_count: 1,
        tasks: [mkActiveSummary(task, summary, head, now)],
      },
    };

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 2,
        event: {
          type: "session_summary_delta",
          workspace_id: "ws-1",
          snapshot_rev: 2,
          delta: {
            session_id: "session-1",
            task_id: "task-1",
            activity: { is_working: true, last_turn_status: "running" },
            last_message_at: later,
            last_message_preview: "updated preview",
            last_event_seq: 5,
            state_rev: 2,
          },
        },
      }),
    );

    const snapshot = store.getSnapshot();
    const updated = snapshot.tasksById["task-1"].sessions[0];
    expect(updated.activity?.is_working).toBe(true);
    expect(updated.activity?.last_turn_status).toBe("running");
    expect(updated.last_message_at).toBe(later);
    expect(updated.last_message_preview).toBe("updated preview");
    expect(updated.last_event_seq).toBe(5);
    expect(updated.state_rev).toBe(2);
  });

  it("does not regress monotonic session summary fields when applying session_summary_delta", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const now = "2024-01-01T00:00:00.000Z";
    const later = "2024-01-01T02:00:00.000Z";
    const task = mkTask("task-1", "ws-1", now);
    const session = mkSession("session-1", "task-1", "ws-1", now);
    const head = mkHead(session);
    const seededSummary = { ...mkSummary(session, now), last_message_at: later, last_event_seq: 10, state_rev: 5 };

    const activeSnapshot: WorkspaceActiveSnapshot = {
      workspace_id: "ws-1",
      snapshot_rev: 1,
      archived_rev: 0,
      active: {
        total_count: 1,
        tasks: [mkActiveSummary(task, seededSummary, head, now)],
      },
    };

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 2,
        event: {
          type: "session_summary_delta",
          workspace_id: "ws-1",
          snapshot_rev: 2,
          delta: {
            session_id: "session-1",
            task_id: "task-1",
            activity: { is_working: false, last_turn_status: "completed" },
            last_message_at: now,
            last_event_seq: 3,
            state_rev: 2,
          },
        },
      }),
    );

    const updated = store.getSnapshot().tasksById["task-1"].sessions[0];
    expect(updated.activity?.is_working).toBe(false);
    expect(updated.last_message_at).toBe(later);
    expect(updated.last_event_seq).toBe(10);
    expect(updated.state_rev).toBe(5);
  });

  it("treats no-op session_summary_delta as no change", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const now = "2024-01-01T00:00:00.000Z";
    const later = "2024-01-01T02:00:00.000Z";
    const task = mkTask("task-1", "ws-1", now);
    const session = mkSession("session-1", "task-1", "ws-1", now);
    const head = mkHead(session);
    const seededSummary = {
      ...mkSummary(session, now),
      last_message_at: later,
      last_message_preview: "hello",
      last_event_seq: 10,
      state_rev: 5,
      activity: { is_working: false, last_turn_status: "completed" as const },
    };

    const activeSnapshot: WorkspaceActiveSnapshot = {
      workspace_id: "ws-1",
      snapshot_rev: 1,
      archived_rev: 0,
      active: {
        total_count: 1,
        tasks: [mkActiveSummary(task, seededSummary, head, now)],
      },
    };

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    const changed = (store as any).applySessionSummaryDelta({
      type: "session_summary_delta",
      workspace_id: "ws-1",
      snapshot_rev: 2,
      delta: {
        session_id: "session-1",
        task_id: "task-1",
        // older / identical fields that should not cause any update
        last_message_at: now,
        last_message_preview: "hello",
        last_event_seq: 1,
        state_rev: 2,
        activity: { is_working: false, last_turn_status: "completed" },
      },
    });
    expect(changed).toBe(false);

    const updated = store.getSnapshot().tasksById["task-1"].sessions[0];
    expect(updated.last_message_at).toBe(later);
    expect(updated.last_message_preview).toBe("hello");
    expect(updated.last_event_seq).toBe(10);
    expect(updated.state_rev).toBe(5);
    expect(updated.activity?.is_working).toBe(false);
    expect(updated.activity?.last_turn_status).toBe("completed");
  });
});
