import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import {
  getProviderOptions,
  getSessionHead,
  getSessionHistory,
  idToString,
  listSessionArtifacts,
  listTurnTools,
  trackDiff,
  type Artifact,
  type Message,
  type ProviderOptions,
  type Session,
  type SessionEvent,
  type SessionTurn,
  type SessionTurnTool,
  type SessionTurnToolSummary,
  type WorkspaceCatchupEvent,
} from "../api/client";
import type { WorkspaceCatchupEventSource } from "./workspaceCatchupStore";
import { loadSessionAcpMetaV1, loadSessionHeadV1, saveSessionAcpMetaV1, saveSessionHeadV1 } from "./uiStateStore";

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

type ConnectionStatus = "connecting" | "connected" | "disconnected" | "idle";

export type SessionSupervisorSnapshot = {
  connection: ConnectionStatus;
  sessions: Record<string, SessionCacheEntry>;
};

export type SessionCacheEntry = {
  sessionId: string;
  session?: Session;
  acpModels?: any;
  acpModes?: any;
  acpCurrentModelId?: string;
  turns: SessionTurn[];
  turnToolsByTurnId: Record<string, SessionTurnTool[]>;
  turnToolsLoading: string[];
  toolSummariesReady: boolean;
  hasMoreTurns: boolean;
  events: SessionEvent[];
  messages: Message[];
  artifacts: Artifact[];
  artifactsLoading: boolean;
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
  force?: boolean;
  silent?: boolean;
};

type AcpMeta = {
  models?: any;
  modes?: any;
  currentModelId?: string;
};

const readAcpCurrentModelId = (models: any): string | undefined => {
  if (!models || typeof models !== "object") return;
  return models.currentModelId ?? models.current_model_id ?? undefined;
};

const hasModelList = (models: any): boolean => {
  const list =
    models?.availableModels ??
    models?.available_models ??
    models?.models ??
    [];
  return Array.isArray(list) && list.length > 0;
};

const extractAcpMetaFromEvent = (event: SessionEvent): AcpMeta | null => {
  if (event.event_type !== "init") return null;
  const payload = (event as any)?.payload_json ?? {};
  const models = payload?.models ?? undefined;
  const modes = payload?.modes ?? undefined;
  if (!models && !modes) return null;
  return {
    models,
    modes,
    currentModelId: readAcpCurrentModelId(models),
  };
};

