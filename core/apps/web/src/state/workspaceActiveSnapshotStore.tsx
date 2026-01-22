import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type {
  Session,
  SessionHeadSnapshot,
  SessionSummary,
  SessionSnapshotSummary,
  Task,
  WorkspaceActiveHeadBatch,
  WorkspaceActiveSnapshot,
  WorkspaceActiveSnapshotEvent,
  WorkspaceActiveSnapshotSessionSubscription,
  WorkspaceActiveSnapshotStreamMessage,
  WorkspaceActiveTaskSummary,
  WorkspaceIndexCursor,
  WorkspaceTaskSummary,
} from "@ctx/types";
import {
  getDaemonBaseUrl,
  getHealth,
  getWorkspaceActiveSnapshot,
  idToString,
  listWorkspaceArchivedTaskSummaries,
  type WorkspaceActiveSnapshotClientMessage,
  type WorkspaceActiveSnapshotParams,
} from "../api/client";
import {
  loadWorkspaceActiveSnapshotV1,
  saveWorkspaceActiveSnapshotV1,
  type PersistedWorkspaceActiveSnapshotV1,
  type PersistedWorkspaceActiveTaskSummaryV1,
} from "./uiStateStore";
import { parseWsJson } from "../utils/wsJson";

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
  snapshotRev: number;
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
  setSubscriptions: (subscriptions: WorkspaceActiveSnapshotSessionSubscription[]) => void;
};

const ACTIVE_PAGE_SIZE = 50;

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

const normalizeSubscriptions = (
  subs: WorkspaceActiveSnapshotSessionSubscription[],
  lastSeqBySession: Map<string, number>,
): WorkspaceActiveSnapshotSessionSubscription[] => {
  const seen = new Set<string>();
  const out: WorkspaceActiveSnapshotSessionSubscription[] = [];
  for (const sub of subs) {
    const id = idToString(sub.session_id);
    if (!id || seen.has(id)) continue;
    seen.add(id);
    let afterSeq =
      typeof sub.after_seq === "number" ? sub.after_seq : lastSeqBySession.get(id);
    if (!Number.isFinite(afterSeq as number) || (afterSeq as number) < 0) {
      afterSeq = 0;
    }
    out.push({ session_id: id, after_seq: afterSeq ?? 0 });
  }
  out.sort((a, b) => String(idToString(a.session_id)).localeCompare(String(idToString(b.session_id))));
  return out;
};

const isWorkspaceActiveSnapshotStreamMessage = (
  payload: unknown,
): payload is WorkspaceActiveSnapshotStreamMessage => {
  if (!payload || typeof payload !== "object") return false;
  const type = (payload as { type?: string }).type;
  return type === "snapshot" || type === "event" || type === "reset_required";
};

const sortSessionSummaries = (summaries: SessionSnapshotSummary[]): SessionSnapshotSummary[] => {
  return summaries
    .slice()
    .sort((a, b) => String(a.session.created_at ?? "").localeCompare(String(b.session.created_at ?? "")));
};

const mergeSessionSummaries = (
  existing: SessionSnapshotSummary[],
  incoming: SessionSnapshotSummary[],
): SessionSnapshotSummary[] => {
  if (existing.length === 0) return incoming;
  if (incoming.length === 0) return existing;
  const byId = new Map<string, SessionSnapshotSummary>();
  for (const summary of existing) {
    const id = idToString(summary.session.id);
    if (!id) continue;
    byId.set(id, summary);
  }
  for (const summary of incoming) {
    const id = idToString(summary.session.id);
    if (!id) continue;
    byId.set(id, summary);
  }
  return sortSessionSummaries(Array.from(byId.values()));
};

