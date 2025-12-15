import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import {
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

const authToken = (): string | null => {
  try {
    return sessionStorage.getItem("contextAuthToken");
  } catch {
    return null;
  }
};

class SessionSupervisor {
  private listeners = new Set<() => void>();
  private snapshot: SessionSupervisorSnapshot = { connection: "connecting", sessions: {} };
  private entries = new Map<string, InternalEntry>();
  private ws: WebSocket | null = null;
  private wsUrl: string | null = null;
  private pollTimer: number | null = null;
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
    this.ensureLoaded(String(sessionId), opts).catch(() => {});
  };

  refreshQueue = (sessionId: string) => {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    this.refreshQueueAndDiff(entry).catch(() => {});
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
    if (this.snapshot.connection !== "connected") {
      this.ensurePolling();
    }
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
    for (const ev of incoming) {
      const eid = idToString(ev.id) || `${ev.created_at}-${ev.event_type}`;
      if (entry.eventIdSet.has(eid)) continue;
      entry.eventIdSet.add(eid);
      entry.events.push(ev);
    }
    entry.events.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
    while (entry.events.length > MAX_EVENTS_PER_SESSION) {
      const first = entry.events.shift();
      if (!first) break;
      const fid = idToString(first.id) || `${first.created_at}-${first.event_type}`;
      entry.eventIdSet.delete(fid);
    }
    const last = entry.events[entry.events.length - 1];
    if (last) entry.lastEventId = idToString(last.id) || entry.lastEventId;
  }

  private async connect() {
    const token = authToken();
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";

    // Prefer same-origin (works under dev proxy); fall back to daemon_url -> ws base if provided.
    let baseWs: string | null = null;
    try {
      this.health = await getHealth();
      const base = String(this.health.daemon_url || "").trim();
      if (base) {
        baseWs = base.startsWith("https://") ? base.replace(/^https:\/\//, "wss://") : base.replace(/^http:\/\//, "ws://");
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
  }

  private openWebSocket(url: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      let opened = false;
      const onOpen = () => {
        opened = true;
        this.ws = ws;
        this.wsUrl = url;
        this.snapshot = { ...this.snapshot, connection: "connected" };
        this.publish();
        this.stopPolling();
        this.updateWsSubscriptions();
        resolve();
      };
      const onError = () => {
        if (!opened) reject(new Error("ws connect failed"));
        this.snapshot = { ...this.snapshot, connection: "disconnected" };
        this.publish();
      };
      const onClose = () => {
        this.ws = null;
        this.snapshot = { ...this.snapshot, connection: "disconnected" };
        this.publish();
        this.ensurePolling();
        // reconnect
        window.setTimeout(() => this.connect().catch(() => {}), 800);
      };
      ws.addEventListener("open", onOpen, { once: true });
      ws.addEventListener("error", onError);
      ws.addEventListener("close", onClose);
      ws.addEventListener("message", (ev) => {
        try {
          const data = JSON.parse(String(ev.data ?? "{}"));
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

          if (data.event_type === "done" || data.event_type === "turn_interrupted") {
            this.refreshQueueAndDiff(entry).catch(() => {});
          }
          this.publish();
        } catch {
          // ignore
        }
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
      this.upsertEvents(entry, evs);
      entry.updatedAtMs = Date.now();
      this.publish();
    } catch {
      // ignore
    }
  }

  private async refreshQueueAndDiff(entry: InternalEntry) {
    const sid = entry.sessionId;
    entry.messages = await listMessages(sid);
    entry.queue = await listQueue(sid);
    if (entry.wantDiffCount > 0 && entry.trackId) {
      const d = await trackDiff(entry.trackId);
      entry.diff = d.diff;
    }
    entry.updatedAtMs = Date.now();
    this.publish();
  }

  private ensurePolling() {
    if (this.pollTimer) return;
    this.pollTimer = window.setInterval(() => {
      const ids = this.computeSubscribedSet();
      for (const sid of ids) {
        this.backfillSession(sid).catch(() => {});
      }
    }, POLL_INTERVAL_MS);
  }

  private stopPolling() {
    if (this.pollTimer) {
      window.clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
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
