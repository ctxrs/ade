import type {
  Message,
  Session,
  SessionEvent,
  SessionHeadDelta,
  SessionHeadSnapshot,
  SessionHeadWindow,
  SessionSummary,
  SessionSnapshotSummary,
  SessionTurn,
  Task,
  WorkspaceActiveSnapshot,
  WorkspaceActiveSnapshotEvent,
  WorkspaceActiveTaskSummary,
  WorkspaceIndexCursor,
  WorkspaceTaskSummary,
} from "@ctx/types";
import {
  resolveDaemonBaseUrl,
  resolveDaemonWsBaseUrl,
  getDaemonBaseUrl,
  getHealth,
  idToString,
  listWorkspaceArchivedTaskSummaries,
  type WorkspaceActiveSnapshotClientMessage,
} from "../api/client";
import {
  loadWorkspaceActiveSnapshotV1,
  saveWorkspaceActiveSnapshotV1,
  type PersistedWorkspaceActiveSnapshotV1,
  type PersistedWorkspaceActiveTaskSummaryV1,
} from "./uiStateStore";
import { parseWsJson } from "../utils/wsJson";
import type {
  WorkspaceActiveSnapshotCommand,
  WorkspaceActiveSnapshotPatch,
  WorkspaceActiveSnapshotWorkerMessage,
} from "./workspaceActiveSnapshotProtocol";

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
  getWorktreeRoot: (worktreeId: string) => string | null;
  setSubscribedSessionIds?: (sessionIds: string[]) => void;
  setForegroundTaskId?: (taskId: string | null) => void;
};

type WorkspaceActiveSnapshotStoreOptions = {
  disableCache?: boolean;
  disableWorker?: boolean;
  onPersistRequested?: () => void;
  onPatch?: (patch: WorkspaceActiveSnapshotPatch) => void;
  patchFlushMs?: number;
  authToken?: string | null;
  wsBaseUrl?: string | null;
  listWorkspaceArchivedTaskSummaries?: typeof listWorkspaceArchivedTaskSummaries;
};

const ACTIVE_PAGE_SIZE = 50;
const SNAPSHOT_WAIT_MS = 1200;
const FOREGROUND_TASK_DEBOUNCE_MS = 150;
const WORKSPACE_PATCH_FLUSH_MS = 50;
const shouldRequestSnapshot = (reason: string): boolean => {
  switch (reason) {
    case "ws_open":
    case "reset_required":
    case "snapshot_rev_reset":
    case "stream_seq_gap":
    case "stream_seq_reset":
      return true;
    default:
      return false;
  }
};

const authToken = (): string | null => {
  try {
    return sessionStorage.getItem("ctxAuthToken");
  } catch {
    return null;
  }
};

const dedupeUrls = (urls: string[]): string[] => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const url of urls) {
    if (seen.has(url)) continue;
    seen.add(url);
    out.push(url);
  }
  return out;
};

