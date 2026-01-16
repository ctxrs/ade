import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type {
  Session,
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  Task,
  WorkspaceActiveSnapshotEvent,
  WorkspaceActiveSnapshotSessionSubscription,
  WorkspaceActiveTaskSummary,
} from "@ctx/types";
import {
  getDaemonBaseUrl,
  getHealth,
  getWorkspaceActiveSnapshot,
  idToString,
  listWorkspaceTasks,
  type WorkspaceActiveSnapshotClientMessage,
  type WorkspaceActiveSnapshotParams,
} from "../api/client";
import { parseWsJson } from "../utils/wsJson";

export type WorkspaceActiveSnapshotItem = {
  id: string;
  task: Task;
  sessions: SessionSnapshotSummary[];
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

const sortSessionSummaries = (summaries: SessionSnapshotSummary[]): SessionSnapshotSummary[] => {
  return summaries
    .slice()
    .sort((a, b) => String(a.session.created_at ?? "").localeCompare(String(b.session.created_at ?? "")));
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
  private archivedCursor: { sort_at: string; task_id: { 0: string } | string } | null = null;
  private archivedIndex: Task[] = [];
  private archivedIndexLoaded = false;
  private ws: WebSocket | null = null;
  private reconnectTimer: number | null = null;
  private reconnectDelayMs = 1000;
  private snapshotRev = 0;
  private subscriptions: WorkspaceActiveSnapshotSessionSubscription[] = [];
  private sessionLastEventSeq = new Map<string, number>();
  private subscriptionKey = "";
  private destroyed = false;

  constructor(private workspaceId: string) {
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
      this.archivedIndexLoaded = false;
    }
    this.updateCountsForMove(existing, updated);
    this.placeInOrders(updated);
    this.publish();
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
      this.snapshotRev = Math.max(this.snapshotRev, snapshot.snapshot_rev ?? 0);
      this.totalActive = snapshot.active.total_count ?? this.totalActive;
      if (reset) {
        for (const [id, item] of this.tasks.entries()) {
          if (!item.task.archived_at) {
            this.tasks.delete(id);
          }
        }
        this.activeOrder = [];
        this.sessionHeadsById.clear();
      }

      const activeTasks = snapshot.active.tasks ?? [];
      const nextActiveIds = new Set<string>();
      for (const summary of activeTasks) {
        const normalized = this.normalizeActiveSummary(summary);
        nextActiveIds.add(normalized.id);
        this.tasks.set(normalized.id, normalized);
        this.placeInOrders(normalized);
      }

      if (this.activeLimit >= this.totalActive) {
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
      this.archivedIndexLoaded = false;
    }
    if (!firstLoad && !this.archivedCursor) {
      this.hasMoreArchived = false;
      this.publish();
      return;
    }
    this.setFetchState("archived", "loading");
    try {
      if (!this.archivedIndexLoaded) {
        const tasks = await listWorkspaceTasks(this.workspaceId);
        this.archivedIndex = tasks
          .filter((task) => Boolean(task.archived_at))
          .sort((a, b) => {
            const aSort = this.taskSortMs(a);
            const bSort = this.taskSortMs(b);
            if (aSort !== bSort) return bSort - aSort;
            return String(idToString(b.id)).localeCompare(String(idToString(a.id)));
          });
        this.archivedIndexLoaded = true;
        this.totalArchived = this.archivedIndex.length;
      }

      const startIndex = this.archivedCursor ? this.findArchivedStartIndex(this.archivedCursor) : 0;
      const pageTasks = this.archivedIndex.slice(startIndex, startIndex + ACTIVE_PAGE_SIZE);
      const summaries = await Promise.all(pageTasks.map((task) => this.buildArchivedItem(task)));
      summaries.forEach((summary) => {
        if (summary) {
          this.upsertArchivedItem(summary, { adjustCounts: false });
        }
      });

      const hasMore = startIndex + pageTasks.length < this.archivedIndex.length;
      this.archivedCursor = hasMore && pageTasks.length
        ? { sort_at: this.taskSortAt(pageTasks[pageTasks.length - 1]), task_id: pageTasks[pageTasks.length - 1].id }
        : null;
      this.hasMoreArchived = hasMore;
      this.archivedLoaded = true;
      this.publish();
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
    const evt = parsed as WorkspaceActiveSnapshotEvent;
    if (evt.type === "ready" && typeof evt.snapshot_rev === "number") {
      if (evt.snapshot_rev !== this.snapshotRev) {
        this.snapshotRev = evt.snapshot_rev;
        this.ensureActiveSnapshot(true).catch(() => {});
      }
    }
    if (evt.snapshot_rev && evt.snapshot_rev > this.snapshotRev + 1) {
      this.ensureActiveSnapshot(true).catch(() => {});
    }
    if (evt.snapshot_rev) {
      this.snapshotRev = Math.max(this.snapshotRev, evt.snapshot_rev);
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
  }

  private upsertActiveSummary(summary: WorkspaceActiveTaskSummary) {
    const normalized = this.normalizeActiveSummary(summary);
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
  }

  private normalizeActiveSummary(summary: WorkspaceActiveTaskSummary): WorkspaceActiveSnapshotItem {
    const id = idToString(summary.task.id);
    const sortAtMs = Date.parse(summary.sort_at ?? "") || Date.now();
    const primarySessionId = idToString(summary.primary_session?.session?.id ?? "");
    const primaryHeadId = idToString(summary.primary_session_head?.session?.id ?? "");
    if (primaryHeadId) {
      this.sessionHeadsById.set(primaryHeadId, summary.primary_session_head);
    }
    const primary = summary.primary_session ? [this.normalizeSessionSummary(summary.primary_session)] : [];
    const sessions = summary.sessions.map((s) => this.normalizeSessionSummary(s));
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

    return {
      id,
      task: { ...summary.task },
      sessions: sortSessionSummaries(merged),
      primarySessionHead: summary.primary_session_head ?? null,
      primarySessionId: primarySessionId || null,
      sortAtMs,
      sort_at: summary.sort_at ?? null,
    };
  }

  private taskSortAt(task: Task): string {
    return task.archived_at ?? task.created_at ?? task.updated_at ?? "";
  }

  private taskSortMs(task: Task): number {
    return Date.parse(this.taskSortAt(task)) || 0;
  }

  private findArchivedStartIndex(cursor: { sort_at: string; task_id: { 0: string } | string }): number {
    const cursorId = idToString(cursor.task_id);
    if (!cursorId) return 0;
    const cursorSortMs = Date.parse(cursor.sort_at ?? "") || 0;
    const idx = this.archivedIndex.findIndex((task) => {
      const id = idToString(task.id);
      if (!id || id !== cursorId) return false;
      const sortMs = this.taskSortMs(task);
      return sortMs === cursorSortMs;
    });
    return idx >= 0 ? idx + 1 : 0;
  }

  private buildArchivedItem(task: Task): WorkspaceActiveSnapshotItem | null {
    const id = idToString(task.id);
    if (!id) return null;
    const existing = this.tasks.get(id);
    const summaries = existing?.sessions ?? [];
    const sessionList = summaries.map((summary) => summary.session).filter(Boolean);
    const primarySessionId = this.pickArchivedSessionId(task, sessionList);
    // TODO: hydrate archived session summaries on selection (avoid list fan-out).
    const sortAt = this.taskSortAt(task);
    return {
      id,
      task: { ...task },
      sessions: sortSessionSummaries(summaries),
      primarySessionId: primarySessionId || null,
      primarySessionHead: existing?.primarySessionHead ?? null,
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

  private normalizeSessionSummary(summary: SessionSnapshotSummary): SessionSnapshotSummary {
    return {
      session: { ...summary.session },
      last_message_at: summary.last_message_at ?? null,
      last_message_preview: summary.last_message_preview ?? null,
      last_event_seq: summary.last_event_seq ?? null,
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
      hasMoreActive: this.hasMoreActive,
      hasMoreArchived: this.hasMoreArchived,
      archivedLoaded: this.archivedLoaded,
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
