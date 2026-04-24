import type {
  SessionHeadSnapshot,
  Task,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import {
  getDaemonConnectionReadiness,
  idToString,
  recordClientCounterMetric,
  recordClientHistogramMetric,
} from "../../api/client";
import { getAnalyticsSurface } from "../../utils/analytics/context";
import { getInstallId } from "../../utils/analytics/identity";
import {
  trackRendererHeartbeatMissed,
  trackWorkerPatchApply,
  trackWorkerPatchFlush,
} from "../../utils/analytics";
import { isDesktopApp } from "../../utils/desktop";
import { noteQueueAgeSample } from "../foregroundFreshnessTelemetry";
import type {
  WorkspaceActiveSnapshotCommand,
  WorkspaceActiveSnapshotPatch,
  WorkspaceActiveSnapshotWorkerMessage,
} from "../workspaceActiveSnapshotProtocol";
import type { SessionSubscriptionCursor } from "../sessionSubscription";
import {
  loadWorkspaceActiveSnapshotV1,
  saveWorkspaceActiveSnapshotV1,
  type PersistedWorkspaceActiveSnapshotV1,
} from "../uiStateStore";
import type { WorkspaceActiveSnapshotStoreState } from "./storeState";
import type { WorkspaceActiveSnapshotState } from "./storeTypes";
import {
  resolveWorkerConnectionState,
  type WorkerAuthUpdateConfig,
} from "./workerConnection";

const patchKindFor = (patch: WorkspaceActiveSnapshotPatch): "replace" | "diff" =>
  patch.snapshot ? "replace" : "diff";

const WORKER_PATCH_LOG_SAMPLE_INTERVAL = 25;
const WORKER_PATCH_LOG_EVENT_THRESHOLD = 16;
const WORKER_PATCH_LOG_AGE_THRESHOLD_MS = 100;

const shouldRecordWorkerPatch = (
  seq: number,
  eventCount: number,
  publishSnapshot: boolean,
  persist: boolean,
  oldestAgeMs: number | null,
): boolean => {
  if (eventCount >= WORKER_PATCH_LOG_EVENT_THRESHOLD) return true;
  if (publishSnapshot || persist) return true;
  if (typeof oldestAgeMs === "number" && oldestAgeMs >= WORKER_PATCH_LOG_AGE_THRESHOLD_MS) {
    return true;
  }
  return seq % WORKER_PATCH_LOG_SAMPLE_INTERVAL === 0;
};

const estimatePatchBytes = (patch: WorkspaceActiveSnapshotPatch): number => {
  try {
    return new Blob([JSON.stringify(patch)]).size;
  } catch {
    return 0;
  }
};

const nowMs = (): number => {
  if (typeof performance !== "undefined" && typeof performance.now === "function") {
    return (performance.timeOrigin ?? Date.now()) + performance.now();
  }
  return Date.now();
};

const sameIdList = (left: readonly string[], right: readonly string[]): boolean => {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
};

const sameRecordRefs = <T,>(left: Record<string, T>, right: Record<string, T>): boolean => {
  if (left === right) return true;
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) return false;
  for (const key of leftKeys) {
    if (!(key in right) || left[key] !== right[key]) {
      return false;
    }
  }
  return true;
};

const diffRecordEntries = <T,>(
  previous: Record<string, T>,
  next: Record<string, T>,
): { upserts?: Record<string, T>; deletes?: string[] } => {
  const upserts: Record<string, T> = {};
  const deletes: string[] = [];
  for (const [key, value] of Object.entries(next)) {
    if (!(key in previous) || previous[key] !== value) {
      upserts[key] = value;
    }
  }
  for (const key of Object.keys(previous)) {
    if (!(key in next)) {
      deletes.push(key);
    }
  }
  return {
    ...(Object.keys(upserts).length > 0 ? { upserts } : {}),
    ...(deletes.length > 0 ? { deletes } : {}),
  };
};

