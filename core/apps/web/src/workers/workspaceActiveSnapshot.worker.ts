import type { WorkspaceArchivedPage, WorkspaceIndexCursor } from "@ctx/types";
import { workerFetchJson, setWorkerClientConfig } from "../api/workerClient";
import { WorkspaceActiveSnapshotStoreImpl } from "../state/workspaceActiveSnapshotStoreCore";
import type {
  WorkspaceActiveSnapshotCommand,
  WorkspaceActiveSnapshotWorkerMessage,
} from "../state/workspaceActiveSnapshotProtocol";
import type { PersistedWorkspaceActiveSnapshotV1 } from "../state/uiStateStore";

const setAuth = (baseUrl?: string | null, authToken?: string | null, runId?: string | null) => {
  setWorkerClientConfig({ baseUrl, authToken, runId });
};

const idToString = (id: string | null | undefined): string => {
  if (id === null || id === undefined) return "";
  if (typeof id !== "string") {
    throw new Error("Expected id to be a string");
  }
  return id;
};

let latestConnectionSeq = -1;

const listWorkspaceArchivedTaskSummaries = (
  workspaceId: string,
  params?: { limit?: number; cursor?: WorkspaceIndexCursor | null },
): Promise<WorkspaceArchivedPage> => {
  const search = new URLSearchParams();
  if (params?.limit) search.set("limit", String(params.limit));
  if (params?.cursor) {
    const cursorSortAt = String(params.cursor.sort_at ?? "").trim();
    const cursorTaskId = idToString(params.cursor.task_id);
    if (cursorSortAt) search.set("cursor_sort_at", cursorSortAt);
    if (cursorTaskId) search.set("cursor_task_id", cursorTaskId);
  }
  const qs = search.toString();
  const suffix = qs ? `?${qs}` : "";
  const path = `/api/workspaces/${workspaceId}/archived_task_summaries${suffix}`;
  return workerFetchJson<WorkspaceArchivedPage>(path);
};

let store: WorkspaceActiveSnapshotStoreImpl | null = null;
let pendingSeed: PersistedWorkspaceActiveSnapshotV1 | null = null;
let pendingSubscribedSessionIds: string[] | null = null;
let pendingForegroundTaskId: string | null = null;

const ensureStore = (cmd: Extract<WorkspaceActiveSnapshotCommand, { type: "init" }>) => {
  if (cmd.connectionSeq < latestConnectionSeq) return;
  latestConnectionSeq = cmd.connectionSeq;
  setAuth(cmd.baseUrl, cmd.authToken, cmd.runId);
  if (store) return;
  store = new WorkspaceActiveSnapshotStoreImpl(cmd.workspaceId, {
    disableCache: true,
    disableWorker: true,
    authToken: cmd.authToken ?? null,
    wsBaseUrl: cmd.wsBaseUrl ?? null,
    e2eEnabled: cmd.e2eEnabled ?? false,
    listWorkspaceArchivedTaskSummaries,
    onPatch: (patch) => {
      const message: WorkspaceActiveSnapshotWorkerMessage = { type: "patch", patch };
      self.postMessage(message);
    },
  });
  store.init();
  if (pendingSeed) {
    store.seedCachedSnapshot(pendingSeed);
    pendingSeed = null;
  }
  if (pendingSubscribedSessionIds) {
    store.setSubscribedSessionIds?.(pendingSubscribedSessionIds);
    pendingSubscribedSessionIds = null;
  }
  if (pendingForegroundTaskId !== null) {
    store.setForegroundTaskId?.(pendingForegroundTaskId);
    pendingForegroundTaskId = null;
  }
};

self.onmessage = (event: MessageEvent<WorkspaceActiveSnapshotCommand>) => {
  const cmd = event.data;
  if (!cmd) return;
  switch (cmd.type) {
    case "init":
      ensureStore(cmd);
      return;
    case "update_auth":
      if (cmd.connectionSeq < latestConnectionSeq) return;
      latestConnectionSeq = cmd.connectionSeq;
      setAuth(cmd.baseUrl, cmd.authToken, cmd.runId);
      store?.updateAuthConfig({
        authToken: cmd.authToken ?? null,
        wsBaseUrl: cmd.wsBaseUrl ?? null,
      });
      return;
    case "seed_cache":
      if (!store) {
        pendingSeed = cmd.snapshot;
        return;
      }
      store.seedCachedSnapshot(cmd.snapshot);
      return;
    case "set_subscribed_session_ids":
      if (!store) {
        pendingSubscribedSessionIds = cmd.sessionIds;
        return;
      }
      store.setSubscribedSessionIds?.(cmd.sessionIds);
      return;
    case "set_foreground_task_id":
      if (!store) {
        pendingForegroundTaskId = cmd.taskId;
        return;
      }
      store.setForegroundTaskId?.(cmd.taskId);
      return;
    case "ensure_archived_loaded":
      store?.ensureArchivedLoaded();
      return;
    case "load_more_archived":
      store?.loadMoreArchived();
      return;
    case "apply_task_update":
      store?.applyTaskUpdate(cmd.task);
      return;
    case "e2e_set_enabled":
      store?.setE2EEnabled(cmd.enabled);
      return;
    case "e2e_close_stream":
      store?.e2eCloseActiveSnapshotStream();
      return;
    case "e2e_set_drop_messages":
      store?.e2eSetDropActiveSnapshotMessages(cmd.drop);
      return;
    case "e2e_dispatch_stream_message":
      store?.e2eDispatchActiveSnapshotStreamMessage(cmd.payload);
      return;
    default:
      return;
  }
};
