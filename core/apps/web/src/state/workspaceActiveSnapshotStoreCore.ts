import type {
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  Task,
  WorktreeVcsSnapshot,
  WorkspaceActiveSnapshot,
  WorkspaceActiveSnapshotEvent,
  WorkspaceActiveSnapshotSessionSummaryDeltaEvent,
  WorkspaceActiveSnapshotTaskDeltaEvent,
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
import { parseWsJson } from "../utils/wsJson";
import { emitUiDiagnostic, normalizeDiagnosticErrorMessage } from "./diagnosticsChannel";
import type {
  WorkspaceActiveSnapshotCommand,
  WorkspaceActiveSnapshotPatch,
  WorkspaceActiveSnapshotWorkerMessage,
} from "./workspaceActiveSnapshotProtocol";
import type { SessionSubscriptionCursor } from "./sessionSubscription";
import { WorkspaceActiveSnapshotStoreState } from "./workspaceActiveSnapshot/storeState";
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
  scheduleForegroundTaskFlush as scheduleActiveSnapshotForegroundTaskFlush,
  setDropActiveSnapshotMessages,
  setE2EEnabled,
  setForegroundTaskId,
  setSubscribedSessions,
  unwrapEvent,
  type WorkspaceActiveSnapshotControlHost,
} from "./workspaceActiveSnapshot/controls";
import {
  resolveWorkerConnectionState,
  type WorkerAuthUpdateConfig,
} from "./workspaceActiveSnapshot/workerConnection";
import {
  readWorkspaceHeadsBatchPayload,
  readWorkspaceSnapshotPayload,
  readWorkspaceStreamRev,
} from "./workspaceActiveSnapshot/transport";
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