export type WorkspaceActiveSnapshotWorkerHost = {
  workspaceId: string;
  destroyed: boolean;
  disableWorker: boolean;
  e2eEnabled: boolean;
  authTokenOverride: string | null;
  wsBaseUrlOverride: string | null;
  state: WorkspaceActiveSnapshotStoreState;
  worker: Worker | null;
  workerStarting: boolean;
  workerAuthReconcileInFlight: boolean;
  pendingWorkerAuthUpdate: WorkerAuthUpdateConfig | null;
  workerConnectionSeq: number;
  useWorker: boolean;
  canonicalStreamUrl: string | null;
  workerPatchEmitter: ((patch: WorkspaceActiveSnapshotPatch) => void) | null;
  workerPatchTimer: ReturnType<typeof globalThis.setTimeout> | null;
  workerPatchPendingEvents: WorkspaceActiveSnapshotEvent[];
  workerPatchPendingPersist: boolean;
  workerPatchDirty: boolean;
  workerPatchOldestEventReceivedAtMs: number | null;
  workerPatchOldestForegroundEventReceivedAtMs: number | null;
  workerPatchFlushMs: number;
  workerPatchFlushSeq: number;
  lastWorkerPatchSnapshot: WorkspaceActiveSnapshotState | null;
  lastWorkerPatchSessionHeads: Record<string, SessionHeadSnapshot>;
  lastWorkerPatchWorktreeRoots: Record<string, string>;
  lastWorkerPatchSnapshotRev: number;
  cacheHydrated: boolean;
  pendingWorkerCache: PersistedWorkspaceActiveSnapshotV1 | null;
  cachePersistTimer: ReturnType<typeof globalThis.setTimeout> | null;
  subscribedSessions: SessionSubscriptionCursor[];
  vcsOpenSessionIds: string[];
  foregroundSessionId: string | null;
  ws: WebSocket | null;
  publish(): void;
  notifyEventListeners(evt: WorkspaceActiveSnapshotEvent): void;
  connectStream(): Promise<void>;
};

export const ensureWorkerAvailable = (host: WorkspaceActiveSnapshotWorkerHost): void => {
  if (host.disableWorker) return;
  if (typeof Worker === "undefined") {
    throw new Error("Workspace active snapshot requires Worker support.");
  }
};

export const postWorkerCommand = (
  host: WorkspaceActiveSnapshotWorkerHost,
  cmd: WorkspaceActiveSnapshotCommand,
): void => {
  if (!host.worker) return;
  host.worker.postMessage(cmd);
};

export const startWorker = async (host: WorkspaceActiveSnapshotWorkerHost): Promise<void> => {
  if (host.worker || host.destroyed || host.workerStarting) return;
  host.workerStarting = true;
  try {
    const connection = await resolveWorkerConnectionState({
      workspaceId: host.workspaceId,
      phase: "worker_init",
      authTokenOverride: host.authTokenOverride,
      wsBaseUrlOverride: host.wsBaseUrlOverride,
    });
    if (host.worker || host.destroyed) {
      return;
    }
    if (isDesktopApp() && !getDaemonConnectionReadiness(connection).isReady) {
      return;
    }
    host.useWorker = true;
    host.worker = new Worker(new URL("../../workers/workspaceActiveSnapshot.worker.ts", import.meta.url), {
      type: "module",
    });
    host.worker.onmessage = (event: MessageEvent<WorkspaceActiveSnapshotWorkerMessage>) => {
      const msg = event.data;
      if (!msg) return;
      if (msg.type === "patch") {
        applyWorkerPatch(host, msg.patch);
        return;
      }
      if (msg.type === "heartbeat_ping") {
        postWorkerCommand(host, { type: "heartbeat_ack", token: msg.token });
        return;
      }
      if (msg.type === "heartbeat_missed") {
        trackRendererHeartbeatMissed({
          source: "workspace_active_worker",
          missedForMs: msg.missedForMs,
          outstandingAcks: msg.outstandingAcks,
        });
      }
    };
    postWorkerCommand(host, {
      type: "init",
      workspaceId: host.workspaceId,
      connectionSeq: ++host.workerConnectionSeq,
      authToken: connection.authToken,
      baseUrl: connection.baseUrl,
      wsBaseUrl: connection.wsBaseUrl || null,
      runId: connection.runId,
      installId: getInstallId(),
      originRuntime: getAnalyticsSurface(),
      e2eEnabled: host.e2eEnabled,
    });
    if (host.subscribedSessions.length > 0) {
      postWorkerCommand(host, {
        type: "set_subscribed_sessions",
        sessions: host.subscribedSessions.slice(),
      });
    }
    if (host.vcsOpenSessionIds.length > 0) {
      postWorkerCommand(host, {
        type: "set_vcs_open_session_ids",
        sessionIds: host.vcsOpenSessionIds.slice(),
      });
    }
    if (host.foregroundSessionId) {
      postWorkerCommand(host, {
        type: "set_foreground_session_id",
        sessionId: host.foregroundSessionId,
      });
    }
    if (host.pendingWorkerCache) {
      postWorkerCommand(host, { type: "seed_cache", snapshot: host.pendingWorkerCache });
      host.pendingWorkerCache = null;
    }
  } finally {
    host.workerStarting = false;
  }
};