class WorkspaceActiveSnapshotStoreImpl implements WorkspaceActiveSnapshotEventSource {
  private listeners = new Set<() => void>();
  private eventListeners = new Set<(event: WorkspaceActiveSnapshotEvent) => void>();
  private snapshot: WorkspaceActiveSnapshotState;
  private tasks = new Map<string, WorkspaceActiveSnapshotItem>();
  private sessionHeadsById = new Map<string, SessionHeadSnapshot>();
  private worktreeRootsById = new Map<string, string>();
  private activeOrder: string[] = [];
  private archivedOrder: string[] = [];
  private totalActive = 0;
  private totalArchived = 0;
  private hasMoreActive = true;
  private hasMoreArchived = false;
  private archivedLoaded = false;
  private activeLimit = ACTIVE_PAGE_SIZE;
  private archivedCursor: WorkspaceIndexCursor | null = null;
  private ws: WebSocket | null = null;
  private reconnectTimer: number | null = null;
  private reconnectDelayMs = 1000;
  private snapshotRev = 0;
  private archivedRev = 0;
  private subscriptions: WorkspaceActiveSnapshotSessionSubscription[] = [];
  private sessionLastEventSeq = new Map<string, number>();
  private subscriptionKey = "";
  private cacheHydrated = false;
  private liveSnapshotApplied = false;
  private cachePersistTimer: number | null = null;
  private destroyed = false;

  constructor(private workspaceId: string) {
    this.snapshot = {
      workspaceId,
      snapshotRev: 0,
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

  setSubscriptions = (subscriptions: WorkspaceActiveSnapshotSessionSubscription[]) => {
    const next = normalizeSubscriptions(subscriptions, this.sessionLastEventSeq);
    const key = next.map((sub) => `${idToString(sub.session_id)}:${sub.after_seq ?? 0}`).join("|");
    if (key === this.subscriptionKey) return;
    this.subscriptionKey = key;
    this.subscriptions = next;
    this.flushSubscriptions();
  };

  init = () => {
    this.destroyed = false;
    void this.hydrateFromCache();
    this.ensureActiveSnapshot(true).catch(() => {});
    this.connectStream().catch(() => {});
  };

  destroy = () => {
    this.destroyed = true;
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        // ignore
      }
      this.ws = null;
    }
    if (this.reconnectTimer) {
      window.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.cachePersistTimer) {
      window.clearTimeout(this.cachePersistTimer);
      this.cachePersistTimer = null;
    }
    this.listeners.clear();
    this.eventListeners.clear();
  };

  loadMoreActive = () => {
    if (!this.hasMoreActive || this.snapshot.fetchState.active === "loading") return;
    this.activeLimit += ACTIVE_PAGE_SIZE;
    this.ensureActiveSnapshot(false).catch(() => {});
  };

  ensureArchivedLoaded = () => {
    if (this.archivedLoaded || this.snapshot.fetchState.archived === "loading") return;
    this.fetchArchivedPage(true).catch(() => {});
  };

  loadMoreArchived = () => {
    if (!this.hasMoreArchived || this.snapshot.fetchState.archived === "loading") return;
    this.fetchArchivedPage(false).catch(() => {});
  };

