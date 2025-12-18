import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import {
  getDaemonBaseUrl,
  getHealth,
  getSession,
  idToString,
  listQueue,
  listMessages,
  listSessionEvents,
  listSessionEventsPage,
  Session,
  SessionEvent,
  Message,
  trackDiff,
} from "../api/client";
import type { Health } from "../api/client";
import { parseWsJson } from "../utils/wsJson";

type ConnectionStatus = "connecting" | "connected" | "disconnected";

export type SessionSupervisorSnapshot = {
  connection: ConnectionStatus;
  sessions: Record<string, SessionCacheEntry>;
};

export type SessionCacheEntry = {
  sessionId: string;
  session?: Session;
  events: SessionEvent[];
  messages: Message[];
  queue: Message[];
  diff?: string;
  diagnosticsByPath?: Record<string, any[]>;
  lastEventId?: string;
  loading: boolean;
  error?: string;
  subscribed: boolean;
  updatedAtMs: number;
};

type OpenOptions = {
  watchDiff?: boolean;
};

type InternalEntry = SessionCacheEntry & {
  refCount: number;
  wantDiffCount: number;
  nextDiffPollAtMs: number;
  warmUntilMs: number;
  eventIdSet: Set<string>;
  eventsHydrated: boolean;
  trackId?: string;
  diagnosticsByPath: Record<string, any[]>;
  fetching: {
    session: boolean;
    events: boolean;
    messages: boolean;
    queue: boolean;
    diff: boolean;
  };
};

const MAX_EVENTS_PER_SESSION = 2000;
const MAX_CACHED_SESSIONS = 30;
const WARM_TTL_MS = 10 * 60 * 1000;
const POLL_INTERVAL_MS = 1500;
const DIFF_POLL_INTERVAL_MS = 900;
const DIFF_POLL_IDLE_INTERVAL_MS = 2500;

const authToken = (): string | null => {
  try {
    return sessionStorage.getItem("contextAuthToken");
  } catch {
    return null;
  }
};