type InternalEntry = SessionCacheEntry & {
  refCount: number;
  wantDiffCount: number;
  warmUntilMs: number;
  acpMetaUpdatedAtMs?: number;
  seqSet: Set<number>;
  turnsHydrated: boolean;
  oldestTurnSeq?: number;
  diffFetchedAtMs?: number;
  toolStatusByKey: Map<string, string>;
  toolIdsByTurn: Map<string, Set<string>>;
  turnToolsLoadingSet: Set<string>;
  turnToolsHydratedByTurnId: Record<string, boolean>;
  artifactsLoaded: boolean;
  artifactsFetchedAtMs?: number;
  trackId?: string;
  diagnosticsByPath: Record<string, any[]>;
  loadedFromCache: boolean;
  headFromCache: boolean;
  fetching: {
    head: boolean;
    history: boolean;
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
const HEAD_LIMIT = readTunableInt("contextSessionHeadLimit", TURN_PAGE_LIMIT);
const SUBSCRIBE_LIMIT = readTunableInt("contextSubscribeLimit", 20);
const DIFF_REFRESH_MS = readTunableInt("contextDiffRefreshMs", 30 * 1000);

export class SessionSupervisor {
  private listeners = new Set<() => void>();
  private snapshot: SessionSupervisorSnapshot = { connection: "idle", sessions: {} };
  private entries = new Map<string, InternalEntry>();
  private catchupStore: WorkspaceCatchupEventSource | null = null;
  private catchupUnsub: (() => void) | null = null;
  private catchupSnapshotUnsub: (() => void) | null = null;
  private activeTaskSessionIds: string[] = [];
  private warmSessionIds: string[] = [];
  private subscribedSessionIds: string[] = [];
  private providerOptionsCache = new Map<string, ProviderOptions>();
  private providerOptionsInFlight = new Map<string, Promise<ProviderOptions | undefined>>();

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): SessionSupervisorSnapshot => this.snapshot;

  bindWorkspaceCatchupStore(store: WorkspaceCatchupEventSource | null) {
    if (this.catchupUnsub) {
      this.catchupUnsub();
      this.catchupUnsub = null;
    }
    if (this.catchupSnapshotUnsub) {
      this.catchupSnapshotUnsub();
      this.catchupSnapshotUnsub = null;
    }
    this.catchupStore = store;
    if (!store) {
      this.setConnection("disconnected");
      return;
    }
    this.catchupUnsub = store.subscribeEvents((evt) => this.handleCatchupEvent(evt));
    this.catchupSnapshotUnsub = store.subscribe(() => {
      const state = store.getSnapshot();
      const next = this.mapConnection(state.connection);
      this.setConnection(next);
    });
    this.setConnection(this.mapConnection(store.getSnapshot().connection));
    this.refreshSubscriptions();
  }

  openSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.ensureEntry(sessionId);
    entry.refCount += 1;
    if (opts?.watchDiff) entry.wantDiffCount += 1;
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    this.ensureLoaded(sessionId, opts).catch(() => {});
    this.refreshSubscriptions();
    return () => this.closeSession(sessionId, opts);
  };

  closeSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.entries.get(sessionId);
    if (!entry) return;
    entry.refCount = Math.max(0, entry.refCount - 1);
    if (opts?.watchDiff) entry.wantDiffCount = Math.max(0, entry.wantDiffCount - 1);
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    this.refreshSubscriptions();
    this.publish();
  };

  refreshSession = (sessionId: string, opts?: OpenOptions) => {
    this.ensureLoaded(sessionId, { ...opts, force: true }).catch(() => {});
  };

  refreshQueue = (sessionId: string) => {
    this.ensureLoaded(sessionId, { force: true }).catch(() => {});
  };

  setActiveTaskSessionIds = (sessionIds: string[]) => {
    const next = dedupeIds(sessionIds);
    if (sameIdList(next, this.activeTaskSessionIds)) return;
    this.activeTaskSessionIds = next;
    this.refreshSubscriptions();
  };

  setWarmSessionIds = (sessionIds: string[]) => {
    const next = dedupeIds(sessionIds);
    if (sameIdList(next, this.warmSessionIds)) return;
    this.warmSessionIds = next;
    this.refreshSubscriptions();
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
    entry.diffFetchedAtMs = Date.now();
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  async loadMoreTurns(sessionId: string) {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    if (entry.fetching.history || !entry.hasMoreTurns) return;
    const beforeSeq = entry.oldestTurnSeq;
    if (beforeSeq == null || !Number.isFinite(beforeSeq)) {
      entry.hasMoreTurns = false;
      this.publish();
      return;
    }
    entry.fetching.history = true;
    try {
      const page = await getSessionHistory(sessionId, beforeSeq, TURN_PAGE_LIMIT);
      this.mergeTurns(entry, page.turns);
      this.mergeMessages(entry, page.messages);
      entry.hasMoreTurns = page.has_more;
      entry.oldestTurnSeq = page.next_cursor ?? entry.oldestTurnSeq;
      entry.updatedAtMs = Date.now();
      this.publish();
      await this.persistHead(entry);
    } finally {
      entry.fetching.history = false;
    }
  }

  async loadTurnTools(sessionId: string, turnId: string) {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    if (entry.turnToolsHydratedByTurnId?.[turnId]) return;
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
      entry.turnToolsHydratedByTurnId[turnId] = true;
    } finally {
      entry.turnToolsLoadingSet.delete(turnId);
      entry.turnToolsLoading = [...entry.turnToolsLoadingSet];
      entry.updatedAtMs = Date.now();
      this.publish();
    }
  }

  private mapConnection(connection: "idle" | "connecting" | "connected" | "disconnected"): ConnectionStatus {
    if (connection === "connected") return "connected";
    if (connection === "disconnected") return "disconnected";
    if (connection === "connecting") return "connecting";
    return "idle";
  }

  private setConnection(next: ConnectionStatus) {
    const prev = this.snapshot.connection;
    if (prev === next) return;
    this.snapshot = { ...this.snapshot, connection: next };
    if (next === "connected" && prev !== "connected") {
      this.refreshSubscribedHeads();
    }
    for (const l of this.listeners) l();
  }

  private publish() {
    this.evictIfNeeded();
    const sessions: Record<string, SessionCacheEntry> = {};
    for (const [id, e] of this.entries) {
      sessions[id] = {
        sessionId: e.sessionId,
        session: e.session,
        acpModels: e.acpModels,
        acpModes: e.acpModes,
        acpCurrentModelId: e.acpCurrentModelId,
        turns: e.turns,
        turnToolsByTurnId: e.turnToolsByTurnId,
        turnToolsLoading: [...e.turnToolsLoadingSet],
        toolSummariesReady: e.toolSummariesReady,
        hasMoreTurns: e.hasMoreTurns,
        events: e.events,
        messages: e.messages,
        artifacts: e.artifacts,
        artifactsLoading: e.artifactsLoading,
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
    const existing = this.entries.get(sessionId);
    if (existing) return existing;
    const entry: InternalEntry = {
      sessionId,
      session: undefined,
      acpModels: undefined,
      acpModes: undefined,
      acpCurrentModelId: undefined,
      turns: [],
      turnToolsByTurnId: {},
      turnToolsLoading: [],
      toolSummariesReady: false,
      hasMoreTurns: true,
      events: [],
      messages: [],
      artifacts: [],
      artifactsLoading: false,
      queue: [],
      diff: undefined,
      diagnosticsByPath: {},
      lastEventSeq: undefined,
      loading: false,
      error: undefined,
      subscribed: false,
      updatedAtMs: Date.now(),
      refCount: 0,
      wantDiffCount: 0,
      warmUntilMs: Date.now() + WARM_TTL_MS,
      acpMetaUpdatedAtMs: undefined,
      seqSet: new Set<number>(),
      turnsHydrated: false,
      oldestTurnSeq: undefined,
      diffFetchedAtMs: undefined,
      toolStatusByKey: new Map(),
      toolIdsByTurn: new Map(),
      turnToolsLoadingSet: new Set(),
      turnToolsHydratedByTurnId: {},
      artifactsLoaded: false,
      artifactsFetchedAtMs: undefined,
      trackId: undefined,
      loadedFromCache: false,
      headFromCache: false,
      fetching: {
        head: false,
        history: false,
        diff: false,
      },
    };
    this.entries.set(sessionId, entry);
    return entry;
  }

  private applyAcpMeta(entry: InternalEntry, meta: AcpMeta, opts?: { persist?: boolean }): boolean {
    const nextModels = meta.models ?? entry.acpModels;
    const nextModes = meta.modes ?? entry.acpModes;
    const nextCurrent =
      meta.currentModelId ?? readAcpCurrentModelId(nextModels) ?? entry.acpCurrentModelId;
    const modelsChanged = JSON.stringify(nextModels ?? null) !== JSON.stringify(entry.acpModels ?? null);
    const modesChanged = JSON.stringify(nextModes ?? null) !== JSON.stringify(entry.acpModes ?? null);
    const currentChanged = nextCurrent !== entry.acpCurrentModelId;
    if (!modelsChanged && !modesChanged && !currentChanged) return false;

    entry.acpModels = nextModels;
    entry.acpModes = nextModes;
    entry.acpCurrentModelId = nextCurrent;
    entry.acpMetaUpdatedAtMs = Date.now();
    if (opts?.persist !== false && (nextModels || nextModes || nextCurrent)) {
      saveSessionAcpMetaV1(entry.sessionId, {
        models: nextModels,
        modes: nextModes,
        currentModelId: nextCurrent,
      }).catch(() => {});
    }
    return true;
  }

  private applyAcpMetaFromEvents(entry: InternalEntry, events: SessionEvent[]): boolean {
    for (let i = events.length - 1; i >= 0; i -= 1) {
      const meta = extractAcpMetaFromEvent(events[i]);
      if (meta) {
        return this.applyAcpMeta(entry, meta);
      }
    }
    return false;
  }

  private providerOptionsKey(session: Session): string {
    return `${idToString(session.workspace_id)}:${session.provider_id}`;
  }

  private seedAcpMetaFromProviderOptions(entry: InternalEntry, opts?: ProviderOptions): boolean {
    if (!opts?.models && !opts?.modes) return false;
    return this.applyAcpMeta(entry, {
      models: opts.models,
      modes: opts.modes,
      currentModelId: readAcpCurrentModelId(opts.models),
    });
  }

  private async ensureProviderOptions(entry: InternalEntry) {
    if (entry.acpModels && hasModelList(entry.acpModels)) return;
    const session = entry.session;
    if (!session) return;
    const key = this.providerOptionsKey(session);
    const cached = this.providerOptionsCache.get(key);
    if (cached) {
      if (this.seedAcpMetaFromProviderOptions(entry, cached)) {
        entry.updatedAtMs = Date.now();
        this.publish();
      }
      return;
    }
    const existing = this.providerOptionsInFlight.get(key);
    if (existing) {
      const opts = await existing.catch(() => undefined);
      if (opts && this.seedAcpMetaFromProviderOptions(entry, opts)) {
        entry.updatedAtMs = Date.now();
        this.publish();
      }
      return;
    }
    const workspaceId = idToString(session.workspace_id);
    if (!workspaceId) return;
    const request = getProviderOptions(workspaceId, session.provider_id)
      .then((opts) => {
        this.providerOptionsCache.set(key, opts);
        return opts;
      })
      .catch(() => undefined)
      .finally(() => {
        if (this.providerOptionsInFlight.get(key) === request) {
          this.providerOptionsInFlight.delete(key);
        }
      });
    this.providerOptionsInFlight.set(key, request);
    const opts = await request;
    if (opts && this.seedAcpMetaFromProviderOptions(entry, opts)) {
      entry.updatedAtMs = Date.now();
      this.publish();
    }
  }

  private async ensureLoaded(sessionId: string, opts?: OpenOptions) {
    const entry = this.ensureEntry(sessionId);
    if (!entry.loadedFromCache) {
      entry.loadedFromCache = true;
      await this.loadCachedHead(entry);
    }
    if (entry.fetching.head) return;
    if (entry.turnsHydrated && !opts?.force && !entry.headFromCache) {
      if (opts?.watchDiff) {
        void this.refreshDiff(entry);
      }
      return;
    }
    entry.fetching.head = true;
    if (!opts?.silent) {
      entry.loading = true;
      this.publish();
    }
    try {
      const head = await getSessionHead(sessionId, HEAD_LIMIT, true);
      this.applyHead(entry, head);
      await this.persistHead(entry);
      await this.ensureArtifacts(entry);
    } catch (e: any) {
      if (!opts?.silent) {
        entry.error = e?.message ?? "Failed to load session";
      }
    } finally {
      if (!opts?.silent) {
        entry.loading = false;
      }
      entry.fetching.head = false;
      entry.updatedAtMs = Date.now();
      this.publish();
    }
    if (opts?.watchDiff) {
      void this.refreshDiff(entry);
    }
  }

  private async ensureArtifacts(entry: InternalEntry, opts?: { force?: boolean }) {
    if (entry.artifactsLoading) return;
    if (entry.artifactsLoaded && !opts?.force) return;
    entry.artifactsLoading = true;
    entry.updatedAtMs = Date.now();
    this.publish();
    try {
      const artifacts = await listSessionArtifacts(entry.sessionId);
      entry.artifacts = artifacts;
      entry.artifactsLoaded = true;
      entry.artifactsFetchedAtMs = Date.now();
    } catch {
      // ignore artifact load errors (missing session or daemon offline)
    } finally {
      entry.artifactsLoading = false;
      entry.updatedAtMs = Date.now();
      this.publish();
    }
  }

  private async loadCachedHead(entry: InternalEntry) {
    try {
      const cached = await loadSessionHeadV1(entry.sessionId);
      if (!cached?.head) return;
      this.applyHead(entry, cached.head, { fromCache: true });
      const cachedMeta = await loadSessionAcpMetaV1(entry.sessionId);
      if (cachedMeta) {
        const changed = this.applyAcpMeta(
          entry,
          {
            models: cachedMeta.models,
            modes: cachedMeta.modes,
            currentModelId: cachedMeta.currentModelId,
          },
          { persist: false },
        );
        if (changed) {
          entry.updatedAtMs = Date.now();
          this.publish();
        }
      }
    } catch {
      // ignore cache errors
    }
  }

  private applyHead(
    entry: InternalEntry,
    head: {
      session: Session;
      turns: SessionTurn[];
      tool_summaries?: SessionTurnToolSummary[];
      events?: SessionEvent[];
      messages: Message[];
      last_event_seq: number;
      has_more_turns: boolean;
    },
    opts?: { fromCache?: boolean },
  ) {
    entry.headFromCache = Boolean(opts?.fromCache);
    entry.session = head.session;
    entry.trackId = idToString(head.session.track_id);
    entry.turnsHydrated = true;
    entry.hasMoreTurns = head.has_more_turns;
    entry.lastEventSeq = head.last_event_seq;
    this.mergeTurns(entry, head.turns ?? []);
    this.mergeEvents(entry, head.events ?? []);
    this.mergeMessages(entry, head.messages ?? []);
    this.applyAcpMetaFromEvents(entry, head.events ?? []);
    if (!entry.acpModels || !hasModelList(entry.acpModels)) {
      void this.ensureProviderOptions(entry);
    }
    if (head.tool_summaries && head.tool_summaries.length > 0) {
      const hydrated = entry.turnToolsHydratedByTurnId;
      const nextByTurn: Record<string, SessionTurnTool[]> = {};
      for (const summary of head.tool_summaries) {
        const turnId = idToString(summary.turn_id);
        if (!turnId) continue;
        if (hydrated[turnId]) continue;
        const list = nextByTurn[turnId] ?? [];
        list.push({
          session_id: summary.session_id,
          tool_call_id: summary.tool_call_id,
          turn_id: summary.turn_id,
          tool_kind: summary.tool_kind,
          title: summary.title,
          status: summary.status,
          input_json: summary.input_preview ?? null,
          output_text: null,
          created_at: summary.created_at,
          updated_at: summary.updated_at,
          summary_only: true,
        } as SessionTurnTool & { summary_only: boolean });
        nextByTurn[turnId] = list;
      }
      if (Object.keys(nextByTurn).length > 0) {
        entry.turnToolsByTurnId = {
          ...entry.turnToolsByTurnId,
          ...nextByTurn,
        };
        for (const turnId of Object.keys(nextByTurn)) {
          if (!hydrated[turnId]) hydrated[turnId] = false;
        }
      }
    }
    entry.toolSummariesReady = true;
    if (!opts?.fromCache) {
      entry.error = undefined;
    }
    entry.updatedAtMs = Date.now();
    this.publish();
  }

  private async refreshDiff(entry: InternalEntry, force = false) {
    if (entry.fetching.diff) return;
    if (!entry.trackId) return;
    if (entry.wantDiffCount <= 0) return;
    if (!force && entry.diff !== undefined && entry.diffFetchedAtMs) {
      const ageMs = Date.now() - entry.diffFetchedAtMs;
      if (ageMs < DIFF_REFRESH_MS) return;
    }
    entry.fetching.diff = true;
    try {
      const resp = await trackDiff(entry.trackId);
      entry.diff = resp.diff ?? "";
      entry.diffFetchedAtMs = Date.now();
      entry.updatedAtMs = Date.now();
      this.publish();
    } finally {
      entry.fetching.diff = false;
    }
  }

  private async persistHead(entry: InternalEntry) {
    if (!entry.session) return;
    const tool_summaries = buildToolSummaries(entry.turnToolsByTurnId);
    const head = {
      session: entry.session,
      turns: entry.turns,
      events: entry.events,
      messages: entry.messages,
      tool_summaries,
      last_event_seq: entry.lastEventSeq ?? 0,
      has_more_turns: entry.hasMoreTurns,
    };
    await saveSessionHeadV1(entry.sessionId, head);
  }

  private refreshSubscriptions() {
    const openIds = Array.from(this.entries.values())
      .filter((entry) => entry.refCount > 0)
      .map((entry) => entry.sessionId);
    const combined = mergeOrderedIds(openIds, this.activeTaskSessionIds, this.warmSessionIds);
    const next = combined.slice(0, SUBSCRIBE_LIMIT);
    const key = next.join("|");
    const prev = this.subscribedSessionIds.join("|");
    if (key === prev) return;
    const nextSet = new Set(next);
    const prevSet = new Set(this.subscribedSessionIds);
    this.subscribedSessionIds = next;
    for (const entry of this.entries.values()) {
      entry.subscribed = nextSet.has(entry.sessionId);
    }
    for (const sessionId of next) {
      if (!prevSet.has(sessionId)) {
        this.ensureLoaded(sessionId, { silent: true }).catch(() => {});
      }
    }
    if (this.catchupStore) {
      const subs = next.map((sessionId) => {
        const entry = this.entries.get(sessionId);
        const afterSeq = entry?.lastEventSeq;
        return {
          session_id: sessionId,
          ...(typeof afterSeq === "number" ? { after_seq: afterSeq } : {}),
        };
      });
      this.catchupStore.setSubscriptions(subs);
    }
    this.publish();
  }

  private refreshSubscribedHeads() {
    for (const sessionId of this.subscribedSessionIds) {
      this.ensureLoaded(sessionId, { force: true, silent: true }).catch(() => {});
    }
  }

  private mergeTurns(entry: InternalEntry, incoming: SessionTurn[]) {
    if (incoming.length === 0) return;
    const byId = new Map<string, SessionTurn>();
    for (const t of entry.turns) {
      const id = idToString(t.turn_id);
      if (id) byId.set(id, t);
    }
    for (const t of incoming) {
      const id = idToString(t.turn_id);
      if (!id) continue;
      const prev = byId.get(id);
      byId.set(id, prev ? mergeTurn(prev, t) : t);
    }
    const next = Array.from(byId.values()).sort(this.compareTurnOrder.bind(this));
    entry.turns = next;
    entry.oldestTurnSeq = next[0]?.start_seq ?? entry.oldestTurnSeq;
  }

  private mergeMessages(entry: InternalEntry, incoming: Message[]) {
    if (incoming.length === 0) return;
    const byId = new Map<string, Message>();
    for (const m of entry.messages) {
      const id = idToString(m.id);
      if (id) byId.set(id, m);
    }
    for (const m of incoming) {
      const id = idToString(m.id);
      if (!id) continue;
      byId.set(id, m);
    }
    const next = Array.from(byId.values()).sort((a, b) => {
      const c = String(a.created_at).localeCompare(String(b.created_at));
      if (c !== 0) return c;
      const sa = Number(a.turn_sequence ?? Number.NaN);
      const sb = Number(b.turn_sequence ?? Number.NaN);
      if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
      if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
      if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
      return String(idToString(a.id)).localeCompare(String(idToString(b.id)));
    });
    entry.messages = next;
    entry.queue = next.filter((m) => m.delivery === "queued");
  }

  private mergeEvents(entry: InternalEntry, incoming: SessionEvent[]) {
    if (incoming.length === 0) return;
    const bySeq = new Map<number, SessionEvent>();
    for (const ev of entry.events) {
      if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
    }
    for (const ev of incoming) {
      if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
    }
    const next = Array.from(bySeq.values()).sort((a, b) => a.seq - b.seq);
    const trimmed = next.length > EVENT_BUFFER_LIMIT ? next.slice(-EVENT_BUFFER_LIMIT) : next;
    entry.events = trimmed;
    entry.seqSet = new Set(trimmed.map((ev) => ev.seq));
  }

  private compareTurnOrder(a: SessionTurn, b: SessionTurn): number {
    const sa = Number(a.start_seq ?? Number.NaN);
    const sb = Number(b.start_seq ?? Number.NaN);
    if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) {
      return sa - sb;
    }
    return String(a.started_at).localeCompare(String(b.started_at));
  }

  private ensureTurnFromEvent(entry: InternalEntry, event: SessionEvent): SessionTurn | null {
    const turnId = idToString(event.turn_id);
    if (!turnId) return null;
    const existing = entry.turns.find((t) => idToString(t.turn_id) === turnId);
    if (existing) return existing;
    const createdAt = event.created_at ?? new Date().toISOString();
    const status = deriveTurnStatusFromEvent(String(event.event_type ?? ""));
    const turn: SessionTurn = {
      turn_id: event.turn_id ?? turnId,
      session_id: event.session_id,
      run_id: event.run_id ?? null,
      user_message_id: event.payload_json?.user_message_id ?? null,
      status,
      start_seq: event.seq ?? null,
      end_seq: null,
      started_at: createdAt,
      updated_at: createdAt,
      assistant_partial: "",
      thought_partial: "",
      metrics_json: null,
      tool_total: 0,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 0,
      tool_failed: 0,
    };
    entry.turns = [...entry.turns, turn].sort(this.compareTurnOrder.bind(this));
    entry.oldestTurnSeq = entry.turns[0]?.start_seq ?? entry.oldestTurnSeq;
    return turn;
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
        if (!shouldRenderThoughtChunk(event)) break;
        const fragment = String(event.payload_json?.content_fragment ?? "");
        if (fragment) {
          turn.thought_partial = appendFragment(turn.thought_partial, fragment);
          changed = true;
        }
        break;
      }
      case "assistant_message_inserted": {
        turn.assistant_partial = "";
        changed = true;
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

    if (event.payload_json?.context_window) {
      turn.metrics_json = event.payload_json.context_window;
      changed = true;
    }

    if (!changed) return false;
    turn.updated_at = event.created_at ?? turn.updated_at;
    entry.turns[idx] = { ...turn };
    return true;
  }

  private applyArtifactsEvent(entry: InternalEntry, event: SessionEvent): boolean {
    if (String(event.event_type) !== "artifacts_set") return false;
    const artifacts = Array.isArray(event.payload_json?.artifacts)
      ? (event.payload_json.artifacts as Artifact[])
      : [];
    entry.artifacts = artifacts;
    entry.artifactsLoaded = true;
    entry.artifactsFetchedAtMs = Date.now();
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

  private handleCatchupEvent(evt: WorkspaceCatchupEvent) {
    if (evt.type === "track_upsert") {
      const trackId = idToString(evt.track.track.id);
      if (!trackId) return;
      for (const entry of this.entries.values()) {
        if (entry.trackId !== trackId) continue;
        if (entry.wantDiffCount <= 0) continue;
        void this.refreshDiff(entry, true);
      }
      return;
    }
    if (evt.type === "session_gap") {
      const sid = idToString(evt.session_id);
      if (!sid) return;
      const entry = this.entries.get(sid);
      if (!entry) return;
      entry.lastEventSeq = evt.after_seq;
      this.resetEntryForGap(entry);
      this.ensureLoaded(sid, { force: true, silent: true }).catch(() => {});
      return;
    }
    if (evt.type !== "session_head_delta") return;
    const delta = evt.delta;
    const sid = idToString(delta.session_id);
    if (!sid) return;
    const entry = this.entries.get(sid);
    if (!entry) return;

    let changed = false;
    if (typeof delta.last_event_seq === "number") {
      if (!entry.lastEventSeq || delta.last_event_seq > entry.lastEventSeq) {
        entry.lastEventSeq = delta.last_event_seq;
        changed = true;
      }
    }

    if (delta.turn) {
      this.mergeTurns(entry, [delta.turn]);
      changed = true;
    }

    if (delta.message) {
      this.mergeMessages(entry, [delta.message]);
      changed = true;
    }

    if (delta.event) {
      if (!entry.seqSet.has(delta.event.seq)) {
        entry.seqSet.add(delta.event.seq);
        entry.events.push(delta.event);
        if (entry.events.length > EVENT_BUFFER_LIMIT) {
          const overflow = entry.events.length - EVENT_BUFFER_LIMIT;
          const removed = entry.events.splice(0, overflow);
          removed.forEach((e) => entry.seqSet.delete(e.seq));
        }
        changed = true;
      }
      const meta = extractAcpMetaFromEvent(delta.event);
      if (meta && this.applyAcpMeta(entry, meta)) {
        changed = true;
      }
      this.ensureTurnFromEvent(entry, delta.event);
      if (this.applyEventToTurns(entry, delta.event)) {
        changed = true;
      }
      if (this.applyArtifactsEvent(entry, delta.event)) {
        changed = true;
      }
    }

    if (changed) {
      entry.updatedAtMs = Date.now();
      this.publish();
    }
  }

  private resetEntryForGap(entry: InternalEntry) {
    entry.turns = [];
    entry.events = [];
    entry.messages = [];
    entry.artifacts = [];
    entry.artifactsLoaded = false;
    entry.artifactsLoading = false;
    entry.artifactsFetchedAtMs = undefined;
    entry.queue = [];
    entry.turnToolsByTurnId = {};
    entry.turnToolsHydratedByTurnId = {};
    entry.turnToolsLoadingSet.clear();
    entry.turnToolsLoading = [];
    entry.toolStatusByKey.clear();
    entry.toolIdsByTurn.clear();
    entry.seqSet.clear();
    entry.turnsHydrated = false;
    entry.toolSummariesReady = false;
    entry.hasMoreTurns = true;
    entry.updatedAtMs = Date.now();
    this.publish();
  }
}

const SessionSupervisorContext = createContext<SessionSupervisor | null>(null);

export function SessionSupervisorProvider({ children }: { children: React.ReactNode }) {
  const supRef = useRef<SessionSupervisor | null>(null);
  if (!supRef.current) {
    supRef.current = new SessionSupervisor();
  }

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
  const stableOpts = useMemo(
    () => ({
      watchDiff: opts?.watchDiff ?? false,
      force: opts?.force ?? false,
      silent: opts?.silent ?? false,
    }),
    [opts?.watchDiff, opts?.force, opts?.silent],
  );
  useEffect(() => {
    if (!sessionId) return;
    return sup.openSession(String(sessionId), stableOpts);
  }, [sup, sessionId, stableOpts]);
}

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

const mergePartial = (p: string, n: string): string => {
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
};

const appendFragment = (p: string | null | undefined, f: string | null | undefined): string => {
  if (!p) return f || "";
  if (!f) return p;
  if (f.startsWith(p)) return f;
  if (p.endsWith(f)) return p;
  return `${p}${f}`;
};

const pickFirstString = (...values: any[]): string | null => {
  for (const v of values) {
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return null;
};

const isNonToolStatus = (value: string): boolean => {
  const s = value.trim().toLowerCase();
  return ![
    "pending",
    "queued",
    "running",
    "in_progress",
    "completed",
    "failed",
    "error",
    "ok",
    "success",
    "succeeded",
  ].includes(s);
};

function isStatusUpdateMeta(meta: any): boolean {
  if (!meta || typeof meta !== "object") return false;
  const codexMeta = meta?.codex ?? {};
  const reasoningKind = codexMeta?.reasoning_kind ?? codexMeta?.reasoningKind;
  if (reasoningKind === "status") return true;

  const statusText = pickFirstString(
    meta?.status_text,
    meta?.statusText,
    meta?.status_string,
    meta?.statusString,
    codexMeta?.status_text,
    codexMeta?.statusText,
    codexMeta?.status_string,
    codexMeta?.statusString,
  );
  if (statusText) return true;

  const statusValue =
    typeof meta?.status === "string"
      ? meta.status
      : typeof codexMeta?.status === "string"
        ? codexMeta.status
        : null;
  if (statusValue && isNonToolStatus(statusValue)) return true;

  return false;
}

function shouldRenderThoughtChunk(ev: SessionEvent): boolean {
  const payload = ev.payload_json ?? {};
  const meta =
    payload?.acp_update?._meta ??
    payload?.acp_update?.meta ??
    payload?._meta ??
    payload?.meta ??
    {};
  if (meta?.heartbeat === true) return false;
  if (isStatusUpdateMeta(meta)) return false;
  const reasoningKind = meta?.codex?.reasoning_kind ?? meta?.codex?.reasoningKind;
  if (reasoningKind === "summary") return false;
  return true;
}

const extractToolCallId = (event: SessionEvent): string | null => {
  const payload = event.payload_json ?? {};
  const direct = payload?.tool_call_id ?? payload?.tool_call?.id ?? payload?.tool?.id;
  if (typeof direct === "string" && direct.trim()) return String(direct);
  const fromUpdate = payload?.acp_update?.tool_call_id ?? payload?.acp_update?.tool_call?.id;
  if (typeof fromUpdate === "string" && fromUpdate.trim()) return String(fromUpdate);
  return null;
};

const normalizeToolStatus = (raw: string, eventType: string): string => {
  const s = String(raw ?? "").toLowerCase();
  if (s === "inprogress" || s === "in_progress" || s === "running") return "in_progress";
  if (s === "pending" || s === "queued") return "pending";
  if (s === "completed" || s === "complete" || s === "ok" || s === "succeeded") return "completed";
  if (s === "failed" || s === "error") return "failed";
  if (eventType === "tool_result") return "completed";
  return s || "pending";
};

const extractToolStatus = (event: SessionEvent): string | null => {
  const payload = event.payload_json ?? {};
  const direct = payload?.tool_status ?? payload?.tool?.status ?? payload?.status;
  if (typeof direct === "string" && direct.trim()) return normalizeToolStatus(direct, String(event.event_type ?? ""));
  const fromUpdate = payload?.acp_update?.tool_status ?? payload?.acp_update?.tool?.status;
  if (typeof fromUpdate === "string" && fromUpdate.trim()) return normalizeToolStatus(fromUpdate, String(event.event_type ?? ""));
  if (event.event_type === "tool_result") return "completed";
  if (event.event_type === "tool_call") return "pending";
  return null;
};

const toolStatusBucket = (status?: string | null): string | null => {
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

const deriveTurnStatusFromEvent = (eventType: string): SessionTurn["status"] => {
  switch (eventType) {
    case "done":
    case "assistant_complete":
      return "completed";
    case "turn_interrupted":
      return "interrupted";
    case "error":
      return "failed";
    default:
      return "running";
  }
};

const dedupeIds = (ids: string[]): string[] => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of ids) {
    const id = String(raw || "").trim();
    if (!id || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
};

const mergeOrderedIds = (...groups: string[][]): string[] => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const group of groups) {
    for (const raw of group) {
      const id = String(raw || "").trim();
      if (!id || seen.has(id)) continue;
      seen.add(id);
      out.push(id);
    }
  }
  return out;
};

const TOOL_INPUT_PREVIEW_KEYS = [
  "command",
  "query",
  "pattern",
  "regex",
  "text",
  "path",
  "file",
  "filename",
  "file_path",
  "filePath",
  "filepath",
  "target",
  "paths",
  "paths_total",
  "files",
  "file_paths",
  "filePaths",
  "glob",
  "parsed_cmd",
  "diff_stats",
  "url",
  "uri",
  "href",
  "method",
  "cwd",
  "root",
];

const toolInputPreview = (input: unknown): Record<string, unknown> | null => {
  if (!input || typeof input !== "object" || Array.isArray(input)) return null;
  const obj = input as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const key of TOOL_INPUT_PREVIEW_KEYS) {
    if (obj[key] !== undefined) out[key] = obj[key];
  }
  return Object.keys(out).length > 0 ? out : null;
};

const buildToolSummaries = (byTurn: Record<string, SessionTurnTool[]>): SessionTurnToolSummary[] => {
  const out: SessionTurnToolSummary[] = [];
  for (const tools of Object.values(byTurn)) {
    for (const tool of tools) {
      out.push({
        session_id: tool.session_id,
        tool_call_id: tool.tool_call_id,
        turn_id: tool.turn_id,
        tool_kind: tool.tool_kind ?? null,
        title: tool.title ?? null,
        status: tool.status ?? null,
        input_preview: toolInputPreview(tool.input_json) ?? undefined,
        created_at: tool.created_at,
        updated_at: tool.updated_at,
      });
    }
  }
  return out;
};

const sameIdList = (a: string[], b: string[]): boolean => {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
};