export const queueWorkerAuthUpdate = (
  host: WorkspaceActiveSnapshotWorkerHost,
  opts: WorkerAuthUpdateConfig,
): void => {
  host.pendingWorkerAuthUpdate = {
    authToken: opts.authToken ?? null,
    wsBaseUrl: opts.wsBaseUrl ?? null,
    baseUrl: opts.baseUrl,
    runId: opts.runId ?? null,
  };
  if (host.workerAuthReconcileInFlight || host.destroyed) return;
  host.workerAuthReconcileInFlight = true;
  void runWorkerAuthReconcileLoop(host);
};

const runWorkerAuthReconcileLoop = async (
  host: WorkspaceActiveSnapshotWorkerHost,
): Promise<void> => {
  try {
    while (!host.destroyed) {
      const pending = host.pendingWorkerAuthUpdate;
      if (!pending) return;
      host.pendingWorkerAuthUpdate = null;
      const connection = await resolveWorkerConnectionState({
        workspaceId: host.workspaceId,
        phase: "worker_update_auth",
        authTokenOverride: host.authTokenOverride,
        wsBaseUrlOverride: host.wsBaseUrlOverride,
        opts: pending,
      });
      if (!host.worker || host.destroyed) return;
      if (host.pendingWorkerAuthUpdate) {
        continue;
      }
      postWorkerCommand(host, {
        type: "update_auth",
        connectionSeq: ++host.workerConnectionSeq,
        authToken: connection.authToken,
        baseUrl: connection.baseUrl,
        wsBaseUrl: connection.wsBaseUrl,
        runId: connection.runId,
      });
    }
  } finally {
    host.workerAuthReconcileInFlight = false;
    if (!host.destroyed && host.pendingWorkerAuthUpdate) {
      host.workerAuthReconcileInFlight = true;
      void runWorkerAuthReconcileLoop(host);
    }
  }
};

export const updateAuthConfig = (
  host: WorkspaceActiveSnapshotWorkerHost,
  opts: {
    authToken?: string | null;
    wsBaseUrl?: string | null;
    baseUrl?: string | null;
    runId?: string | null;
  },
): void => {
  const nextAuth = opts.authToken ?? null;
  const nextWs = opts.wsBaseUrl ?? null;
  const authChanged = host.authTokenOverride !== nextAuth;
  const wsChanged = host.wsBaseUrlOverride !== nextWs;
  host.authTokenOverride = nextAuth;
  host.wsBaseUrlOverride = nextWs;
  if (authChanged || wsChanged) {
    host.canonicalStreamUrl = null;
  }

  if (host.worker) {
    queueWorkerAuthUpdate(host, {
      authToken: nextAuth,
      wsBaseUrl: nextWs,
      baseUrl: opts.baseUrl,
      runId: opts.runId ?? null,
    });
    return;
  }

  if (!host.disableWorker) {
    if (!host.destroyed) {
      void startWorker(host).catch(() => {});
    }
    return;
  }
  if (!authChanged && !wsChanged) return;
  if (host.ws) {
    try {
      host.ws.close();
    } catch {
      // ignore
    }
    host.ws = null;
    if (host.state.setConnection("disconnected")) {
      host.publish();
    }
  }
  if (!host.destroyed) {
    void host.connectStream().catch(() => {});
  }
};

