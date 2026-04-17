import type {
  SessionHeadSnapshot,
  Task,
  WorkspaceActiveSnapshot,
  WorktreeVcsSnapshot,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import {
  getDaemonClientConfig,
  getDaemonConnectionReadiness,
  subscribeDaemonConfig,
  idToString,
  listWorkspaceArchivedTaskSummaries,
} from "../api/client";
import {
  loadWorkspaceActiveSnapshotV1,
  saveWorkspaceActiveSnapshotV1,
  type PersistedWorkspaceActiveSnapshotV1,
  type PersistedWorkspaceActiveTaskSummaryV1,
} from "./uiStateStore";
import { isDesktopApp } from "../utils/desktop";
import { emitUiDiagnostic, normalizeDiagnosticErrorMessage } from "./diagnosticsChannel";
import type {
  WorkspaceActiveSnapshotCommand,
  WorkspaceActiveSnapshotPatch,
  WorkspaceActiveSnapshotWorkerMessage,
} from "./workspaceActiveSnapshotProtocol";
import type { SessionSubscriptionCursor } from "./sessionSubscription";
import { WorkspaceActiveSnapshotStoreState } from "./workspaceActiveSnapshot/storeState";
import { noteQueueAgeSample, noteWorkspaceStreamReset } from "./foregroundFreshnessTelemetry";
import type {
  WorkspaceActiveSnapshotEventSource,
  WorkspaceActiveSnapshotItem,
  WorkspaceActiveSnapshotState,
} from "./workspaceActiveSnapshot/storeTypes";
import {
  closeActiveSnapshotStream,
  flushSubscriptions as flushActiveSnapshotSubscriptions,
  getCanonicalStreamUrl,
  notifyEventListeners,
  setDropActiveSnapshotMessages,
  setE2EEnabled,
  setForegroundSessionId,
  setSubscribedSessions,
  unwrapEvent,
  type WorkspaceActiveSnapshotControlHost,
} from "./workspaceActiveSnapshot/controls";
import {
  applySessionSummaryDelta as applyWorkspaceSessionSummaryDelta,
  applyWorkspaceSnapshot as applyWorkspaceStreamSnapshot,
  connectStream as connectWorkspaceStream,
  enqueueStreamMessage as enqueueWorkspaceStreamMessage,
  fetchArchivedPage as fetchArchivedWorkspacePage,
  handleStreamMessage as handleWorkspaceStreamMessage,
  openWebSocket as openWorkspaceStreamWebSocket,
  scheduleReconnect as scheduleWorkspaceStreamReconnect,
  type WorkspaceActiveSnapshotStreamHost,
} from "./workspaceActiveSnapshot/streamRuntime";
import {
  resolveWorkerConnectionState,
  type WorkerAuthUpdateConfig,
} from "./workspaceActiveSnapshot/workerConnection";
export type {
  WorkspaceActiveSnapshotEventSource,
  WorkspaceActiveSnapshotItem,
  WorkspaceActiveSnapshotState,
} from "./workspaceActiveSnapshot/storeTypes";

type WorkspaceActiveSnapshotStoreOptions = {
  disableCache?: boolean;
  disableWorker?: boolean;
  e2eEnabled?: boolean;
  onPersistRequested?: () => void;
  onPatch?: (patch: WorkspaceActiveSnapshotPatch) => void;
  patchFlushMs?: number;
  authToken?: string | null;
  wsBaseUrl?: string | null;
  listWorkspaceArchivedTaskSummaries?: typeof listWorkspaceArchivedTaskSummaries;
};

const SNAPSHOT_WAIT_MS = 1200;
const WORKSPACE_PATCH_FLUSH_MS = 50;

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

export class WorkspaceActiveSnapshotStoreImpl implements WorkspaceActiveSnapshotEventSource {
  private listeners = new Set<() => void>();
  eventListeners = new Set<(event: WorkspaceActiveSnapshotEvent) => void>();
  readonly state: WorkspaceActiveSnapshotStoreState;
  worker: Worker | null = null;
  private workerStarting = false;
  private workerAuthReconcileInFlight = false;
  private pendingWorkerAuthUpdate: WorkerAuthUpdateConfig | null = null;
  private workerConnectionSeq = 0;
  private useWorker = false;
  private disableCache = false;
  private disableWorker = false;
  e2eEnabled = false;
  e2eDropStreamMessages = false;
  private persistNotifier: (() => void) | null = null;
  workerPatchEmitter: ((patch: WorkspaceActiveSnapshotPatch) => void) | null = null;
  private workerPatchTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  workerPatchPendingEvents: WorkspaceActiveSnapshotEvent[] = [];
  private workerPatchPendingPersist = false;
  private workerPatchDirty = false;
  private workerPatchOldestEventReceivedAtMs: number | null = null;
  private workerPatchOldestForegroundEventReceivedAtMs: number | null = null;
  private workerPatchFlushMs = WORKSPACE_PATCH_FLUSH_MS;
  private lastWorkerPatchSnapshot: WorkspaceActiveSnapshotState | null = null;
  private lastWorkerPatchSessionHeads: Record<string, SessionHeadSnapshot> = {};
  private lastWorkerPatchWorktreeRoots: Record<string, string> = {};
  private lastWorkerPatchSnapshotRev = -1;
  authTokenOverride: string | null = null;
  wsBaseUrlOverride: string | null = null;
  private configUnsubscribe: (() => void) | null = null;
  private listWorkspaceArchivedTaskSummariesFn: typeof listWorkspaceArchivedTaskSummaries;
  subscribedSessions: SessionSubscriptionCursor[] = [];
  foregroundSessionId: string | null = null;
  ws: WebSocket | null = null;
  private connecting = false;
  private reconnectTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private reconnectDelayMs = 1000;
  private lastStreamSeq = 0;
  private allowSnapshotReset = false;
  private cacheHydrated = false;
  private pendingWorkerCache: PersistedWorkspaceActiveSnapshotV1 | null = null;
  private snapshotWaitTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private cachePersistTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private streamQueue: Promise<void> = Promise.resolve();
  private destroyed = false;

  constructor(readonly workspaceId: string, opts?: WorkspaceActiveSnapshotStoreOptions) {
    this.state = new WorkspaceActiveSnapshotStoreState(workspaceId);
    this.disableCache = opts?.disableCache ?? false;
    this.disableWorker = opts?.disableWorker ?? false;
    this.e2eEnabled = opts?.e2eEnabled ?? false;
    this.persistNotifier = opts?.onPersistRequested ?? null;
    this.workerPatchEmitter = opts?.onPatch ?? null;
    this.workerPatchFlushMs = opts?.patchFlushMs ?? WORKSPACE_PATCH_FLUSH_MS;
    this.authTokenOverride = opts?.authToken ?? null;
    this.wsBaseUrlOverride = opts?.wsBaseUrl ?? null;
    this.listWorkspaceArchivedTaskSummariesFn =
      opts?.listWorkspaceArchivedTaskSummaries ?? listWorkspaceArchivedTaskSummaries;
  }

  private ensureWorkerAvailable() {
    if (this.disableWorker) return;
    if (typeof Worker === "undefined") {
      throw new Error("Workspace active snapshot requires Worker support.");
    }
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  subscribeEvents = (listener: (event: WorkspaceActiveSnapshotEvent) => void): (() => void) => {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  };

  getSnapshot = (): WorkspaceActiveSnapshotState => this.state.getSnapshot();

  getSessionHeadSnapshot = (sessionId: string): SessionHeadSnapshot | null => {
    return this.state.getSessionHeadSnapshot(sessionId);
  };

  getWorktreeRoot = (worktreeId: string): string | null => {
    return this.state.getWorktreeRoot(worktreeId);
  };

  getWorktreeVcsSnapshot = (worktreeId: string): WorktreeVcsSnapshot | null => {
    return this.state.getWorktreeVcsSnapshot(worktreeId);
  };

  getSessionHeadsSnapshot = (): Record<string, SessionHeadSnapshot> => {
    return this.state.getSessionHeadsSnapshot();
  };

  getWorktreeRootsSnapshot = (): Record<string, string> => {
    return this.state.getWorktreeRootsSnapshot();
  };

  getWorktreeVcsSnapshots = (): WorktreeVcsSnapshot[] => {
    return this.state.getWorktreeVcsSnapshots();
  };

  getSnapshotRev = (): number => this.state.getSnapshotRev();

  setE2EEnabled = (enabled: boolean) => setE2EEnabled(this, enabled);

  e2eCloseActiveSnapshotStream = () => closeActiveSnapshotStream(this);

  e2eSetDropActiveSnapshotMessages = (drop: boolean) =>
    setDropActiveSnapshotMessages(this, drop);

  e2eGetCanonicalStreamUrl = (): string | null => getCanonicalStreamUrl(this);

  setSubscribedSessions = (sessions: SessionSubscriptionCursor[]) =>
    setSubscribedSessions(this, sessions);

  setForegroundSessionId = (sessionId: string | null) =>
    setForegroundSessionId(this, sessionId);

  init = () => {
    this.destroyed = false;
    const cachePromise = this.disableCache ? Promise.resolve() : this.hydrateFromCache();
    if (this.disableWorker) {
      this.connectStream().catch(() => {});
      return;
    }
    if (!this.configUnsubscribe && typeof window !== "undefined") {
      this.configUnsubscribe = subscribeDaemonConfig((config) => {
        this.updateAuthConfig({
          authToken: config.authToken ?? null,
          wsBaseUrl: config.wsBaseUrl ?? null,
          baseUrl: config.baseUrl ?? null,
          runId: config.runId ?? null,
        });
      });
    }
    this.ensureWorkerAvailable();
    cachePromise.finally(() => {
      if (!this.destroyed) {
        this.startWorker().catch(() => {});
      }
    });
  };

  private async startWorker() {
    if (this.worker || this.destroyed || this.workerStarting) return;
    this.workerStarting = true;
    try {
      const connection = await resolveWorkerConnectionState({
        workspaceId: this.workspaceId,
        phase: "worker_init",
        authTokenOverride: this.authTokenOverride,
        wsBaseUrlOverride: this.wsBaseUrlOverride,
      });
      if (this.worker || this.destroyed) {
        return;
      }
      if (isDesktopApp() && !getDaemonConnectionReadiness(connection).isReady) {
        return;
      }
      this.useWorker = true;
      this.worker = new Worker(new URL("../workers/workspaceActiveSnapshot.worker.ts", import.meta.url), {
        type: "module",
      });
      this.worker.onmessage = (event: MessageEvent<WorkspaceActiveSnapshotWorkerMessage>) => {
        const msg = event.data;
        if (!msg) return;
        if (msg.type === "patch") {
          this.applyWorkerPatch(msg.patch);
        }
      };
      this.postWorkerCommand({
        type: "init",
        workspaceId: this.workspaceId,
        connectionSeq: ++this.workerConnectionSeq,
        authToken: connection.authToken,
        baseUrl: connection.baseUrl,
        wsBaseUrl: connection.wsBaseUrl || null,
        runId: connection.runId,
        e2eEnabled: this.e2eEnabled,
      });
      if (this.subscribedSessions.length > 0) {
        this.postWorkerCommand({
          type: "set_subscribed_sessions",
          sessions: this.subscribedSessions.slice(),
        });
      }
      if (this.foregroundSessionId) {
        this.postWorkerCommand({
          type: "set_foreground_session_id",
          sessionId: this.foregroundSessionId,
        });
      }
      if (this.pendingWorkerCache) {
        this.postWorkerCommand({ type: "seed_cache", snapshot: this.pendingWorkerCache });
        this.pendingWorkerCache = null;
      }
    } finally {
      this.workerStarting = false;
    }
  }

  private queueWorkerAuthUpdate(opts: WorkerAuthUpdateConfig) {
    this.pendingWorkerAuthUpdate = {
      authToken: opts.authToken ?? null,
      wsBaseUrl: opts.wsBaseUrl ?? null,
      baseUrl: opts.baseUrl,
      runId: opts.runId ?? null,
    };
    if (this.workerAuthReconcileInFlight || this.destroyed) return;
    this.workerAuthReconcileInFlight = true;
    void this.runWorkerAuthReconcileLoop();
  }

  private async runWorkerAuthReconcileLoop() {
    try {
      while (!this.destroyed) {
        const pending = this.pendingWorkerAuthUpdate;
        if (!pending) return;
        this.pendingWorkerAuthUpdate = null;
        const connection = await resolveWorkerConnectionState({
          workspaceId: this.workspaceId,
          phase: "worker_update_auth",
          authTokenOverride: this.authTokenOverride,
          wsBaseUrlOverride: this.wsBaseUrlOverride,
          opts: pending,
        });
        if (!this.worker || this.destroyed) return;
        // Newer auth arrived while we were awaiting bridge sync; discard stale result.
        if (this.pendingWorkerAuthUpdate) {
          continue;
        }
        this.postWorkerCommand({
          type: "update_auth",
          connectionSeq: ++this.workerConnectionSeq,
          authToken: connection.authToken,
          baseUrl: connection.baseUrl,
          wsBaseUrl: connection.wsBaseUrl,
          runId: connection.runId,
        });
      }
    } finally {
      this.workerAuthReconcileInFlight = false;
      if (!this.destroyed && this.pendingWorkerAuthUpdate) {
        this.workerAuthReconcileInFlight = true;
        void this.runWorkerAuthReconcileLoop();
      }
    }
  }

  updateAuthConfig = (opts: {
    authToken?: string | null;
    wsBaseUrl?: string | null;
    baseUrl?: string | null;
    runId?: string | null;
  }) => {
    const nextAuth = opts.authToken ?? null;
    const nextWs = opts.wsBaseUrl ?? null;
    const authChanged = this.authTokenOverride !== nextAuth;
    const wsChanged = this.wsBaseUrlOverride !== nextWs;
    this.authTokenOverride = nextAuth;
    this.wsBaseUrlOverride = nextWs;

    if (this.worker) {
      this.queueWorkerAuthUpdate({
        authToken: nextAuth,
        wsBaseUrl: nextWs,
        baseUrl: opts.baseUrl,
        runId: opts.runId ?? null,
      });
      return;
    }

    if (!this.disableWorker) {
      if (!this.destroyed) {
        this.startWorker().catch(() => {});
      }
      return;
    }
    if (!authChanged && !wsChanged) return;
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        // ignore
      }
      this.ws = null;
      if (this.state.setConnection("disconnected")) {
        this.publish();
      }
    }
    if (!this.destroyed) {
      this.connectStream().catch(() => {});
    }
  };

  postWorkerCommand(cmd: WorkspaceActiveSnapshotCommand) {
    if (!this.worker) return;
    this.worker.postMessage(cmd);
  }

  private applyWorkerPatch(patch: WorkspaceActiveSnapshotPatch) {
    if (this.destroyed) return;
    const appliedAtMs = nowMs();
    if (typeof patch.oldestEventReceivedAtMs === "number") {
      noteQueueAgeSample("workspace", appliedAtMs - patch.oldestEventReceivedAtMs, {
        source: "worker_patch",
      });
    }
    if (typeof patch.oldestForegroundEventReceivedAtMs === "number") {
      noteQueueAgeSample("foreground", appliedAtMs - patch.oldestForegroundEventReceivedAtMs, {
        source: "worker_patch",
      });
    }
    this.state.applyWorkerPatch(patch);
    if (patch.publishSnapshot !== false) {
      this.publish();
    }
    if (patch.persist) {
      this.schedulePersistCache();
    }
    for (const event of patch.events) {
      this.notifyEventListeners(event);
    }
  }

  private getForegroundSessionId(): string {
    return String(this.foregroundSessionId ?? "").trim();
  }

  private isForegroundSessionEvent(evt: WorkspaceActiveSnapshotEvent): boolean {
    const foregroundSessionId = this.getForegroundSessionId();
    if (!foregroundSessionId) return false;
    switch (evt.type) {
      case "session_head_delta":
        return idToString(evt.delta.session_id) === foregroundSessionId;
      case "session_summary":
        return idToString(evt.summary.session.id) === foregroundSessionId;
      case "session_summary_delta":
        return idToString(evt.delta.session_id) === foregroundSessionId;
      case "session_gap":
        return idToString(evt.session_id) === foregroundSessionId;
      default:
        return false;
    }
  }

  destroy = () => {
    this.destroyed = true;
    this.pendingWorkerAuthUpdate = null;
    this.workerAuthReconcileInFlight = false;
    this.workerConnectionSeq = 0;
    if (this.worker) {
      this.worker.terminate();
      this.worker = null;
    }
    if (this.configUnsubscribe) {
      this.configUnsubscribe();
      this.configUnsubscribe = null;
    }
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        // ignore
      }
      this.ws = null;
    }
    if (this.reconnectTimer) {
      globalThis.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.clearSnapshotWarning();
    if (this.cachePersistTimer) {
      globalThis.clearTimeout(this.cachePersistTimer);
      this.cachePersistTimer = null;
    }
    if (this.workerPatchTimer) {
      globalThis.clearTimeout(this.workerPatchTimer);
      this.workerPatchTimer = null;
    }
    this.workerPatchPendingEvents = [];
    this.workerPatchPendingPersist = false;
    this.workerPatchDirty = false;
    this.workerPatchOldestEventReceivedAtMs = null;
    this.workerPatchOldestForegroundEventReceivedAtMs = null;
    this.lastWorkerPatchSnapshot = null;
    this.lastWorkerPatchSessionHeads = {};
    this.lastWorkerPatchWorktreeRoots = {};
    this.lastWorkerPatchSnapshotRev = -1;
    this.listeners.clear();
    this.eventListeners.clear();
  };

  loadMoreActive = () => {
    return;
  };

  ensureArchivedLoaded = () => {
    if (this.getSnapshot().archivedLoaded || this.state.getFetchState("archived") === "loading") return;
    if (this.worker) {
      this.postWorkerCommand({ type: "ensure_archived_loaded" });
      return;
    }
    this.fetchArchivedPage(true).catch(() => {});
  };

  loadMoreArchived = () => {
    if (!this.getSnapshot().hasMoreArchived || this.state.getFetchState("archived") === "loading") return;
    if (this.worker) {
      this.postWorkerCommand({ type: "load_more_archived" });
      return;
    }
    this.fetchArchivedPage(false).catch(() => {});
  };

  applyTaskUpdate(task: Task) {
    if (this.worker) {
      this.postWorkerCommand({ type: "apply_task_update", task });
      return;
    }
    if (!this.state.applyTaskUpdate(task)) return;
    this.publish();
    this.schedulePersistCache();
  }

  seedCachedSnapshot(cached: PersistedWorkspaceActiveSnapshotV1) {
    if (!cached || this.destroyed) return;
    try {
      this.state.applyCachedSnapshot(cached);
      this.publish();
    } catch {
      // ignore invalid cache payloads
    }
  }

  private async hydrateFromCache() {
    if (this.cacheHydrated) return;
    this.cacheHydrated = true;
    try {
      const cached = await loadWorkspaceActiveSnapshotV1(this.workspaceId);
      if (!cached || this.destroyed || this.state.hasLiveSnapshotApplied()) return;
      this.state.applyCachedSnapshot(cached);
      this.publish();
      if (this.worker) {
        this.postWorkerCommand({ type: "seed_cache", snapshot: cached });
      } else if (!this.disableWorker) {
        this.pendingWorkerCache = cached;
      }
    } catch {
      // ignore cache errors
    }
  }

  scheduleSnapshotWarning(reason: string) {
    if (this.destroyed) return;
    const snapshot = this.getSnapshot();
    if (snapshot.initialized && (reason === "ws_open" || reason === "ready")) {
      return;
    }
    if (this.snapshotWaitTimer) {
      globalThis.clearTimeout(this.snapshotWaitTimer);
    }
    this.snapshotWaitTimer = globalThis.setTimeout(() => {
      this.snapshotWaitTimer = null;
      if (this.destroyed) return;
      emitUiDiagnostic({
        source: "workspace_stream",
        code: "workspace.snapshot_wait_timeout",
        severity: "warning",
        message: "Workspace active snapshot was not received from the stream in time.",
        context: {
          workspaceId: this.workspaceId,
          reason,
          snapshotRev: this.state.getSnapshotRev(),
          connection: this.getSnapshot().connection,
        },
      });
      console.error(
        `[ctx] Workspace active snapshot not received over WS (${reason}).`,
        {
          workspaceId: this.workspaceId,
          snapshotRev: this.state.getSnapshotRev(),
          connection: this.getSnapshot().connection,
        },
      );
    }, SNAPSHOT_WAIT_MS);
  }

  private clearSnapshotWarning() {
    if (!this.snapshotWaitTimer) return;
    globalThis.clearTimeout(this.snapshotWaitTimer);
    this.snapshotWaitTimer = null;
  }

  private schedulePersistCache() {
    if (this.workerPatchEmitter) {
      this.workerPatchPendingPersist = true;
      this.scheduleWorkerPatchFlush();
      return;
    }
    if (this.destroyed || this.cachePersistTimer) return;
    this.cachePersistTimer = globalThis.setTimeout(() => {
      this.cachePersistTimer = null;
      this.persistCache().catch(() => {});
    }, 300);
  }

  scheduleWorkerPatchFlush() {
    if (!this.workerPatchEmitter || this.workerPatchTimer) return;
    this.workerPatchTimer = globalThis.setTimeout(() => {
      this.workerPatchTimer = null;
      this.flushWorkerPatch();
    }, this.workerPatchFlushMs);
  }

  flushWorkerPatchNow() {
    if (this.workerPatchTimer) {
      globalThis.clearTimeout(this.workerPatchTimer);
      this.workerPatchTimer = null;
    }
    this.flushWorkerPatch();
  }

  private flushWorkerPatch() {
    if (!this.workerPatchEmitter) return;
    if (
      !this.workerPatchDirty &&
      !this.workerPatchPendingPersist &&
      this.workerPatchPendingEvents.length === 0
    ) {
      return;
    }
    const events = this.workerPatchPendingEvents.slice();
    this.workerPatchPendingEvents = [];
    const snapshot = this.getSnapshot();
    const sessionHeads = this.getSessionHeadsSnapshot();
    const worktreeRoots = this.getWorktreeRootsSnapshot();
    const activeSessionIds = this.state.getActiveSessionIds();
    const snapshotRev = this.state.getSnapshotRev();
    const forceSnapshotReplace =
      this.lastWorkerPatchSnapshot == null || snapshotRev < this.lastWorkerPatchSnapshotRev;

    let patch: WorkspaceActiveSnapshotPatch;
    if (forceSnapshotReplace) {
      patch = {
        snapshot,
        sessionHeadUpserts: sessionHeads,
        worktreeRootUpserts: worktreeRoots,
        events,
        snapshotRev,
        archivedRev: this.state.getArchivedRev(),
        activeSessionIds,
        publishSnapshot: true,
        persist: this.workerPatchPendingPersist,
        oldestEventReceivedAtMs: this.workerPatchOldestEventReceivedAtMs,
        oldestForegroundEventReceivedAtMs: this.workerPatchOldestForegroundEventReceivedAtMs,
      };
    } else {
      const previousSnapshot = this.lastWorkerPatchSnapshot!;
      const previousSessionHeads = this.lastWorkerPatchSessionHeads;
      const previousWorktreeRoots = this.lastWorkerPatchWorktreeRoots;
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
        archivedRev: this.state.getArchivedRev(),
        activeSessionIds,
        publishSnapshot: this.workerPatchDirty || shellChanged || taskChanged,
        persist: this.workerPatchPendingPersist,
        oldestEventReceivedAtMs: this.workerPatchOldestEventReceivedAtMs,
        oldestForegroundEventReceivedAtMs: this.workerPatchOldestForegroundEventReceivedAtMs,
      };
    }
    this.workerPatchDirty = false;
    this.workerPatchPendingPersist = false;
    this.workerPatchOldestEventReceivedAtMs = null;
    this.workerPatchOldestForegroundEventReceivedAtMs = null;
    this.lastWorkerPatchSnapshot = snapshot;
    this.lastWorkerPatchSessionHeads = sessionHeads;
    this.lastWorkerPatchWorktreeRoots = worktreeRoots;
    this.lastWorkerPatchSnapshotRev = snapshotRev;
    this.workerPatchEmitter(patch);
  }

  private async persistCache() {
    if (this.destroyed) return;
    try {
      await saveWorkspaceActiveSnapshotV1(this.workspaceId, this.state.buildPersistedSnapshot());
    } catch {
      // ignore cache errors
    }
  }

  private applyWorkspaceSnapshot(snapshot: WorkspaceActiveSnapshot, heads?: SessionHeadSnapshot[] | null) {
    applyWorkspaceStreamSnapshot(this as unknown as WorkspaceActiveSnapshotStreamHost, snapshot, heads);
  }

  private async fetchArchivedPage(firstLoad: boolean) {
    await fetchArchivedWorkspacePage(this as unknown as WorkspaceActiveSnapshotStreamHost, firstLoad);
  }

  private async connectStream() {
    await connectWorkspaceStream(this as unknown as WorkspaceActiveSnapshotStreamHost);
  }

  private openWebSocket(url: string) {
    return openWorkspaceStreamWebSocket(
      this as unknown as WorkspaceActiveSnapshotStreamHost,
      url,
    );
  }

  private scheduleReconnect() {
    scheduleWorkspaceStreamReconnect(this as unknown as WorkspaceActiveSnapshotStreamHost);
  }

  enqueueStreamMessage(data: unknown) {
    enqueueWorkspaceStreamMessage(this as unknown as WorkspaceActiveSnapshotStreamHost, data);
  }

  private async handleStreamMessage(
    input: unknown | { data: unknown; receivedAtMs: number },
  ): Promise<void> {
    await handleWorkspaceStreamMessage(
      this as unknown as WorkspaceActiveSnapshotStreamHost,
      input,
    );
  }

  private applySessionSummaryDelta(evt: WorkspaceActiveSnapshotEvent & { type: "session_summary_delta" }) {
    return applyWorkspaceSessionSummaryDelta(
      this as unknown as WorkspaceActiveSnapshotStreamHost,
      evt,
    );
  }

  private notifyEventListeners(evt: WorkspaceActiveSnapshotEvent) {
    notifyEventListeners(this, evt);
  }

  flushSubscriptions = (reason = "subscribe") =>
    flushActiveSnapshotSubscriptions(this, reason);

  publish() {
    if (this.workerPatchEmitter) {
      this.workerPatchDirty = true;
      this.scheduleWorkerPatchFlush();
    }
    for (const listener of this.listeners) {
      listener();
    }
  }
}
