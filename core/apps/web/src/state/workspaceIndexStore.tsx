import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type {
  Task,
  WorkspaceIndexCursor,
  WorkspaceIndexEvent,
  WorkspaceIndexPage,
  WorkspaceTaskSummary,
} from "@context/types";
import {
  getDaemonBaseUrl,
  getHealth,
  getWorkspaceIndex,
  idToString,
  type WorkspaceIndexParams,
} from "../api/client";
import { parseWsJson } from "../utils/wsJson";

export type WorkspaceIndexItem = WorkspaceTaskSummary & {
  id: string;
  sortAtMs: number;
};

export type WorkspaceIndexSnapshot = {
  workspaceId: string;
  initialized: boolean;
  connection: "idle" | "connecting" | "connected" | "disconnected";
  tasksById: Record<string, WorkspaceIndexItem>;
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

const authToken = (): string | null => {
  try {
    return sessionStorage.getItem("contextAuthToken");
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

class WorkspaceIndexStoreImpl {
  private listeners = new Set<() => void>();
  private snapshot: WorkspaceIndexSnapshot;
  private tasks = new Map<string, WorkspaceIndexItem>();
  private activeOrder: string[] = [];
  private archivedOrder: string[] = [];
  private activeCursor: WorkspaceIndexCursor | null = null;
  private archivedCursor: WorkspaceIndexCursor | null = null;
  private hasMoreActive = true;
  private hasMoreArchived = false;
  private archivedLoaded = false;
  private activeFetch: Promise<void> | null = null;
  private archivedFetch: Promise<void> | null = null;
  private ws: WebSocket | null = null;
  private reconnectTimer: number | null = null;
  private reconnectDelayMs = 1000;
  private snapshotRev = 0;
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

  getSnapshot = (): WorkspaceIndexSnapshot => this.snapshot;

  init = () => {
    this.ensureActivePage(true).catch(() => {});
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
    const updated: WorkspaceIndexItem = {
      ...existing,
      task,
    };
    this.tasks.set(id, updated);
    this.placeInOrders(updated);
    this.publish();
  }

  private async ensureActivePage(reset: boolean) {
    if (this.destroyed) return;
    if (reset) {
      this.activeCursor = null;
      this.hasMoreActive = true;
      this.tasks.clear();
      this.activeOrder = [];
      this.archivedOrder = [];
      this.archivedCursor = null;
      this.archivedLoaded = false;
      this.hasMoreArchived = false;
    } else if (!this.activeCursor && this.snapshot.initialized) {
      this.hasMoreActive = false;
      return;
    }

    this.setFetchState("active", "loading");
    const params: WorkspaceIndexParams = {
      limit: 50,
      cursor: reset ? null : this.activeCursor,
    };
    try {
      const page = await getWorkspaceIndex(this.workspaceId, params);
      this.snapshotRev = page.snapshot_rev;
      this.totalActive = page.total_active;
      this.totalArchived = page.total_archived;
      if (reset) {
        this.tasks.clear();
        this.activeOrder = [];
        this.archivedOrder = [];
      }
      page.tasks.forEach((summary) => this.upsertSummary(summary));
      this.activeCursor = page.next_cursor ?? null;
      this.hasMoreActive = Boolean(page.next_cursor);
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
      return;
    }
    this.setFetchState("archived", "loading");
    try {
      const page = await getWorkspaceIndex(this.workspaceId, {
        limit: 50,
        includeArchived: true,
        cursor: this.archivedCursor,
      });
      this.snapshotRev = Math.max(this.snapshotRev, page.snapshot_rev);
      page.tasks.forEach((summary) => this.upsertSummary(summary));
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
    const sameOrigin = `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/api/workspaces/${this.workspaceId}/index/stream${qs}`;
    urls.push(sameOrigin);

    const configured = getDaemonBaseUrl();
    if (configured) {
      const wsBase = configured.startsWith("https://")
        ? configured.replace(/^https:\/\//, "wss://")
        : configured.replace(/^http:\/\//, "ws://");
      urls.push(`${wsBase}/api/workspaces/${this.workspaceId}/index/stream${qs}`);
    } else {
      try {
        const health = await getHealth();
        const base = String(health.daemon_url || "").trim();
        if (base) {
          const wsBase = base.startsWith("https://")
            ? base.replace(/^https:\/\//, "wss://")
            : base.replace(/^http:\/\//, "ws://");
          urls.push(`${wsBase}/api/workspaces/${this.workspaceId}/index/stream${qs}`);
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
        reject(new Error("workspace index ws timeout"));
      }, 4000);

      ws.onopen = () => {
        opened = true;
        window.clearTimeout(timeoutId);
        this.ws = ws;
        this.snapshot.connection = "connected";
        this.publish();
        resolve();
      };

      ws.onmessage = (event) => {
        this.handleStreamMessage(event.data);
      };

      ws.onerror = () => {
        window.clearTimeout(timeoutId);
        if (!opened) {
          reject(new Error("workspace index ws error"));
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
    const evt = parsed as WorkspaceIndexEvent;
    if (evt.snapshot_rev && evt.snapshot_rev > this.snapshotRev + 1) {
      this.ensureActivePage(true).catch(() => {});
    }
    if (evt.snapshot_rev) {
      this.snapshotRev = Math.max(this.snapshotRev, evt.snapshot_rev);
    }
    switch (evt.type) {
      case "task_upsert":
        this.upsertSummary(evt.task);
        this.publish();
        break;
      case "task_delete": {
        const deleteId = idToString(evt.task_id);
        if (deleteId) {
          this.tasks.delete(deleteId);
          this.activeOrder = this.activeOrder.filter((id) => id !== deleteId);
          this.archivedOrder = this.archivedOrder.filter((id) => id !== deleteId);
          if (this.totalActive > 0) this.totalActive -= 1;
          this.snapshot.totalActive = Math.max(0, this.snapshot.totalActive - 1);
          this.publish();
        }
        break;
      }
      default:
        break;
    }
  }

  private upsertSummary(summary: WorkspaceTaskSummary) {
    const normalized = this.normalizeSummary(summary);
    this.tasks.set(normalized.id, normalized);
    this.placeInOrders(normalized);
  }

  private placeInOrders(item: WorkspaceIndexItem) {
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

  private normalizeSummary(summary: WorkspaceTaskSummary): WorkspaceIndexItem {
    const id = idToString(summary.task.id);
    const sortAtMs = Date.parse(summary.sort_at) || Date.now();
    return {
      ...summary,
      id,
      sort_at: summary.sort_at,
      provider_ids: summary.provider_ids ? [...summary.provider_ids] : [],
      tracks: summary.tracks.map((t) => ({
        track: { ...t.track },
        sessions: t.sessions ? [...t.sessions] : [],
      })),
      sortAtMs,
    };
  }

  private setFetchState(target: "active" | "archived", state: "idle" | "loading" | "error") {
    if (this.snapshot.fetchState[target] === state) return;
    this.snapshot.fetchState = { ...this.snapshot.fetchState, [target]: state };
    this.publish();
  }

  private publish() {
    const tasksById: Record<string, WorkspaceIndexItem> = {};
    for (const [id, item] of this.tasks.entries()) {
      tasksById[id] = item;
    }
    this.snapshot = {
      ...this.snapshot,
      tasksById,
      activeIds: [...this.activeOrder],
      archivedIds: [...this.archivedOrder],
      totalActive: this.totalActive ?? this.snapshot.totalActive,
      totalArchived: this.totalArchived ?? this.snapshot.totalArchived,
      hasMoreActive: this.hasMoreActive,
      hasMoreArchived: this.hasMoreArchived,
      archivedLoaded: this.archivedLoaded,
    };
    for (const listener of this.listeners) listener();
  }

  private totalActive = 0;
  private totalArchived = 0;
}

const WorkspaceIndexContext = createContext<WorkspaceIndexStoreImpl | null>(null);

export const WorkspaceIndexProvider = ({ workspaceId, children }: { workspaceId: string; children: React.ReactNode }) => {
  const storeRef = useRef<WorkspaceIndexStoreImpl | null>(null);
  if (!storeRef.current || storeRef.current.getSnapshot().workspaceId !== workspaceId) {
    storeRef.current?.destroy();
    storeRef.current = new WorkspaceIndexStoreImpl(workspaceId);
    storeRef.current.init();
  }

  useEffect(() => {
    return () => {
      storeRef.current?.destroy();
      storeRef.current = null;
    };
  }, [workspaceId]);

  return <WorkspaceIndexContext.Provider value={storeRef.current}>{children}</WorkspaceIndexContext.Provider>;
};

export const useWorkspaceIndexStore = (): WorkspaceIndexStoreImpl => {
  const store = useContext(WorkspaceIndexContext);
  if (!store) throw new Error("WorkspaceIndexStore missing provider");
  return store;
};

export const useWorkspaceIndexSnapshot = (): WorkspaceIndexSnapshot => {
  const store = useWorkspaceIndexStore();
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
};