export const applyWorkerPatch = (
  host: WorkspaceActiveSnapshotWorkerHost,
  patch: WorkspaceActiveSnapshotPatch,
): void => {
  if (host.destroyed) return;
  const appliedAtMs = nowMs();
  const oldestEventAgeMs =
    typeof patch.oldestEventReceivedAtMs === "number"
      ? Math.max(0, appliedAtMs - patch.oldestEventReceivedAtMs)
      : null;
  const oldestForegroundEventAgeMs =
    typeof patch.oldestForegroundEventReceivedAtMs === "number"
      ? Math.max(0, appliedAtMs - patch.oldestForegroundEventReceivedAtMs)
      : null;
  const patchKind = patchKindFor(patch);
  if (typeof patch.oldestEventReceivedAtMs === "number") {
    noteQueueAgeSample("workspace", oldestEventAgeMs ?? 0, {
      source: "worker_patch",
    });
  }
  if (typeof patch.oldestForegroundEventReceivedAtMs === "number") {
    noteQueueAgeSample("foreground", oldestForegroundEventAgeMs ?? 0, {
      source: "worker_patch",
    });
  }
  const applyStartedAtMs = nowMs();
  host.state.applyWorkerPatch(patch);
  recordClientHistogramMetric(
    "workspace.active_snapshot.worker_patch_apply_ms",
    "ms",
    Math.max(0, nowMs() - applyStartedAtMs),
    {
      patch_kind: patchKind,
    },
  );
  recordClientHistogramMetric(
    "workspace.active_snapshot.worker_patch_event_count",
    "count",
    patch.events.length,
    {
      patch_kind: patchKind,
    },
  );
  if (patch.publishSnapshot !== false) {
    host.publish();
  }
  if (patch.persist) {
    schedulePersistCache(host);
  }
  for (const event of patch.events) {
    host.notifyEventListeners(event);
  }
  const applyDurationMs = Math.max(0, nowMs() - applyStartedAtMs);
  if (
    shouldRecordWorkerPatch(
      host.workerPatchFlushSeq,
      patch.events.length,
      patch.publishSnapshot !== false,
      patch.persist,
      oldestEventAgeMs,
    ) || applyDurationMs >= 250
  ) {
    trackWorkerPatchApply({
      source: "worker_patch",
      eventCount: patch.events.length,
      applyDurationMs,
      publishSnapshot: patch.publishSnapshot !== false,
      persist: patch.persist,
      oldestEventAgeStartMs: oldestEventAgeMs,
      oldestForegroundEventAgeStartMs: oldestForegroundEventAgeMs,
    });
  }
};

export const destroyWorkerRuntime = (host: WorkspaceActiveSnapshotWorkerHost): void => {
  host.pendingWorkerAuthUpdate = null;
  host.workerAuthReconcileInFlight = false;
  host.workerConnectionSeq = 0;
  host.useWorker = false;
  host.workerStarting = false;
  host.pendingWorkerCache = null;
  if (host.worker) {
    host.worker.terminate();
    host.worker = null;
  }
  if (host.cachePersistTimer) {
    globalThis.clearTimeout(host.cachePersistTimer);
    host.cachePersistTimer = null;
  }
  if (host.workerPatchTimer) {
    globalThis.clearTimeout(host.workerPatchTimer);
    host.workerPatchTimer = null;
  }
  host.workerPatchPendingEvents = [];
  host.workerPatchPendingPersist = false;
  host.workerPatchDirty = false;
  host.workerPatchOldestEventReceivedAtMs = null;
  host.workerPatchOldestForegroundEventReceivedAtMs = null;
  host.lastWorkerPatchSnapshot = null;
  host.lastWorkerPatchSessionHeads = {};
  host.lastWorkerPatchWorktreeRoots = {};
  host.lastWorkerPatchSnapshotRev = -1;
};