  applyTaskUpdate(task: Task) {
    const id = idToString(task.id);
    if (!id) return;
    const existing = this.tasks.get(id);
    if (!existing) return;
    const prevArchived = Boolean(existing.task.archived_at);
    const nextArchived = Boolean(task.archived_at);
    const stableSortAt = task.archived_at ?? task.created_at ?? existing.sort_at;
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

  private async hydrateFromCache() {
    if (this.cacheHydrated) return;
    this.cacheHydrated = true;
    try {
      const cached = await loadWorkspaceActiveSnapshotV1(this.workspaceId);
      if (!cached || this.destroyed || this.liveSnapshotApplied) return;
      this.applyCachedActiveSnapshot(cached);
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
      const normalized = this.normalizeActiveSummary(summary);
      nextActiveIds.add(normalized.id);
      this.tasks.set(normalized.id, normalized);
      this.placeInOrders(normalized);
    }

    const totalCount = Number.isFinite(cached.active?.totalCount) ? cached.active.totalCount : nextActiveIds.size;
    this.totalActive = Math.max(totalCount, nextActiveIds.size);
    this.snapshotRev = Math.max(this.snapshotRev, cached.snapshotRev ?? 0);
    this.rebuildSessionLastEventSeq();
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

  private schedulePersistCache() {
    if (this.destroyed || this.cachePersistTimer) return;
    this.cachePersistTimer = window.setTimeout(() => {
      this.cachePersistTimer = null;
      this.persistCache().catch(() => {});
    }, 300);
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
    const sortAt =
      item.sort_at ||
      this.taskSortAt(item.task) ||
      item.task.updated_at ||
      item.task.created_at ||
      "";
    return {
      task: item.task,
      primary_session: primary ?? null,
      primary_session_head: head ?? null,
      sessions,
      sort_at: sortAt,
    };
  }

  private async ensureActiveSnapshot(reset: boolean) {
    if (this.destroyed) return;
    if (reset && this.snapshot.fetchState.active === "loading") return;

    this.setFetchState("active", "loading");
    const params: WorkspaceActiveSnapshotParams = {
      limit: this.activeLimit,
    };
    try {
      const snapshot = await getWorkspaceActiveSnapshot(this.workspaceId, params);
      const nextTotalActive = snapshot.active?.total_count ?? this.totalActive;
      const dropMissing = this.activeLimit >= nextTotalActive;
      this.applyActiveSnapshot(snapshot, { reset, dropMissing });
    } catch {
      this.setFetchState("active", "error");
      return;
    }
    this.setFetchState("active", "idle");
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
      const page = await listWorkspaceArchivedTaskSummaries(this.workspaceId, {
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

  private applyActiveSnapshot(
    snapshot: WorkspaceActiveSnapshot,
    opts?: {
      reset?: boolean;
      dropMissing?: boolean;
      activeHeads?: SessionHeadSnapshot[] | null;
    },
  ) {
    const nextSnapshotRev = typeof snapshot.snapshot_rev === "number" ? snapshot.snapshot_rev : 0;
    this.snapshotRev = Math.max(this.snapshotRev, nextSnapshotRev);
    if (typeof snapshot.archived_rev === "number" && snapshot.archived_rev > this.archivedRev) {
      this.archivedRev = snapshot.archived_rev;
      this.archivedLoaded = false;
      this.archivedCursor = null;
    }

    const active = snapshot.active;
    const nextTotalActive = Number.isFinite(active?.total_count) ? active.total_count : this.totalActive;
    this.totalActive = nextTotalActive;

    const reset = opts?.reset ?? false;
    const previousTasks = reset ? new Map(this.tasks) : null;
    const archivedHeads = reset ? this.collectArchivedHeads() : null;
    if (reset) {
      for (const [id, item] of this.tasks.entries()) {
        if (!item.task.archived_at) {
          this.tasks.delete(id);
        }
      }
      this.activeOrder = [];
      this.sessionHeadsById.clear();
      if (archivedHeads) {
        for (const [sessionId, head] of archivedHeads) {
          this.sessionHeadsById.set(sessionId, head);
        }
      }
    }

    this.applyActiveHeads(opts?.activeHeads);

    const activeTasks = Array.isArray(active?.tasks) ? active.tasks : [];
    const nextActiveIds = new Set<string>();
    for (const summary of activeTasks) {
      const id = idToString(summary.task.id);
      const existing = (previousTasks ?? this.tasks).get(id);
      const normalized = this.normalizeActiveSummary(summary, existing);
      nextActiveIds.add(normalized.id);
      this.tasks.set(normalized.id, normalized);
      this.placeInOrders(normalized);
    }

    const dropMissing = opts?.dropMissing ?? this.activeLimit >= nextTotalActive;
    if (dropMissing) {
      const prevActive = [...this.activeOrder];
      for (const id of prevActive) {
        if (nextActiveIds.has(id)) continue;
        const existing = this.tasks.get(id);
        if (!existing) continue;
        if (!existing.task.archived_at) {
          this.removeTask(id, { adjustCounts: false });
        }
      }
    }

    this.rebuildSessionLastEventSeq();
    this.snapshot.initialized = true;
    this.liveSnapshotApplied = true;
    this.publish();
    this.schedulePersistCache();
  }

  private applyActiveHeads(heads?: SessionHeadSnapshot[] | null) {
    if (!Array.isArray(heads)) return;
    for (const head of heads) {
      this.rememberSessionHead(head);
    }
  }

  private readActiveHeads(batch?: WorkspaceActiveHeadBatch | null): SessionHeadSnapshot[] {
    if (!batch || !Array.isArray(batch.heads)) return [];
    return batch.heads;
  }

  private async connectStream() {
    if (this.destroyed || this.ws) return;
    this.snapshot.connection = "connecting";
    this.publish();
    const urls = await this.resolveWsUrls();
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
  }

  private async resolveWsUrls(): Promise<string[]> {
    const token = authToken();
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";
    const urls: string[] = [];
    const sameOrigin = `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/api/workspaces/${this.workspaceId}/active_snapshot/stream${qs}`;
    urls.push(sameOrigin);

    const configured = getDaemonBaseUrl();
    if (configured) {
      const wsBase = configured.startsWith("https://")
        ? configured.replace(/^https:\/\//, "wss://")
        : configured.replace(/^http:\/\//, "ws://");
      urls.push(`${wsBase}/api/workspaces/${this.workspaceId}/active_snapshot/stream${qs}`);
    } else {
      try {
        const health = await getHealth();
        const base = String(health.daemon_url || "").trim();
        if (base) {
          const wsBase = base.startsWith("https://")
            ? base.replace(/^https:\/\//, "wss://")
            : base.replace(/^http:\/\//, "ws://");
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
      let opened = false;
      const timeoutId = window.setTimeout(() => {
        if (opened) return;
        try {
          ws.close();
        } catch {
          // ignore
        }
        reject(new Error("workspace active snapshot ws timeout"));
      }, 4000);

      ws.onopen = () => {
        opened = true;
        window.clearTimeout(timeoutId);
        this.ws = ws;
        this.reconnectDelayMs = 1000;
        this.snapshot.connection = "connected";
        this.publish();
        this.flushSubscriptions();
        resolve();
      };

      ws.onmessage = (event) => {
        this.handleStreamMessage(event.data);
      };

      ws.onerror = () => {
        window.clearTimeout(timeoutId);
        if (!opened) {
          reject(new Error("workspace active snapshot ws error"));
        }
      };

      ws.onclose = () => {
        this.ws = null;
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
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      this.connectStream().catch(() => {});
    }, delay);
  }

  private async handleStreamMessage(data: unknown) {
    const parsed = await parseWsJson(data);
    if (!parsed || typeof parsed !== "object") return;
    if (isWorkspaceActiveSnapshotStreamMessage(parsed)) {
      this.handleStreamEnvelope(parsed);
      return;
    }
    this.applyWorkspaceEvent(parsed as WorkspaceActiveSnapshotEvent);
  }

  private handleStreamEnvelope(message: WorkspaceActiveSnapshotStreamMessage) {
    switch (message.type) {
      case "snapshot": {
        const snapshot = message.active_snapshot;
        if (!snapshot || typeof snapshot !== "object") return;
        if (typeof snapshot.snapshot_rev === "number" && snapshot.snapshot_rev < this.snapshotRev) {
          this.ensureActiveSnapshot(true).catch(() => {});
          return;
        }
        const activeHeads = this.readActiveHeads(message.active_heads);
        const totalCount = snapshot.active?.total_count ?? 0;
        const taskCount = Array.isArray(snapshot.active?.tasks) ? snapshot.active.tasks.length : 0;
        const snapshotComplete = taskCount >= totalCount;
        const reset = !this.snapshot.initialized || snapshotComplete;
        this.applyActiveSnapshot(snapshot, { reset, dropMissing: snapshotComplete, activeHeads });
        break;
      }
      case "event":
        if (message.event) {
          this.applyWorkspaceEvent(message.event);
        }
        break;
      case "reset_required":
        this.ensureActiveSnapshot(true).catch(() => {});
        break;
      default:
        break;
    }
  }

  private applyWorkspaceEvent(evt: WorkspaceActiveSnapshotEvent) {
    if (typeof evt.snapshot_rev === "number") {
      if (evt.snapshot_rev < this.snapshotRev) {
        this.snapshotRev = evt.snapshot_rev;
        this.ensureActiveSnapshot(true).catch(() => {});
      } else if (evt.snapshot_rev > this.snapshotRev + 1) {
        this.ensureActiveSnapshot(true).catch(() => {});
      } else if (evt.type === "ready" && evt.snapshot_rev !== this.snapshotRev) {
        this.ensureActiveSnapshot(true).catch(() => {});
      }
      this.snapshotRev = evt.snapshot_rev;
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
        this.flushSubscriptions();
        break;
      case "active_task_upsert":
        this.upsertActiveSummary(evt.task);
        this.publish();
        break;
      case "active_task_delete":
        this.removeTask(idToString(evt.task_id), { adjustCounts: true });
        this.publish();
        break;
      case "archived_task_upsert": {
        const head = evt.snapshot?.head ?? null;
        const item = this.buildArchivedItem(evt.task, head);
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
      case "session_gap":
        break;
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

  private notifyEventListeners(evt: WorkspaceActiveSnapshotEvent) {
    for (const listener of this.eventListeners) {
      listener(evt);
    }
  }

  private flushSubscriptions() {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const message: WorkspaceActiveSnapshotClientMessage = {
      type: "subscribe",
      sessions: this.subscriptions,
    };
    try {
      ws.send(JSON.stringify(message));
    } catch {
      // ignore send errors
    }
  }

  private rebuildSessionLastEventSeq() {
    this.sessionLastEventSeq.clear();
    for (const task of this.tasks.values()) {
      for (const summary of task.sessions) {
        const id = idToString(summary.session.id);
        if (!id) continue;
        if (typeof summary.last_event_seq === "number") {
          this.sessionLastEventSeq.set(id, summary.last_event_seq);
        }
      }
    }
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
    this.rebuildSessionLastEventSeq();
    this.schedulePersistCache();
  }

  private upsertActiveSummary(summary: WorkspaceActiveTaskSummary) {
    const id = idToString(summary.task.id);
    const prior = id ? this.tasks.get(id) : undefined;
    const normalized = this.normalizeActiveSummary(summary, prior);
    const existing = this.tasks.get(normalized.id);
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
    this.rebuildSessionLastEventSeq();
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
    this.rebuildSessionLastEventSeq();
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
    this.rebuildSessionLastEventSeq();
    this.schedulePersistCache();
  }

  private readPrimarySessionHead(summary: unknown): SessionHeadSnapshot | null {
    if (!summary || typeof summary !== "object") return null;
    const rec = summary as Record<string, unknown>;
    const head = (rec as any).primary_session_head ?? (rec as any).primarySessionHead ?? null;
    if (!head || typeof head !== "object") return null;
    return head as SessionHeadSnapshot;
  }

  private readPrimarySessionId(summary: unknown): string | null {
    if (!summary || typeof summary !== "object") return null;
    const rec = summary as Record<string, any>;
    const fromTask = idToString(rec?.task?.primary_session_id ?? "");
    if (fromTask) return fromTask;
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
    this.sessionHeadsById.set(sessionId, head);
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
    existing?: WorkspaceActiveSnapshotItem | null,
  ): WorkspaceActiveSnapshotItem {
    const id = idToString(summary.task.id);
    const sortAt = summary.sort_at ?? this.taskSortAt(summary.task) ?? "";
    const sortAtMs = Date.parse(sortAt) || Date.now();
    let primarySessionId = this.readPrimarySessionId(summary) || idToString(summary.primary_session?.session?.id ?? "");
    let primaryHead = this.readPrimarySessionHead(summary);
    if (!primaryHead && primarySessionId) {
      primaryHead = this.sessionHeadsById.get(primarySessionId) ?? null;
    }
    if (!primaryHead && existing?.primarySessionHead) {
      primaryHead = existing.primarySessionHead;
    }
    this.rememberSessionHead(primaryHead);
    const primary = summary.primary_session ? [this.normalizeSessionSummary(summary.primary_session)] : [];
    const sessionsRaw = Array.isArray(summary.sessions) ? summary.sessions : [];
    const sessions = sessionsRaw.map((s) => this.normalizeSessionSummary(s));
    const merged: SessionSnapshotSummary[] = [];
    const seen = new Set<string>();
    const addSummary = (item: SessionSnapshotSummary) => {
      const sid = idToString(item.session.id);
      if (!sid || seen.has(sid)) return;
      seen.add(sid);
      merged.push(item);
    };
    primary.forEach(addSummary);
    sessions.forEach(addSummary);
    const existingSessions = existing?.sessions ?? [];
    const combined = mergeSessionSummaries(existingSessions, merged);
    if (!primarySessionId && existing?.primarySessionId) {
      primarySessionId = existing.primarySessionId;
    }

    return {
      id,
      task: { ...summary.task },
      sessions: sortSessionSummaries(combined),
      primarySessionHead: primaryHead ?? null,
      primarySessionId: primarySessionId || null,
      sortAtMs,
      sort_at: sortAt || null,
    };
  }

  private taskSortAt(task: Task): string {
    return task.archived_at ?? task.created_at ?? task.updated_at ?? "";
  }

  private buildArchivedItem(
    summary: WorkspaceTaskSummary,
    primaryHead?: SessionHeadSnapshot | null,
  ): WorkspaceActiveSnapshotItem | null {
    const task = summary.task;
    const id = idToString(task.id);
    if (!id) return null;
    const existing = this.tasks.get(id);
    const providerIds = (summary.provider_ids ?? []).filter(Boolean);
    const summaries = existing?.sessions ?? [];
    const sessionList = summaries.map((item) => item.session).filter(Boolean);
    const summarySessions = Array.isArray(summary.sessions) ? summary.sessions : [];
    const summaryHead = primaryHead ?? this.readPrimarySessionHead(summary);
    if (summaryHead) {
      this.rememberSessionHead(summaryHead);
    } else if (existing?.primarySessionHead) {
      this.rememberSessionHead(existing.primarySessionHead);
    }
    const summaryPrimaryId = this.readPrimarySessionId(summary);
    const primarySessionId =
      summaryPrimaryId ||
      this.pickArchivedSessionIdFromSummaries(task, summarySessions) ||
      this.pickArchivedSessionId(task, sessionList);
    let primarySessionHead = summaryHead ?? null;
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
    return primaryId || null;
  }

  private pickArchivedSessionIdFromSummaries(task: Task, sessions: SessionSummary[]): string | null {
    const primaryId = idToString(task.primary_session_id ?? "");
    return primaryId || null;
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
      snapshotRev: this.snapshotRev,
    };
    for (const l of this.listeners) l();
  }
}

const WorkspaceActiveSnapshotContext = createContext<WorkspaceActiveSnapshotStoreImpl | null>(null);

export function WorkspaceActiveSnapshotProvider({
  workspaceId,
  children,
}: {
  workspaceId: string;
  children: React.ReactNode;
}) {
  const storeRef = useRef<WorkspaceActiveSnapshotStoreImpl | null>(null);
  const lastWorkspaceRef = useRef<string | null>(null);
  if (!storeRef.current || lastWorkspaceRef.current !== workspaceId) {
    storeRef.current?.destroy();
    storeRef.current = new WorkspaceActiveSnapshotStoreImpl(workspaceId);
    lastWorkspaceRef.current = workspaceId;
  }

  useEffect(() => {
    storeRef.current?.init();
    return () => storeRef.current?.destroy();
  }, [workspaceId]);

  return (
    <WorkspaceActiveSnapshotContext.Provider value={storeRef.current}>
      {children}
    </WorkspaceActiveSnapshotContext.Provider>
  );
}

export function useWorkspaceActiveSnapshotStore() {
  const store = useContext(WorkspaceActiveSnapshotContext);
  if (!store) throw new Error("WorkspaceActiveSnapshotProvider missing");
  return store;
}

export function useWorkspaceActiveSnapshotSnapshot(): WorkspaceActiveSnapshotState {
  const store = useWorkspaceActiveSnapshotStore();
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
}

export function useWorkspaceActiveSnapshotEvents(handler: (event: WorkspaceActiveSnapshotEvent) => void) {
  const store = useWorkspaceActiveSnapshotStore();
  const stableHandler = useMemo(() => handler, [handler]);
  useEffect(() => store.subscribeEvents(stableHandler), [store, stableHandler]);
}
