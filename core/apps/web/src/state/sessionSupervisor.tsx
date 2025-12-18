import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import {
  getDaemonBaseUrl,
  getHealth,
  getSession,
  idToString,
  listQueue,
  listMessages,
  listSessionEventsPage,
  listSessionEventsTail,
  listSessionTurnsPage,
  listTurnTools,
  Session,
  SessionEvent,
  SessionTurn,
  SessionTurnTool,
  SessionTurnStatus,
  Message,
  trackDiff,
} from "../api/client";
import type { Health } from "../api/client";
import { parseWsJson } from "../utils/wsJson";

type ConnectionStatus = "connecting" | "connected" | "disconnected";

const readTunableInt = (key: string, fallback: number) => {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return fallback;
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
    return parsed;
  } catch {
    return fallback;
  }
};

export type SessionSupervisorSnapshot = {
  connection: ConnectionStatus;
  sessions: Record<string, SessionCacheEntry>;
};

export type SessionCacheEntry = {
  sessionId: string;
  session?: Session;
  turns: SessionTurn[];
  turnToolsByTurnId: Record<string, SessionTurnTool[]>;
  turnToolsLoading: string[];
  hasMoreTurns: boolean;
  events: SessionEvent[];
  messages: Message[];
  queue: Message[];
  diff?: string;
  diagnosticsByPath?: Record<string, any[]>;
  lastEventSeq?: number;
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
  seqSet: Set<number>;
  turnsHydrated: boolean;
  oldestTurnSeq?: number;
  toolStatusByKey: Map<string, string>;
  toolIdsByTurn: Map<string, Set<string>>;
  turnToolsLoadingSet: Set<string>;
  eventsHydrated: boolean;
  trackId?: string;
  diagnosticsByPath: Record<string, any[]>;
  fetching: {
    session: boolean;
    turns: boolean;
    events: boolean;
    messages: boolean;
    queue: boolean;
    diff: boolean;
  };
};