export const applyTaskUpdate = (
  host: WorkspaceActiveSnapshotWorkerHost,
  task: Task,
): void => {
  if (host.worker) {
    postWorkerCommand(host, { type: "apply_task_update", task });
    return;
  }
  if (!host.state.applyTaskUpdate(task)) return;
  host.publish();
  schedulePersistCache(host);
};

export const seedCachedSnapshot = (
  host: WorkspaceActiveSnapshotWorkerHost,
  cached: PersistedWorkspaceActiveSnapshotV1,
): void => {
  if (!cached || host.destroyed) return;
  try {
    host.state.applyCachedSnapshot(cached);
    host.publish();
  } catch {
    // ignore invalid cache payloads
  }
};

export const hydrateFromCache = async (
  host: WorkspaceActiveSnapshotWorkerHost,
): Promise<void> => {
  if (host.cacheHydrated) return;
  host.cacheHydrated = true;
  try {
    const cached = await loadWorkspaceActiveSnapshotV1(host.workspaceId);
    if (!cached || host.destroyed || host.state.hasLiveSnapshotApplied()) return;
    host.state.applyCachedSnapshot(cached);
    host.publish();
    if (host.worker) {
      postWorkerCommand(host, { type: "seed_cache", snapshot: cached });
    } else if (!host.disableWorker) {
      host.pendingWorkerCache = cached;
    }
  } catch {
    // ignore cache errors
  }
};

export const schedulePersistCache = (host: WorkspaceActiveSnapshotWorkerHost): void => {
  if (host.workerPatchEmitter) {
    host.workerPatchPendingPersist = true;
    scheduleWorkerPatchFlush(host);
    return;
  }
  if (host.destroyed || host.cachePersistTimer) return;
  host.cachePersistTimer = globalThis.setTimeout(() => {
    host.cachePersistTimer = null;
    void persistCache(host);
  }, 300);
};

export const scheduleWorkerPatchFlush = (host: WorkspaceActiveSnapshotWorkerHost): void => {
  if (!host.workerPatchEmitter || host.workerPatchTimer) return;
  host.workerPatchTimer = globalThis.setTimeout(() => {
    host.workerPatchTimer = null;
    flushWorkerPatch(host);
  }, host.workerPatchFlushMs);
};

export const flushWorkerPatchNow = (host: WorkspaceActiveSnapshotWorkerHost): void => {
  if (host.workerPatchTimer) {
    globalThis.clearTimeout(host.workerPatchTimer);
    host.workerPatchTimer = null;
  }
  flushWorkerPatch(host);
};

