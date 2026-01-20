import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import {
  getProviderOptions,
  getSessionHistory,
  getSessionSnapshot,
  getSessionState,
  idToString,
  listSessionArtifacts,
  listSessionSubagentInvocations,
  listTurnTools,
  type Artifact,
  type GitStatusSummary,
  type Message,
  type ProviderOptions,
  type Session,
  type SessionEvent,
  type SessionHead,
  type SessionHeadSnapshot,
  type SessionHeadWindow,
  type SessionState,
  type SessionSummaryCheckpoint,
  type SessionTurn,
  type SessionTurnTool,
  type SessionTurnToolSummary,
  type SubagentInvocation,
  type WorkspaceActiveSnapshotEvent,
} from "../api/client";
import type { WorkspaceActiveSnapshotEventSource, WorkspaceActiveSnapshotState } from "./workspaceActiveSnapshotStore";
import {
  loadSessionAcpMetaV1,
  loadSessionHeadV1,
  loadSessionHistoryPageV1,
  saveSessionAcpMetaV1,
  saveSessionHeadV1,
  saveSessionHistoryPageV1,
} from "./uiStateStore";

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
  localMessages: Message[];
  artifacts: Artifact[];
  artifactsLoading: boolean;
  subagentInvocations: SubagentInvocation[];
  subagentInvocationsLoading: boolean;
  stateLoaded: boolean;
  stateLoading: boolean;
  stateRev?: number;
  queue: Message[];
  diff?: string;
  gitStatusSummary?: GitStatusSummary | null;
  summaryCheckpoint?: SessionSummaryCheckpoint | null;
  headWindow?: SessionHeadWindow | null;
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

const readString = (value: unknown): string | undefined => {
  if (typeof value === "string") return value;
  return undefined;
};

const readBool = (value: unknown): boolean | undefined => {
  if (typeof value === "boolean") return value;
  return undefined;
};

const readNumber = (value: unknown): number | undefined => {
  const num = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(num)) return undefined;
  return num;
};

