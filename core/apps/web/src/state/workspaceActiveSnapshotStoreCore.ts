import type {
  Session,
  SessionHeadDelta,
  SessionHeadSnapshot,
  SessionSummary,
  SessionSnapshotSummary,
  Task,
  WorktreeVcsSnapshot,
  WorkspaceActiveSnapshot,
  WorkspaceActiveSnapshotEvent,
  WorkspaceActiveSnapshotSessionSummaryDeltaEvent,
  WorkspaceActiveSnapshotTaskDeltaEvent,
  WorkspaceActiveTaskSummary,
  WorkspaceIndexCursor,
  WorkspaceTaskSummary,
} from "@ctx/types";
import {
  getDaemonClientConfig,
  getDaemonConnection,
  getDaemonConnectionReadiness,
  syncDesktopDaemonConnectionFromBridge,
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
import { parseWsJson } from "../utils/wsJson";
import { emitUiDiagnostic, normalizeDiagnosticErrorMessage } from "./diagnosticsChannel";
import type {
  WorkspaceActiveSnapshotCommand,
  WorkspaceActiveSnapshotPatch,
  WorkspaceActiveSnapshotWorkerMessage,
} from "./workspaceActiveSnapshotProtocol";
import {
  emptySessionHeadWindow,
  mergeSessionEvents,
  mergeSessionMessages,
  mergeSessionToolSummaries,
  mergeSessionTurns,
  sanitizeSessionHeadSnapshot,
} from "./sessionHeadState";
import {
  asRecord,
  collectWorkspaceActivePrimarySessionIds,
  hasOwnProperty,
  mapWorktreeVcsSnapshots,
  projectPrimarySessionHeadOntoTasks,
  readString,
  resolvePrimarySessionId,
  sortSessionSummaries,
} from "./workspaceActiveSnapshot/projection";
import {
  readWorkspaceHeadsBatchPayload,
  readWorkspaceSnapshotPayload,
  readWorkspaceStreamRev,
  toWorkspaceHttpBaseUrl,
} from "./workspaceActiveSnapshot/transport";
import { buildWorkspaceActiveSubscribeMessage } from "./workspaceActiveSnapshot/subscriptions";

export type WorkspaceActiveSnapshotItem = {
  id: string;
  task: Task;
  sessions: SessionSnapshotSummary[];
  providerIds?: string[];
  primarySessionHead?: SessionHeadSnapshot | null;
  primarySessionId?: string | null;
  sort_at?: string | null;
  sortAtMs: number;
};

export type WorkspaceActiveSnapshotState = {
  workspaceId: string;
  initialized: boolean;
  connection: "idle" | "connecting" | "connected" | "disconnected";
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
  activeIds: string[];
  archivedIds: string[];
  totalActive: number;
  totalArchived: number;
  archivedRev: number;
  worktreeVcsById: Record<string, WorktreeVcsSnapshot>;
  fetchState: {
    active: "idle" | "loading" | "error";
    archived: "idle" | "loading" | "error";
  };
  hasMoreActive: boolean;
  hasMoreArchived: boolean;
  archivedLoaded: boolean;
};

export type WorkspaceActiveSnapshotEventSource = {
  subscribe: (listener: () => void) => () => void;
  subscribeEvents: (listener: (event: WorkspaceActiveSnapshotEvent) => void) => () => void;
  getSnapshot: () => WorkspaceActiveSnapshotState;
  getSessionHeadSnapshot: (sessionId: string) => SessionHeadSnapshot | null;
  getSessionHeadsSnapshot?: () => Record<string, SessionHeadSnapshot>;
  getWorktreeRoot: (worktreeId: string) => string | null;
  getWorktreeVcsSnapshot: (worktreeId: string) => WorktreeVcsSnapshot | null;
  setSubscribedSessionIds?: (sessionIds: string[]) => void;
  setForegroundTaskId?: (taskId: string | null) => void;
};

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

type WorkerAuthUpdateConfig = {
  authToken?: string | null;
  wsBaseUrl?: string | null;
  baseUrl?: string | null;
  runId?: string | null;
};

const ACTIVE_PAGE_SIZE = 50;
const SNAPSHOT_WAIT_MS = 1200;
const FOREGROUND_TASK_DEBOUNCE_MS = 150;
const WORKSPACE_PATCH_FLUSH_MS = 50;

export class WorkspaceActiveSnapshotStoreImpl implements WorkspaceActiveSnapshotEventSource {
  private listeners = new Set<() => void>();
  private eventListeners = new Set<(event: WorkspaceActiveSnapshotEvent) => void>();
  private snapshot: WorkspaceActiveSnapshotState;
  private tasks = new Map<string, WorkspaceActiveSnapshotItem>();
  private sessionHeadsById = new Map<string, SessionHeadSnapshot>();
  private worktreeRootsById = new Map<string, string>();
  private worker: Worker | null = null;
  private workerStarting = false;
  private workerAuthReconcileInFlight = false;
  private pendingWorkerAuthUpdate: WorkerAuthUpdateConfig | null = null;
  private workerConnectionSeq = 0;
  private useWorker = false;
  private disableCache = false;
  private disableWorker = false;
  private e2eEnabled = false;
  private e2eDropStreamMessages = false;
  private persistNotifier: (() => void) | null = null;
  private workerPatchEmitter: ((patch: WorkspaceActiveSnapshotPatch) => void) | null = null;
  private workerPatchTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private workerPatchPendingEvents: WorkspaceActiveSnapshotEvent[] = [];
  private workerPatchPendingPersist = false;
  private workerPatchDirty = false;
  private workerPatchFlushMs = WORKSPACE_PATCH_FLUSH_MS;
  private authTokenOverride: string | null = null;
  private wsBaseUrlOverride: string | null = null;
  private configUnsubscribe: (() => void) | null = null;
  private listWorkspaceArchivedTaskSummariesFn: typeof listWorkspaceArchivedTaskSummaries;
  private subscribedSessionIds: string[] = [];
  private activeSessionIds: string[] = [];
  private foregroundTaskId: string | null = null;
  private foregroundTaskTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private activeOrder: string[] = [];
  private archivedOrder: string[] = [];
  private totalActive = 0;
  private totalArchived = 0;
  private hasMoreActive = true;
  private hasMoreArchived = false;
  private archivedLoaded = false;
  private archivedCursor: WorkspaceIndexCursor | null = null;
  private ws: WebSocket | null = null;
  private connecting = false;
  private reconnectTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private reconnectDelayMs = 1000;
  private snapshotRev = 0;
  private archivedRev = 0;
  private lastStreamSeq = 0;
  private allowSnapshotReset = false;
  private cacheHydrated = false;
  private liveSnapshotApplied = false;
  private pendingWorkerCache: PersistedWorkspaceActiveSnapshotV1 | null = null;
  private snapshotWaitTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private cachePersistTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private streamQueue: Promise<void> = Promise.resolve();
  private destroyed = false;

  constructor(private workspaceId: string, opts?: WorkspaceActiveSnapshotStoreOptions) {
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
    this.snapshot = {
      workspaceId,
      initialized: false,
      connection: "idle",
      tasksById: {},
      activeIds: [],
      archivedIds: [],
      totalActive: 0,
      totalArchived: 0,
      archivedRev: 0,
      worktreeVcsById: {},
      fetchState: { active: "idle", archived: "idle" },
      hasMoreActive: true,
      hasMoreArchived: false,
      archivedLoaded: false,
    };
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

  getSnapshot = (): WorkspaceActiveSnapshotState => this.snapshot;

  getSessionHeadSnapshot = (sessionId: string): SessionHeadSnapshot | null => {
    const id = idToString(sessionId);
    if (!id) return null;
    return this.sessionHeadsById.get(id) ?? null;
  };

  getWorktreeRoot = (worktreeId: string): string | null => {
    const id = idToString(worktreeId);
    if (!id) return null;
    return this.worktreeRootsById.get(id) ?? null;
  };

  getWorktreeVcsSnapshot = (worktreeId: string): WorktreeVcsSnapshot | null => {
    const id = idToString(worktreeId);
    if (!id) return null;
    return this.snapshot.worktreeVcsById[id] ?? null;
  };

  getSessionHeadsSnapshot = (): Record<string, SessionHeadSnapshot> => {
    const out: Record<string, SessionHeadSnapshot> = {};
    for (const [id, head] of this.sessionHeadsById.entries()) {
      out[id] = head;
    }
    return out;
  };

  getWorktreeRootsSnapshot = (): Record<string, string> => {
    const out: Record<string, string> = {};
    for (const [id, root] of this.worktreeRootsById.entries()) {
      out[id] = root;
    }
    return out;
  };

  getWorktreeVcsSnapshots = (): WorktreeVcsSnapshot[] => {
    return Object.values(this.snapshot.worktreeVcsById ?? {});
  };

  getSnapshotRev = (): number => this.snapshotRev;

  setE2EEnabled = (enabled: boolean) => {
    this.e2eEnabled = enabled;
    if (!enabled) {
      this.e2eDropStreamMessages = false;
    }
    if (this.worker) {
      this.postWorkerCommand({ type: "e2e_set_enabled", enabled });
    }
  };

  e2eCloseActiveSnapshotStream = () => {
    if (!this.e2eEnabled) return;
    if (this.worker) {
      this.postWorkerCommand({ type: "e2e_close_stream" });
      if (this.snapshot.connection !== "disconnected") {
        this.snapshot.connection = "disconnected";
        this.publish();
      }
      return;
    }
    try {
      this.ws?.close();
    } catch {
      // ignore
    }
    if (this.snapshot.connection !== "disconnected") {
      this.snapshot.connection = "disconnected";
      this.publish();
    }
  };

  e2eSetDropActiveSnapshotMessages = (drop: boolean) => {
    if (!this.e2eEnabled) return;
    if (this.worker) {
      this.postWorkerCommand({ type: "e2e_set_drop_messages", drop });
      return;
    }
    this.e2eDropStreamMessages = drop;
  };

  e2eDispatchActiveSnapshotStreamMessage = (payload: unknown) => {
    if (!this.e2eEnabled) return;
    const normalized = this.normalizeE2EStreamPayload(payload);
    if (!normalized) return;
    if (this.worker) {
      this.postWorkerCommand({ type: "e2e_dispatch_stream_message", payload: normalized });
      return;
    }
    this.enqueueStreamMessage(normalized);
  };

  e2eGetCanonicalStreamUrl = (): string | null => {
    if (!this.e2eEnabled) return null;
    const daemonConfig = getDaemonClientConfig();
    const wsBaseUrl = this.wsBaseUrlOverride ?? daemonConfig.wsBaseUrl ?? null;
    const token = this.authTokenOverride ?? daemonConfig.authToken;
    if (!wsBaseUrl) return null;
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";
    return `${wsBaseUrl.replace(/\/+$/, "")}/api/workspaces/${this.workspaceId}/active_snapshot/stream${qs}`;
  };

  setSubscribedSessionIds = (sessionIds: string[]) => {
    const activeSet = new Set(this.activeSessionIds);
    const next = sessionIds
      .map((id) => String(id || "").trim())
      .filter((id) => id.length > 0 && !activeSet.has(id));
    const deduped = Array.from(new Set(next));
    if (deduped.join("|") === this.subscribedSessionIds.join("|")) return;
    this.subscribedSessionIds = deduped;
    if (this.worker) {
      this.postWorkerCommand({ type: "set_subscribed_session_ids", sessionIds: deduped });
      return;
    }
    this.flushSubscriptions("session_ids");
  };

  setForegroundTaskId = (taskId: string | null) => {
    const normalized = typeof taskId === "string" ? taskId.trim() : "";
    const next = normalized ? normalized : null;
    if (next === this.foregroundTaskId) return;
    this.foregroundTaskId = next;
    if (this.worker) {
      this.postWorkerCommand({ type: "set_foreground_task_id", taskId: next });
      return;
    }
    this.scheduleForegroundTaskFlush();
  };

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

  private emitWorkerConnectionMissingDiagnostic = (
    phase: "worker_init" | "worker_update_auth",
    bridgeKind: "none" | "local" | "ssh" | null,
    syncError: string | null,
    missing: "base" | "auth" = "base",
  ) => {
    const connection = getDaemonConnection();
    const bridgeConnected = bridgeKind === "local" || bridgeKind === "ssh";
    const missingAuth = missing === "auth";
    const code = missingAuth
      ? bridgeConnected
        ? "workspace.worker_desktop_bridge_missing_auth"
        : "workspace.worker_connection_missing"
      : bridgeConnected
        ? "workspace.worker_desktop_bridge_missing_base"
        : "workspace.worker_connection_missing";
    const message = missingAuth
      ? bridgeConnected
        ? "Desktop bridge is connected, but worker daemon auth token is missing."
        : "Worker daemon auth token is missing."
      : bridgeConnected
        ? "Desktop bridge is connected, but worker daemon HTTP base URL is missing."
        : "Worker daemon HTTP base URL is missing.";
    emitUiDiagnostic({
      source: "workspace_snapshot",
      code,
      severity: "warning",
      message,
      context: {
        workspaceId: this.workspaceId,
        phase,
        missing,
        bridgeKind,
        connectionSource: connection.source ?? null,
        syncError: syncError ?? undefined,
      },
    });
  };

  private async resolveWorkerConnectionState(
    phase: "worker_init" | "worker_update_auth",
    opts?: WorkerAuthUpdateConfig,
  ): Promise<{
    authToken: string | null;
    wsBaseUrl: string | null;
    baseUrl: string | null;
    runId: string | null;
  }> {
    let daemonConfig = getDaemonClientConfig();
    let bridgeKind: "none" | "local" | "ssh" | null = null;
    let syncError: string | null = null;

    const readState = () => {
      const authToken = opts?.authToken ?? this.authTokenOverride ?? daemonConfig.authToken ?? null;
      const wsBaseUrl = opts?.wsBaseUrl ?? this.wsBaseUrlOverride ?? daemonConfig.wsBaseUrl ?? null;
      const baseUrl =
        opts?.baseUrl ?? daemonConfig.baseUrl ?? (wsBaseUrl ? toWorkspaceHttpBaseUrl(wsBaseUrl) : null);
      const runId = opts?.runId ?? daemonConfig.runId ?? null;
      return {
        authToken,
        wsBaseUrl,
        baseUrl,
        runId,
      };
    };

    let state = readState();
    let readiness = getDaemonConnectionReadiness(state);
    if (isDesktopApp() && !readiness.isReady) {
      const synced = await syncDesktopDaemonConnectionFromBridge({
        force: true,
        probeHealth: true,
        reason: phase,
      });
      daemonConfig = getDaemonClientConfig();
      bridgeKind = synced.info?.kind ?? null;
      syncError = synced.error;
      state = readState();
      readiness = getDaemonConnectionReadiness(state);
    }

    if (isDesktopApp() && !readiness.isReady) {
      this.emitWorkerConnectionMissingDiagnostic(
        phase,
        bridgeKind,
        syncError,
        readiness.missing ?? "base",
      );
    }
    return state;
  }

  private async startWorker() {
    if (this.worker || this.destroyed || this.workerStarting) return;
    this.workerStarting = true;
    try {
      const connection = await this.resolveWorkerConnectionState("worker_init");
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
      if (this.subscribedSessionIds.length > 0) {
        this.postWorkerCommand({ type: "set_subscribed_session_ids", sessionIds: this.subscribedSessionIds.slice() });
      }
      if (this.foregroundTaskId) {
        this.postWorkerCommand({ type: "set_foreground_task_id", taskId: this.foregroundTaskId });
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
        const connection = await this.resolveWorkerConnectionState("worker_update_auth", pending);
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
      this.snapshot.connection = "disconnected";
      this.publish();
    }
    if (!this.destroyed) {
      this.connectStream().catch(() => {});
    }
  };

  private postWorkerCommand(cmd: WorkspaceActiveSnapshotCommand) {
    if (!this.worker) return;
    this.worker.postMessage(cmd);
  }

  private applyWorkerPatch(patch: WorkspaceActiveSnapshotPatch) {
    if (this.destroyed) return;
    this.snapshot = patch.snapshot;
    this.tasks = new Map(Object.entries(patch.snapshot.tasksById));
    this.activeOrder = patch.snapshot.activeIds.slice();
    this.archivedOrder = patch.snapshot.archivedIds.slice();
    this.totalActive = patch.snapshot.totalActive;
    this.totalArchived = patch.snapshot.totalArchived;
    this.archivedRev = patch.snapshot.archivedRev;
    this.hasMoreActive = patch.snapshot.hasMoreActive;
    this.hasMoreArchived = patch.snapshot.hasMoreArchived;
    this.archivedLoaded = patch.snapshot.archivedLoaded;
    this.activeSessionIds = patch.activeSessionIds.slice();
    this.sessionHeadsById = new Map(Object.entries(patch.sessionHeads));
    this.worktreeRootsById = new Map(Object.entries(patch.worktreeRoots));
    this.snapshotRev = patch.snapshotRev;
    this.archivedRev = patch.archivedRev;
    this.publish();
    if (patch.persist) {
      this.schedulePersistCache();
    }
    for (const event of patch.events) {
      this.notifyEventListeners(event);
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
    if (this.foregroundTaskTimer) {
      globalThis.clearTimeout(this.foregroundTaskTimer);
      this.foregroundTaskTimer = null;
    }
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
    this.listeners.clear();
    this.eventListeners.clear();
  };

  loadMoreActive = () => {
    return;
  };

  ensureArchivedLoaded = () => {
    if (this.archivedLoaded || this.snapshot.fetchState.archived === "loading") return;
    if (this.worker) {
      this.postWorkerCommand({ type: "ensure_archived_loaded" });
      return;
    }
    this.fetchArchivedPage(true).catch(() => {});
  };

  loadMoreArchived = () => {
    if (!this.hasMoreArchived || this.snapshot.fetchState.archived === "loading") return;
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
    const id = idToString(task.id);
    if (!id) return;
    const existing = this.tasks.get(id);
    if (!existing) return;
    const prevArchived = Boolean(existing.task.archived_at);
    const nextArchived = Boolean(task.archived_at);
    const stableSortAt = this.taskSortAt(task, existing.sort_at);
    const stableSortAtMs = Date.parse(stableSortAt ?? "") || existing.sortAtMs || Date.now();
    const updated: WorkspaceActiveSnapshotItem = {
      ...existing,
      task: { ...task },
      sortAtMs: stableSortAtMs,
      sort_at: stableSortAt ?? existing.sort_at,
    };
    this.tasks.set(id, updated);
    if (prevArchived !== nextArchived) {
      this.archivedLoaded = false;
      this.archivedCursor = null;
      this.hasMoreArchived = true;
    }
    this.updateCountsForMove(existing, updated);
    this.placeInOrders(updated);
    this.publish();
    this.schedulePersistCache();
  }

  seedCachedSnapshot(cached: PersistedWorkspaceActiveSnapshotV1) {
    if (!cached || this.destroyed) return;
    try {
      this.applyCachedActiveSnapshot(cached);
    } catch {
      // ignore invalid cache payloads
    }
  }

  private async hydrateFromCache() {
    if (this.cacheHydrated) return;
    this.cacheHydrated = true;
    try {
      const cached = await loadWorkspaceActiveSnapshotV1(this.workspaceId);
      if (!cached || this.destroyed || this.liveSnapshotApplied) return;
      this.applyCachedActiveSnapshot(cached);
      if (this.worker) {
        this.postWorkerCommand({ type: "seed_cache", snapshot: cached });
      } else if (!this.disableWorker) {
        this.pendingWorkerCache = cached;
      }
    } catch {
      // ignore cache errors
    }
  }

  private applyCachedActiveSnapshot(cached: PersistedWorkspaceActiveSnapshotV1) {
    this.snapshotRev = Math.max(this.snapshotRev, cached.snapshotRev ?? 0);
    this.archivedRev = Math.max(this.archivedRev, cached.archivedRev ?? 0);
    const activeTasks = Array.isArray(cached.active?.tasks) ? cached.active.tasks : [];
    const archivedHeads = this.collectArchivedHeads();
    for (const [id, item] of this.tasks.entries()) {
      if (!item.task.archived_at) {
        this.tasks.delete(id);
      }
    }
    this.activeOrder = [];
    this.sessionHeadsById.clear();
    for (const [sessionId, head] of archivedHeads) {
      this.sessionHeadsById.set(sessionId, head);
    }

    const nextActiveIds = new Set<string>();
    for (const summary of activeTasks) {
      const task = summary?.task;
      if (!task || typeof task !== "object") continue;
      if (task.archived_at) continue;
      const existing = this.tasks.get(idToString(task.id));
      const normalized = this.normalizeActiveSummary(summary, existing);
      nextActiveIds.add(normalized.id);
      this.tasks.set(normalized.id, normalized);
      this.placeInOrders(normalized);
    }

    const totalCountRaw = cached.active?.totalCount;
    const totalCount =
      typeof totalCountRaw === "number" && Number.isFinite(totalCountRaw)
        ? totalCountRaw
        : nextActiveIds.size;
    this.totalActive = Math.max(totalCount, nextActiveIds.size);
    this.snapshotRev = Math.max(this.snapshotRev, cached.snapshotRev ?? 0);
    this.activeSessionIds = this.collectActiveSessionIds();
    this.snapshot.worktreeVcsById = mapWorktreeVcsSnapshots(cached.worktreeVcsSnapshots ?? []);
    this.snapshot.initialized = true;
    this.publish();
    if (this.needsCacheMigration(cached)) {
      this.schedulePersistCache();
    }
  }

  private needsCacheMigration(cached: PersistedWorkspaceActiveSnapshotV1): boolean {
    const legacy = cached as PersistedWorkspaceActiveSnapshotV1 & {
      tasks?: PersistedWorkspaceActiveTaskSummaryV1[];
      totalCount?: number;
    };
    return Array.isArray(legacy.tasks);
  }

  private scheduleSnapshotWarning(reason: string) {
    if (this.destroyed) return;
    if (this.snapshot.initialized && (reason === "ws_open" || reason === "ready")) {
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
          snapshotRev: this.snapshotRev,
          connection: this.snapshot.connection,
        },
      });
      console.error(
        `[ctx] Workspace active snapshot not received over WS (${reason}).`,
        {
          workspaceId: this.workspaceId,
          snapshotRev: this.snapshotRev,
          connection: this.snapshot.connection,
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

  private scheduleWorkerPatchFlush() {
    if (!this.workerPatchEmitter || this.workerPatchTimer) return;
    this.workerPatchTimer = globalThis.setTimeout(() => {
      this.workerPatchTimer = null;
      this.flushWorkerPatch();
    }, this.workerPatchFlushMs);
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
    const patch: WorkspaceActiveSnapshotPatch = {
      snapshot: this.snapshot,
      sessionHeads: this.getSessionHeadsSnapshot(),
      worktreeRoots: this.getWorktreeRootsSnapshot(),
      events,
      snapshotRev: this.snapshotRev,
      archivedRev: this.archivedRev,
      activeSessionIds: this.activeSessionIds.slice(),
      persist: this.workerPatchPendingPersist,
    };
    this.workerPatchDirty = false;
    this.workerPatchPendingPersist = false;
    this.workerPatchEmitter(patch);
  }

  private async persistCache() {
    if (this.destroyed) return;
    const tasks: PersistedWorkspaceActiveTaskSummaryV1[] = [];
    for (const id of this.activeOrder) {
      const item = this.tasks.get(id);
      if (!item || item.task.archived_at) continue;
      const summary = this.buildPersistedSummary(item);
      if (summary) tasks.push(summary);
    }
    const totalCount = Math.max(this.totalActive, tasks.length);
    try {
      await saveWorkspaceActiveSnapshotV1(this.workspaceId, {
        snapshotRev: this.snapshotRev,
        archivedRev: this.archivedRev,
        worktreeVcsSnapshots: this.getWorktreeVcsSnapshots(),
        active: {
          tasks,
          totalCount,
        },
      });
    } catch {
      // ignore cache errors
    }
  }

  private shouldReplaceHead(prev: SessionHeadSnapshot | null | undefined, next: SessionHeadSnapshot): boolean {
    if (!prev) return true;
    const prevSeq = typeof prev.last_event_seq === "number" ? prev.last_event_seq : -1;
    const nextSeq = typeof next.last_event_seq === "number" ? next.last_event_seq : -1;
    if (prevSeq >= 0 && nextSeq >= 0 && nextSeq < prevSeq) return false;
    return true;
  }

  private applyActiveHeads(heads: SessionHeadSnapshot[]): boolean {
    if (!Array.isArray(heads) || heads.length === 0) return false;
    let changed = false;
    for (const head of heads) {
      if (!head || typeof head !== "object") continue;
      const sessionId = idToString(head.session?.id ?? "");
      if (!sessionId) continue;
      const sanitized = sanitizeSessionHeadSnapshot(head);
      const prev = this.sessionHeadsById.get(sessionId);
      if (this.shouldReplaceHead(prev, sanitized)) {
        this.sessionHeadsById.set(sessionId, sanitized);
        changed = true;
      }
      if (projectPrimarySessionHeadOntoTasks(this.tasks, sanitized)) {
        changed = true;
      }
    }
    return changed;
  }

  private buildPersistedSummary(
    item: WorkspaceActiveSnapshotItem,
  ): PersistedWorkspaceActiveTaskSummaryV1 | null {
    if (!item.task) return null;
    const sessions = Array.isArray(item.sessions) ? item.sessions : [];
    const primaryId = resolvePrimarySessionId(item);
    let primary = primaryId
      ? sessions.find((summary) => idToString(summary.session.id) === primaryId) ?? null
      : null;
    if (!primary && sessions.length > 0) {
      primary = sessions[0] ?? null;
    }
    if (!primary && item.primarySessionHead?.session) {
      primary = this.sessionToSummary(item.primarySessionHead.session);
    }
    const head =
      item.primarySessionHead ||
      (primaryId ? this.sessionHeadsById.get(primaryId) ?? null : null);
    const sortAt = this.taskSortAt(item.task, item.sort_at);
    return {
      task: item.task,
      primary_session: primary ?? null,
      primary_session_head: head ? sanitizeSessionHeadSnapshot(head) : null,
      sessions,
      sort_at: sortAt,
    };
  }

  private applyWorkspaceSnapshot(snapshot: WorkspaceActiveSnapshot, heads?: SessionHeadSnapshot[] | null) {
    if (this.destroyed || !snapshot || typeof snapshot !== "object") return;
    const incomingRev = typeof snapshot.snapshot_rev === "number" ? snapshot.snapshot_rev : 0;
    const allowLower = !this.liveSnapshotApplied || this.allowSnapshotReset;
    if (incomingRev < this.snapshotRev && !allowLower) {
      return;
    }
    this.snapshotRev = incomingRev < this.snapshotRev ? incomingRev : Math.max(this.snapshotRev, incomingRev);
    this.allowSnapshotReset = false;
    if (typeof snapshot.archived_rev === "number" && snapshot.archived_rev > this.archivedRev) {
      this.archivedRev = snapshot.archived_rev;
      this.archivedLoaded = false;
      this.archivedCursor = null;
    }

    const archivedHeads = this.collectArchivedHeads();
    for (const [id, item] of this.tasks.entries()) {
      if (!item.task.archived_at) {
        this.tasks.delete(id);
      }
    }
    this.activeOrder = [];
    this.sessionHeadsById.clear();
    for (const [sessionId, head] of archivedHeads) {
      this.sessionHeadsById.set(sessionId, head);
    }

    const activeTasks = snapshot.active?.tasks ?? [];
    const nextActiveIds = new Set<string>();
    for (const summary of activeTasks) {
      const existing = this.tasks.get(idToString(summary.task.id));
      const normalized = this.normalizeActiveSummary(summary, existing);
      nextActiveIds.add(normalized.id);
      this.tasks.set(normalized.id, normalized);
      this.placeInOrders(normalized);
    }

    const nextTotalActive = Number.isFinite(snapshot.active?.total_count)
      ? snapshot.active.total_count
      : nextActiveIds.size;
    this.totalActive = Math.max(nextTotalActive, nextActiveIds.size);
    if (Array.isArray(heads) && heads.length > 0) {
      this.applyActiveHeads(heads);
    }
    this.activeSessionIds = collectWorkspaceActivePrimarySessionIds({
      activeIds: this.activeOrder,
      archivedIds: this.archivedOrder,
      tasksById: Object.fromEntries(this.tasks.entries()),
    });
    this.snapshot.worktreeVcsById = mapWorktreeVcsSnapshots(snapshot.worktree_vcs_snapshots ?? []);
    this.snapshot.initialized = true;
    this.liveSnapshotApplied = true;
    this.clearSnapshotWarning();
    this.publish();
    this.schedulePersistCache();
  }

  private async fetchArchivedPage(firstLoad: boolean) {
    if (this.destroyed) return;
    if (firstLoad) {
      this.archivedCursor = null;
    }
    if (!firstLoad && !this.archivedCursor) {
      this.hasMoreArchived = false;
      this.publish();
      return;
    }
    this.setFetchState("archived", "loading");
    try {
      const page = await this.listWorkspaceArchivedTaskSummariesFn(this.workspaceId, {
        limit: ACTIVE_PAGE_SIZE,
        cursor: this.archivedCursor ?? undefined,
      });
      if (typeof page.archived_rev === "number" && page.archived_rev > this.archivedRev) {
        this.archivedRev = page.archived_rev;
      }
      this.totalArchived = page.total_archived ?? this.totalArchived;
      const summaries = await Promise.all(page.tasks.map((task) => this.buildArchivedItem(task)));
      summaries.forEach((summary) => {
        if (summary) {
          this.upsertArchivedItem(summary, { adjustCounts: false });
        }
      });

      this.archivedCursor = page.next_cursor ?? null;
      this.hasMoreArchived = Boolean(page.next_cursor);
      this.archivedLoaded = true;
      this.publish();
    } catch (err) {
      emitUiDiagnostic({
        source: "workspace_snapshot",
        code: "workspace.archived_load_failed",
        severity: "warning",
        message: "Archived task summaries failed to load.",
        context: {
          workspaceId: this.workspaceId,
          firstLoad,
          error: normalizeDiagnosticErrorMessage(err, "Archived task load failed."),
        },
      });
      this.setFetchState("archived", "error");
      return;
    }
    this.setFetchState("archived", "idle");
  }

  private async connectStream() {
    if (this.destroyed || this.ws || this.connecting) return;
    this.connecting = true;
    this.snapshot.connection = "connecting";
    this.publish();
    try {
      const daemonConfig = getDaemonClientConfig();
      const wsBaseUrl = this.wsBaseUrlOverride ?? daemonConfig.wsBaseUrl ?? null;
      const token = this.authTokenOverride ?? daemonConfig.authToken;
      if (!wsBaseUrl) {
        emitUiDiagnostic({
          source: "workspace_stream",
          code: "workspace.stream_connection_missing",
          severity: "warning",
          message: "Workspace stream connection is not configured.",
          context: {
            workspaceId: this.workspaceId,
          },
        });
        this.snapshot.connection = "disconnected";
        this.publish();
        return;
      }
      const qs = token ? `?token=${encodeURIComponent(token)}` : "";
      const url = `${wsBaseUrl.replace(/\/+$/, "")}/api/workspaces/${this.workspaceId}/active_snapshot/stream${qs}`;
      if (this.destroyed) return;
      try {
        await this.openWebSocket(url);
        return;
      } catch (err) {
        emitUiDiagnostic({
          source: "workspace_stream",
          code: "workspace.stream_connect_failed",
          severity: "warning",
          message: "Workspace stream connection failed; reconnect scheduled.",
          context: {
            workspaceId: this.workspaceId,
            url,
            error: err instanceof Error && err.message ? err.message : String(err),
          },
        });
        this.snapshot.connection = "disconnected";
        this.publish();
        this.scheduleReconnect();
      }
    } finally {
      this.connecting = false;
    }
  }

  private openWebSocket(url: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      this.ws = ws;
      let opened = false;
      const timeoutId = globalThis.setTimeout(() => {
        if (opened) return;
        try {
          ws.close();
        } catch {
          // ignore
        }
        if (this.ws === ws) {
          this.ws = null;
        }
        reject(new Error("workspace active snapshot ws timeout"));
      }, 4000);

      ws.onopen = () => {
        opened = true;
        globalThis.clearTimeout(timeoutId);
        if (this.destroyed) {
          try {
            ws.close();
          } catch {
            // ignore
          }
          if (this.ws === ws) {
            this.ws = null;
          }
          resolve();
          return;
        }
        this.reconnectDelayMs = 1000;
        this.lastStreamSeq = 0;
        this.snapshot.connection = "connected";
        this.publish();
        this.flushSubscriptions("ws_open");
        resolve();
      };

      ws.onmessage = (event) => {
        this.enqueueStreamMessage(event.data);
      };

      ws.onerror = () => {
        globalThis.clearTimeout(timeoutId);
        if (!opened) {
          if (this.ws === ws) {
            this.ws = null;
          }
          reject(new Error("workspace active snapshot ws error"));
        }
      };

      ws.onclose = () => {
        if (this.ws === ws) {
          this.ws = null;
        }
        this.snapshot.connection = "disconnected";
        this.publish();
        this.scheduleReconnect();
      };
    });
  }

  private scheduleReconnect() {
    if (this.reconnectTimer || this.destroyed) return;
    const delay = this.reconnectDelayMs;
    this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, 15000);
    this.reconnectTimer = globalThis.setTimeout(() => {
      this.reconnectTimer = null;
      this.connectStream().catch(() => {});
    }, delay);
  }

  private enqueueStreamMessage(data: unknown) {
    this.streamQueue = this.streamQueue
      .then(() => this.handleStreamMessage(data))
      .catch(() => {});
  }

  private async handleStreamMessage(data: unknown) {
    if (this.e2eDropStreamMessages) return;
    const parsed = await parseWsJson(data);
    if (!parsed || typeof parsed !== "object") return;
    const streamRev = readWorkspaceStreamRev(parsed);
    if (typeof streamRev === "number") {
      if (this.lastStreamSeq > 0 && streamRev < this.lastStreamSeq) {
        this.lastStreamSeq = streamRev;
        this.allowSnapshotReset = true;
        this.flushSubscriptions("stream_seq_reset");
      } else if (this.lastStreamSeq > 0 && streamRev > this.lastStreamSeq + 1) {
        this.allowSnapshotReset = true;
        this.flushSubscriptions("stream_seq_gap");
      }
      this.lastStreamSeq = Math.max(this.lastStreamSeq, streamRev);
    }
    const normalized = this.unwrapEvent(parsed);
    if (!normalized || typeof normalized !== "object") return;
    const parsedType = (normalized as { type?: string }).type;
    if (parsedType === "reset_required") {
      const latestRev =
        (normalized as { latest_rev?: number }).latest_rev ??
        (normalized as { latestRev?: number }).latestRev ??
        0;
      if (typeof latestRev === "number") {
        this.snapshotRev = Math.max(this.snapshotRev, latestRev);
      }
      this.lastStreamSeq = 0;
      this.allowSnapshotReset = true;
      this.liveSnapshotApplied = false;
      this.flushSubscriptions("reset_required");
      return;
    }
    const wsSnapshot = readWorkspaceSnapshotPayload(parsed);
    if (wsSnapshot) {
      this.applyWorkspaceSnapshot(wsSnapshot.snapshot, wsSnapshot.heads);
      return;
    }
    const headsBatch = readWorkspaceHeadsBatchPayload(parsed);
    if (headsBatch) {
      const batchRev = headsBatch.snapshotRev;
      if (typeof batchRev === "number") {
        if (batchRev < this.snapshotRev) {
          this.snapshotRev = batchRev;
          this.allowSnapshotReset = true;
          if (this.liveSnapshotApplied) {
            this.flushSubscriptions("snapshot_rev_reset");
          }
        } else {
          this.snapshotRev = Math.max(this.snapshotRev, batchRev);
        }
      }
      let changed = false;
      for (const delta of headsBatch.deltas) {
        if (this.applySessionHeadDelta(delta)) {
          changed = true;
        }
        this.notifyEventListeners({
          type: "session_head_delta",
          workspace_id: this.workspaceId,
          snapshot_rev: batchRev,
          delta,
        });
      }
      if (changed) {
        this.publish();
        this.schedulePersistCache();
      }
      return;
    }
    const evt = normalized as WorkspaceActiveSnapshotEvent;
    if (typeof evt.snapshot_rev === "number") {
      if (evt.snapshot_rev < this.snapshotRev) {
        this.snapshotRev = evt.snapshot_rev;
        this.allowSnapshotReset = true;
        if (this.liveSnapshotApplied) {
          this.flushSubscriptions("snapshot_rev_reset");
        }
      } else {
        this.snapshotRev = Math.max(this.snapshotRev, evt.snapshot_rev);
      }
    }
    if ("archived_rev" in evt && typeof evt.archived_rev === "number") {
      if (evt.archived_rev !== this.archivedRev) {
        this.archivedRev = evt.archived_rev;
        this.archivedLoaded = false;
        this.archivedCursor = null;
      }
    }

    switch (evt.type) {
      case "ready":
        this.snapshot.connection = "connected";
        this.publish();
        break;
      case "task_delta":
        if (this.applyTaskDelta(evt)) {
          this.activeSessionIds = this.collectActiveSessionIds();
          this.publish();
        }
        break;
      case "active_task_upsert":
        this.upsertActiveSummary(evt.task);
        this.activeSessionIds = this.collectActiveSessionIds();
        this.publish();
        break;
      case "active_task_delete":
        this.removeTask(idToString(evt.task_id), { adjustCounts: true });
        this.activeSessionIds = this.collectActiveSessionIds();
        this.publish();
        break;
      case "archived_task_upsert": {
        const item = this.buildArchivedItem(evt.task, null);
        if (item) {
          this.upsertArchivedItem(item);
          this.publish();
        }
        break;
      }
      case "archived_task_delete":
        this.removeTask(idToString(evt.task_id), { adjustCounts: true });
        this.publish();
        break;
      case "session_summary":
        this.applySessionSummary(evt.summary);
        this.publish();
        break;
      case "session_summary_delta":
        if (this.applySessionSummaryDelta(evt)) {
          this.publish();
        }
        break;
      case "session_head_delta":
        if (this.applySessionHeadDelta(evt.delta)) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      case "session_head_seed":
        if (this.applySessionHeadSeed(evt.head)) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      case "session_gap": {
        this.flushSubscriptions("session_gap");
        break;
      }
      case "worktree_bootstrap": {
        const worktreeId = idToString(evt.notice.worktree_id);
        const root = String(evt.notice.worktree_root ?? "").trim();
        if (worktreeId && root && this.worktreeRootsById.get(worktreeId) !== root) {
          this.worktreeRootsById.set(worktreeId, root);
          this.publish();
        }
        break;
      }
      case "worktree_vcs_snapshot": {
        const worktreeId = idToString(evt.snapshot.worktree_id);
        if (!worktreeId) break;
        const prev = this.snapshot.worktreeVcsById[worktreeId];
        if (prev?.rev === evt.snapshot.rev) break;
        this.snapshot.worktreeVcsById = {
          ...this.snapshot.worktreeVcsById,
          [worktreeId]: evt.snapshot,
        };
        this.publish();
        this.schedulePersistCache();
        break;
      }
      default:
        break;
    }

    this.notifyEventListeners(evt);
  }

  private normalizeE2EStreamPayload(payload: unknown): string | null {
    if (typeof payload === "string") return payload;
    try {
      return JSON.stringify(payload);
    } catch {
      return null;
    }
  }

  private unwrapEvent(value: unknown): unknown {
    if (!value || typeof value !== "object") return value;
    const rec = value as { type?: string; event?: unknown };
    if (rec.type === "event" && rec.event && typeof rec.event === "object") {
      return rec.event;
    }
    return value;
  }

  private notifyEventListeners(evt: WorkspaceActiveSnapshotEvent) {
    for (const listener of this.eventListeners) {
      listener(evt);
    }
    if (this.workerPatchEmitter) {
      this.workerPatchPendingEvents.push(evt);
      this.scheduleWorkerPatchFlush();
    }
  }

  private flushSubscriptions(reason = "subscribe") {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const { message, requestSnapshot } = buildWorkspaceActiveSubscribeMessage(
      reason,
      this.foregroundTaskId,
      this.subscribedSessionIds,
    );
    if (requestSnapshot) {
      this.scheduleSnapshotWarning(reason);
    }
    try {
      ws.send(JSON.stringify(message));
    } catch {
      // ignore send errors
    }
  }

  private collectActiveSessionIds(): string[] {
    return collectWorkspaceActivePrimarySessionIds({
      activeIds: this.activeOrder,
      archivedIds: this.archivedOrder,
      tasksById: Object.fromEntries(this.tasks.entries()),
    });
  }

  private refreshActiveSessionSubscriptions(reason: string) {
    const next = this.collectActiveSessionIds();
    if (next.join("|") === this.activeSessionIds.join("|")) return;
    this.activeSessionIds = next;
    this.flushSubscriptions(reason);
  }

  private scheduleForegroundTaskFlush() {
    if (this.foregroundTaskTimer) {
      globalThis.clearTimeout(this.foregroundTaskTimer);
    }
    this.foregroundTaskTimer = globalThis.setTimeout(() => {
      this.foregroundTaskTimer = null;
      this.flushSubscriptions("foreground_task");
    }, FOREGROUND_TASK_DEBOUNCE_MS);
  }

  private removeTask(taskId: string | undefined, opts?: { adjustCounts?: boolean }) {
    const deleteId = idToString(taskId ?? "");
    if (!deleteId) return;
    const existing = this.tasks.get(deleteId);
    if (!existing) return;
    if (existing.primarySessionId) {
      this.sessionHeadsById.delete(existing.primarySessionId);
    }
    this.tasks.delete(deleteId);
    this.activeOrder = this.activeOrder.filter((id) => id !== deleteId);
    this.archivedOrder = this.archivedOrder.filter((id) => id !== deleteId);
    if (opts?.adjustCounts) {
      if (existing.task.archived_at) {
        this.totalArchived = Math.max(0, this.totalArchived - 1);
      } else {
        this.totalActive = Math.max(0, this.totalActive - 1);
      }
    }
    this.schedulePersistCache();
  }

  private upsertActiveSummary(summary: WorkspaceActiveTaskSummary) {
    const existing = this.tasks.get(idToString(summary.task.id));
    const normalized = this.normalizeActiveSummary(summary, existing);
    if (existing?.primarySessionId && existing.primarySessionId !== normalized.primarySessionId) {
      this.sessionHeadsById.delete(existing.primarySessionId);
    }
    this.tasks.set(normalized.id, normalized);
    if (existing) {
      this.updateCountsForMove(existing, normalized);
    } else {
      this.totalActive += 1;
    }
    this.placeInOrders(normalized);
    this.schedulePersistCache();
  }

  private upsertArchivedItem(item: WorkspaceActiveSnapshotItem, opts?: { adjustCounts?: boolean }) {
    const existing = this.tasks.get(item.id);
    this.tasks.set(item.id, item);
    const adjustCounts = opts?.adjustCounts ?? true;
    if (adjustCounts) {
      if (existing) {
        this.updateCountsForMove(existing, item);
      } else {
        this.totalArchived += 1;
      }
    }
    this.placeInOrders(item);
  }

  private updateCountsForMove(prev: WorkspaceActiveSnapshotItem, next: WorkspaceActiveSnapshotItem) {
    const prevArchived = Boolean(prev.task.archived_at);
    const nextArchived = Boolean(next.task.archived_at);
    if (prevArchived === nextArchived) return;
    if (prevArchived) {
      this.totalArchived = Math.max(0, this.totalArchived - 1);
      this.totalActive += 1;
    } else {
      this.totalActive = Math.max(0, this.totalActive - 1);
      this.totalArchived += 1;
    }
  }

  private applyTaskDelta(evt: WorkspaceActiveSnapshotTaskDeltaEvent): boolean {
    const delta = evt.delta;
    const taskId = idToString(delta?.task?.id ?? "");
    if (!taskId) return false;

    const existing = this.tasks.get(taskId);
    if (!existing) return false;

    if (delta.kind === "archived") {
      this.removeTask(taskId, { adjustCounts: true });
      return true;
    }

    // The server-side projection only emits `task_delta` for tasks it already has
    // in its active snapshot. Mirror that behavior here to avoid "creating" tasks
    // from partial context.
    const nextTask = { ...existing.task, ...delta.task, id: existing.task.id, workspace_id: existing.task.workspace_id };
    const sortAt = this.taskSortAt(nextTask, existing.sort_at ?? null);
    const sortAtMs = Date.parse(sortAt) || existing.sortAtMs || Date.now();
    const nextPrimarySessionId = idToString(nextTask.primary_session_id ?? "") || null;
    if (existing.primarySessionId && existing.primarySessionId !== nextPrimarySessionId) {
      this.sessionHeadsById.delete(existing.primarySessionId);
    }
    const primarySessionHead = nextPrimarySessionId ? (this.sessionHeadsById.get(nextPrimarySessionId) ?? null) : null;
    const nextItem: WorkspaceActiveSnapshotItem = {
      ...existing,
      task: nextTask,
      primarySessionId: nextPrimarySessionId,
      primarySessionHead,
      sortAtMs,
      sort_at: sortAt || null,
    };
    this.tasks.set(taskId, nextItem);
    this.placeInOrders(nextItem);
    this.schedulePersistCache();
    return true;
  }

  private applySessionSummaryDelta(evt: WorkspaceActiveSnapshotSessionSummaryDeltaEvent): boolean {
    const delta = evt.delta;
    const sessionId = idToString(delta.session_id ?? "");
    if (!sessionId) return false;
    const taskIdHint = idToString(delta.task_id ?? "");

    const tryUpdate = (taskId: string): boolean => {
      const task = this.tasks.get(taskId);
      if (!task) return false;
      const nextSessions = task.sessions.slice();
      const sessionIdx = nextSessions.findIndex((s) => idToString(s.session.id) === sessionId);
      if (sessionIdx < 0) return false;
      const current = nextSessions[sessionIdx];
      const nextSummary: SessionSnapshotSummary = { ...current };
      let changed = false;

      if (hasOwnProperty(delta, "last_message_at")) {
        const incoming = delta.last_message_at;
        if (typeof incoming === "string" && incoming) {
          const current = nextSummary.last_message_at;
          const incMs = Date.parse(incoming);
          const curMs = current ? Date.parse(current) : NaN;
          const shouldUpdate =
            !current ||
            (Number.isFinite(incMs) && Number.isFinite(curMs) ? incMs > curMs : incoming > current);
          if (shouldUpdate && nextSummary.last_message_at !== incoming) {
            nextSummary.last_message_at = incoming;
            changed = true;
          }
        }
      }
      if (hasOwnProperty(delta, "last_message_preview")) {
        const incoming = delta.last_message_preview;
        if (typeof incoming === "string") {
          const next = incoming.length ? incoming : null;
          if (nextSummary.last_message_preview !== next) {
            nextSummary.last_message_preview = next;
            changed = true;
          }
        } else if (incoming === null) {
          if (nextSummary.last_message_preview !== null) {
            nextSummary.last_message_preview = null;
            changed = true;
          }
        }
      }
      if (hasOwnProperty(delta, "last_event_seq")) {
        const incoming = delta.last_event_seq;
        if (typeof incoming === "number") {
          const next = Math.max(nextSummary.last_event_seq ?? incoming, incoming);
          if (nextSummary.last_event_seq !== next) {
            nextSummary.last_event_seq = next;
            changed = true;
          }
        }
      }
      if (typeof delta.state_rev === "number") {
        const current = nextSummary.state_rev ?? 0;
        if (delta.state_rev > current) {
          nextSummary.state_rev = delta.state_rev;
          changed = true;
        }
      }
      if (hasOwnProperty(delta, "activity")) {
        if (delta.activity) {
          const prevActivity = nextSummary.activity ?? { is_working: false, last_turn_status: null };
          const nextActivity = { ...prevActivity };
          if (typeof delta.activity.is_working === "boolean") {
            nextActivity.is_working = delta.activity.is_working;
          }
          if (hasOwnProperty(delta.activity, "last_turn_status")) {
            nextActivity.last_turn_status = delta.activity.last_turn_status ?? null;
          }
          if (
            nextSummary.activity?.is_working !== nextActivity.is_working ||
            (nextSummary.activity?.last_turn_status ?? null) !== (nextActivity.last_turn_status ?? null)
          ) {
            nextSummary.activity = nextActivity;
            changed = true;
          }
        }
      }

      if (!changed) return false;
      nextSessions[sessionIdx] = nextSummary;
      this.tasks.set(taskId, { ...task, sessions: sortSessionSummaries(nextSessions) });
      this.schedulePersistCache();
      return true;
    };

    if (taskIdHint && tryUpdate(taskIdHint)) return true;

    for (const taskId of this.tasks.keys()) {
      if (taskIdHint && taskId === taskIdHint) continue;
      if (tryUpdate(taskId)) return true;
    }
    return false;
  }

  private applySessionSummary(summary: SessionSnapshotSummary) {
    const taskId = idToString(summary.session.task_id);
    if (!taskId) return;
    const task = this.tasks.get(taskId);
    if (!task) return;
    const nextSessions = task.sessions.slice();
    const sessionId = idToString(summary.session.id);
    const sessionIdx = nextSessions.findIndex((s) => idToString(s.session.id) === sessionId);
    const normalized = this.normalizeSessionSummary(summary);
    if (sessionIdx >= 0) {
      nextSessions[sessionIdx] = normalized;
    } else {
      nextSessions.push(normalized);
    }
    this.tasks.set(taskId, { ...task, sessions: sortSessionSummaries(nextSessions) });
    this.schedulePersistCache();
  }

  private seedHeadSnapshot(sessionId: string): SessionHeadSnapshot | null {
    for (const item of this.tasks.values()) {
      for (const summary of item.sessions) {
        const candidateId = idToString(summary.session.id);
        if (candidateId !== sessionId) continue;
        return {
          session: summary.session,
          turns: [],
          tool_summaries: [],
          events: [],
          messages: [],
          last_event_seq: summary.last_event_seq ?? 0,
          state_rev: summary.state_rev ?? 0,
          activity: summary.activity ?? { is_working: false },
          has_more_turns: false,
          history_cursor: null,
          has_more_history: false,
          summary_checkpoint: undefined,
          head_window: emptySessionHeadWindow(),
        };
      }
    }
    return null;
  }

  private applySessionHeadDelta(delta: SessionHeadDelta): boolean {
    const sessionId = idToString(delta?.session_id ?? "");
    if (!sessionId) return false;
    let existing = this.sessionHeadsById.get(sessionId);
    if (!existing) {
      const seeded = this.seedHeadSnapshot(sessionId);
      if (!seeded) return false;
      existing = seeded;
      this.sessionHeadsById.set(sessionId, seeded);
    }
    let changed = false;
    let turns = existing.turns ?? [];
    let toolSummaries = Array.isArray(existing.tool_summaries) ? existing.tool_summaries : [];
    let messages = existing.messages ?? [];
    let events = existing.events ?? [];
    if (delta.turn) {
      turns = mergeSessionTurns(turns, [delta.turn]);
      changed = true;
    }
    if (delta.message) {
      messages = mergeSessionMessages(messages, [delta.message]);
      changed = true;
    }
    if (delta.event) {
      events = mergeSessionEvents(events, [delta.event]);
      changed = true;
    }
    const incomingToolSummaries = Array.isArray(delta.tool_summaries) ? delta.tool_summaries : [];
    if (incomingToolSummaries.length > 0) {
      toolSummaries = mergeSessionToolSummaries(toolSummaries, incomingToolSummaries, turns);
      changed = true;
    }
    const next: SessionHeadSnapshot = sanitizeSessionHeadSnapshot({
      ...existing,
      turns,
      tool_summaries: toolSummaries,
      messages,
      events,
      ...(typeof delta.last_event_seq === "number" ? { last_event_seq: delta.last_event_seq } : {}),
      ...(typeof delta.state_rev === "number" ? { state_rev: delta.state_rev } : {}),
    });

    if (!changed && next.last_event_seq === existing.last_event_seq) {
      return false;
    }
    if (!this.shouldReplaceHead(existing, next)) return false;
    this.sessionHeadsById.set(sessionId, next);
    changed = true;
    if (projectPrimarySessionHeadOntoTasks(this.tasks, next)) {
      changed = true;
    }
    return changed;
  }

  private applySessionHeadSeed(head: SessionHeadSnapshot | null | undefined): boolean {
    if (!head) return false;
    const sessionId = idToString(head?.session?.id ?? "");
    if (!sessionId) return false;
    const sanitized = sanitizeSessionHeadSnapshot(head);
    const prev = this.sessionHeadsById.get(sessionId);
    if (!this.shouldReplaceHead(prev, sanitized)) return false;
    this.sessionHeadsById.set(sessionId, sanitized);
    let changed = true;
    if (projectPrimarySessionHeadOntoTasks(this.tasks, sanitized)) {
      changed = true;
    }
    return changed;
  }

  private readPrimarySessionHead(summary: unknown): SessionHeadSnapshot | null {
    if (!summary || typeof summary !== "object") return null;
    const rec = summary as Record<string, unknown>;
    const head = rec.primary_session_head ?? rec.primarySessionHead ?? null;
    if (!head || typeof head !== "object") return null;
    return sanitizeSessionHeadSnapshot(head as SessionHeadSnapshot);
  }

  private readPrimarySessionId(summary: unknown): string | null {
    const rec = asRecord(summary);
    if (Object.keys(rec).length === 0) return null;
    const fromPrimary = idToString(readString(asRecord(asRecord(rec.primary_session).session).id) ?? "");
    if (fromPrimary) return fromPrimary;
    const fromHead = idToString(
      readString(asRecord(asRecord(rec.primary_session_head).session).id) ??
        readString(asRecord(asRecord(rec.primarySessionHead).session).id) ??
        "",
    );
    if (fromHead) return fromHead;
    return null;
  }

  private rememberSessionHead(head: SessionHeadSnapshot | null) {
    if (!head) return;
    const sessionId = idToString(head?.session?.id ?? "");
    if (!sessionId) return;
    const sanitized = sanitizeSessionHeadSnapshot(head);
    const prev = this.sessionHeadsById.get(sessionId);
    if (!this.shouldReplaceHead(prev, sanitized)) return;
    this.sessionHeadsById.set(sessionId, sanitized);
  }

  private collectArchivedHeads(): Map<string, SessionHeadSnapshot> {
    const archived = new Map<string, SessionHeadSnapshot>();
    for (const item of this.tasks.values()) {
      if (!item.task.archived_at) continue;
      const primaryId = resolvePrimarySessionId(item);
      if (!primaryId) continue;
      const head = this.sessionHeadsById.get(primaryId) ?? item.primarySessionHead ?? null;
      if (head) archived.set(primaryId, head);
    }
    return archived;
  }

  private normalizeActiveSummary(
    summary: WorkspaceActiveTaskSummary | PersistedWorkspaceActiveTaskSummaryV1,
    existing?: WorkspaceActiveSnapshotItem,
  ): WorkspaceActiveSnapshotItem {
    const id = idToString(summary.task.id);
    const summaryHasPrimary = hasOwnProperty(summary, "primary_session") || hasOwnProperty(summary, "primarySession");
    const summaryHasSessions = hasOwnProperty(summary, "sessions");
    const summaryHasHead =
      hasOwnProperty(summary, "primary_session_head") || hasOwnProperty(summary, "primarySessionHead");
    const summaryHasSortAt = hasOwnProperty(summary, "sort_at") || hasOwnProperty(summary, "sortAt");

    const fallbackSortAt = summaryHasSortAt
      ? (summary as PersistedWorkspaceActiveTaskSummaryV1).sort_at ?? null
      : existing?.sort_at ?? null;
    const sortAt = this.taskSortAt(summary.task, fallbackSortAt);
    const sortAtMs = Date.parse(sortAt) || existing?.sortAtMs || Date.now();
    const existingPrimarySessionId = resolvePrimarySessionId(existing);
    const primarySessionId =
      this.readPrimarySessionId(summary) ||
      existingPrimarySessionId ||
      idToString((summary as PersistedWorkspaceActiveTaskSummaryV1).primary_session?.session?.id ?? "");

    let primaryHead = summaryHasHead ? this.readPrimarySessionHead(summary) : existing?.primarySessionHead ?? null;
    if (!primaryHead && primarySessionId) {
      primaryHead = this.sessionHeadsById.get(primarySessionId) ?? null;
    }
    this.rememberSessionHead(primaryHead);

    const existingSessions = existing?.sessions ?? [];
    const primaryFromSummary = summaryHasPrimary
      ? (summary as PersistedWorkspaceActiveTaskSummaryV1).primary_session
      : null;
    let primarySummary = primaryFromSummary ? this.normalizeSessionSummary(primaryFromSummary) : null;
    if (!primarySummary && primarySessionId) {
      primarySummary =
        existingSessions.find((item) => idToString(item.session.id) === primarySessionId) ?? null;
    }

    const sessionsRaw = summaryHasSessions
      ? Array.isArray((summary as PersistedWorkspaceActiveTaskSummaryV1).sessions)
        ? (summary as PersistedWorkspaceActiveTaskSummaryV1).sessions
        : []
      : existingSessions;
    const sessions = sessionsRaw.map((s) => this.normalizeSessionSummary(s));

    const merged: SessionSnapshotSummary[] = [];
    const seen = new Set<string>();
    const addSummary = (item: SessionSnapshotSummary) => {
      const sid = idToString(item.session.id);
      if (!sid || seen.has(sid)) return;
      seen.add(sid);
      merged.push(item);
    };
    if (primarySummary) {
      addSummary(primarySummary);
    }
    sessions.forEach(addSummary);

    return {
      id,
      task: { ...summary.task },
      sessions: sortSessionSummaries(merged),
      primarySessionHead: primaryHead ?? null,
      primarySessionId: primarySessionId || null,
      sortAtMs,
      sort_at: sortAt || null,
    };
  }

  private taskSortAt(task: Task, fallback?: string | null): string {
    if (task.archived_at) return task.archived_at;
    if (task.created_at) return task.created_at;
    if (fallback) return fallback;
    return task.updated_at ?? "";
  }

  private buildArchivedItem(
    summary: WorkspaceTaskSummary,
    primaryHead?: SessionHeadSnapshot | null,
  ): WorkspaceActiveSnapshotItem | null {
    void primaryHead;
    const task = summary.task;
    const id = idToString(task.id);
    if (!id) return null;
    const existing = this.tasks.get(id);
    const providerIds = (summary.provider_ids ?? []).filter(Boolean);
    const summaries = existing?.sessions ?? [];
    const sessionList = summaries.map((item) => item.session).filter(Boolean);
    const summarySessions = Array.isArray(summary.sessions) ? summary.sessions : [];
    if (existing?.primarySessionHead) {
      this.rememberSessionHead(existing.primarySessionHead);
    }
    const summaryPrimaryId = this.readPrimarySessionId(summary);
    const primarySessionId =
      summaryPrimaryId ||
      this.pickArchivedSessionIdFromSummaries(task, summarySessions) ||
      this.pickArchivedSessionId(task, sessionList);
    let primarySessionHead = null;
    if (!primarySessionHead && primarySessionId) {
      primarySessionHead = this.sessionHeadsById.get(primarySessionId) ?? null;
    }
    if (!primarySessionHead && existing?.primarySessionHead) {
      primarySessionHead = existing.primarySessionHead;
    }
    // TODO: hydrate archived session summaries on selection (avoid list fan-out).
    const sortAt = this.taskSortAt(task);
    return {
      id,
      task: { ...task },
      sessions: sortSessionSummaries(summaries),
      providerIds: providerIds.length ? providerIds : existing?.providerIds,
      primarySessionId: primarySessionId || null,
      primarySessionHead: primarySessionHead ?? null,
      sortAtMs: Date.parse(sortAt) || Date.now(),
      sort_at: sortAt || null,
    };
  }

  private pickArchivedSessionId(task: Task, sessions: Session[]): string | null {
    const primaryId = idToString(task.primary_session_id ?? "");
    if (primaryId && (sessions.length === 0 || sessions.some((s) => idToString(s.id) === primaryId))) {
      return primaryId;
    }
    const nonSubagents = sessions.filter((session) => session.relationship !== "sub_agent");
    const pool = nonSubagents.length ? nonSubagents : sessions;
    const selected = pool[0];
    return selected ? idToString(selected.id) : null;
  }

  private pickArchivedSessionIdFromSummaries(task: Task, sessions: SessionSummary[]): string | null {
    const primaryId = idToString(task.primary_session_id ?? "");
    if (primaryId && (sessions.length === 0 || sessions.some((s) => idToString(s.id) === primaryId))) {
      return primaryId;
    }
    const nonSubagents = sessions.filter((session) => session.relationship !== "sub_agent");
    const pool = nonSubagents.length ? nonSubagents : sessions;
    const selected = pool[0];
    return selected ? idToString(selected.id) : null;
  }

  private normalizeSessionSummary(summary: SessionSnapshotSummary): SessionSnapshotSummary {
    return {
      session: { ...summary.session },
      last_message_at: summary.last_message_at ?? null,
      last_message_preview: summary.last_message_preview ?? null,
      last_event_seq: summary.last_event_seq ?? null,
      state_rev: summary.state_rev ?? undefined,
      activity: summary.activity ?? { is_working: false, last_turn_status: null },
      unread: summary.unread,
    };
  }

  private sessionToSummary(session: Session): SessionSnapshotSummary {
    return this.normalizeSessionSummary({
      session,
      last_message_at: null,
      last_message_preview: null,
      last_event_seq: null,
      state_rev: undefined,
      activity: { is_working: false, last_turn_status: null },
      unread: undefined,
    });
  }

  private placeInOrders(item: WorkspaceActiveSnapshotItem) {
    const { id } = item;
    this.activeOrder = this.activeOrder.filter((existing) => existing !== id);
    this.archivedOrder = this.archivedOrder.filter((existing) => existing !== id);
    if (item.task.archived_at) {
      this.archivedOrder.splice(this.findInsertIndex(this.archivedOrder, item.sortAtMs, id), 0, id);
    } else {
      this.activeOrder.splice(this.findInsertIndex(this.activeOrder, item.sortAtMs, id), 0, id);
    }
  }

  private findInsertIndex(order: string[], sortAt: number, id: string): number {
    let low = 0;
    let high = order.length;
    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      const midId = order[mid];
      const midItem = this.tasks.get(midId);
      const midSort = midItem?.sortAtMs ?? 0;
      if (sortAt > midSort || (sortAt === midSort && id > midId)) {
        high = mid;
      } else {
        low = mid + 1;
      }
    }
    return low;
  }

  private setFetchState(target: "active" | "archived", state: "idle" | "loading" | "error") {
    if (this.snapshot.fetchState[target] === state) return;
    this.snapshot.fetchState = { ...this.snapshot.fetchState, [target]: state };
    this.publish();
  }

  private publish() {
    const tasksById: Record<string, WorkspaceActiveSnapshotItem> = {};
    for (const [id, item] of this.tasks.entries()) {
      tasksById[id] = item;
    }
    this.hasMoreActive = this.totalActive > this.activeOrder.length;
    this.snapshot = {
      ...this.snapshot,
      tasksById,
      activeIds: [...this.activeOrder],
      archivedIds: [...this.archivedOrder],
      totalActive: this.totalActive,
      totalArchived: this.totalArchived,
      archivedRev: this.archivedRev,
      hasMoreActive: this.hasMoreActive,
      hasMoreArchived: this.hasMoreArchived,
      archivedLoaded: this.archivedLoaded,
    };
    if (this.workerPatchEmitter) {
      this.workerPatchDirty = true;
      this.scheduleWorkerPatchFlush();
    }
    for (const l of this.listeners) l();
  }
}