export class SessionSupervisor {
  private listeners = new Set<() => void>();
  private snapshot: SessionSupervisorSnapshot = { connection: "connecting", sessions: {} };
  private entries = new Map<string, InternalEntry>();
  private ws: WebSocket | null = null;
  private wsUrl: string | null = null;
  private reconnectTimer: number | null = null;
  private reconnectBackoffMs = 800;
  private pollTimer: number | null = null;
  private diffPollTimer: number | null = null;
  private resubscribeTimer: number | null = null;
  private health: Health | null = null;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): SessionSupervisorSnapshot => this.snapshot;

  openSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.ensureEntry(sessionId);
    entry.refCount += 1;
    if (opts?.watchDiff) entry.wantDiffCount += 1;
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    this.kick();
    this.ensureLoaded(sessionId, opts).catch(() => {});
    return () => this.closeSession(sessionId, opts);
  };

  closeSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.entries.get(sessionId);
    if (!entry) return;
    entry.refCount = Math.max(0, entry.refCount - 1);
    if (opts?.watchDiff) entry.wantDiffCount = Math.max(0, entry.wantDiffCount - 1);
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    this.kick();
  };

  refreshSession = (sessionId: string, opts?: OpenOptions) => {
    const sid = String(sessionId);
    this.ensureLoaded(sid, opts)
      .catch(() => {})
      .finally(() => {
        this.backfillSession(sid).catch(() => {});
      });
  };

  refreshQueue = (sessionId: string) => {
    const entry = this.ensureEntry(String(sessionId));
    return this.refreshQueueAndDiff(entry).catch(() => {});
  };

  setSession = (session: Session) => {
    const sessionId = idToString(session.id);
    if (!sessionId) return;
    const entry = this.ensureEntry(sessionId);
    entry.session = session;
    entry.trackId = idToString(session.track_id);
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  setDiff = (sessionId: string, diff: string) => {
    const entry = this.ensureEntry(sessionId);
    entry.diff = diff;
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  init = () => {
    this.connect().catch(() => {});
  };

  private publish() {
    this.evictIfNeeded();
    const sessions: Record<string, SessionCacheEntry> = {};
    for (const [id, e] of this.entries) {
      sessions[id] = {
        sessionId: e.sessionId,
        session: e.session,
        events: e.events,
        messages: e.messages,
        queue: e.queue,
        diff: e.diff,
        diagnosticsByPath: e.diagnosticsByPath,
        lastEventId: e.lastEventId,
        loading: e.loading,
        error: e.error,
        subscribed: e.subscribed,
        updatedAtMs: e.updatedAtMs,
      };
    }
    this.snapshot = { connection: this.snapshot.connection, sessions };
    for (const l of this.listeners) l();
  }

  private evictIfNeeded() {
    if (this.entries.size <= MAX_CACHED_SESSIONS) return;
    const now = Date.now();
    const candidates = [...this.entries.values()]
      .filter((e) => e.refCount === 0 && now > e.warmUntilMs)
      .sort((a, b) => a.updatedAtMs - b.updatedAtMs);
    for (const c of candidates) {
      if (this.entries.size <= MAX_CACHED_SESSIONS) break;
      this.entries.delete(c.sessionId);
    }
  }

  private ensureEntry(sessionId: string): InternalEntry {
    const id = String(sessionId);
    const existing = this.entries.get(id);
    if (existing) return existing;
    const entry: InternalEntry = {
      sessionId: id,
      session: undefined,
      events: [],
      messages: [],
      queue: [],
      diff: undefined,
      diagnosticsByPath: {},
      lastEventId: undefined,
      loading: true,
      error: undefined,
      subscribed: false,
      updatedAtMs: Date.now(),
      refCount: 0,
      wantDiffCount: 0,
      nextDiffPollAtMs: 0,
      warmUntilMs: Date.now() + WARM_TTL_MS,
      eventIdSet: new Set(),
      eventsHydrated: false,
      trackId: undefined,
      fetching: { session: false, events: false, messages: false, queue: false, diff: false },
    };
    this.entries.set(id, entry);
    this.publish();
    return entry;
  }

  private computeSubscribedSet(): string[] {
    const now = Date.now();
    const ids: string[] = [];
    for (const [id, e] of this.entries) {
      const keepWarm = e.refCount > 0 || now < e.warmUntilMs;
      if (keepWarm) ids.push(id);
    }
    return ids;
  }

  private scheduleResubscribe() {
    if (this.resubscribeTimer) window.clearTimeout(this.resubscribeTimer);
    this.resubscribeTimer = window.setTimeout(() => {
      this.resubscribeTimer = null;
      this.updateWsSubscriptions();
    }, 100);
  }

  private kick() {
    this.scheduleResubscribe();
    // Always keep a lightweight polling backfill running for "warm" sessions.
    // Even when the global WebSocket is connected, events can be missed after long-idle periods
    // (e.g. replying to older sessions). Backfill is cheap (cursor-based) and deduped.
    this.ensurePolling();
    this.ensureDiffPolling();
  }

  private async ensureLoaded(sessionId: string, opts?: OpenOptions) {
    const entry = this.ensureEntry(sessionId);

    const needSession = !entry.session && !entry.fetching.session;
    // Always hydrate events at least once via the full events endpoint.
    // WebSocket/backfill can race and populate a partial event list that may miss prior user_message events,
    // which would make the conversation header appear to “skip” a user turn.
    const needEvents = !entry.eventsHydrated && !entry.fetching.events;
    const needMessages = entry.messages.length === 0 && !entry.fetching.messages;
    const needQueue = entry.queue.length === 0 && !entry.fetching.queue;
    const needDiff = (opts?.watchDiff ?? false) && !entry.fetching.diff && entry.wantDiffCount > 0;

    entry.loading = true;
    this.publish();

    try {
      if (!this.health) {
        this.health = await getHealth().catch(() => null);
      }

      if (needSession) {
        entry.fetching.session = true;
        const s = await getSession(sessionId);
        entry.session = s;
        entry.trackId = idToString(s.track_id);
        entry.fetching.session = false;
      }

      if (needEvents) {
        entry.fetching.events = true;
        const evs = await listSessionEvents(sessionId);
        this.upsertEvents(entry, evs);
        entry.eventsHydrated = true;
        entry.fetching.events = false;
      }

      if (needMessages) {
        entry.fetching.messages = true;
        entry.messages = await listMessages(sessionId);
        entry.fetching.messages = false;
      }

      if (needQueue) {
        entry.fetching.queue = true;
        entry.queue = await listQueue(sessionId);
        entry.fetching.queue = false;
      }

      if (needDiff) {
        entry.fetching.diff = true;
        const trackId = entry.trackId;
        if (trackId) {
          const d = await trackDiff(trackId);
          entry.diff = d.diff;
        }
        entry.fetching.diff = false;
      }

      entry.error = undefined;
    } catch (e: any) {
      entry.error = e?.message ?? String(e);
    } finally {
      entry.loading = false;
      entry.updatedAtMs = Date.now();
      this.publish();
    }
  }

  private upsertEvents(entry: InternalEntry, incoming: SessionEvent[]) {
    if (incoming.length === 0) return;

    const lastExistingCreatedAt =
      entry.events.length > 0 ? String(entry.events[entry.events.length - 1]?.created_at ?? "") : null;

    let prevCreatedAt = lastExistingCreatedAt;
    let needsSort = false;
    const toAdd: SessionEvent[] = [];

    for (const ev of incoming) {
      const eid = idToString(ev.id) || `${ev.created_at}-${ev.event_type}`;
      if (entry.eventIdSet.has(eid)) continue;
      entry.eventIdSet.add(eid);

      const createdAt = String(ev.created_at ?? "");
      if (prevCreatedAt && createdAt.localeCompare(prevCreatedAt) < 0) needsSort = true;
      prevCreatedAt = createdAt;

      toAdd.push(ev);
    }

    if (toAdd.length === 0) return;

    entry.events.push(...toAdd);
    if (needsSort) {
      entry.events.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
    }
    while (entry.events.length > MAX_EVENTS_PER_SESSION) {
      const first = entry.events.shift();
      if (!first) break;
      const fid = idToString(first.id) || `${first.created_at}-${first.event_type}`;
      entry.eventIdSet.delete(fid);
    }
    const last = entry.events[entry.events.length - 1];
    if (last) entry.lastEventId = idToString(last.id) || entry.lastEventId;
  }

  // We only refresh durable data (Messages/Queue) once the turn is fully finalized.
  // `assistant_complete` is a UI boundary, but it can be observed before the daemon has
  // finished inserting the assistant message into the Messages table (event-first publish).
  private isRefreshBoundaryEventType(eventType: unknown): boolean {
    return eventType === "done" || eventType === "turn_interrupted";
  }

  private isMidTurn(entry: InternalEntry): boolean {
    const lastType = entry.events.length > 0 ? String(entry.events[entry.events.length - 1].event_type ?? "") : "";
    return lastType !== "" && !this.isRefreshBoundaryEventType(lastType);
  }

  private shouldPollDiff(entry: InternalEntry): boolean {
    if (entry.wantDiffCount <= 0) return false;
    if (!entry.trackId) return false;
    return true;
  }

  private async connect() {
    if (this.ws && (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)) {
      return;
    }
    const token = authToken();
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";

    // Prefer same-origin (works under dev proxy); fall back to configured daemon base URL; then to daemon_url from /api/health.
    let baseWs: string | null = null;
    const configuredBase = getDaemonBaseUrl();
    if (configuredBase) {
      baseWs = configuredBase.startsWith("https://")
        ? configuredBase.replace(/^https:\/\//, "wss://")
        : configuredBase.replace(/^http:\/\//, "ws://");
    }
    try {
      this.health = await getHealth();
      const base = String(this.health.daemon_url || "").trim();
      if (base && !baseWs) {
        baseWs = base.startsWith("https://")
          ? base.replace(/^https:\/\//, "wss://")
          : base.replace(/^http:\/\//, "ws://");
      }
    } catch {
      // ignore
    }

    const wsUrl =
      `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/api/stream${qs}`;
    const wsUrlAlt = baseWs ? `${baseWs}/api/stream${qs}` : null;

    const urls = wsUrlAlt && wsUrlAlt !== wsUrl ? [wsUrl, wsUrlAlt] : [wsUrl];
    this.snapshot = { ...this.snapshot, connection: "connecting" };
    this.publish();

    for (const candidate of urls) {
      try {
        await this.openWebSocket(candidate);
        return;
      } catch {
        // try next
      }
    }

    this.snapshot = { ...this.snapshot, connection: "disconnected" };
    this.publish();
    this.ensurePolling();
    this.scheduleReconnect();
  }

  private openWebSocket(url: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      let opened = false;
      const timeoutMs = 3000;
      const connectTimeout = window.setTimeout(() => {
        if (opened) return;
        try {
          ws.close();
        } catch {}
        reject(new Error("ws connect timeout"));
      }, timeoutMs);
      const onOpen = () => {
        opened = true;
        window.clearTimeout(connectTimeout);
        this.ws = ws;
        this.wsUrl = url;
        this.reconnectBackoffMs = 800;
        if (this.reconnectTimer) {
          window.clearTimeout(this.reconnectTimer);
          this.reconnectTimer = null;
        }
        this.snapshot = { ...this.snapshot, connection: "connected" };
        this.publish();
        this.stopPolling();
        this.updateWsSubscriptions();
        resolve();
      };
      const onError = () => {
        window.clearTimeout(connectTimeout);
        if (!opened) reject(new Error("ws connect failed"));
        this.snapshot = { ...this.snapshot, connection: "disconnected" };
        this.publish();
      };
      const onClose = () => {
        window.clearTimeout(connectTimeout);
        this.ws = null;
        this.snapshot = { ...this.snapshot, connection: "disconnected" };
        this.publish();
        this.ensurePolling();
        this.scheduleReconnect();
      };
      ws.addEventListener("open", onOpen, { once: true });
      ws.addEventListener("error", onError);
      ws.addEventListener("close", onClose);
      ws.addEventListener("message", (ev) => {
        void (async () => {
          try {
            const data = await parseWsJson(ev.data ?? "{}");
            if (!data) return;
            const sid = idToString(data.session_id);
            if (!sid) return;
            const entry = this.entries.get(sid);
            if (!entry) return;
            if (String(data.type || "") === "lsp_diagnostics") {
              const path = String(data.path || "");
              if (path) {
                entry.diagnosticsByPath[path] = Array.isArray(data.diagnostics) ? data.diagnostics : [];
              }
              entry.updatedAtMs = Date.now();
              this.publish();
              return;
            }
            this.upsertEvents(entry, [data as SessionEvent]);
            entry.updatedAtMs = Date.now();
            // Keep hot sessions warm when they are producing events.
            entry.warmUntilMs = Date.now() + WARM_TTL_MS;

            if (this.isRefreshBoundaryEventType(data.event_type)) {
              this.refreshQueueAndDiff(entry).catch(() => {});
            } else if (this.isMidTurn(entry) && this.shouldPollDiff(entry)) {
              this.ensureDiffPolling();
              // Diff is computed from git and can safely be refreshed mid-turn.
              this.refreshDiff(entry).catch(() => {});
            }
            this.publish();
          } catch {
            // ignore
          }
        })();
      });
    });
  }

  private updateWsSubscriptions() {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;

    const ids = this.computeSubscribedSet();
    const nextSet = new Set(ids);
    for (const [id, e] of this.entries) {
      const should = nextSet.has(id);
      if (e.subscribed !== should) {
        e.subscribed = should;
      }
    }
    this.publish();

    ws.send(JSON.stringify({ type: "set", session_ids: ids }));

    // Backfill deltas for subscribed sessions.
    for (const sid of ids) {
      this.backfillSession(sid).catch(() => {});
    }
  }

  private async backfillSession(sessionId: string) {
    const entry = this.entries.get(sessionId);
    if (!entry) return;
    const after = entry.lastEventId;
    try {
      const evs = await listSessionEventsPage(sessionId, after, 500);
      const sawRefreshBoundary = evs.some((e) => this.isRefreshBoundaryEventType(e.event_type));
      this.upsertEvents(entry, evs);
      entry.updatedAtMs = Date.now();
      this.publish();
      // When disconnected (polling mode), we won't get the WS-triggered refresh that keeps Messages in sync.
      // Refreshing here ensures assistant replies show up after daemon/webapp restarts.
      if (sawRefreshBoundary) {
        await this.refreshQueueAndDiff(entry);
      } else if (this.isMidTurn(entry) && this.shouldPollDiff(entry)) {
        this.ensureDiffPolling();
        await this.refreshDiff(entry);
      }
    } catch {
      // ignore
    }
  }

  private async refreshQueueAndDiff(entry: InternalEntry) {
    const sid = entry.sessionId;
    entry.messages = await listMessages(sid);
    entry.queue = await listQueue(sid);
    await this.refreshDiff(entry);
    entry.updatedAtMs = Date.now();
    this.publish();
  }

  private async refreshDiff(entry: InternalEntry) {
    if (entry.fetching.diff) return;
    if (entry.wantDiffCount <= 0) return;
    if (!entry.trackId) return;
    entry.fetching.diff = true;
    try {
      const d = await trackDiff(entry.trackId);
      const next = d.diff ?? "";
      if (next !== (entry.diff ?? "")) {
        entry.diff = next;
        entry.updatedAtMs = Date.now();
        this.publish();
      }
    } finally {
      entry.fetching.diff = false;
    }
  }

  private ensurePolling() {
    if (this.pollTimer) return;
    this.pollTimer = window.setInterval(() => {
      const ids = this.computeSubscribedSet();
      for (const sid of ids) {
        const entry = this.entries.get(sid);
        if (!entry) continue;
        const lastType = entry.events.length > 0 ? String(entry.events[entry.events.length - 1].event_type ?? "") : "";
        const doneLike = this.isRefreshBoundaryEventType(lastType);
        // When connected, only poll sessions that appear to be mid-turn to avoid unnecessary load.
        // When disconnected, poll everything warm.
        if (this.snapshot.connection === "connected" && doneLike) continue;
        this.backfillSession(sid).catch(() => {});
      }
    }, POLL_INTERVAL_MS);
  }

  private ensureDiffPolling() {
    if (this.diffPollTimer) return;
    this.diffPollTimer = window.setInterval(() => {
      const ids = this.computeSubscribedSet();
      let anyNeeded = false;
      const now = Date.now();
      for (const sid of ids) {
        const entry = this.entries.get(sid);
        if (!entry) continue;
        if (!this.shouldPollDiff(entry)) continue;
        anyNeeded = true;
        if (entry.fetching.diff) continue;
        if (entry.nextDiffPollAtMs > now) continue;
        entry.nextDiffPollAtMs = now + (this.isMidTurn(entry) ? DIFF_POLL_INTERVAL_MS : DIFF_POLL_IDLE_INTERVAL_MS);
        this.refreshDiff(entry).catch(() => {});
      }
      if (!anyNeeded) this.stopDiffPolling();
    }, DIFF_POLL_INTERVAL_MS);
  }

  private stopPolling() {
    if (this.pollTimer) {
      window.clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  private stopDiffPolling() {
    if (this.diffPollTimer) {
      window.clearInterval(this.diffPollTimer);
      this.diffPollTimer = null;
    }
  }

  private scheduleReconnect() {
    if (this.reconnectTimer) return;
    const delay = this.reconnectBackoffMs;
    this.reconnectBackoffMs = Math.min(10_000, Math.floor(this.reconnectBackoffMs * 1.5));
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      this.connect().catch(() => {});
    }, delay);
  }
}

const SessionSupervisorContext = createContext<SessionSupervisor | null>(null);

export function SessionSupervisorProvider({ children }: { children: React.ReactNode }) {
  const supRef = useRef<SessionSupervisor | null>(null);
  if (!supRef.current) {
    supRef.current = new SessionSupervisor();
  }

  useEffect(() => {
    supRef.current?.init();
    return () => {
      // best-effort close
    };
  }, []);

  return (
    <SessionSupervisorContext.Provider value={supRef.current}>
      {children}
    </SessionSupervisorContext.Provider>
  );
}

export function useSessionSupervisor() {
  const sup = useContext(SessionSupervisorContext);
  if (!sup) throw new Error("SessionSupervisorProvider missing");
  return sup;
}

export function useSessionCacheSnapshot(): SessionSupervisorSnapshot {
  const sup = useSessionSupervisor();
  return useSyncExternalStore(sup.subscribe, sup.getSnapshot, sup.getSnapshot);
}

export function useSessionEntry(sessionId: string): SessionCacheEntry | null {
  const snap = useSessionCacheSnapshot();
  return snap.sessions[String(sessionId)] ?? null;
}

export function useOpenSession(sessionId: string, opts?: OpenOptions) {
  const sup = useSessionSupervisor();
  const stableOpts = useMemo(() => ({ watchDiff: opts?.watchDiff ?? false }), [opts?.watchDiff]);
  useEffect(() => {
    if (!sessionId) return;
    return sup.openSession(String(sessionId), stableOpts);
  }, [sup, sessionId, stableOpts]);
}