const ACTIVE_PAGE_SIZE = 50;
const SNAPSHOT_WAIT_MS = 1200;
const WORKSPACE_PATCH_FLUSH_MS = 50;

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
  private workerPatchFlushMs = WORKSPACE_PATCH_FLUSH_MS;
  authTokenOverride: string | null = null;
  wsBaseUrlOverride: string | null = null;
  private configUnsubscribe: (() => void) | null = null;
  private listWorkspaceArchivedTaskSummariesFn: typeof listWorkspaceArchivedTaskSummaries;
  subscribedSessions: SessionSubscriptionCursor[] = [];
  foregroundTaskId: string | null = null;
  foregroundTaskTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
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

  setForegroundTaskId = (taskId: string | null) =>
    setForegroundTaskId(this, taskId);

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
    this.state.applyWorkerPatch(patch);
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
      snapshot: this.getSnapshot(),
      sessionHeads: this.getSessionHeadsSnapshot(),
      worktreeRoots: this.getWorktreeRootsSnapshot(),
      events,
      snapshotRev: this.state.getSnapshotRev(),
      archivedRev: this.state.getArchivedRev(),
      activeSessionIds: this.state.getActiveSessionIds(),
      persist: this.workerPatchPendingPersist,
    };
    this.workerPatchDirty = false;
    this.workerPatchPendingPersist = false;
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
    if (this.destroyed || !snapshot || typeof snapshot !== "object") return;
    const incomingRev = typeof snapshot.snapshot_rev === "number" ? snapshot.snapshot_rev : 0;
    const currentRev = this.state.getSnapshotRev();
    const allowLower = !this.state.hasLiveSnapshotApplied() || this.allowSnapshotReset;
    if (incomingRev < currentRev && !allowLower) {
      return;
    }
    const resetSnapshotRev = incomingRev < currentRev;
    this.allowSnapshotReset = false;
    this.state.applyWorkspaceSnapshot(snapshot, heads, { resetSnapshotRev });
    this.clearSnapshotWarning();
    this.publish();
    this.schedulePersistCache();
  }

  private async fetchArchivedPage(firstLoad: boolean) {
    if (this.destroyed) return;
    if (firstLoad) {
      this.state.resetArchivedCursor();
    }
    const cursor = this.state.getArchivedCursor();
    if (!firstLoad && !cursor) {
      if (this.state.markArchivedExhausted()) {
        this.publish();
      }
      return;
    }
    this.setFetchState("archived", "loading");
    try {
      const page = await this.listWorkspaceArchivedTaskSummariesFn(this.workspaceId, {
        limit: ACTIVE_PAGE_SIZE,
        cursor: cursor ?? undefined,
      });
      const summaries = await Promise.all(page.tasks.map((task) => this.state.buildArchivedItem(task)));
      this.state.applyArchivedPage(page, summaries);
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
    if (this.state.setConnection("connecting")) {
      this.publish();
    }
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
        if (this.state.setConnection("disconnected")) {
          this.publish();
        }
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
        if (this.state.setConnection("disconnected")) {
          this.publish();
        }
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
        if (this.state.setConnection("connected")) {
          this.publish();
        }
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
        if (this.state.setConnection("disconnected")) {
          this.publish();
        }
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

  enqueueStreamMessage(data: unknown) {
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
    const normalized = unwrapEvent(parsed);
    if (!normalized || typeof normalized !== "object") return;
    const parsedType = (normalized as { type?: string }).type;
    if (parsedType === "reset_required") {
      const latestRev =
        (normalized as { latest_rev?: number }).latest_rev ??
        (normalized as { latestRev?: number }).latestRev ??
        0;
      if (typeof latestRev === "number") {
        this.state.updateSnapshotRev(latestRev);
      }
      this.lastStreamSeq = 0;
      this.allowSnapshotReset = true;
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
        if (batchRev < this.state.getSnapshotRev()) {
          this.state.updateSnapshotRev(batchRev, { allowReset: true });
          this.allowSnapshotReset = true;
          if (this.state.hasLiveSnapshotApplied()) {
            this.flushSubscriptions("snapshot_rev_reset");
          }
        } else {
          this.state.updateSnapshotRev(batchRev);
        }
      }
      let changed = false;
      for (const delta of headsBatch.deltas) {
        if (this.state.applySessionHeadDelta(delta)) {
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
      if (evt.snapshot_rev < this.state.getSnapshotRev()) {
        this.state.updateSnapshotRev(evt.snapshot_rev, { allowReset: true });
        this.allowSnapshotReset = true;
        if (this.state.hasLiveSnapshotApplied()) {
          this.flushSubscriptions("snapshot_rev_reset");
        }
      } else {
        this.state.updateSnapshotRev(evt.snapshot_rev);
      }
    }
    if ("archived_rev" in evt && typeof evt.archived_rev === "number") {
      this.state.updateArchivedRev(evt.archived_rev);
    }

    let flushAfterNotifyReason: string | null = null;
    switch (evt.type) {
      case "ready":
        if (this.state.setConnection("connected")) {
          this.publish();
        }
        break;
      case "task_delta":
        if (this.applyTaskDelta(evt)) {
          this.publish();
        }
        break;
      case "active_task_upsert":
        if (this.state.upsertActiveSummary(evt.task)) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      case "active_task_delete":
        if (this.state.removeTask(idToString(evt.task_id), { adjustCounts: true })) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      case "archived_task_upsert": {
        const item = this.state.buildArchivedItem(evt.task, null);
        if (item && this.state.upsertArchivedItem(item)) {
          this.publish();
        }
        break;
      }
      case "archived_task_delete":
        if (this.state.removeTask(idToString(evt.task_id), { adjustCounts: true })) {
          this.publish();
        }
        break;
      case "session_summary":
        if (this.state.applySessionSummary(evt.summary)) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      case "session_summary_delta":
        if (this.applySessionSummaryDelta(evt)) {
          this.publish();
        }
        break;
      case "session_head_delta":
        if (this.state.applySessionHeadDelta(evt.delta)) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      case "session_head_seed":
        if (this.state.applySessionHeadSeed(evt.head)) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      case "session_gap": {
        flushAfterNotifyReason = "session_gap";
        break;
      }
      case "worktree_bootstrap": {
        if (this.state.applyWorktreeRoot(idToString(evt.notice.worktree_id), String(evt.notice.worktree_root ?? ""))) {
          this.publish();
        }
        break;
      }
      case "worktree_vcs_snapshot": {
        if (this.state.applyWorktreeVcsSnapshot(evt.snapshot)) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      }
      default:
        break;
    }

    this.notifyEventListeners(evt);
    if (flushAfterNotifyReason) {
      this.flushSubscriptions(flushAfterNotifyReason);
    }
  }

  private notifyEventListeners(evt: WorkspaceActiveSnapshotEvent) {
    notifyEventListeners(this, evt);
  }

  flushSubscriptions = (reason = "subscribe") =>
    flushActiveSnapshotSubscriptions(this, reason);

  scheduleForegroundTaskFlush = () =>
    scheduleActiveSnapshotForegroundTaskFlush(this);

  private applyTaskDelta(evt: WorkspaceActiveSnapshotTaskDeltaEvent): boolean {
    const changed = this.state.applyTaskDelta(evt);
    if (changed) {
      this.schedulePersistCache();
    }
    return changed;
  }

  private applySessionSummaryDelta(evt: WorkspaceActiveSnapshotSessionSummaryDeltaEvent): boolean {
    const changed = this.state.applySessionSummaryDelta(evt);
    if (changed) {
      this.schedulePersistCache();
    }
    return changed;
  }

  private setFetchState(target: "active" | "archived", state: "idle" | "loading" | "error") {
    if (!this.state.setFetchState(target, state)) return;
    this.publish();
  }

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
