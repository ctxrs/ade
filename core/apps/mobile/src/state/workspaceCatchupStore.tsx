import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type {
  SessionCatchupSummary,
  Task,
  WorkspaceCatchupCursor,
  WorkspaceCatchupEvent,
  WorkspaceCatchupSnapshot,
  WorkspaceCatchupTaskSummary,
  WorkspaceCatchupTrackSummary,
} from "@context/types";

import {
  getWorkspaceCatchup,
  idToString,
  type ConnectionConfig,
  type WorkspaceCatchupParams,
} from "../api/client";
import { parseWsJson } from "../utils/wsJson";
import { loadWorkspaceCatchupV1, saveWorkspaceCatchupV1 } from "./uiStateStore";
import { useConnection } from "./ConnectionProvider";

export type WorkspaceCatchupItem = WorkspaceCatchupTaskSummary & {
  id: string;
  sortAtMs: number;
};

export type WorkspaceCatchupState = {
  workspaceId: string | null;
  initialized: boolean;
  connection: "idle" | "connecting" | "connected" | "disconnected";
  tasksById: Record<string, WorkspaceCatchupItem>;
  activeIds: string[];
  archivedIds: string[];
  totalActive: number;
  totalArchived: number;
  fetchState: {
    active: "idle" | "loading" | "error";
    archived: "idle" | "loading" | "error";
  };
  hasMoreActive: boolean;
  hasMoreArchived: boolean;
  archivedLoaded: boolean;
};