const normalizeGitStatusSummaryInput = (value: unknown, entries?: unknown): Partial<GitStatusSummary> => {
  if (!value || typeof value !== "object") {
    return Array.isArray(entries) ? { entries: entries as GitStatusSummary["entries"] } : {};
  }
  const src = value as Record<string, unknown>;
  const out: Partial<GitStatusSummary> = {};
  const raw = readString(src.raw);
  if (raw !== undefined) out.raw = raw;
  const summaryLine = readString(src.summary_line);
  if (summaryLine !== undefined) out.summary_line = summaryLine;
  const summaryLineAlt = readString(src.summaryLine);
  if (summaryLineAlt !== undefined) out.summaryLine = summaryLineAlt;
  const summary = readString(src.summary);
  if (summary !== undefined) out.summary = summary;
  const status = readString(src.status);
  if (status !== undefined) out.status = status;
  if (Array.isArray(src.lines)) out.lines = src.lines as string[];
  const branch = readString(src.branch);
  if (branch !== undefined) out.branch = branch;
  const upstream = readString(src.upstream);
  if (upstream !== undefined) out.upstream = upstream;
  const ahead = readNumber(src.ahead);
  if (ahead !== undefined) out.ahead = ahead;
  const behind = readNumber(src.behind);
  if (behind !== undefined) out.behind = behind;
  const detached = readBool(src.detached);
  if (detached !== undefined) out.detached = detached;
  const staged = readNumber(src.staged);
  if (staged !== undefined) out.staged = staged;
  const unstaged = readNumber(src.unstaged);
  if (unstaged !== undefined) out.unstaged = unstaged;
  const untracked = readNumber(src.untracked);
  if (untracked !== undefined) out.untracked = untracked;
  if (Array.isArray(src.entries)) out.entries = src.entries as GitStatusSummary["entries"];
  if (Array.isArray(entries)) out.entries = entries as GitStatusSummary["entries"];
  return out;
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
  warmUntilMs: number;
  acpMetaUpdatedAtMs?: number;
  seqSet: Set<number>;
  turnsHydrated: boolean;
  oldestTurnSeq?: number;
  toolStatusByKey: Map<string, string>;
  toolIdsByTurn: Map<string, Set<string>>;
  turnToolsLoadingSet: Set<string>;
  turnToolsHydratedByTurnId: Record<string, boolean>;
  artifactsLoaded: boolean;
  artifactsFetchedAtMs?: number;
  subagentInvocationsLoaded: boolean;
  subagentInvocationsFetchedAtMs?: number;
  stateLoaded: boolean;
  stateLoading: boolean;
  stateRev?: number;
  stateAppliedRev?: number;
  stateFetchToken: number;
  diagnosticsByPath: Record<string, any[]>;
  loadedFromCache: boolean;
  headFromCache: boolean;
  fetching: {
    head: boolean;
    history: boolean;
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

export class SessionSupervisor {
  private listeners = new Set<() => void>();
  private snapshot: SessionSupervisorSnapshot = { connection: "idle", sessions: {} };
  private entries = new Map<string, InternalEntry>();
  private snapshotStore: WorkspaceActiveSnapshotEventSource | null = null;
  private snapshotUnsub: (() => void) | null = null;
  private snapshotStateUnsub: (() => void) | null = null;
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

  bindWorkspaceActiveSnapshotStore(store: WorkspaceActiveSnapshotEventSource | null) {
    if (this.snapshotUnsub) {
      this.snapshotUnsub();
      this.snapshotUnsub = null;
    }
    if (this.snapshotStateUnsub) {
      this.snapshotStateUnsub();
      this.snapshotStateUnsub = null;
    }
    this.snapshotStore = store;
    if (!store) {
      this.setConnection("disconnected");
      return;
    }
    this.snapshotUnsub = store.subscribeEvents((evt) => this.handleWorkspaceEvent(evt));
    this.snapshotStateUnsub = store.subscribe(() => {
      const state = store.getSnapshot();
      const next = this.mapConnection(state.connection);
      this.setConnection(next);
      this.syncActiveSnapshot(state);
    });
    this.setConnection(this.mapConnection(store.getSnapshot().connection));
    this.refreshSubscriptions();
  }

  openSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.ensureEntry(sessionId);
    entry.refCount += 1;
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    this.ensureLoaded(sessionId, opts).catch(() => {});
    this.refreshSubscriptions();
    return () => this.closeSession(sessionId, opts);
  };

  closeSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.entries.get(sessionId);
    if (!entry) return;
    entry.refCount = Math.max(0, entry.refCount - 1);
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
    entry.updatedAtMs = Date.now();
    this.publish();
  };


  setMessages = (sessionId: string, messages: Message[], opts?: { replace?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (opts?.replace) {
      entry.messages = [];
      entry.queue = [];
    }
    this.mergeMessages(entry, messages);
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  setLocalMessages = (sessionId: string, messages: Message[], opts?: { replace?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (opts?.replace) {
      entry.localMessages = [];
    }
    this.mergeLocalMessages(entry, messages);
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  replaceMessage = (sessionId: string, localId: string, message: Message) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    const local = idToString(localId);
    if (local) {
      entry.localMessages = entry.localMessages.filter((m) => idToString(m.id) !== local);
    }
    const next = entry.messages.filter((m) => idToString(m.id) !== local);
    next.push(message);
    entry.messages = [];
    entry.queue = [];
    this.mergeMessages(entry, next);
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  setError = (sessionId: string, error: string | null) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    entry.error = error ?? undefined;
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  replaceSessionId = (oldSessionId: string, newSessionId: string) => {
    const from = String(oldSessionId || "").trim();
    const to = String(newSessionId || "").trim();
    if (!from || !to || from === to) return;
    const entry = this.entries.get(from);
    if (!entry) return;
    this.entries.delete(from);
    entry.sessionId = to;
    if (entry.session) {
      entry.session = { ...entry.session, id: to };
    }
    entry.turns = entry.turns.map((turn) =>
      idToString(turn.session_id) === from ? { ...turn, session_id: to } : turn,
    );
    entry.messages = entry.messages.map((msg) =>
      idToString(msg.session_id) === from ? { ...msg, session_id: to } : msg,
    );
    entry.localMessages = entry.localMessages.map((msg) =>
      idToString(msg.session_id) === from ? { ...msg, session_id: to } : msg,
    );
    entry.queue = entry.queue.map((msg) =>
      idToString(msg.session_id) === from ? { ...msg, session_id: to } : msg,
    );
    this.activeTaskSessionIds = this.activeTaskSessionIds.map((id) => (id === from ? to : id));
    this.warmSessionIds = this.warmSessionIds.map((id) => (id === from ? to : id));
    this.subscribedSessionIds = this.subscribedSessionIds.map((id) => (id === from ? to : id));
    this.entries.set(to, entry);
    this.refreshSubscriptions();
    this.publish();
  };

  replaceSessionTaskId = (sessionId: string, taskId: string) => {
    const id = String(sessionId || "").trim();
    const nextTaskId = String(taskId || "").trim();
    if (!id || !nextTaskId) return;
    const entry = this.entries.get(id);
    if (!entry) return;
    if (entry.session) {
      entry.session = { ...entry.session, task_id: nextTaskId };
    }
    entry.messages = entry.messages.map((msg) => ({ ...msg, task_id: nextTaskId }));
    entry.localMessages = entry.localMessages.map((msg) => ({ ...msg, task_id: nextTaskId }));
    entry.queue = entry.queue.map((msg) => ({ ...msg, task_id: nextTaskId }));
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  dropSessionEntry = (sessionId: string) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    if (!this.entries.has(id)) return;
    this.entries.delete(id);
    this.activeTaskSessionIds = this.activeTaskSessionIds.filter((entryId) => entryId !== id);
    this.warmSessionIds = this.warmSessionIds.filter((entryId) => entryId !== id);
    this.subscribedSessionIds = this.subscribedSessionIds.filter((entryId) => entryId !== id);
    this.refreshSubscriptions();
    this.publish();
  };

  setDiff = (sessionId: string, diff: string) => {
    const entry = this.ensureEntry(sessionId);
    entry.diff = diff;
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  setGitStatusSummary = (sessionId: string, summary: GitStatusSummary | null) => {
    const entry = this.ensureEntry(sessionId);
    entry.gitStatusSummary = summary;
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
      const cached = await loadSessionHistoryPageV1(sessionId, beforeSeq, TURN_PAGE_LIMIT);
      if (cached?.page) {
        const page = cached.page;
        this.mergeTurns(entry, page.turns);
        this.mergeMessages(entry, page.messages);
        entry.hasMoreTurns = page.has_more;
        entry.oldestTurnSeq = page.next_cursor ?? entry.oldestTurnSeq;
        entry.updatedAtMs = Date.now();
        this.publish();
        await this.persistHead(entry);
        return;
      }
      const page = await getSessionHistory(sessionId, beforeSeq, TURN_PAGE_LIMIT);
      this.mergeTurns(entry, page.turns);
      this.mergeMessages(entry, page.messages);
      entry.hasMoreTurns = page.has_more;
      entry.oldestTurnSeq = page.next_cursor ?? entry.oldestTurnSeq;
      entry.updatedAtMs = Date.now();
      this.publish();
      await saveSessionHistoryPageV1(sessionId, beforeSeq, TURN_PAGE_LIMIT, page);
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
      // TEMP: Keep only summary-level tool data to reduce memory pressure in the webapp.
      // Restore full tool payload hydration after the native migration stabilizes.
      const summarized = tools.map(summarizeToolPayload);
      entry.turnToolsByTurnId = {
        ...entry.turnToolsByTurnId,
        [turnId]: summarized,
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
        localMessages: e.localMessages,
        artifacts: e.artifacts,
        artifactsLoading: e.artifactsLoading,
        subagentInvocations: e.subagentInvocations,
        subagentInvocationsLoading: e.subagentInvocationsLoading,
        stateLoaded: e.stateLoaded,
        stateLoading: e.stateLoading,
        stateRev: e.stateRev,
        queue: e.queue,
        diff: e.diff,
        gitStatusSummary: e.gitStatusSummary ?? null,
        summaryCheckpoint: e.summaryCheckpoint ?? null,
        headWindow: e.headWindow ?? null,
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
      localMessages: [],
      artifacts: [],
      artifactsLoading: false,
      subagentInvocations: [],
      subagentInvocationsLoading: false,
      stateLoaded: false,
      stateLoading: false,
      stateRev: undefined,
      stateAppliedRev: undefined,
      stateFetchToken: 0,
      queue: [],
      diff: undefined,
      gitStatusSummary: null,
      summaryCheckpoint: null,
      headWindow: null,
      diagnosticsByPath: {},
      lastEventSeq: undefined,
      loading: false,
      error: undefined,
      subscribed: false,
      updatedAtMs: Date.now(),
      refCount: 0,
      warmUntilMs: Date.now() + WARM_TTL_MS,
      acpMetaUpdatedAtMs: undefined,
      seqSet: new Set<number>(),
      turnsHydrated: false,
      oldestTurnSeq: undefined,
      toolStatusByKey: new Map(),
      toolIdsByTurn: new Map(),
      turnToolsLoadingSet: new Set(),
      turnToolsHydratedByTurnId: {},
      artifactsLoaded: false,
      artifactsFetchedAtMs: undefined,
      subagentInvocationsLoaded: false,
      subagentInvocationsFetchedAtMs: undefined,
      loadedFromCache: false,
      headFromCache: false,
      fetching: {
        head: false,
        history: false,
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

  private applyGitStatusSnapshotFromEvents(entry: InternalEntry, events: SessionEvent[]): boolean {
    for (let i = events.length - 1; i >= 0; i -= 1) {
      const event = events[i];
      if (String(event.event_type) !== "notice") continue;
      const payload = event.payload_json;
      if (payload?.kind !== "git_status_snapshot") continue;
      const partial = normalizeGitStatusSummaryInput(payload.summary, payload.entries);
      if (Object.keys(partial).length === 0) return false;
      entry.gitStatusSummary = { ...(entry.gitStatusSummary ?? {}), ...partial };
      return true;
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
      void this.loadCachedHead(entry);
    }
    const seeded = this.seedHeadFromActiveSnapshot(entry);
    if (seeded) {
      entry.error = undefined;
      entry.updatedAtMs = Date.now();
      if (entry.turnsHydrated && !opts?.force && !entry.headFromCache) {
        void this.ensureState(entry);
        this.publish();
        return;
      }
    }
    if (entry.fetching.head) return;
    if (entry.turnsHydrated && !opts?.force && !entry.headFromCache) {
      void this.ensureState(entry);
      return;
    }
    entry.fetching.head = true;
    if (!opts?.silent) {
      entry.loading = true;
      this.publish();
    }
    try {
      const snapshot = await getSessionSnapshot(sessionId, HEAD_LIMIT, true);
      this.applyHead(entry, snapshot.head);
      this.applyState(entry, snapshot.state ?? null, snapshot.head?.state_rev ?? snapshot.summary?.state_rev);
      await this.persistHead(entry);
      void this.ensureArtifacts(entry);
      void this.ensureSubagentInvocations(entry);
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
  }

  private async ensureState(entry: InternalEntry, opts?: { force?: boolean }) {
    if (entry.stateLoading) return;
    if (entry.stateLoaded && !opts?.force) return;
    entry.stateLoading = true;
    const requestRev = entry.stateRev;
    entry.stateFetchToken += 1;
    const fetchToken = entry.stateFetchToken;
    entry.updatedAtMs = Date.now();
    this.publish();
    try {
      const state = await getSessionState(entry.sessionId);
      if (entry.stateFetchToken !== fetchToken) return;
      if (
        typeof requestRev === "number" &&
        typeof entry.stateRev === "number" &&
        entry.stateRev !== requestRev
      ) {
        return;
      }
      this.applyState(entry, state, requestRev ?? entry.stateRev);
    } catch {
      // ignore state load errors (missing session or daemon offline)
    } finally {
      if (entry.stateFetchToken === fetchToken) {
        entry.stateLoading = false;
      }
      entry.updatedAtMs = Date.now();
      this.publish();
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

  private async ensureSubagentInvocations(entry: InternalEntry, opts?: { force?: boolean }) {
    if (entry.subagentInvocationsLoading) return;
    if (entry.subagentInvocationsLoaded && !opts?.force) return;
    entry.subagentInvocationsLoading = true;
    entry.updatedAtMs = Date.now();
    this.publish();
    try {
      const invocations = await listSessionSubagentInvocations(entry.sessionId);
      entry.subagentInvocations = invocations;
      entry.subagentInvocationsLoaded = true;
      entry.subagentInvocationsFetchedAtMs = Date.now();
    } catch {
      // ignore invocation load errors
    } finally {
      entry.subagentInvocationsLoading = false;
      entry.updatedAtMs = Date.now();
      this.publish();
    }
  }

  private async loadCachedHead(entry: InternalEntry) {
    try {
      const cached = await loadSessionHeadV1(entry.sessionId);
      if (!cached?.head) return;
      const cachedSeq = typeof cached.head.last_event_seq === "number" ? cached.head.last_event_seq : -1;
      const currentSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
      if (currentSeq >= 0 && cachedSeq >= 0 && cachedSeq < currentSeq) return;
      if (entry.turnsHydrated && !entry.headFromCache) return;
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

  private seedHeadFromActiveSnapshot(entry: InternalEntry): boolean {
    const store = this.snapshotStore;
    if (!store) return false;
    const applySnapshot = (head: SessionHeadSnapshot): boolean => {
      const nextSeq = typeof head.last_event_seq === "number" ? head.last_event_seq : -1;
      const prevSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
      if (entry.turnsHydrated && prevSeq >= nextSeq) {
        if (!entry.session) {
          entry.session = head.session;
          return true;
        }
        return false;
      }
      this.applyHead(entry, head as SessionHead);
      return true;
    };

    const direct = store.getSessionHeadSnapshot(entry.sessionId);
    if (direct && applySnapshot(direct)) return true;

    const state = store.getSnapshot();
    for (const taskId of state.activeIds) {
      const item = state.tasksById[taskId];
      const head = item?.primarySessionHead;
      if (!head) continue;
      const sessionId = idToString(head.session?.id);
      if (!sessionId || sessionId !== entry.sessionId) continue;
      return applySnapshot(head);
    }
    return false;
  }

  private applyHead(
    entry: InternalEntry,
    head: SessionHead,
    opts?: { fromCache?: boolean },
  ) {
    entry.headFromCache = Boolean(opts?.fromCache);
    entry.session = head.session;
    entry.summaryCheckpoint = head.summary_checkpoint ?? null;
    entry.headWindow = head.head_window ?? null;
    entry.turnsHydrated = true;
    entry.hasMoreTurns = head.has_more_turns;
    entry.lastEventSeq = head.last_event_seq;
    const headStateRev = (head as any)?.state_rev ?? (head as any)?.stateRev;
    if (typeof headStateRev === "number") {
      entry.stateRev = headStateRev;
    }
    this.mergeTurns(entry, head.turns ?? []);
    this.mergeEvents(entry, head.events ?? []);
    this.mergeMessages(entry, head.messages ?? []);
    this.applyAcpMetaFromEvents(entry, head.events ?? []);
    this.applyGitStatusSnapshotFromEvents(entry, head.events ?? []);
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

  private applyState(entry: InternalEntry, state: SessionState | null, stateRev?: number) {
    if (!state) return;
    if (
      typeof stateRev === "number" &&
      typeof entry.stateRev === "number" &&
      stateRev < entry.stateRev
    ) {
      return;
    }
    entry.stateLoaded = true;
    entry.stateLoading = false;
    if (typeof stateRev === "number") {
      entry.stateRev = stateRev;
      entry.stateAppliedRev = stateRev;
    }
    entry.artifacts = Array.isArray(state.artifacts) ? state.artifacts : [];
    entry.artifactsLoaded = true;
    entry.artifactsLoading = false;
    entry.artifactsFetchedAtMs = Date.now();
    entry.gitStatusSummary = state.git_status ?? null;
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
      summary_checkpoint: entry.summaryCheckpoint ?? null,
      head_window: entry.headWindow ?? null,
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
        const entry = this.ensureEntry(sessionId);
        entry.subscribed = true;
      }
    }
    if (this.snapshotStore) {
      const subs = next.map((sessionId) => {
        const entry = this.entries.get(sessionId);
        const afterSeq = entry?.lastEventSeq;
        return {
          session_id: sessionId,
          ...(typeof afterSeq === "number" ? { after_seq: afterSeq } : {}),
        };
      });
      this.snapshotStore.setSubscriptions(subs);
    }
    this.publish();
  }

  private refreshSubscribedHeads() {
    for (const sessionId of this.subscribedSessionIds) {
      const entry = this.entries.get(sessionId);
      if (!entry || entry.refCount === 0) continue;
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
    const next = Array.from(byId.values()).sort(this.compareMessageOrder);
    entry.messages = next;
    entry.queue = next.filter((m) => m.delivery === "queued");
  }


  private mergeLocalMessages(entry: InternalEntry, incoming: Message[]) {
    entry.localMessages = this.mergeMessageList(entry.localMessages, incoming);
  }

  private mergeMessageList(existing: Message[], incoming: Message[]): Message[] {
    if (incoming.length === 0) return existing;
    const byId = new Map<string, Message>();
    for (const m of existing) {
      const id = idToString(m.id);
      if (id) byId.set(id, m);
    }
    for (const m of incoming) {
      const id = idToString(m.id);
      if (!id) continue;
      byId.set(id, m);
    }
    return Array.from(byId.values()).sort(this.compareMessageOrder);
  }

  private compareMessageOrder(a: Message, b: Message): number {
    const c = String(a.created_at).localeCompare(String(b.created_at));
    if (c !== 0) return c;
    const sa = Number(a.turn_sequence ?? Number.NaN);
    const sb = Number(b.turn_sequence ?? Number.NaN);
    if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
    if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
    if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
    return String(idToString(a.id)).localeCompare(String(idToString(b.id)));
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
    entry.artifactsLoading = false;
    entry.artifactsFetchedAtMs = Date.now();
    return true;
  }

  private applyGitStatusSnapshotNotice(entry: InternalEntry, event: SessionEvent): boolean {
    if (String(event.event_type) !== "notice") return false;
    const payload = event.payload_json;
    if (payload?.kind !== "git_status_snapshot") return false;
    const partial = normalizeGitStatusSummaryInput(payload.summary, payload.entries);
    if (Object.keys(partial).length === 0) return false;
    entry.gitStatusSummary = { ...(entry.gitStatusSummary ?? {}), ...partial };
    return true;
  }

  private applySubagentInvocationNotice(entry: InternalEntry, event: SessionEvent): boolean {
    if (String(event.event_type) !== "notice") return false;
    const kind = event.payload_json?.kind;
    if (kind !== "subagent_invocation_created" && kind !== "subagent_invocation_updated") return false;
    void this.ensureSubagentInvocations(entry, { force: true });
    return false;
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

  private handleWorkspaceEvent(evt: WorkspaceActiveSnapshotEvent) {
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
    if (evt.type === "active_task_upsert") {
      const head = evt.task.primary_session_head;
      if (head) {
        const sessionId = idToString(head.session?.id);
        if (sessionId) {
          const entry = this.ensureEntry(sessionId);
          const applied = this.applyActiveSnapshotHead(entry, head);
          if (applied) {
            entry.updatedAtMs = Date.now();
            this.publish();
          }
        }
      }
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
    if (typeof delta.state_rev === "number") {
      const next = delta.state_rev;
      const prev = typeof entry.stateRev === "number" ? entry.stateRev : 0;
      if (next > prev) {
        entry.stateRev = next;
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
      if (this.applyGitStatusSnapshotNotice(entry, delta.event)) {
        changed = true;
      }
      if (this.applySubagentInvocationNotice(entry, delta.event)) {
        changed = true;
      }
    }

    if (changed) {
      entry.updatedAtMs = Date.now();
      this.publish();
    }
  }

  private syncActiveSnapshot(state: WorkspaceActiveSnapshotState) {
    let changed = false;
    for (const taskId of state.activeIds) {
      const item = state.tasksById[taskId];
      const head = item?.primarySessionHead;
      if (!head) continue;
      const sessionId = idToString(head.session?.id);
      if (!sessionId) continue;
      const entry = this.ensureEntry(sessionId);
      if (this.applyActiveSnapshotHead(entry, head)) {
        entry.updatedAtMs = Date.now();
        changed = true;
      }
    }
    if (changed) {
      this.publish();
    }
  }

  private applyActiveSnapshotHead(entry: InternalEntry, head: SessionHead | SessionHeadSnapshot): boolean {
    const nextSeq = typeof head.last_event_seq === "number" ? head.last_event_seq : -1;
    const prevSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
    if (entry.turnsHydrated && prevSeq >= nextSeq) {
      if (!entry.session) {
        entry.session = head.session;
        return true;
      }
      return false;
    }
    this.applyHead(entry, head as SessionHead);
    void this.persistHead(entry);
    entry.error = undefined;
    return true;
  }

  private resetEntryForGap(entry: InternalEntry) {
    entry.turns = [];
    entry.events = [];
    entry.messages = [];
    entry.artifacts = [];
    entry.artifactsLoaded = false;
    entry.artifactsLoading = false;
    entry.artifactsFetchedAtMs = undefined;
    entry.stateLoaded = false;
    entry.stateLoading = false;
    entry.stateRev = undefined;
    entry.stateAppliedRev = undefined;
    entry.subagentInvocations = [];
    entry.subagentInvocationsLoaded = false;
    entry.subagentInvocationsLoading = false;
    entry.subagentInvocationsFetchedAtMs = undefined;
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

const summarizeToolPayload = (
  tool: SessionTurnTool,
): SessionTurnTool & { summary_only: boolean } => ({
  ...tool,
  input_json: toolInputPreview(tool.input_json) ?? null,
  output_text: null,
  summary_only: true,
});

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
