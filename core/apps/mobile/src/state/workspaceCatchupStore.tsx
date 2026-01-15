import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { Buffer } from "buffer";
import type {
  Session,
  SessionSnapshotSummary,
  Task,
  WorkspaceActiveSnapshotClientMessage,
  WorkspaceActiveSnapshotEvent,
  WorkspaceActiveSnapshotSessionSubscription,
  WorkspaceActiveTaskSummary,
} from "@ctx/types";

import {
  getSessionSnapshot,
  getWorkspaceActiveSnapshot,
  idToString,
  listTaskSessions,
  listTasks,
  type ConnectionConfig,
  type WorkspaceActiveSnapshotParams,
} from "../api/client";
import type { E2eeEnvelope } from "../utils/e2ee";
import { decryptPayload, encryptPayload } from "../utils/e2ee";
import { parseWsJson } from "../utils/wsJson";
import { useConnection } from "./ConnectionProvider";
import { nextSecureSeq } from "./secureSeq";

export type WorkspaceTrackSummary = {
  track: {
    id: string;
    label: string;
    status: string;
  };
  primary_session_id?: { 0: string } | string | null;
  sessions: SessionSnapshotSummary[];
};

export type WorkspaceCatchupItem = {
  id: string;
  task: Task;
  sessions: SessionSnapshotSummary[];
  tracks: WorkspaceTrackSummary[];
  sort_at?: string | null;
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
  subscribeEvents: (listener: (event: WorkspaceActiveSnapshotEvent) => void) => () => void;
  getSnapshot: () => WorkspaceCatchupState;
  setSubscriptions: (subscriptions: WorkspaceActiveSnapshotSessionSubscription[]) => void;
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

const sortSessionSummaries = (summaries: SessionSnapshotSummary[]): SessionSnapshotSummary[] => {
  return summaries
    .slice()
    .sort((a, b) => String(a.session.created_at ?? "").localeCompare(String(b.session.created_at ?? "")));
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

const decodeText = (bytes: Uint8Array): string => {
  if (typeof TextDecoder !== "undefined") {
    return new TextDecoder().decode(bytes);
  }
  return Buffer.from(bytes).toString("utf-8");
};

const isSecureEnvelope = (value: any): value is E2eeEnvelope =>
  value &&
  typeof value.device_id === "string" &&
  typeof value.seq === "number" &&
  typeof value.nonce === "string" &&
  typeof value.ciphertext === "string";

export class WorkspaceCatchupStoreImpl implements WorkspaceCatchupEventSource {
  private listeners = new Set<() => void>();
  private eventListeners = new Set<(event: WorkspaceActiveSnapshotEvent) => void>();
  private snapshot: WorkspaceCatchupState;
  private tasks = new Map<string, WorkspaceCatchupItem>();
  private activeOrder: string[] = [];
  private archivedOrder: string[] = [];
  private activeLimit = 50;
  private archivedCursor: { sort_at: string; task_id: { 0: string } | string } | null = null;
  private archivedIndex: Task[] = [];
  private archivedIndexLoaded = false;
  private totalActive = 0;
  private totalArchived = 0;
  private hasMoreActive = true;
  private hasMoreArchived = false;
  private archivedLoaded = false;
  private ws: WebSocket | null = null;
  private secureContext: { key: Uint8Array; deviceId: string } | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelayMs = 1000;
  private snapshotRev = 0;
  private subscriptions: WorkspaceActiveSnapshotSessionSubscription[] = [];
  private sessionLastEventSeq = new Map<string, number>();
  private subscriptionKey = "";
  private destroyed = false;
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

  subscribeEvents = (listener: (event: WorkspaceActiveSnapshotEvent) => void): (() => void) => {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  };

  getSnapshot = (): WorkspaceCatchupState => this.snapshot;

  setSubscriptions = (subscriptions: WorkspaceActiveSnapshotSessionSubscription[]) => {
    const next = normalizeSubscriptions(subscriptions, this.sessionLastEventSeq);
    const key = next.map((sub) => `${idToString(sub.session_id)}:${sub.after_seq ?? 0}`).join("|");
    if (key === this.subscriptionKey) return;
    this.subscriptionKey = key;
    this.subscriptions = next;
    void this.flushSubscriptions();
  };

  init = () => {
    this.destroyed = false;
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
    this.activeLimit += 50;
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
    const prevArchived = Boolean(existing.task.archived_at);
    const nextArchived = Boolean(task.archived_at);
    const stableSortAt = task.archived_at ?? task.created_at ?? existing.sort_at;
    const stableSortAtMs = Date.parse(stableSortAt ?? "") || existing.sortAtMs || Date.now();
    const updated: WorkspaceCatchupItem = {
      ...existing,
      task: { ...task },
      tracks: this.buildTracks(task, existing.sessions),
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

  applyTaskDelete(taskId: string) {
    this.deleteTask(taskId);
    this.archivedIndexLoaded = false;
    this.publish();
  }

  private async ensureActivePage(reset: boolean) {
    if (this.destroyed) return;
    if (reset && this.snapshot.fetchState.active === "loading") return;

    this.setFetchState("active", "loading");
    const params: WorkspaceActiveSnapshotParams = {
      limit: this.activeLimit,
    };
    try {
      const snapshot = await getWorkspaceActiveSnapshot(this.conn, this.workspaceId, params);
      this.snapshotRev = Math.max(this.snapshotRev, snapshot.snapshot_rev ?? 0);
      this.totalActive = snapshot.active.total_count ?? this.totalActive;
      if (reset) {
        this.tasks.clear();
        this.activeOrder = [];
        this.archivedOrder = [];
        this.archivedCursor = null;
        this.archivedIndexLoaded = false;
        this.archivedLoaded = false;
        this.hasMoreArchived = false;
      }

      const nextActiveIds = new Set<string>();
      for (const summary of snapshot.active.tasks ?? []) {
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
            this.deleteTask(id, { adjustCounts: false });
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
        const tasks = await listTasks(this.conn, this.workspaceId);
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
      const pageTasks = this.archivedIndex.slice(startIndex, startIndex + 50);
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

  private async ensureSecureContext(): Promise<void> {
    if (!this.conn.deviceId || !this.conn.daemonPublicKey) {
      this.secureContext = null;
      return;
    }
    const { getSecureConnectionContext } = await import("../utils/secureConnection");
    this.secureContext = await getSecureConnectionContext(
      this.conn.deviceId,
      this.conn.daemonPublicKey,
    );
  }

  private async connectStream() {
    if (this.destroyed || this.ws) return;
    this.snapshot.connection = "connecting";
    this.publish();
    try {
      await this.ensureSecureContext();
    } catch {
      this.snapshot.connection = "disconnected";
      this.publish();
      this.scheduleReconnect();
      return;
    }
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
    if (this.secureContext) {
      const qs = `?device_id=${encodeURIComponent(this.secureContext.deviceId)}`;
      const url = `${toWsUrl(this.conn, `/api/mobile/secure/workspaces/${this.workspaceId}/stream`)}${qs}`;
      return dedupeUrls([url]);
    }
    const qs = this.conn.token ? `?token=${encodeURIComponent(this.conn.token)}` : "";
    const url = `${toWsUrl(this.conn, `/api/workspaces/${this.workspaceId}/active_snapshot/stream`)}${qs}`;
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
        reject(new Error("workspace snapshot ws timeout"));
      }, 4000);

      ws.onopen = () => {
        opened = true;
        clearTimeout(timeoutId);
        this.ws = ws;
        this.snapshot.connection = "connected";
        this.publish();
        void this.flushSubscriptions();
        resolve();
      };

      ws.onmessage = (event) => {
        this.handleStreamMessage(event.data);
      };

      ws.onerror = () => {
        clearTimeout(timeoutId);
        if (!opened) {
          reject(new Error("workspace snapshot ws error"));
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
    let evt: WorkspaceActiveSnapshotEvent | null = null;
    if (this.secureContext) {
      if (!isSecureEnvelope(parsed)) return;
      if (parsed.device_id !== this.secureContext.deviceId) return;
      try {
        const plaintext = decryptPayload(
          this.secureContext.key,
          this.secureContext.deviceId,
          parsed.seq,
          parsed,
        );
        evt = JSON.parse(decodeText(plaintext)) as WorkspaceActiveSnapshotEvent;
      } catch {
        return;
      }
    } else {
      evt = parsed as WorkspaceActiveSnapshotEvent;
    }
    if (!evt) return;
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
      case "active_task_upsert":
        this.upsertActiveSummary(evt.task);
        this.publish();
        break;
      case "active_task_delete":
        this.deleteTask(evt.task_id, { adjustCounts: true });
        this.publish();
        break;
      case "session_summary":
        this.applySessionSummary(evt.summary);
        this.publish();
        break;
      case "session_head_delta":
      case "session_gap":
      case "worktree_bootstrap":
        break;
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

  private async flushSubscriptions() {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const message: WorkspaceActiveSnapshotClientMessage = {
      type: "subscribe",
      sessions: this.subscriptions,
    };
    await this.sendWsMessage(message);
  }

  private async sendWsMessage(message: WorkspaceActiveSnapshotClientMessage) {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const payload = JSON.stringify(message);
    if (!this.secureContext) {
      try {
        ws.send(payload);
      } catch {
        // ignore send errors
      }
      return;
    }
    try {
      const seq = await nextSecureSeq(this.secureContext.deviceId);
      const bytes = new Uint8Array(Buffer.from(payload, "utf-8"));
      const envelope = encryptPayload(this.secureContext.key, this.secureContext.deviceId, seq, bytes);
      ws.send(JSON.stringify(envelope));
    } catch {
      // ignore secure send errors
    }
  }

  private deleteTask(taskId: { 0: string } | string, opts?: { adjustCounts?: boolean }) {
    const deleteId = idToString(taskId);
    if (!deleteId) return;
    const existing = this.tasks.get(deleteId);
    if (!existing) return;
    this.tasks.delete(deleteId);
    this.activeOrder = this.activeOrder.filter((id) => id !== deleteId);
    this.archivedOrder = this.archivedOrder.filter((id) => id !== deleteId);
    if (opts?.adjustCounts !== false) {
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
    this.tasks.set(normalized.id, normalized);
    if (existing) {
      this.updateCountsForMove(existing, normalized);
    } else {
      this.totalActive += 1;
    }
    this.placeInOrders(normalized);
    this.rebuildSessionLastEventSeq();
  }

  private upsertArchivedItem(item: WorkspaceCatchupItem, opts?: { adjustCounts?: boolean }) {
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
    const ordered = sortSessionSummaries(nextSessions);
    this.tasks.set(taskId, {
      ...task,
      sessions: ordered,
      tracks: this.buildTracks(task.task, ordered),
    });
    this.rebuildSessionLastEventSeq();
  }

  private normalizeActiveSummary(summary: WorkspaceActiveTaskSummary): WorkspaceCatchupItem {
    const id = idToString(summary.task.id);
    const sortAtMs = Date.parse(summary.sort_at ?? "") || Date.now();
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
    const ordered = sortSessionSummaries(merged);
    return {
      id,
      task: { ...summary.task },
      sessions: ordered,
      tracks: this.buildTracks(summary.task, ordered),
      sortAtMs,
      sort_at: summary.sort_at ?? null,
    };
  }

  private buildTracks(task: Task, sessions: SessionSnapshotSummary[]): WorkspaceTrackSummary[] {
    const taskId = idToString(task.id);
    const label = "Track";
    const status = sessions.some((s) => s.session.status === "active" || s.session.status === "running")
      ? "active"
      : "completed";
    if (!taskId) return [];
    return [
      {
        track: { id: taskId, label, status },
        primary_session_id: task.primary_session_id ?? null,
        sessions,
      },
    ];
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

  private async buildArchivedItem(task: Task): Promise<WorkspaceCatchupItem | null> {
    const id = idToString(task.id);
    if (!id) return null;
    const sessions = await listTaskSessions(this.conn, id);
    let summaries = sessions.map((session) => this.sessionToSummary(session));
    const preferredId = this.pickArchivedSessionId(task, sessions);
    if (preferredId) {
      try {
        const snapshot = await getSessionSnapshot(this.conn, preferredId, 200, false);
        const snapshotId = idToString(snapshot.summary.session.id);
        if (snapshotId) {
          const normalized = this.normalizeSessionSummary(snapshot.summary);
          const idx = summaries.findIndex((summary) => idToString(summary.session.id) === snapshotId);
          if (idx >= 0) {
            summaries[idx] = normalized;
          } else {
            summaries.push(normalized);
          }
        }
      } catch {
        // ignore archived snapshot errors
      }
    }
    const sortAt = this.taskSortAt(task);
    const ordered = sortSessionSummaries(summaries);
    return {
      id,
      task: { ...task },
      sessions: ordered,
      tracks: this.buildTracks(task, ordered),
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

export function useWorkspaceCatchupEvents(handler: (event: WorkspaceActiveSnapshotEvent) => void) {
  const store = useWorkspaceCatchupStore();
  const stableHandler = useMemo(() => handler, [handler]);
  useEffect(() => store.subscribeEvents(stableHandler), [store, stableHandler]);
}