export type WorkspaceCatchupEventSource = {
  subscribe: (listener: () => void) => () => void;
  subscribeEvents: (listener: (event: WorkspaceCatchupEvent) => void) => () => void;
  getSnapshot: () => WorkspaceCatchupState;
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

const ensureSlash = (input: string): string => {
  if (!input.trim()) return "";
  return input.endsWith("/") ? input : `${input}/`;
};

const toWsUrl = (conn: ConnectionConfig, path: string): string => {
  const normalized = ensureSlash(conn.baseUrl);
  const absolute = path.startsWith("/") ? path.slice(1) : path;
  const base = `${normalized}${absolute}`;
  return base.startsWith("https://")
    ? base.replace(/^https:\/\//, "wss://")
    : base.replace(/^http:\/\//, "ws://");
};

export class WorkspaceCatchupStoreImpl implements WorkspaceCatchupEventSource {
  private listeners = new Set<() => void>();
  private eventListeners = new Set<(event: WorkspaceCatchupEvent) => void>();
  private snapshot: WorkspaceCatchupState;
  private tasks = new Map<string, WorkspaceCatchupItem>();
  private activeOrder: string[] = [];
  private archivedOrder: string[] = [];
  private activeCursor: WorkspaceCatchupCursor | null = null;
  private archivedCursor: WorkspaceCatchupCursor | null = null;
  private totalActive = 0;
  private totalArchived = 0;
  private hasMoreActive = true;
  private hasMoreArchived = false;
  private archivedLoaded = false;
  private ws: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelayMs = 1000;
  private snapshotRev = 0;
  private destroyed = false;
  private persistTimer: ReturnType<typeof setTimeout> | null = null;
  private streamEnabled: boolean;

  constructor(
    private workspaceId: string,
    private conn: ConnectionConfig,
    opts?: { streamEnabled?: boolean },
  ) {
    this.streamEnabled = opts?.streamEnabled ?? true;
    this.snapshot = {
      workspaceId,
      initialized: false,
      connection: "idle",
      tasksById: {},
      activeIds: [],
      archivedIds: [],
      totalActive: 0,
      totalArchived: 0,
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

  subscribeEvents = (listener: (event: WorkspaceCatchupEvent) => void): (() => void) => {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  };

  getSnapshot = (): WorkspaceCatchupState => this.snapshot;

  init = () => {
    this.loadCached().catch(() => {});
    this.ensureActivePage(true).catch(() => {});
    if (this.streamEnabled) {
      this.connectStream().catch(() => {});
    }
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
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.persistTimer) {
      clearTimeout(this.persistTimer);
      this.persistTimer = null;
    }
    this.listeners.clear();
    this.eventListeners.clear();
  };

  refreshActive = () => {
    this.ensureActivePage(true).catch(() => {});
  };

  refreshArchived = () => {
    this.fetchArchivedPage(true).catch(() => {});
  };

  loadMoreActive = () => {
    if (!this.hasMoreActive || this.snapshot.fetchState.active === "loading") return;
    this.ensureActivePage(false).catch(() => {});
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
    const updated: WorkspaceCatchupItem = {
      ...existing,
      task: { ...task },
      sortAtMs:
        Date.parse(task.archived_at ?? task.updated_at ?? "") ||
        existing.sortAtMs ||
        Date.now(),
    };
    this.tasks.set(id, updated);
    this.updateCountsForMove(existing, updated);
    this.placeInOrders(updated);
    this.publish();
  }

  applyTaskDelete(taskId: string) {
    this.deleteTask(taskId);
    this.publish();
  }

  private async loadCached() {
    if (this.destroyed) return;
    try {
      const cached = await loadWorkspaceCatchupV1(this.workspaceId);
      if (!cached?.snapshot) return;
      this.applySnapshot(cached.snapshot, { isCache: true });
    } catch {
      // ignore cache errors
    }
  }

  private applySnapshot(snapshot: WorkspaceCatchupSnapshot, opts?: { isCache?: boolean }) {
    if (this.destroyed) return;
    const isCache = opts?.isCache ?? false;
    const active = snapshot.active;
    const archived = snapshot.archived ?? null;

    this.snapshotRev = Math.max(this.snapshotRev, snapshot.snapshot_rev ?? 0);
    this.totalActive = active.total_count ?? this.totalActive;
    this.totalArchived = archived?.total_count ?? this.totalArchived;

    if (!isCache) {
      this.tasks.clear();
      this.activeOrder = [];
      this.archivedOrder = [];
    }

    active.tasks.forEach((summary) => this.upsertSummary(summary));
    this.activeCursor = active.next_cursor ?? null;
    this.hasMoreActive = Boolean(active.next_cursor);

    if (archived) {
      archived.tasks.forEach((summary) => this.upsertSummary(summary));
      this.archivedCursor = archived.next_cursor ?? null;
      this.hasMoreArchived = Boolean(archived.next_cursor);
      this.archivedLoaded = true;
    }

    this.snapshot.initialized = true;
    this.publish();
  }

  private async ensureActivePage(reset: boolean) {
    if (this.destroyed) return;
    if (!reset && !this.activeCursor && this.snapshot.initialized) {
      this.hasMoreActive = false;
      this.publish();
      return;
    }

    this.setFetchState("active", "loading");
    const params: WorkspaceCatchupParams = {
      limit: 50,
      activeCursor: reset ? null : this.activeCursor,
      includeArchived: false,
    };
    try {
      const page = await getWorkspaceCatchup(this.conn, this.workspaceId, params);
      this.snapshotRev = Math.max(this.snapshotRev, page.snapshot_rev ?? 0);
      this.totalActive = page.active.total_count ?? this.totalActive;
      this.totalArchived = page.archived?.total_count ?? this.totalArchived;
      if (reset) {
        this.tasks.clear();
        this.activeOrder = [];
        this.archivedOrder = [];
        this.archivedCursor = null;
        this.archivedLoaded = false;
        this.hasMoreArchived = false;
      }
      page.active.tasks.forEach((summary) => this.upsertSummary(summary));
      this.activeCursor = page.active.next_cursor ?? null;
      this.hasMoreActive = Boolean(page.active.next_cursor);
      this.snapshot.initialized = true;
      this.publish();
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
    } else if (!this.archivedCursor) {
      this.hasMoreArchived = false;
      this.publish();
      return;
    }
    this.setFetchState("archived", "loading");
    try {
      const page = await getWorkspaceCatchup(this.conn, this.workspaceId, {
        limit: 50,
        includeArchived: true,
        archivedOnly: true,
        archivedCursor: firstLoad ? null : this.archivedCursor,
      });
      this.snapshotRev = Math.max(this.snapshotRev, page.snapshot_rev ?? 0);
      this.totalActive = page.active.total_count ?? this.totalActive;
      this.totalArchived = page.archived?.total_count ?? this.totalArchived;
      if (page.archived) {
        page.archived.tasks.forEach((summary) => this.upsertSummary(summary));
        this.archivedCursor = page.archived.next_cursor ?? null;
        this.hasMoreArchived = Boolean(page.archived.next_cursor);
        this.archivedLoaded = true;
        this.publish();
      }
    } catch {
      this.setFetchState("archived", "error");
      return;
    }
    this.setFetchState("archived", "idle");
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
    const qs = this.conn.token ? `?token=${encodeURIComponent(this.conn.token)}` : "";
    const url = `${toWsUrl(this.conn, `/api/workspaces/${this.workspaceId}/stream`)}${qs}`;
    return dedupeUrls([url]);
  }

  private openWebSocket(url: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      let opened = false;
      const timeoutId = setTimeout(() => {
        if (opened) return;
        try {
          ws.close();
        } catch {
          // ignore
        }
        reject(new Error("workspace catchup ws timeout"));
      }, 4000);

      ws.onopen = () => {
        opened = true;
        clearTimeout(timeoutId);
        this.ws = ws;
        this.snapshot.connection = "connected";
        this.publish();
        resolve();
      };

      ws.onmessage = (event) => {
        this.handleStreamMessage(event.data);
      };

      ws.onerror = () => {
        clearTimeout(timeoutId);
        if (!opened) {
          reject(new Error("workspace catchup ws error"));
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
    if (this.reconnectTimer || this.destroyed || !this.streamEnabled) return;
    const delay = this.reconnectDelayMs;
    this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, 15000);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connectStream().catch(() => {});
    }, delay);
  }

  private async handleStreamMessage(data: unknown) {
    const parsed = await parseWsJson(data);
    if (!parsed || typeof parsed !== "object") return;
    const evt = parsed as WorkspaceCatchupEvent;
    if (evt.snapshot_rev && evt.snapshot_rev > this.snapshotRev + 1) {
      this.ensureActivePage(true).catch(() => {});
    }
    if (evt.snapshot_rev) {
      this.snapshotRev = Math.max(this.snapshotRev, evt.snapshot_rev);
    }
    switch (evt.type) {
      case "ready":
        this.snapshot.connection = "connected";
        this.publish();
        break;
      case "task_upsert":
        this.upsertSummary(evt.task);
        this.publish();
        break;
      case "task_delete":
        this.deleteTask(evt.task_id);
        this.publish();
        break;
      case "track_upsert":
        this.applyTrackSummary(evt.track);
        this.publish();
        break;
      case "session_summary":
        this.applySessionSummary(evt.summary);
        this.publish();
        break;
      case "session_head_delta":
        this.notifyEventListeners(evt);
        break;
      default:
        break;
    }

    if (evt.type !== "session_head_delta") {
      this.notifyEventListeners(evt);
    }
  }

  private notifyEventListeners(evt: WorkspaceCatchupEvent) {
    for (const listener of this.eventListeners) {
      listener(evt);
    }
  }

  private deleteTask(taskId: { 0: string } | string) {
    const deleteId = idToString(taskId);
    if (!deleteId) return;
    const existing = this.tasks.get(deleteId);
    if (!existing) return;
    this.tasks.delete(deleteId);
    this.activeOrder = this.activeOrder.filter((id) => id !== deleteId);
    this.archivedOrder = this.archivedOrder.filter((id) => id !== deleteId);
    if (existing.task.archived_at) {
      this.totalArchived = Math.max(0, this.totalArchived - 1);
    } else {
      this.totalActive = Math.max(0, this.totalActive - 1);
    }
  }

  private upsertSummary(summary: WorkspaceCatchupTaskSummary) {
    const normalized = this.normalizeSummary(summary);
    const existing = this.tasks.get(normalized.id);
    this.tasks.set(normalized.id, normalized);
    if (existing) {
      this.updateCountsForMove(existing, normalized);
    } else {
      if (normalized.task.archived_at) {
        this.totalArchived += 1;
      } else {
        this.totalActive += 1;
      }
    }
    this.placeInOrders(normalized);
  }

  private updateCountsForMove(prev: WorkspaceCatchupItem, next: WorkspaceCatchupItem) {
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

  private applyTrackSummary(summary: WorkspaceCatchupTrackSummary) {
    const taskId = idToString(summary.track.task_id);
    if (!taskId) return;
    const task = this.tasks.get(taskId);
    if (!task) return;
    const trackId = idToString(summary.track.id);
    if (!trackId) return;
    const nextTracks = task.tracks.slice();
    const idx = nextTracks.findIndex((t) => idToString(t.track.id) === trackId);
    const normalized = this.normalizeTrackSummary(summary);
    if (idx >= 0) {
      nextTracks[idx] = normalized;
    } else {
      nextTracks.push(normalized);
    }
    nextTracks.sort((a, b) => String(a.track.created_at ?? "").localeCompare(String(b.track.created_at ?? "")));
    this.tasks.set(taskId, { ...task, tracks: nextTracks });
  }

  private applySessionSummary(summary: SessionCatchupSummary) {
    const taskId = idToString(summary.session.task_id);
    if (!taskId) return;
    const task = this.tasks.get(taskId);
    if (!task) return;
    const trackId = idToString(summary.session.track_id);
    if (!trackId) return;
    const nextTracks = task.tracks.slice();
    const trackIdx = nextTracks.findIndex((t) => idToString(t.track.id) === trackId);
    if (trackIdx < 0) return;
    const track = nextTracks[trackIdx];
    const nextSessions = track.sessions.slice();
    const sessionId = idToString(summary.session.id);
    const sessionIdx = nextSessions.findIndex((s) => idToString(s.session.id) === sessionId);
    const normalized = this.normalizeSessionSummary(summary);
    if (sessionIdx >= 0) {
      nextSessions[sessionIdx] = normalized;
    } else {
      nextSessions.push(normalized);
    }
    nextSessions.sort((a, b) => String(a.session.created_at ?? "").localeCompare(String(b.session.created_at ?? "")));
    nextTracks[trackIdx] = { ...track, sessions: nextSessions };
    this.tasks.set(taskId, { ...task, tracks: nextTracks });
  }

  private normalizeSummary(summary: WorkspaceCatchupTaskSummary): WorkspaceCatchupItem {
    const id = idToString(summary.task.id);
    const sortAtMs = Date.parse(summary.sort_at) || Date.now();
    return {
      ...summary,
      id,
      task: { ...summary.task },
      tracks: summary.tracks.map((t) => this.normalizeTrackSummary(t)),
      sortAtMs,
    };
  }

  private normalizeTrackSummary(summary: WorkspaceCatchupTrackSummary): WorkspaceCatchupTrackSummary {
    return {
      track: { ...summary.track },
      primary_session_id: summary.primary_session_id,
      diff_summary: summary.diff_summary ? { ...summary.diff_summary } : summary.diff_summary,
      sessions: summary.sessions.map((s) => this.normalizeSessionSummary(s)),
    };
  }

  private normalizeSessionSummary(summary: SessionCatchupSummary): SessionCatchupSummary {
    return {
      session: { ...summary.session },
      last_message_at: summary.last_message_at ?? null,
      last_message_preview: summary.last_message_preview ?? null,
      last_event_seq: summary.last_event_seq ?? null,
      activity: summary.activity ?? { is_working: false, last_turn_status: null },
      unread: summary.unread,
    };
  }

  private placeInOrders(item: WorkspaceCatchupItem) {
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
    const tasksById: Record<string, WorkspaceCatchupItem> = {};
    for (const [id, item] of this.tasks.entries()) {
      tasksById[id] = item;
    }
    this.snapshot = {
      ...this.snapshot,
      tasksById,
      activeIds: [...this.activeOrder],
      archivedIds: [...this.archivedOrder],
      totalActive: this.totalActive,
      totalArchived: this.totalArchived,
      hasMoreActive: this.hasMoreActive,
      hasMoreArchived: this.hasMoreArchived,
      archivedLoaded: this.archivedLoaded,
    };
    for (const l of this.listeners) l();
    this.schedulePersist();
  }

  private schedulePersist() {
    if (this.persistTimer || this.destroyed) return;
    this.persistTimer = setTimeout(() => {
      this.persistTimer = null;
      this.persistSnapshot().catch(() => {});
    }, 500);
  }

  private async persistSnapshot() {
    if (this.destroyed || !this.snapshot.initialized) return;
    const activeTasks = this.activeOrder
      .map((id) => this.tasks.get(id))
      .filter((t): t is WorkspaceCatchupItem => Boolean(t))
      .map((t) => this.stripItemForPersist(t));
    const archivedTasks = this.archivedOrder
      .map((id) => this.tasks.get(id))
      .filter((t): t is WorkspaceCatchupItem => Boolean(t))
      .map((t) => this.stripItemForPersist(t));

    const snapshot: WorkspaceCatchupSnapshot = {
      workspace_id: this.workspaceId,
      snapshot_rev: this.snapshotRev,
      active: {
        tasks: activeTasks,
        next_cursor: this.activeCursor ?? null,
        total_count: this.totalActive,
      },
      archived: this.archivedLoaded
        ? {
          tasks: archivedTasks,
          next_cursor: this.archivedCursor ?? null,
          total_count: this.totalArchived,
        }
        : null,
    };
    await saveWorkspaceCatchupV1(this.workspaceId, snapshot);
  }

  private stripItemForPersist(item: WorkspaceCatchupItem): WorkspaceCatchupTaskSummary {
    return {
      task: { ...item.task },
      tracks: item.tracks.map((t) => ({
        track: { ...t.track },
        primary_session_id: t.primary_session_id,
        diff_summary: t.diff_summary ? { ...t.diff_summary } : t.diff_summary,
        sessions: t.sessions.map((s) => ({
          session: { ...s.session },
          last_message_at: s.last_message_at ?? null,
          last_message_preview: s.last_message_preview ?? null,
          last_event_seq: s.last_event_seq ?? null,
          unread: s.unread,
        })),
      })),
      sort_at: item.sort_at,
    };
  }
}

const WorkspaceCatchupContext = createContext<WorkspaceCatchupStoreImpl | null>(null);

export function WorkspaceCatchupProvider({
  workspaceId,
  children,
}: {
  workspaceId: string | null;
  children: React.ReactNode;
}) {
  const { config } = useConnection();
  const storeRef = useRef<WorkspaceCatchupStoreImpl | null>(null);
  const lastKeyRef = useRef<string | null>(null);
  const key = config && workspaceId ? `${config.baseUrl}::${workspaceId}` : null;

  if (!config || !workspaceId) {
    storeRef.current?.destroy();
    storeRef.current = null;
    lastKeyRef.current = null;
  } else if (!storeRef.current || lastKeyRef.current !== key) {
    storeRef.current?.destroy();
    storeRef.current = new WorkspaceCatchupStoreImpl(workspaceId, config);
    lastKeyRef.current = key;
  }

  useEffect(() => {
    const store = storeRef.current;
    store?.init();
    return () => store?.destroy();
  }, [key]);

  return (
    <WorkspaceCatchupContext.Provider value={storeRef.current}>
      {children}
    </WorkspaceCatchupContext.Provider>
  );
}

export function useWorkspaceCatchupStore() {
  const store = useContext(WorkspaceCatchupContext);
  if (!store) throw new Error("WorkspaceCatchupProvider missing");
  return store;
}

export function useMaybeWorkspaceCatchupStore(): WorkspaceCatchupStoreImpl | null {
  return useContext(WorkspaceCatchupContext);
}

export function useWorkspaceCatchupSnapshot(): WorkspaceCatchupState {
  const store = useWorkspaceCatchupStore();
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
}

export function useMaybeWorkspaceCatchupSnapshot(): WorkspaceCatchupState | null {
  const store = useMaybeWorkspaceCatchupStore();
  const subscribe = useMemo(() => store?.subscribe ?? (() => () => {}), [store]);
  const getSnapshot = useMemo(
    () => store?.getSnapshot ?? (() => null),
    [store],
  );
  const snap = useSyncExternalStore<WorkspaceCatchupState | null>(
    subscribe,
    getSnapshot as () => WorkspaceCatchupState | null,
    getSnapshot as () => WorkspaceCatchupState | null,
  );
  return store ? (snap as WorkspaceCatchupState) : null;
}

export function useWorkspaceCatchupEvents(handler: (event: WorkspaceCatchupEvent) => void) {
  const store = useWorkspaceCatchupStore();
  const stableHandler = useMemo(() => handler, [handler]);
  useEffect(() => store.subscribeEvents(stableHandler), [store, stableHandler]);
}