const flushWorkerPatch = (host: WorkspaceActiveSnapshotWorkerHost): void => {
  if (!host.workerPatchEmitter) return;
  if (
    !host.workerPatchDirty &&
    !host.workerPatchPendingPersist &&
    host.workerPatchPendingEvents.length === 0
  ) {
    return;
  }
  host.workerPatchFlushSeq += 1;
  const flushStartedAtMs = nowMs();
  const events = host.workerPatchPendingEvents.slice();
  host.workerPatchPendingEvents = [];
  const snapshot = host.state.getSnapshot();
  const sessionHeads = host.state.getSessionHeadsSnapshot();
  const worktreeRoots = host.state.getWorktreeRootsSnapshot();
  const activeSessionIds = host.state.getActiveSessionIds();
  const snapshotRev = host.state.getSnapshotRev();
  const forceSnapshotReplace =
    host.lastWorkerPatchSnapshot == null || snapshotRev < host.lastWorkerPatchSnapshotRev;

  let patch: WorkspaceActiveSnapshotPatch;
  if (forceSnapshotReplace) {
    patch = {
      snapshot,
      sessionHeadUpserts: sessionHeads,
      worktreeRootUpserts: worktreeRoots,
      events,
      snapshotRev,
      archivedRev: host.state.getArchivedRev(),
      activeSessionIds,
      publishSnapshot: true,
      persist: host.workerPatchPendingPersist,
      oldestEventReceivedAtMs: host.workerPatchOldestEventReceivedAtMs,
      oldestForegroundEventReceivedAtMs: host.workerPatchOldestForegroundEventReceivedAtMs,
    };
  } else {
    const previousSnapshot = host.lastWorkerPatchSnapshot!;
    const previousSessionHeads = host.lastWorkerPatchSessionHeads;
    const previousWorktreeRoots = host.lastWorkerPatchWorktreeRoots;
    const taskDiff = diffRecordEntries(previousSnapshot.tasksById, snapshot.tasksById);
    const sessionHeadDiff = diffRecordEntries(previousSessionHeads, sessionHeads);
    const worktreeRootDiff = diffRecordEntries(previousWorktreeRoots, worktreeRoots);
    const shell: NonNullable<WorkspaceActiveSnapshotPatch["shell"]> = {};

    if (snapshot.initialized !== previousSnapshot.initialized) {
      shell.initialized = snapshot.initialized;
    }
    if (snapshot.liveSnapshotApplied !== previousSnapshot.liveSnapshotApplied) {
      shell.liveSnapshotApplied = snapshot.liveSnapshotApplied;
    }
    if (snapshot.connection !== previousSnapshot.connection) {
      shell.connection = snapshot.connection;
    }
    if (!sameIdList(snapshot.activeIds, previousSnapshot.activeIds)) {
      shell.activeIds = snapshot.activeIds.slice();
    }
    if (!sameIdList(snapshot.archivedIds, previousSnapshot.archivedIds)) {
      shell.archivedIds = snapshot.archivedIds.slice();
    }
    if (snapshot.totalActive !== previousSnapshot.totalActive) {
      shell.totalActive = snapshot.totalActive;
    }
    if (snapshot.totalArchived !== previousSnapshot.totalArchived) {
      shell.totalArchived = snapshot.totalArchived;
    }
    if (snapshot.archivedRev !== previousSnapshot.archivedRev) {
      shell.archivedRev = snapshot.archivedRev;
    }
    if (
      snapshot.fetchState.active !== previousSnapshot.fetchState.active ||
      snapshot.fetchState.archived !== previousSnapshot.fetchState.archived
    ) {
      shell.fetchState = { ...snapshot.fetchState };
    }
    if (snapshot.hasMoreActive !== previousSnapshot.hasMoreActive) {
      shell.hasMoreActive = snapshot.hasMoreActive;
    }
    if (snapshot.hasMoreArchived !== previousSnapshot.hasMoreArchived) {
      shell.hasMoreArchived = snapshot.hasMoreArchived;
    }
    if (snapshot.archivedLoaded !== previousSnapshot.archivedLoaded) {
      shell.archivedLoaded = snapshot.archivedLoaded;
    }
    if (!sameRecordRefs(snapshot.worktreeVcsById, previousSnapshot.worktreeVcsById)) {
      shell.worktreeVcsById = snapshot.worktreeVcsById;
    }

    const shellChanged = Object.keys(shell).length > 0;
    const taskChanged = Boolean(taskDiff.upserts) || Boolean(taskDiff.deletes);
    patch = {
      ...(shellChanged ? { shell } : {}),
      ...(taskDiff.upserts ? { taskUpserts: taskDiff.upserts } : {}),
      ...(taskDiff.deletes ? { taskDeletes: taskDiff.deletes } : {}),
      ...(sessionHeadDiff.upserts ? { sessionHeadUpserts: sessionHeadDiff.upserts } : {}),
      ...(sessionHeadDiff.deletes ? { sessionHeadDeletes: sessionHeadDiff.deletes } : {}),
      ...(worktreeRootDiff.upserts ? { worktreeRootUpserts: worktreeRootDiff.upserts } : {}),
      ...(worktreeRootDiff.deletes ? { worktreeRootDeletes: worktreeRootDiff.deletes } : {}),
      events,
      snapshotRev,
      archivedRev: host.state.getArchivedRev(),
      activeSessionIds,
      publishSnapshot: host.workerPatchDirty || shellChanged || taskChanged,
      persist: host.workerPatchPendingPersist,
      oldestEventReceivedAtMs: host.workerPatchOldestEventReceivedAtMs,
      oldestForegroundEventReceivedAtMs: host.workerPatchOldestForegroundEventReceivedAtMs,
    };
  }

  const patchKind = patchKindFor(patch);
  host.workerPatchDirty = false;
  host.workerPatchPendingPersist = false;
  host.workerPatchOldestEventReceivedAtMs = null;
  host.workerPatchOldestForegroundEventReceivedAtMs = null;
  host.lastWorkerPatchSnapshot = snapshot;
  host.lastWorkerPatchSessionHeads = sessionHeads;
  host.lastWorkerPatchWorktreeRoots = worktreeRoots;
  host.lastWorkerPatchSnapshotRev = snapshotRev;
  const oldestEventAgeMs =
    typeof patch.oldestEventReceivedAtMs === "number"
      ? Math.max(0, nowMs() - patch.oldestEventReceivedAtMs)
      : null;
  const oldestForegroundEventAgeMs =
    typeof patch.oldestForegroundEventReceivedAtMs === "number"
      ? Math.max(0, nowMs() - patch.oldestForegroundEventReceivedAtMs)
      : null;
  if (
    shouldRecordWorkerPatch(
      host.workerPatchFlushSeq,
      patch.events.length,
      patch.publishSnapshot !== false,
      patch.persist,
      oldestEventAgeMs,
    )
  ) {
    trackWorkerPatchFlush({
      source: "worker_patch",
      eventCount: patch.events.length,
      activeSessionCount: patch.activeSessionIds.length,
      publishSnapshot: patch.publishSnapshot !== false,
      persist: patch.persist,
      patchBytesEstimate: estimatePatchBytes(patch),
      oldestEventAgeMs,
      oldestForegroundEventAgeMs,
    });
  }
  recordClientCounterMetric("workspace.active_snapshot.worker_patch_flush_count", {
    patch_kind: patchKind,
    persist: patch.persist ? "true" : "false",
  });
  recordClientHistogramMetric(
    "workspace.active_snapshot.worker_patch_flush_ms",
    "ms",
    Math.max(0, nowMs() - flushStartedAtMs),
    {
      patch_kind: patchKind,
      persist: patch.persist ? "true" : "false",
    },
  );
  host.workerPatchEmitter(patch);
};

const persistCache = async (host: WorkspaceActiveSnapshotWorkerHost): Promise<void> => {
  if (host.destroyed) return;
  try {
    await saveWorkspaceActiveSnapshotV1(host.workspaceId, host.state.buildPersistedSnapshot());
  } catch {
    // ignore cache errors
  }
};

export const isForegroundSessionEvent = (
  foregroundSessionId: string | null,
  evt: WorkspaceActiveSnapshotEvent,
): boolean => {
  const normalizedForegroundSessionId = String(foregroundSessionId ?? "").trim();
  if (!normalizedForegroundSessionId) return false;
  switch (evt.type) {
    case "session_head_delta":
      return idToString(evt.delta.session_id) === normalizedForegroundSessionId;
    case "session_summary":
      return idToString(evt.summary.session.id) === normalizedForegroundSessionId;
    case "session_summary_delta":
      return idToString(evt.delta.session_id) === normalizedForegroundSessionId;
    case "session_gap":
      return idToString(evt.session_id) === normalizedForegroundSessionId;
    default:
      return false;
  }
};
