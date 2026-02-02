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
  const idToString = (id: any): string => (typeof id === "string" ? id : id?.["0"]);
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
  id: { 0: taskId },
  workspace_id: { 0: workspaceId },
  title: "Active task",
  status: "running",
  created_at: now,
  updated_at: now,
});

const mkSession = (sessionId: string, taskId: string, workspaceId: string, now: string): Session => ({
  id: { 0: sessionId },
  task_id: { 0: taskId },
  workspace_id: { 0: workspaceId },
  worktree_id: { 0: "wt-1" },
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
});