const toWsBaseUrl = (base: string): string => {
  const trimmed = base.replace(/\/+$/, "");
  if (trimmed.startsWith("ws://") || trimmed.startsWith("wss://")) return trimmed;
  if (trimmed.startsWith("https://")) return trimmed.replace(/^https:\/\//, "wss://");
  if (trimmed.startsWith("http://")) return trimmed.replace(/^http:\/\//, "ws://");
  return trimmed;
};

const toHttpBaseUrl = (base: string): string => {
  const trimmed = base.replace(/\/+$/, "");
  if (trimmed.startsWith("ws://")) return trimmed.replace(/^ws:\/\//, "http://");
  if (trimmed.startsWith("wss://")) return trimmed.replace(/^wss:\/\//, "https://");
  return trimmed;
};

const sortSessionSummaries = (summaries: SessionSnapshotSummary[]): SessionSnapshotSummary[] => {
  return summaries
    .slice()
    .sort((a, b) => String(a.session.created_at ?? "").localeCompare(String(b.session.created_at ?? "")));
};

const hasOwnProperty = (value: unknown, key: string): boolean => {
  if (!value || typeof value !== "object") return false;
  return Object.prototype.hasOwnProperty.call(value, key);
};

const isWorkspaceActiveSnapshot = (value: unknown): value is WorkspaceActiveSnapshot => {
  if (!value || typeof value !== "object") return false;
  const active = (value as WorkspaceActiveSnapshot).active as { tasks?: unknown } | undefined;
  if (!active || typeof active !== "object") return false;
  return Array.isArray(active.tasks);
};

const readWorkspaceStreamRev = (value: unknown): number | null => {
  if (!value || typeof value !== "object") return null;
  const rec = value as { rev?: unknown };
  return typeof rec.rev === "number" ? rec.rev : null;
};

const readWorkspaceSnapshotPayload = (
  value: unknown,
): { snapshot: WorkspaceActiveSnapshot; heads: SessionHeadSnapshot[] } | null => {
  if (!value || typeof value !== "object") return null;
  const rec = value as Record<string, unknown>;
  const directSnapshot =
    rec.type === "snapshot" && isWorkspaceActiveSnapshot(value) ? (value as WorkspaceActiveSnapshot) : null;
  const candidate =
    (rec.snapshot as WorkspaceActiveSnapshot | undefined) ??
    (rec.active_snapshot as WorkspaceActiveSnapshot | undefined) ??
    (rec.activeSnapshot as WorkspaceActiveSnapshot | undefined) ??
    directSnapshot ??
    null;
  if (!isWorkspaceActiveSnapshot(candidate)) return null;
  const headsPayload =
    (rec.heads as unknown) ??
    (rec.active_heads as unknown) ??
    (rec.activeHeads as unknown) ??
    [];
  const headsArray = Array.isArray(headsPayload)
    ? headsPayload
    : Array.isArray((headsPayload as { heads?: unknown }).heads)
      ? (headsPayload as { heads: SessionHeadSnapshot[] }).heads
      : [];
  return { snapshot: candidate, heads: headsArray };
};

const readWorkspaceHeadsBatchPayload = (
  value: unknown,
): { snapshotRev: number; deltas: SessionHeadDelta[] } | null => {
  if (!value || typeof value !== "object") return null;
  const rec = value as Record<string, unknown>;
  if (rec.type !== "heads_batch") return null;
  const snapshotRev =
    (rec.snapshot_rev as number | undefined) ?? (rec.snapshotRev as number | undefined) ?? 0;
  const deltas = Array.isArray(rec.deltas) ? (rec.deltas as SessionHeadDelta[]) : [];
  return { snapshotRev, deltas };
};

const PARTIAL_EVENT_TYPES = new Set(["assistant_chunk", "thought_chunk"]);
const HEAD_EVENT_BUFFER_LIMIT = 800;

const isPartialEvent = (event: SessionEvent | null | undefined): boolean => {
  if (!event) return false;
  return PARTIAL_EVENT_TYPES.has(String(event.event_type ?? ""));
};

const stripTurnPartials = (turns: SessionTurn[]): SessionTurn[] => {
  return turns.map((turn) => ({
    ...turn,
    assistant_partial: null,
    thought_partial: null,
  }));
};

const stripPartialEvents = (events: SessionEvent[]): SessionEvent[] => {
  return events.filter((event) => !isPartialEvent(event));
};

const emptyHeadWindow = (): SessionHeadWindow => ({
  turn_limit: 0,
  message_limit: 0,
  event_limit: 0,
  byte_limit: 0,
  turn_count: 0,
  message_count: 0,
  event_count: 0,
  bytes: 0,
  truncated: false,
});

const sanitizeHeadSnapshot = (head: SessionHeadSnapshot): SessionHeadSnapshot => {
  const turns = Array.isArray(head.turns) ? stripTurnPartials(head.turns) : head.turns ?? [];
  const events = Array.isArray(head.events) ? stripPartialEvents(head.events) : head.events ?? [];
  return {
    ...head,
    turns,
    events,
  };
};

const mergePartial = (p: string, n: string): string => {
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
};

const mergeTurn = (prev: SessionTurn, next: SessionTurn): SessionTurn => {
  const assistant_partial = mergePartial(prev.assistant_partial ?? "", next.assistant_partial ?? "");
  const thought_partial = mergePartial(prev.thought_partial ?? "", next.thought_partial ?? "");
  return {
    ...prev,
    ...next,
    assistant_partial,
    thought_partial,
    tool_total: Math.max(prev.tool_total ?? 0, next.tool_total ?? 0),
    tool_pending: Math.max(prev.tool_pending ?? 0, next.tool_pending ?? 0),
    tool_running: Math.max(prev.tool_running ?? 0, next.tool_running ?? 0),
    tool_completed: Math.max(prev.tool_completed ?? 0, next.tool_completed ?? 0),
    tool_failed: Math.max(prev.tool_failed ?? 0, next.tool_failed ?? 0),
  };
};

const compareTurnOrder = (a: SessionTurn, b: SessionTurn): number => {
  const sa = Number(a.start_seq ?? Number.NaN);
  const sb = Number(b.start_seq ?? Number.NaN);
  if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) {
    return sa - sb;
  }
  return String(a.started_at).localeCompare(String(b.started_at));
};

const mergeTurns = (prev: SessionTurn[], incoming: SessionTurn[]): SessionTurn[] => {
  if (incoming.length === 0) return prev;
  const byId = new Map<string, SessionTurn>();
  for (const t of prev) {
    const id = idToString(t.turn_id);
    if (id) byId.set(id, t);
  }
  for (const t of incoming) {
    const id = idToString(t.turn_id);
    if (!id) continue;
    const existing = byId.get(id);
    byId.set(id, existing ? mergeTurn(existing, t) : t);
  }
  return Array.from(byId.values()).sort(compareTurnOrder);
};

const mergeMessages = (prev: Message[], incoming: Message[]): Message[] => {
  if (incoming.length === 0) return prev;
  const byId = new Map<string, Message>();
  for (const msg of prev) {
    const id = idToString(msg.id);
    if (id) byId.set(id, msg);
  }
  for (const msg of incoming) {
    const id = idToString(msg.id);
    if (!id) continue;
    byId.set(id, msg);
  }
  return Array.from(byId.values()).sort((a, b) => {
    const c = String(a.created_at).localeCompare(String(b.created_at));
    if (c !== 0) return c;
    const sa = Number(a.turn_sequence ?? Number.NaN);
    const sb = Number(b.turn_sequence ?? Number.NaN);
    if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
    if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
    if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
    return String(idToString(a.id)).localeCompare(String(idToString(b.id)));
  });
};

const mergeEvents = (prev: SessionEvent[], incoming: SessionEvent[]): SessionEvent[] => {
  if (incoming.length === 0) return prev;
  const bySeq = new Map<number, SessionEvent>();
  for (const ev of prev) {
    if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
  }
  for (const ev of incoming) {
    if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
  }
  const next = Array.from(bySeq.values()).sort((a, b) => a.seq - b.seq);
  return next.length > HEAD_EVENT_BUFFER_LIMIT ? next.slice(-HEAD_EVENT_BUFFER_LIMIT) : next;
};

export class WorkspaceActiveSnapshotStoreImpl implements WorkspaceActiveSnapshotEventSource {
  private listeners = new Set<() => void>();
  private eventListeners = new Set<(event: WorkspaceActiveSnapshotEvent) => void>();
  private snapshot: WorkspaceActiveSnapshotState;
  private tasks = new Map<string, WorkspaceActiveSnapshotItem>();
  private sessionHeadsById = new Map<string, SessionHeadSnapshot>();
  private worktreeRootsById = new Map<string, string>();
  private worker: Worker | null = null;
  private useWorker = false;
  private disableCache = false;
  private disableWorker = false;
  private persistNotifier: (() => void) | null = null;
  private workerPatchEmitter: ((patch: WorkspaceActiveSnapshotPatch) => void) | null = null;
  private workerPatchTimer: number | null = null;
  private workerPatchPendingEvents: WorkspaceActiveSnapshotEvent[] = [];
  private workerPatchPendingPersist = false;
  private workerPatchDirty = false;
  private workerPatchFlushMs = WORKSPACE_PATCH_FLUSH_MS;
  private authTokenOverride: string | null = null;
  private wsBaseUrlOverride: string | null = null;
  private listWorkspaceArchivedTaskSummariesFn: typeof listWorkspaceArchivedTaskSummaries;
  private subscribedSessionIds: string[] = [];
  private activeSessionIds: string[] = [];
  private foregroundTaskId: string | null = null;
  private foregroundTaskTimer: number | null = null;
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
  private reconnectTimer: number | null = null;
  private reconnectDelayMs = 1000;
  private snapshotRev = 0;
  private archivedRev = 0;
  private lastStreamSeq = 0;
  private allowSnapshotReset = false;
  private cacheHydrated = false;
  private liveSnapshotApplied = false;
  private pendingWorkerCache: PersistedWorkspaceActiveSnapshotV1 | null = null;
  private snapshotWaitTimer: number | null = null;
  private cachePersistTimer: number | null = null;
  private streamQueue: Promise<void> = Promise.resolve();
  private destroyed = false;

  constructor(private workspaceId: string, opts?: WorkspaceActiveSnapshotStoreOptions) {
    this.disableCache = opts?.disableCache ?? false;
    this.disableWorker = opts?.disableWorker ?? false;
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

  getSnapshotRev = (): number => this.snapshotRev;

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
    this.ensureWorkerAvailable();
    cachePromise.finally(() => {
      if (!this.destroyed) {
        this.startWorker();
      }
    });
  };

  private startWorker() {
    if (this.worker || this.destroyed) return;
    this.useWorker = true;
    this.worker = new Worker(new URL("../workers/workspaceActiveSnapshot.worker.ts", import.meta.url), {
      type: "module",
    });
    this.worker.onmessage = (event: MessageEvent<WorkspaceActiveSnapshotWorkerMessage>) => {
      const msg = event.data;
      if (msg?.type !== "patch") return;
      this.applyWorkerPatch(msg.patch);
    };
    const auth = this.authTokenOverride ?? authToken();
    const wsBaseUrl = this.wsBaseUrlOverride ?? resolveDaemonWsBaseUrl();
    const baseUrl = resolveDaemonBaseUrl() ?? (wsBaseUrl ? toHttpBaseUrl(wsBaseUrl) : null);
    this.postWorkerCommand({
      type: "init",
      workspaceId: this.workspaceId,
      authToken: auth,
      baseUrl,
      wsBaseUrl: wsBaseUrl || null,
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
  }

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
    if (this.worker) {
      this.worker.terminate();
      this.worker = null;
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
    this.applyCachedActiveSnapshot(cached);
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

    const totalCount = Number.isFinite(cached.active?.totalCount) ? cached.active.totalCount : nextActiveIds.size;
    this.totalActive = Math.max(totalCount, nextActiveIds.size);
    this.snapshotRev = Math.max(this.snapshotRev, cached.snapshotRev ?? 0);
    this.activeSessionIds = this.collectActiveSessionIds();
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
        active: {
          tasks,
          totalCount,
        },
      });
    } catch {
      // ignore cache errors
    }
  }

  private pickPrimarySessionId(item: WorkspaceActiveSnapshotItem): string | null {
    const direct = item.primarySessionId || idToString(item.task.primary_session_id ?? "");
    if (direct) return direct;
    const headId = idToString(item.primarySessionHead?.session?.id ?? "");
    if (headId) return headId;
    const summary = item.sessions?.[0];
    const sessionId = idToString(summary?.session?.id ?? "");
    return sessionId || null;
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
    const bySession = new Map<string, SessionHeadSnapshot>();
    const byTask = new Map<string, SessionHeadSnapshot>();
    for (const head of heads) {
      if (!head || typeof head !== "object") continue;
      const sessionId = idToString(head.session?.id ?? "");
      if (!sessionId) continue;
      const sanitized = sanitizeHeadSnapshot(head);
      bySession.set(sessionId, sanitized);
      const taskId = idToString(head.session?.task_id ?? "");
      if (taskId && !byTask.has(taskId)) {
        byTask.set(taskId, sanitized);
      }
      const prev = this.sessionHeadsById.get(sessionId);
      if (this.shouldReplaceHead(prev, sanitized)) {
        this.sessionHeadsById.set(sessionId, sanitized);
        changed = true;
      }
    }

    for (const taskId of this.activeOrder) {
      const item = this.tasks.get(taskId);
      if (!item || item.task.archived_at) continue;
      const primarySessionId = this.pickPrimarySessionId(item);
      const head = (primarySessionId && bySession.get(primarySessionId)) ?? byTask.get(taskId);
      if (!head) continue;
      const headSessionId = idToString(head.session?.id ?? "");
      if (primarySessionId && headSessionId && primarySessionId !== headSessionId) {
        continue;
      }
      const existingHeadId = idToString(item.primarySessionHead?.session?.id ?? "");
      if (!primarySessionId && existingHeadId && headSessionId && existingHeadId !== headSessionId) {
        continue;
      }
      const nextPrimarySessionId = primarySessionId || headSessionId;
      const nextItem: WorkspaceActiveSnapshotItem = {
        ...item,
        primarySessionId: nextPrimarySessionId || null,
        primarySessionHead: head,
      };
      this.tasks.set(taskId, nextItem);
      changed = true;
    }
    return changed;
  }

  private buildPersistedSummary(
    item: WorkspaceActiveSnapshotItem,
  ): PersistedWorkspaceActiveTaskSummaryV1 | null {
    if (!item.task) return null;
    const sessions = Array.isArray(item.sessions) ? item.sessions : [];
    const primaryId =
      item.primarySessionId ||
      idToString(item.primarySessionHead?.session?.id ?? "");
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
      primary_session_head: head ? sanitizeHeadSnapshot(head) : null,
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
    this.activeSessionIds = this.collectActiveSessionIds();
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
    } catch {
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
      const urls = await this.resolveWsUrls();
      if (this.destroyed) return;
      for (const url of urls) {
        try {
          await this.openWebSocket(url);
          return;
        } catch {
          continue;
        }
      }
      this.snapshot.connection = "disconnected";
      this.publish();
      this.scheduleReconnect();
    } finally {
      this.connecting = false;
    }
  }

  private async resolveWsUrls(): Promise<string[]> {
    const token = this.authTokenOverride ?? authToken();
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";
    const urls: string[] = [];
    const wsBaseOverride = this.wsBaseUrlOverride ? toWsBaseUrl(this.wsBaseUrlOverride) : "";
    if (wsBaseOverride) {
      urls.push(`${wsBaseOverride}/api/workspaces/${this.workspaceId}/active_snapshot/stream${qs}`);
    }

    const location = typeof globalThis.location === "object" ? globalThis.location : null;
    if (location?.host) {
      const sameOrigin = `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/workspaces/${this.workspaceId}/active_snapshot/stream${qs}`;
      urls.push(sameOrigin);
    }

    const configured = getDaemonBaseUrl();
    if (configured) {
      const wsBase = toWsBaseUrl(configured);
      urls.push(`${wsBase}/api/workspaces/${this.workspaceId}/active_snapshot/stream${qs}`);
    } else if (!wsBaseOverride) {
      try {
        const health = await getHealth();
        const base = String(health.daemon_url || "").trim();
        if (base) {
          const wsBase = toWsBaseUrl(base);
          urls.push(`${wsBase}/api/workspaces/${this.workspaceId}/active_snapshot/stream${qs}`);
        }
      } catch {
        // ignore
      }
    }
    return dedupeUrls(urls);
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
    if (typeof evt.archived_rev === "number") {
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
      case "session_head_delta":
        if (this.applySessionHeadDelta(evt.delta)) {
          this.publish();
          this.schedulePersistCache();
        }
        break;
      case "session_head_reset":
        if (this.applySessionHeadReset(evt.head)) {
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
      default:
        break;
    }

    this.notifyEventListeners(evt);
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
    const requestSnapshot = shouldRequestSnapshot(reason);
    if (requestSnapshot) {
      this.scheduleSnapshotWarning(reason);
    }
    const message: WorkspaceActiveSnapshotClientMessage = {
      type: "subscribe",
      scope: "active",
      include_active_heads: requestSnapshot,
    };
    if (this.foregroundTaskId) {
      message.foreground_task_id = this.foregroundTaskId;
    }
    if (this.subscribedSessionIds.length > 0) {
      message.session_ids = this.subscribedSessionIds.slice();
    }
    try {
      ws.send(JSON.stringify(message));
    } catch {
      // ignore send errors
    }
  }

  private collectActiveSessionIds(): string[] {
    const ids = new Set<string>();
    for (const item of this.tasks.values()) {
      if (item.task.archived_at) continue;
      const primaryId = this.pickPrimarySessionId(item);
      if (primaryId) ids.add(primaryId);
    }
    return Array.from(ids).sort();
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
          head_window: emptyHeadWindow(),
        };
      }
    }
    return null;
  }

  private applySessionHeadDelta(delta: any): boolean {
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
    let messages = existing.messages ?? [];
    let events = existing.events ?? [];
    if (delta.turn) {
      turns = mergeTurns(turns, [delta.turn]);
      changed = true;
    }
    if (delta.message) {
      messages = mergeMessages(messages, [delta.message]);
      changed = true;
    }
    if (delta.event && !isPartialEvent(delta.event)) {
      events = mergeEvents(events, [delta.event]);
      changed = true;
    }
    const next: SessionHeadSnapshot = sanitizeHeadSnapshot({
      ...existing,
      turns,
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
    const headTaskId = idToString(next.session?.task_id ?? "");
    const headSessionId = idToString(next.session?.id ?? "");
    for (const [taskId, item] of this.tasks.entries()) {
      const primaryId =
        item.primarySessionId ||
        idToString(item.task.primary_session_id ?? "");
      const matchesSession = primaryId ? primaryId === headSessionId : false;
      const matchesTask = headTaskId ? headTaskId === taskId : false;
      if (!matchesSession && !matchesTask) continue;
      if (primaryId && !matchesSession) continue;
      const nextItem: WorkspaceActiveSnapshotItem = {
        ...item,
        primarySessionId: primaryId || headSessionId || null,
        primarySessionHead: next,
      };
      this.tasks.set(taskId, nextItem);
      changed = true;
    }
    return changed;
  }

  private applySessionHeadReset(head: SessionHeadSnapshot | null | undefined): boolean {
    if (!head) return false;
    const sessionId = idToString((head as any)?.session?.id ?? "");
    if (!sessionId) return false;
    const sanitized = sanitizeHeadSnapshot(head);
    const prev = this.sessionHeadsById.get(sessionId);
    if (!this.shouldReplaceHead(prev, sanitized)) return false;
    this.sessionHeadsById.set(sessionId, sanitized);
    let changed = true;
    const headTaskId = idToString(sanitized.session?.task_id ?? "");
    const headSessionId = idToString(sanitized.session?.id ?? "");
    for (const [taskId, item] of this.tasks.entries()) {
      const primaryId =
        item.primarySessionId ||
        idToString(item.task.primary_session_id ?? "");
      const matchesSession = primaryId ? primaryId === headSessionId : false;
      const matchesTask = headTaskId ? headTaskId === taskId : false;
      if (!matchesSession && !matchesTask) continue;
      if (primaryId && !matchesSession) continue;
      const nextItem: WorkspaceActiveSnapshotItem = {
        ...item,
        primarySessionId: primaryId || headSessionId || null,
        primarySessionHead: sanitized,
      };
      this.tasks.set(taskId, nextItem);
      changed = true;
    }
    return changed;
  }

  private readPrimarySessionHead(summary: unknown): SessionHeadSnapshot | null {
    if (!summary || typeof summary !== "object") return null;
    const rec = summary as Record<string, unknown>;
    const head = (rec as any).primary_session_head ?? (rec as any).primarySessionHead ?? null;
    if (!head || typeof head !== "object") return null;
    return sanitizeHeadSnapshot(head as SessionHeadSnapshot);
  }

  private readPrimarySessionId(summary: unknown): string | null {
    if (!summary || typeof summary !== "object") return null;
    const rec = summary as Record<string, any>;
    const fromPrimary = idToString(rec?.primary_session?.session?.id ?? "");
    if (fromPrimary) return fromPrimary;
    const fromHead = idToString(rec?.primary_session_head?.session?.id ?? rec?.primarySessionHead?.session?.id ?? "");
    if (fromHead) return fromHead;
    return null;
  }

  private rememberSessionHead(head: SessionHeadSnapshot | null) {
    if (!head) return;
    const sessionId = idToString((head as any)?.session?.id ?? "");
    if (!sessionId) return;
    const sanitized = sanitizeHeadSnapshot(head);
    const prev = this.sessionHeadsById.get(sessionId);
    if (!this.shouldReplaceHead(prev, sanitized)) return;
    this.sessionHeadsById.set(sessionId, sanitized);
  }

  private collectArchivedHeads(): Map<string, SessionHeadSnapshot> {
    const archived = new Map<string, SessionHeadSnapshot>();
    for (const item of this.tasks.values()) {
      if (!item.task.archived_at) continue;
      const primaryId =
        item.primarySessionId || idToString(item.primarySessionHead?.session?.id ?? "");
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
    const primarySessionId =
      this.readPrimarySessionId(summary) ||
      existing?.primarySessionId ||
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