const EVENT_BUFFER_LIMIT = readTunableInt("contextEventBufferLimit", 800);
const TURN_PAGE_LIMIT = readTunableInt("contextTurnPageLimit", 60);
const WARM_SESSION_BUDGET = readTunableInt("contextWarmSessionBudget", 12);
const MAX_CACHED_SESSIONS = readTunableInt(
  "contextMaxCachedSessions",
  Math.max(30, WARM_SESSION_BUDGET * 3),
);
const WARM_TTL_MS = readTunableInt("contextWarmSessionTtlMs", 10 * 60 * 1000);
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
        turns: e.turns,
        turnToolsByTurnId: e.turnToolsByTurnId,
        turnToolsLoading: [...e.turnToolsLoadingSet],
        hasMoreTurns: e.hasMoreTurns,
        events: e.events,
        messages: e.messages,
        queue: e.queue,
        diff: e.diff,
        diagnosticsByPath: e.diagnosticsByPath,
        lastEventSeq: e.lastEventSeq,
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
      turns: [],
      turnToolsByTurnId: {},
      turnToolsLoading: [],
      hasMoreTurns: true,
      events: [],
      messages: [],
      queue: [],
      diff: undefined,
      diagnosticsByPath: {},
      lastEventSeq: undefined,
      loading: true,
      error: undefined,
      subscribed: false,
      updatedAtMs: Date.now(),
      refCount: 0,
      wantDiffCount: 0,
      nextDiffPollAtMs: 0,
      warmUntilMs: Date.now() + WARM_TTL_MS,
      seqSet: new Set(),
      turnsHydrated: false,
      oldestTurnSeq: undefined,
      toolStatusByKey: new Map(),
      toolIdsByTurn: new Map(),
      turnToolsLoadingSet: new Set(),
      eventsHydrated: false,
      trackId: undefined,
      fetching: { session: false, turns: false, events: false, messages: false, queue: false, diff: false },
    };
    this.entries.set(id, entry);
    this.publish();
    return entry;
  }

  private computeSubscribedSet(): string[] {
    const now = Date.now();
    const visible: string[] = [];
    const warm: InternalEntry[] = [];
    for (const [id, e] of this.entries) {
      if (!e.eventsHydrated) continue;
      if (e.refCount > 0) {
        visible.push(id);
        continue;
      }
      if (now < e.warmUntilMs) {
        warm.push(e);
      }
    }
    warm.sort((a, b) => b.updatedAtMs - a.updatedAtMs);
    const warmed = warm.slice(0, WARM_SESSION_BUDGET).map((e) => e.sessionId);
    return [...visible, ...warmed];
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
    const needTurns = !entry.turnsHydrated && !entry.fetching.turns;
    const needEvents = !entry.eventsHydrated && !entry.fetching.events;
    const needMessages = entry.messages.length === 0 && !entry.fetching.messages;
    const needQueue = entry.queue.length === 0 && !entry.fetching.queue;
    const needDiff = (opts?.watchDiff ?? false) && !entry.fetching.diff && entry.wantDiffCount > 0;

    const hasCached =
      entry.turns.length > 0 || entry.messages.length > 0 || entry.events.length > 0;
    entry.loading = !hasCached;
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

      if (needTurns) {
        entry.fetching.turns = true;
        const hadTurns = entry.turns.length > 0;
        const turns = await listSessionTurnsPage(sessionId, undefined, TURN_PAGE_LIMIT);
        this.mergeTurns(entry, turns);
        if (!hadTurns) {
          entry.hasMoreTurns = turns.length >= TURN_PAGE_LIMIT;
        }
        entry.turnsHydrated = true;
        entry.fetching.turns = false;
      }

      if (needEvents) {
        entry.fetching.events = true;
        // Hydrate with a bounded tail; the WS stream is resumable from the last seen seq.
        const evs = await listSessionEventsTail(sessionId, EVENT_BUFFER_LIMIT);
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
      // Hydrating the event tail establishes a baseline cursor (lastEventSeq) which enables WS subscribe.
      // Ensure we resubscribe after hydration completes.
      this.scheduleResubscribe();
    }
  }

  private upsertEvents(entry: InternalEntry, incoming: SessionEvent[]) {
    if (incoming.length === 0) return;
    const sorted = [...incoming].sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0));

    let needsSort = false;
    const lastExistingSeq = entry.events.length > 0 ? (entry.events[entry.events.length - 1]?.seq ?? 0) : 0;
    let prevSeq = lastExistingSeq;

    const toAdd: SessionEvent[] = [];
    for (const ev of sorted) {
      const seq = Number(ev.seq ?? 0);
      if (!Number.isFinite(seq) || seq <= 0) continue;
      if (entry.seqSet.has(seq)) continue;
      entry.seqSet.add(seq);
      if (prevSeq && seq < prevSeq) needsSort = true;
      prevSeq = seq;
      toAdd.push(ev);
    }

    if (toAdd.length === 0) return;
    entry.events.push(...toAdd);
    if (needsSort) {
      entry.events.sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0));
    }
    while (entry.events.length > EVENT_BUFFER_LIMIT) {
      const first = entry.events.shift();
      if (!first) break;
      const seq = Number(first.seq ?? 0);
      if (Number.isFinite(seq)) entry.seqSet.delete(seq);
    }
    const last = entry.events[entry.events.length - 1];
    if (last && Number.isFinite(last.seq)) entry.lastEventSeq = Number(last.seq);
  }

  private mergeTurns(entry: InternalEntry, incoming: SessionTurn[]) {
    if (incoming.length === 0) return;
    const map = new Map<string, SessionTurn>();
    for (const t of entry.turns) {
      const id = idToString(t.turn_id);
      if (!id) continue;
      map.set(id, t);
    }
    for (const t of incoming) {
      const id = idToString(t.turn_id);
      if (!id) continue;
      const prev = map.get(id);
      map.set(id, this.mergeTurn(prev, t));
    }
    const next = [...map.values()];
    next.sort((a, b) => this.compareTurnOrder(a, b));
    entry.turns = next;
    if (next.length > 0) {
      const startSeq = Number(next[0].start_seq ?? Number.NaN);
      if (Number.isFinite(startSeq)) {
        entry.oldestTurnSeq = startSeq;
      }
    }
  }

  private mergeTurn(prev: SessionTurn | undefined, next: SessionTurn): SessionTurn {
    if (!prev) return next;
    const assistant_partial = mergeStreamingText(prev.assistant_partial, next.assistant_partial);
    const thought_partial = mergeStreamingText(prev.thought_partial, next.thought_partial);
    const status =
      this.turnStatusRank(next.status) >= this.turnStatusRank(prev.status)
        ? next.status
        : prev.status;
    const updated_at =
      new Date(next.updated_at).getTime() >= new Date(prev.updated_at).getTime()
        ? next.updated_at
        : prev.updated_at;
    return {
      ...prev,
      ...next,
      status,
      updated_at,
      assistant_partial,
      thought_partial,
      tool_total: Math.max(prev.tool_total ?? 0, next.tool_total ?? 0),
      tool_pending: Math.max(prev.tool_pending ?? 0, next.tool_pending ?? 0),
      tool_running: Math.max(prev.tool_running ?? 0, next.tool_running ?? 0),
      tool_completed: Math.max(prev.tool_completed ?? 0, next.tool_completed ?? 0),
      tool_failed: Math.max(prev.tool_failed ?? 0, next.tool_failed ?? 0),
    };
  }

  private compareTurnOrder(a: SessionTurn, b: SessionTurn): number {
    const sa = Number(a.start_seq ?? Number.NaN);
    const sb = Number(b.start_seq ?? Number.NaN);
    if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) {
      return sa - sb;
    }
    return String(a.started_at).localeCompare(String(b.started_at));
  }

  private turnStatusRank(status: SessionTurnStatus): number {
    switch (status) {
      case "queued":
        return 0;
      case "running":
        return 1;
      case "completed":
        return 2;
      case "interrupted":
        return 3;
      case "failed":
        return 3;
      default:
        return 1;
    }
  }

  private applyEventToTurns(entry: InternalEntry, event: SessionEvent): boolean {
    const turnId = idToString(event.turn_id);
    if (!turnId) return false;
    const idx = entry.turns.findIndex((t) => idToString(t.turn_id) === turnId);
    if (idx < 0) return false;

    const turn = entry.turns[idx];
    let changed = false;
    switch (String(event.event_type)) {
      case "assistant_chunk": {
        const fragment = String(event.payload_json?.content_fragment ?? "");
        if (fragment) {
          turn.assistant_partial = appendFragment(turn.assistant_partial, fragment);
          changed = true;
        }
        break;
      }
      case "thought_chunk": {
        const fragment = String(event.payload_json?.content_fragment ?? "");
        if (fragment) {
          turn.thought_partial = appendFragment(turn.thought_partial, fragment);
          changed = true;
        }
        break;
      }
      case "assistant_complete": {
        const full =
          event.payload_json?.full_content ??
          event.payload_json?.content ??
          turn.assistant_partial;
        if (full) {
          turn.assistant_partial = String(full);
        }
        if (turn.status !== "completed") turn.status = "completed";
        changed = true;
        break;
      }
      case "turn_interrupted": {
        turn.status = "interrupted";
        changed = true;
        break;
      }
      case "error": {
        turn.status = "failed";
        changed = true;
        break;
      }
      case "done": {
        if (turn.status !== "interrupted" && turn.status !== "failed") {
          turn.status = "completed";
        }
        if (event.payload_json?.context_window) {
          turn.metrics_json = event.payload_json.context_window;
        }
        changed = true;
        break;
      }
      case "tool_call":
      case "tool_call_update":
      case "tool_result": {
        if (this.applyToolEventToTurn(entry, turnId, turn, event)) {
          changed = true;
        }
        break;
      }
      default:
        break;
    }

    if (!changed) return false;
    turn.updated_at = event.created_at ?? turn.updated_at;
    entry.turns[idx] = { ...turn };
    return true;
  }

  private applyToolEventToTurn(
    entry: InternalEntry,
    turnId: string,
    turn: SessionTurn,
    event: SessionEvent,
  ): boolean {
    if (turn.status !== "running" && turn.status !== "queued") {
      return false;
    }
    const toolCallId = extractToolCallId(event);
    if (!toolCallId) return false;
    const nextStatus = extractToolStatus(event);
    if (!nextStatus) return false;

    const key = `${turnId}:${toolCallId}`;
    const prevStatus = entry.toolStatusByKey.get(key);
    if (!entry.toolIdsByTurn.has(turnId)) {
      if ((turn.tool_total ?? 0) > 0) {
        return false;
      }
      entry.toolIdsByTurn.set(turnId, new Set());
    }
    const turnToolIds = entry.toolIdsByTurn.get(turnId)!;
    const isNew = !turnToolIds.has(toolCallId);
    if (isNew) {
      turnToolIds.add(toolCallId);
      turn.tool_total = (turn.tool_total ?? 0) + 1;
    }

    const prevBucket = toolStatusBucket(prevStatus);
    const nextBucket = toolStatusBucket(nextStatus);
    if (prevBucket !== nextBucket) {
      applyToolBucketDelta(turn, prevBucket, -1);
      applyToolBucketDelta(turn, nextBucket, 1);
    }
    entry.toolStatusByKey.set(key, nextStatus);
    return true;
  }

  async loadMoreTurns(sessionId: string) {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    if (entry.fetching.turns || !entry.hasMoreTurns) return;
    const beforeSeq = entry.oldestTurnSeq;
    if (!beforeSeq || !Number.isFinite(beforeSeq)) {
      entry.hasMoreTurns = false;
      this.publish();
      return;
    }
    entry.fetching.turns = true;
    try {
      const turns = await listSessionTurnsPage(sessionId, beforeSeq, TURN_PAGE_LIMIT);
      this.mergeTurns(entry, turns);
      if (turns.length < TURN_PAGE_LIMIT) {
        entry.hasMoreTurns = false;
      }
      entry.updatedAtMs = Date.now();
      this.publish();
    } finally {
      entry.fetching.turns = false;
    }
  }

  async loadTurnTools(sessionId: string, turnId: string) {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    if (entry.turnToolsByTurnId[turnId]) return;
    if (entry.turnToolsLoadingSet.has(turnId)) return;
    entry.turnToolsLoadingSet.add(turnId);
    entry.turnToolsLoading = [...entry.turnToolsLoadingSet];
    this.publish();
    try {
      const tools = await listTurnTools(sessionId, turnId);
      entry.turnToolsByTurnId = {
        ...entry.turnToolsByTurnId,
        [turnId]: tools,
      };
    } finally {
      entry.turnToolsLoadingSet.delete(turnId);
      entry.turnToolsLoading = [...entry.turnToolsLoadingSet];
      entry.updatedAtMs = Date.now();
      this.publish();
    }
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
            const event = data as SessionEvent;
            this.upsertEvents(entry, [event]);
            const appliedTurn = this.applyEventToTurns(entry, event);
            entry.updatedAtMs = Date.now();
            // Keep hot sessions warm when they are producing events.
            entry.warmUntilMs = Date.now() + WARM_TTL_MS;

            if (
              event.event_type === "user_message" ||
              event.event_type === "assistant_complete" ||
              this.isRefreshBoundaryEventType(event.event_type)
            ) {
              this.refreshTurns(entry).catch(() => {});
            }

            if (this.isRefreshBoundaryEventType(event.event_type)) {
              this.refreshQueueAndDiff(entry).catch(() => {});
            } else if (this.isMidTurn(entry) && this.shouldPollDiff(entry)) {
              this.ensureDiffPolling();
              // Diff is computed from git and can safely be refreshed mid-turn.
              this.refreshDiff(entry).catch(() => {});
            }
            if (appliedTurn) {
              entry.updatedAtMs = Date.now();
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

    const sessions = ids
      .map((sid) => {
        const entry = this.entries.get(sid);
        if (!entry) return null;
        return { session_id: sid, after_seq: entry.lastEventSeq ?? 0 };
      })
      .filter(Boolean);
    try {
      ws.send(JSON.stringify({ type: "set", sessions }));
    } catch {
      // If send fails synchronously (e.g. socket transitioning), fall back to polling until reconnect.
      this.snapshot = { ...this.snapshot, connection: "disconnected" };
      this.publish();
      try {
        ws.close();
      } catch {
        // ignore
      }
      return;
    }

    // Backfill deltas for subscribed sessions.
    for (const sid of ids) {
      this.backfillSession(sid).catch(() => {});
    }
  }

  private async backfillSession(sessionId: string) {
    const entry = this.entries.get(sessionId);
    if (!entry) return;
    try {
      const afterSeq = entry.lastEventSeq;
      const evs = await listSessionEventsPage(sessionId, afterSeq, 500);
      const sawRefreshBoundary = evs.some((e) => this.isRefreshBoundaryEventType(e.event_type));
      const sawTurnRefresh = evs.some(
        (e) =>
          e.event_type === "user_message" ||
          e.event_type === "assistant_complete" ||
          this.isRefreshBoundaryEventType(e.event_type),
      );
      this.upsertEvents(entry, evs);
      for (const ev of evs) {
        this.applyEventToTurns(entry, ev);
      }
      entry.updatedAtMs = Date.now();
      this.publish();
      // When disconnected (polling mode), we won't get the WS-triggered refresh that keeps Messages in sync.
      // Refreshing here ensures assistant replies show up after daemon/webapp restarts.
      if (sawTurnRefresh) {
        await this.refreshTurns(entry);
      }
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
    await this.refreshTurns(entry);
    await this.refreshDiff(entry);
    entry.updatedAtMs = Date.now();
    this.publish();
  }

  private async refreshTurns(entry: InternalEntry) {
    if (entry.fetching.turns) return;
    entry.fetching.turns = true;
    try {
      const hadTurns = entry.turns.length > 0;
      const turns = await listSessionTurnsPage(entry.sessionId, undefined, TURN_PAGE_LIMIT);
      this.mergeTurns(entry, turns);
      if (!hadTurns) {
        entry.hasMoreTurns = turns.length >= TURN_PAGE_LIMIT;
      }
      entry.updatedAtMs = Date.now();
      this.publish();
    } finally {
      entry.fetching.turns = false;
    }
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
        const visible = entry.refCount > 0;
        // When connected, poll all *visible* warm sessions to guarantee we recover from missed WS events
        // (e.g. broadcaster lag / dropped frames). Non-visible warm sessions still poll only mid-turn to keep load down.
        if (this.snapshot.connection === "connected" && doneLike && !visible) continue;
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

const mergeStreamingText = (prev?: string | null, next?: string | null) => {
  const p = prev ?? "";
  const n = next ?? "";
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
};

const appendFragment = (prev?: string | null, fragment?: string | null) => {
  const p = prev ?? "";
  const f = fragment ?? "";
  if (!p) return f;
  if (!f) return p;
  if (f.startsWith(p)) return f;
  if (p.endsWith(f)) return p;
  return `${p}${f}`;
};

const extractToolCallId = (event: SessionEvent): string | null => {
  const payload = event.payload_json ?? {};
  const direct = payload.tool_call_id ?? payload.toolCallId ?? null;
  if (typeof direct === "string" && direct.trim()) return String(direct);
  const update = payload.acp_update ?? payload;
  const fromUpdate =
    update?.toolCallId ??
    update?.tool_call_id ??
    update?.rawInput?.call_id ??
    update?.raw_input?.call_id ??
    update?.toolCall?.rawInput?.call_id ??
    null;
  if (typeof fromUpdate === "string" && fromUpdate.trim()) return String(fromUpdate);
  return null;
};

const normalizeToolStatus = (status: string, eventType: string) => {
  const s = String(status ?? "").trim().toLowerCase();
  if (s === "inprogress" || s === "in_progress" || s === "running") return "in_progress";
  if (s === "pending" || s === "queued") return "pending";
  if (s === "completed" || s === "complete" || s === "ok" || s === "succeeded") return "completed";
  if (s === "failed" || s === "error") return "failed";
  if (eventType === "tool_result") return "completed";
  return s || "pending";
};

const extractToolStatus = (event: SessionEvent): string | null => {
  const payload = event.payload_json ?? {};
  const update = payload.acp_update ?? payload;
  const raw =
    update?.status ??
    update?.toolCall?.status ??
    null;
  if (typeof raw === "string" && raw.trim()) {
    return normalizeToolStatus(raw, String(event.event_type ?? ""));
  }
  if (event.event_type === "tool_result") return "completed";
  if (event.event_type === "tool_call") return "pending";
  return null;
};

const toolStatusBucket = (status?: string | null) => {
  const s = String(status ?? "").toLowerCase();
  if (s === "pending" || s === "queued") return "pending";
  if (s === "in_progress" || s === "inprogress" || s === "running") return "in_progress";
  if (s === "completed" || s === "complete" || s === "ok" || s === "succeeded") return "completed";
  if (s === "failed" || s === "error") return "failed";
  if (!s) return "pending";
  return "pending";
};

const applyToolBucketDelta = (turn: SessionTurn, bucket: string | null, delta: number) => {
  if (!bucket || delta === 0) return;
  switch (bucket) {
    case "pending":
      turn.tool_pending = Math.max(0, (turn.tool_pending ?? 0) + delta);
      break;
    case "in_progress":
      turn.tool_running = Math.max(0, (turn.tool_running ?? 0) + delta);
      break;
    case "completed":
      turn.tool_completed = Math.max(0, (turn.tool_completed ?? 0) + delta);
      break;
    case "failed":
      turn.tool_failed = Math.max(0, (turn.tool_failed ?? 0) + delta);
      break;
    default:
      break;
  }
};

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
