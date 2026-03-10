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
import { getUiDiagnostics, resetUiDiagnosticsForTests } from "./diagnosticsChannel";
import {
  getActiveProjectionFixture,
  getSessionGapSeedFixture,
} from "../testdata/projectionEquivalenceFixtures";

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
    getDaemonConnectionReadiness: vi.fn((connection: { baseUrl?: string | null; authToken?: string | null }) => {
      const hasBaseUrl = Boolean(connection.baseUrl);
      const hasAuthToken = Boolean(connection.authToken);
      return {
        hasBaseUrl,
        hasAuthToken,
        isReady: hasBaseUrl && hasAuthToken,
        missing: !hasBaseUrl ? "base" : !hasAuthToken ? "auth" : null,
      };
    }),
    getDaemonClientConfig: vi.fn(() => ({
      baseUrl: "http://localhost:4399",
      wsBaseUrl: "ws://localhost:4399",
      authToken: null,
      runId: null,
    })),
    subscribeDaemonConfig: vi.fn(() => () => {}),
    getDaemonConnection: vi.fn(() => ({
      baseUrl: "http://localhost:4399",
      wsBaseUrl: "ws://localhost:4399",
      authToken: null,
      runId: null,
      source: "desktop",
    })),
    syncDesktopDaemonConnectionFromBridge: vi.fn(async () => ({
      config: {
        baseUrl: "http://localhost:4399",
        wsBaseUrl: "ws://localhost:4399",
        authToken: null,
        runId: null,
      },
      info: null,
      synced: false,
      error: null,
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

type MockWs = {
  readyState: number;
  send: ReturnType<typeof vi.fn>;
};

type StoreInternals = {
  handleStreamMessage: (raw: string) => Promise<void>;
  ws?: MockWs;
  connectStream: () => Promise<void>;
  openWebSocket: (url: string) => Promise<void>;
  scheduleReconnect: () => void;
  applySessionSummaryDelta: (delta: unknown) => boolean;
};

type MockDaemonClientConfig = {
  baseUrl: string | null;
  wsBaseUrl: string | null;
  authToken: string | null;
  runId: string | null;
};

const asStoreInternals = (store: object): StoreInternals =>
  store as unknown as StoreInternals;

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};

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

const openWsState = (globalThis.WebSocket as unknown as { OPEN?: number } | undefined)?.OPEN ?? 1;

const mkOpenWs = (): MockWs => ({
  readyState: openWsState,
  send: vi.fn(),
});

describe("WorkspaceActiveSnapshotStore", () => {
  afterEach(() => {
    vi.clearAllMocks();
    resetUiDiagnosticsForTests();
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
    await asStoreInternals(store).handleStreamMessage(
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
    asStoreInternals(store).ws = ws;
    await asStoreInternals(store).handleStreamMessage(JSON.stringify({ type: "reset_required", latest_rev: 5 }));

    expect(getWorkspaceActiveSnapshot).not.toHaveBeenCalled();
    expect(ws.send).toHaveBeenCalled();
    store.destroy();
  });

  it("resubscribes on session_gap", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    const ws = mkOpenWs();
    asStoreInternals(store).ws = ws;
    await asStoreInternals(store).handleStreamMessage(
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
    const payload = JSON.parse(String(ws.send.mock.calls[0]?.[0] ?? "{}"));
    expect(payload.type).toBe("subscribe");
    expect(payload.include_active_heads).toBe(false);
  });

  it("keeps cache and render projections aligned with the shared active fixture", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { buildWorkbenchThreadViewModel } = await import("../pages/SessionPage");

    const fixture = getActiveProjectionFixture();
    const store = new WorkspaceActiveSnapshotStoreImpl(fixture.workspaceId, { disableWorker: true });

    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 4,
        active_snapshot: fixture.activeSnapshot,
        active_heads: fixture.activeHeads,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 4,
        event: {
          type: "session_head_delta",
          workspace_id: fixture.workspaceId,
          snapshot_rev: 4,
          delta: fixture.partialDelta,
        },
      }),
    );

    const snapshot = store.getSnapshot();
    expect(snapshot.activeIds).toEqual([fixture.task.id]);

    const summary = snapshot.tasksById[fixture.task.id]?.sessions[0];
    expect(summary?.last_event_seq).toBe(fixture.expected.summaryLastEventSeq);

    const head = store.getSessionHeadSnapshot(fixture.session.id);
    expect(head?.last_event_seq).toBe(fixture.expected.headLastEventSeq);
    expect((head?.events ?? []).map((event) => event.event_type)).toEqual(
      fixture.expected.stableEventTypes,
    );
    expect((head?.events ?? []).some((event) => event.event_type === "assistant_chunk")).toBe(false);

    const view = buildWorkbenchThreadViewModel(
      head?.turns ?? [],
      head?.messages ?? [],
      fixture.toolsByTurnId,
      head?.events ?? [],
    );
    const items = view.groups[0]?.items ?? [];
    expect(items.map((item) => item.kind)).toEqual(fixture.expected.renderItemKinds);
    expect(items.find((item) => item.kind === "assistant")).toMatchObject({
      kind: "assistant",
      content: fixture.expected.assistantContent,
    });
    expect(items.find((item) => item.kind === "tool")).toMatchObject({
      kind: "tool",
      tool_call_id: fixture.expected.toolCallId,
    });
  });

  it("rehydrates the seeded head from the shared gap fixture", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");

    const fixture = getSessionGapSeedFixture();
    const store = new WorkspaceActiveSnapshotStoreImpl(fixture.workspaceId, { disableWorker: true });

    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 4,
        active_snapshot: fixture.activeSnapshot,
        active_heads: fixture.activeHeads,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    const ws = mkOpenWs();
    asStoreInternals(store).ws = ws;

    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 5,
        event: fixture.gapEvent,
      }),
    );

    expect(ws.send).toHaveBeenCalledTimes(1);

    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 5,
        event: fixture.seedEvent,
      }),
    );

    const head = store.getSessionHeadSnapshot(fixture.session.id);
    expect(head?.last_event_seq).toBe(fixture.expected.headLastEventSeq);
    expect(head?.messages?.[1]?.content).toBe("Seeded response after gap.");
  });

  it("uses one canonical websocket url per connect cycle", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getDaemonClientConfig } = await import("../api/client");
    vi.mocked(getDaemonClientConfig).mockReturnValue({
      baseUrl: "http://daemon.local",
      wsBaseUrl: "ws://daemon.local",
      authToken: "token-1",
      runId: null,
    });
    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    const internals = asStoreInternals(store);
    const openSpy = vi.spyOn(internals, "openWebSocket").mockResolvedValueOnce(undefined);
    const reconnectSpy = vi.spyOn(internals, "scheduleReconnect").mockImplementation(() => {});

    await internals.connectStream();

    expect(openSpy).toHaveBeenCalledTimes(1);
    expect(openSpy).toHaveBeenCalledWith(
      "ws://daemon.local/api/workspaces/ws-1/active_snapshot/stream?token=token-1",
    );
    expect(reconnectSpy).not.toHaveBeenCalled();
    const diagnostics = getUiDiagnostics().filter((event) => event.code === "workspace.stream_connect_failed");
    expect(diagnostics).toHaveLength(0);
  });

  it("emits a single connect-failed diagnostic when canonical websocket connect fails", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getDaemonClientConfig } = await import("../api/client");
    vi.mocked(getDaemonClientConfig).mockReturnValue({
      baseUrl: "http://daemon.local",
      wsBaseUrl: "ws://daemon.local",
      authToken: null,
      runId: null,
    });
    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    const internals = asStoreInternals(store);
    vi.spyOn(internals, "openWebSocket").mockRejectedValueOnce(new Error("workspace active snapshot ws timeout"));
    const reconnectSpy = vi.spyOn(internals, "scheduleReconnect").mockImplementation(() => {});

    await internals.connectStream();

    expect(reconnectSpy).toHaveBeenCalledTimes(1);
    const diagnostics = getUiDiagnostics().filter((event) => event.code === "workspace.stream_connect_failed");
    expect(diagnostics).toHaveLength(1);
    const context = asRecord(diagnostics[0]?.context);
    expect(context.url).toBe("ws://daemon.local/api/workspaces/ws-1/active_snapshot/stream");
    expect(String(context.error ?? "")).toContain("timeout");
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
    await asStoreInternals(store).handleStreamMessage(
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

    await asStoreInternals(store).handleStreamMessage(
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
    asStoreInternals(store).ws = ws;
    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "event",
        rev: 1,
        event: { type: "ready", workspace_id: "ws-1", snapshot_rev: 1, archived_rev: 0 },
      }),
    );
    expect(getWorkspaceActiveSnapshot).not.toHaveBeenCalled();
    expect(ws.send).not.toHaveBeenCalled();

    vi.mocked(getWorkspaceActiveSnapshot).mockClear();
    ws.send.mockClear();
    await asStoreInternals(store).handleStreamMessage(
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
    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    await asStoreInternals(store).handleStreamMessage(
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

    const turn: SessionHeadSnapshot["turns"][number] = {
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
      turns: [turn],
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
    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
        active_heads: activeHeads,
      }),
    );
    await waitForCondition(() => store.getSnapshot().initialized);

    await asStoreInternals(store).handleStreamMessage(
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
    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    await asStoreInternals(store).handleStreamMessage(
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
    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    await asStoreInternals(store).handleStreamMessage(
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
    await asStoreInternals(store).handleStreamMessage(
      JSON.stringify({
        type: "snapshot",
        rev: 1,
        active_snapshot: activeSnapshot,
      }),
    );

    await waitForCondition(() => store.getSnapshot().initialized);

    const changed = asStoreInternals(store).applySessionSummaryDelta({
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

  it("rehydrates desktop connection before worker init when base URL is missing", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getDaemonClientConfig, syncDesktopDaemonConnectionFromBridge } = await import("../api/client");

    const previousTauri = (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
    const previousWorker = globalThis.Worker;

    class WorkerMock {
      static instances: WorkerMock[] = [];
      onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
      postMessage = vi.fn();
      terminate = vi.fn();
      constructor(..._args: unknown[]) {
        WorkerMock.instances.push(this);
      }
    }

    try {
      (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = {};
      vi.stubGlobal("Worker", WorkerMock as unknown as typeof Worker);
      let currentConfig: MockDaemonClientConfig = {
        baseUrl: null,
        wsBaseUrl: null,
        authToken: null,
        runId: null,
      };
      vi.mocked(getDaemonClientConfig).mockImplementation(() => currentConfig);
      vi.mocked(syncDesktopDaemonConnectionFromBridge).mockImplementation(async () => {
        currentConfig = {
          baseUrl: "http://daemon.local",
          wsBaseUrl: "ws://daemon.local",
          authToken: "token-1",
          runId: null,
        };
        return {
          config: {
            baseUrl: null,
            wsBaseUrl: null,
            authToken: null,
            runId: null,
          },
          info: {
            kind: "local",
            base_url: "http://daemon.local",
            token: "token-1",
          },
          synced: true,
          error: null,
        };
      });

      const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
      store.init();
      await waitForCondition(() => WorkerMock.instances.length === 1);
      const initCall = WorkerMock.instances[0]?.postMessage.mock.calls.find(
        ([msg]) => asRecord(msg).type === "init",
      );
      expect(initCall).toBeTruthy();
      expect(asRecord(initCall?.[0]).baseUrl).toBe("http://daemon.local");
      expect(asRecord(initCall?.[0]).authToken).toBe("token-1");
      expect(typeof asRecord(initCall?.[0]).connectionSeq).toBe("number");
      expect(syncDesktopDaemonConnectionFromBridge).toHaveBeenCalledTimes(1);
      const diagnostics = getUiDiagnostics().filter((event) => event.code === "workspace.worker_desktop_bridge_missing_base");
      expect(diagnostics).toHaveLength(0);
      store.destroy();
    } finally {
      if (previousWorker) {
        vi.stubGlobal("Worker", previousWorker);
      } else {
        Reflect.deleteProperty(globalThis as unknown as Record<string, unknown>, "Worker");
      }
      if (previousTauri === undefined) {
        delete (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
      } else {
        (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = previousTauri;
      }
    }
  });

  it("rehydrates desktop auth before worker init when only a persisted base is available", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getDaemonClientConfig, syncDesktopDaemonConnectionFromBridge } = await import("../api/client");

    const previousTauri = (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
    const previousWorker = globalThis.Worker;

    class WorkerMock {
      static instances: WorkerMock[] = [];
      onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
      postMessage = vi.fn();
      terminate = vi.fn();
      constructor(..._args: unknown[]) {
        WorkerMock.instances.push(this);
      }
    }

    try {
      (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = {};
      vi.stubGlobal("Worker", WorkerMock as unknown as typeof Worker);
      let currentConfig: MockDaemonClientConfig = {
        baseUrl: "http://daemon.local",
        wsBaseUrl: "ws://daemon.local",
        authToken: null,
        runId: null,
      };
      vi.mocked(getDaemonClientConfig).mockImplementation(() => currentConfig);
      vi.mocked(syncDesktopDaemonConnectionFromBridge).mockImplementation(async () => {
        currentConfig = {
          baseUrl: "http://daemon.local",
          wsBaseUrl: "ws://daemon.local",
          authToken: "token-1",
          runId: null,
        };
        return {
          config: {
            baseUrl: "http://daemon.local",
            wsBaseUrl: "ws://daemon.local",
            authToken: null,
            runId: null,
          },
          info: {
            kind: "local",
            base_url: "http://daemon.local",
            token: "token-1",
          },
          synced: true,
          error: null,
        };
      });

      const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
      store.init();
      await waitForCondition(() => WorkerMock.instances.length === 1);
      const initCall = WorkerMock.instances[0]?.postMessage.mock.calls.find(
        ([msg]) => asRecord(msg).type === "init",
      );
      expect(initCall).toBeTruthy();
      expect(asRecord(initCall?.[0]).baseUrl).toBe("http://daemon.local");
      expect(asRecord(initCall?.[0]).authToken).toBe("token-1");
      expect(syncDesktopDaemonConnectionFromBridge).toHaveBeenCalledTimes(1);
      store.destroy();
    } finally {
      if (previousWorker) {
        vi.stubGlobal("Worker", previousWorker);
      } else {
        Reflect.deleteProperty(globalThis as unknown as Record<string, unknown>, "Worker");
      }
      if (previousTauri === undefined) {
        delete (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
      } else {
        (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = previousTauri;
      }
    }
  });

  it("emits a desktop bridge invariant diagnostic when worker base URL stays missing", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getDaemonClientConfig, syncDesktopDaemonConnectionFromBridge } = await import("../api/client");

    const previousTauri = (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;

    try {
      (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = {};
      vi.mocked(getDaemonClientConfig).mockReturnValue({
        baseUrl: null,
        wsBaseUrl: null,
        authToken: null,
        runId: null,
      });
      vi.mocked(syncDesktopDaemonConnectionFromBridge).mockResolvedValue({
        config: {
          baseUrl: null,
          wsBaseUrl: null,
          authToken: null,
          runId: null,
        },
        info: {
          kind: "local",
          base_url: null,
          token: null,
        },
        synced: true,
        error: null,
      });

      const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
      await (store as unknown as { resolveWorkerConnectionState: (phase: "worker_init") => Promise<unknown> })
        .resolveWorkerConnectionState("worker_init");
      const diagnostics = getUiDiagnostics().filter((event) => event.code === "workspace.worker_desktop_bridge_missing_base");
      expect(diagnostics).toHaveLength(1);
      expect(asRecord(diagnostics[0]?.context).phase).toBe("worker_init");
      store.destroy();
    } finally {
      if (previousTauri === undefined) {
        delete (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
      } else {
        (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = previousTauri;
      }
    }
  });

  it("waits for desktop auth before starting the worker when bridge sync still lacks a token", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getDaemonClientConfig, syncDesktopDaemonConnectionFromBridge } = await import("../api/client");

    const previousTauri = (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
    const previousWorker = globalThis.Worker;

    class WorkerMock {
      static instances: WorkerMock[] = [];
      onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
      postMessage = vi.fn();
      terminate = vi.fn();
      constructor(..._args: unknown[]) {
        WorkerMock.instances.push(this);
      }
    }

    try {
      (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = {};
      vi.stubGlobal("Worker", WorkerMock as unknown as typeof Worker);
      vi.mocked(getDaemonClientConfig).mockReturnValue({
        baseUrl: "http://daemon.local",
        wsBaseUrl: "ws://daemon.local",
        authToken: null,
        runId: null,
      });
      vi.mocked(syncDesktopDaemonConnectionFromBridge).mockResolvedValue({
        config: {
          baseUrl: "http://daemon.local",
          wsBaseUrl: "ws://daemon.local",
          authToken: null,
          runId: null,
        },
        info: {
          kind: "local",
          base_url: "http://daemon.local",
          token: null,
        },
        synced: true,
        error: null,
      });

      const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
      store.init();
      await waitForCondition(
        () => getUiDiagnostics().some((event) => event.code === "workspace.worker_desktop_bridge_missing_auth"),
      );
      expect(WorkerMock.instances).toHaveLength(0);

      store.updateAuthConfig({
        authToken: "token-1",
        wsBaseUrl: "ws://daemon.local",
        baseUrl: "http://daemon.local",
        runId: null,
      });

      await waitForCondition(() => WorkerMock.instances.length === 1);
      const initCall = WorkerMock.instances[0]?.postMessage.mock.calls.find(
        ([msg]) => asRecord(msg).type === "init",
      );
      expect(initCall).toBeTruthy();
      expect(asRecord(initCall?.[0]).authToken).toBe("token-1");
      expect(syncDesktopDaemonConnectionFromBridge).toHaveBeenCalledTimes(1);
      store.destroy();
    } finally {
      if (previousWorker) {
        vi.stubGlobal("Worker", previousWorker);
      } else {
        Reflect.deleteProperty(globalThis as unknown as Record<string, unknown>, "Worker");
      }
      if (previousTauri === undefined) {
        delete (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
      } else {
        (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = previousTauri;
      }
    }
  });

  it("keeps the snapshot worker stopped until a canonical config update fills auth for a persisted base", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const {
      getDaemonClientConfig,
      subscribeDaemonConfig,
      syncDesktopDaemonConnectionFromBridge,
    } = await import("../api/client");

    type DaemonConfig = {
      baseUrl: string | null;
      wsBaseUrl: string | null;
      authToken: string | null;
      runId: string | null;
    };

    const previousTauri = (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
    const previousWorker = globalThis.Worker;
    let daemonConfig: DaemonConfig = {
      baseUrl: "http://daemon.local",
      wsBaseUrl: "ws://daemon.local",
      authToken: null,
      runId: null,
    };
    let configListener: unknown = null;
    const emitConfigListener = (config: DaemonConfig) => {
      if (typeof configListener !== "function") {
        throw new Error("Expected subscribeDaemonConfig listener to be registered.");
      }
      (configListener as (value: DaemonConfig) => void)(config);
    };

    class WorkerMock {
      static instances: WorkerMock[] = [];
      onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
      postMessage = vi.fn();
      terminate = vi.fn();
      constructor(..._args: unknown[]) {
        WorkerMock.instances.push(this);
      }
    }

    try {
      (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = {};
      vi.stubGlobal("Worker", WorkerMock as unknown as typeof Worker);
      vi.mocked(getDaemonClientConfig).mockImplementation(() => daemonConfig);
      vi.mocked(subscribeDaemonConfig).mockImplementation((listener) => {
        configListener = listener as (config: DaemonConfig) => void;
        return () => {
          configListener = null;
        };
      });
      vi.mocked(syncDesktopDaemonConnectionFromBridge).mockResolvedValue({
        config: daemonConfig,
        info: {
          kind: "local",
          base_url: "http://daemon.local",
          token: null,
        },
        synced: true,
        error: null,
      });

      const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
      store.init();

      await waitForCondition(
        () => getUiDiagnostics().some((event) => event.code === "workspace.worker_desktop_bridge_missing_auth"),
      );
      expect(WorkerMock.instances).toHaveLength(0);

      daemonConfig = {
        baseUrl: "http://daemon.local",
        wsBaseUrl: "ws://daemon.local",
        authToken: "token-1",
        runId: "run-1",
      };
      emitConfigListener(daemonConfig);

      await waitForCondition(() => WorkerMock.instances.length === 1);
      const initCall = WorkerMock.instances[0]?.postMessage.mock.calls.find(
        ([msg]) => asRecord(msg).type === "init",
      );
      expect(initCall).toBeTruthy();
      expect(asRecord(initCall?.[0])).toMatchObject({
        baseUrl: "http://daemon.local",
        wsBaseUrl: "ws://daemon.local",
        authToken: "token-1",
        runId: "run-1",
      });
      expect(syncDesktopDaemonConnectionFromBridge).toHaveBeenCalledTimes(1);
      store.destroy();
    } finally {
      if (previousWorker) {
        vi.stubGlobal("Worker", previousWorker);
      } else {
        Reflect.deleteProperty(globalThis as unknown as Record<string, unknown>, "Worker");
      }
      if (previousTauri === undefined) {
        delete (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
      } else {
        (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = previousTauri;
      }
    }
  });

  it("pushes later canonical base and token rotations into an already running worker", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const {
      getDaemonClientConfig,
      subscribeDaemonConfig,
      syncDesktopDaemonConnectionFromBridge,
    } = await import("../api/client");

    type DaemonConfig = {
      baseUrl: string | null;
      wsBaseUrl: string | null;
      authToken: string | null;
      runId: string | null;
    };

    const previousTauri = (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
    const previousWorker = globalThis.Worker;
    let daemonConfig: DaemonConfig = {
      baseUrl: "http://daemon.old",
      wsBaseUrl: "ws://daemon.old",
      authToken: "token-old",
      runId: "run-old",
    };
    let configListener: unknown = null;
    const emitConfigListener = (config: DaemonConfig) => {
      if (typeof configListener !== "function") {
        throw new Error("Expected subscribeDaemonConfig listener to be registered.");
      }
      (configListener as (value: DaemonConfig) => void)(config);
    };

    class WorkerMock {
      static instances: WorkerMock[] = [];
      onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
      postMessage = vi.fn();
      terminate = vi.fn();
      constructor(..._args: unknown[]) {
        WorkerMock.instances.push(this);
      }
    }

    try {
      (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = {};
      vi.stubGlobal("Worker", WorkerMock as unknown as typeof Worker);
      vi.mocked(getDaemonClientConfig).mockImplementation(() => daemonConfig);
      vi.mocked(subscribeDaemonConfig).mockImplementation((listener) => {
        configListener = listener as (config: DaemonConfig) => void;
        return () => {
          configListener = null;
        };
      });

      const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
      store.init();

      await waitForCondition(() => WorkerMock.instances.length === 1);
      const initCall = WorkerMock.instances[0]?.postMessage.mock.calls.find(
        ([msg]) => asRecord(msg).type === "init",
      );
      expect(initCall).toBeTruthy();
      expect(asRecord(initCall?.[0])).toMatchObject({
        baseUrl: "http://daemon.old",
        wsBaseUrl: "ws://daemon.old",
        authToken: "token-old",
        runId: "run-old",
      });

      daemonConfig = {
        baseUrl: "http://daemon.new",
        wsBaseUrl: "ws://daemon.new",
        authToken: "token-new",
        runId: "run-new",
      };
      emitConfigListener(daemonConfig);

      await waitForCondition(() => {
        const updateCalls = WorkerMock.instances[0]?.postMessage.mock.calls.filter(
          ([msg]) => asRecord(msg).type === "update_auth",
        );
        return Boolean(updateCalls && updateCalls.length === 1);
      });
      const updateCalls = WorkerMock.instances[0]?.postMessage.mock.calls.filter(
        ([msg]) => asRecord(msg).type === "update_auth",
      );
      expect(updateCalls).toHaveLength(1);
      expect(asRecord(updateCalls?.[0]?.[0])).toMatchObject({
        baseUrl: "http://daemon.new",
        wsBaseUrl: "ws://daemon.new",
        authToken: "token-new",
        runId: "run-new",
      });
      expect(syncDesktopDaemonConnectionFromBridge).not.toHaveBeenCalled();
      store.destroy();
    } finally {
      if (previousWorker) {
        vi.stubGlobal("Worker", previousWorker);
      } else {
        Reflect.deleteProperty(globalThis as unknown as Record<string, unknown>, "Worker");
      }
      if (previousTauri === undefined) {
        delete (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
      } else {
        (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = previousTauri;
      }
    }
  });

  it("drops stale worker auth updates when canonical desktop state resolves out of order", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { getDaemonClientConfig, syncDesktopDaemonConnectionFromBridge } = await import("../api/client");

    type BridgeSyncResult = {
      config: {
        baseUrl: string | null;
        wsBaseUrl: string | null;
        authToken: string | null;
        runId: string | null;
      };
      info: {
        kind: "none" | "local" | "ssh";
        base_url: string | null;
        token: string | null;
      } | null;
      synced: boolean;
      error: string | null;
    };

    const previousTauri = (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
    let resolveFirstSync!: (value: BridgeSyncResult) => void;
    const firstSyncPromise = new Promise<BridgeSyncResult>((resolve) => {
      resolveFirstSync = resolve;
    });
    const syncedResult: BridgeSyncResult = {
      config: {
        baseUrl: "http://daemon.local",
        wsBaseUrl: "ws://daemon.local",
        authToken: "bridge-token",
        runId: null,
      },
      info: {
        kind: "local",
        base_url: "http://daemon.local",
        token: "bridge-token",
      },
      synced: true,
      error: null,
    };

    try {
      (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = {};
      let currentConfig: MockDaemonClientConfig = {
        baseUrl: null,
        wsBaseUrl: null,
        authToken: null,
        runId: null,
      };
      vi.mocked(getDaemonClientConfig).mockImplementation(() => currentConfig);
      vi.mocked(syncDesktopDaemonConnectionFromBridge)
        .mockImplementationOnce(async () => firstSyncPromise)
        .mockImplementation(async () => {
          currentConfig = { ...syncedResult.config };
          return syncedResult;
        });

      const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
      const postMessage = vi.fn();
      (store as unknown as { worker: Worker | null }).worker = {
        postMessage,
        terminate: vi.fn(),
      } as unknown as Worker;

      store.updateAuthConfig({
        authToken: "token-old",
        wsBaseUrl: null,
        baseUrl: null,
        runId: "run-old",
      });
      store.updateAuthConfig({
        authToken: "token-new",
        wsBaseUrl: null,
        baseUrl: null,
        runId: "run-new",
      });

      await Promise.resolve();
      expect(postMessage).not.toHaveBeenCalled();
      currentConfig = { ...syncedResult.config };
      resolveFirstSync(syncedResult);

      await waitForCondition(() => {
        const updateCalls = postMessage.mock.calls.filter(([msg]) => asRecord(msg).type === "update_auth");
        return updateCalls.length === 1;
      });
      const updateCalls = postMessage.mock.calls.filter(([msg]) => asRecord(msg).type === "update_auth");
      expect(updateCalls).toHaveLength(1);
      expect(syncDesktopDaemonConnectionFromBridge).toHaveBeenCalledTimes(1);
      const update = asRecord(updateCalls[0]?.[0]);
      expect(update.authToken).toBe("token-new");
      expect(update.wsBaseUrl).toBe("ws://daemon.local");
      expect(update.baseUrl).toBe("http://daemon.local");
      expect(update.runId).toBe("run-new");
      expect(typeof update.connectionSeq).toBe("number");
      store.destroy();
    } finally {
      if (previousTauri === undefined) {
        delete (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__;
      } else {
        (globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__ = previousTauri;
      }
    }
  });

  it("emits archived load diagnostics with root cause details", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStoreCore");
    const { listWorkspaceArchivedTaskSummaries } = await import("../api/client");

    vi.mocked(listWorkspaceArchivedTaskSummaries).mockRejectedValueOnce(new Error("archived fetch boom"));
    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1", { disableWorker: true });
    store.ensureArchivedLoaded();

    await waitForCondition(() => store.getSnapshot().fetchState.archived === "error");
    const diagnostics = getUiDiagnostics().filter((event) => event.code === "workspace.archived_load_failed");
    expect(diagnostics).toHaveLength(1);
    expect(String(asRecord(diagnostics[0]?.context).error ?? "")).toContain("archived fetch boom");
    store.destroy();
  });
});
