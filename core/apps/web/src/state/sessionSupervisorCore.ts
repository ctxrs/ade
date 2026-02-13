import {
  getProviderOptions,
  getSessionHistory,
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
  loadTaskThoughtsV1,
  saveTaskThoughtsV1,
  clearTaskThoughtsV1,
  saveSessionAcpMetaV1,
  saveSessionHeadV1,
  saveSessionHistoryPageV1,
} from "./uiStateStore";
import type { PersistedTaskThoughtsV1 } from "./uiStateStore";
import { getClientSettings } from "./clientSettings";
import { SessionReplicaBridge } from "./sessionReplicaBridge";
import type { SessionReplicaPatch } from "./sessionReplicaProtocol";
import { sendDesktopNotification } from "../utils/desktopNotifications";
import { isAppInForeground } from "../utils/windowFocus";
import { emitUiDiagnostic } from "./diagnosticsChannel";
import { normalizeGitStatusSummaryInput } from "./sessionSupervisor/gitStatusNormalization";
import {
  appendFragment,
  dedupeIds,
  isPartialEvent,
  mergeOrderedIds,
  mergeTurn,
  sameIdList,
  stripPartialEvents,
  stripTurnPartials,
} from "./sessionSupervisor/cachePolicy";
import { pickFirstString, readPayloadString } from "./sessionSupervisor/eventNormalization";
import {
  buildThoughtCacheKey,
  isFinalThoughtEvent,
  isFinalThoughtPayload,
  normalizeFinalThoughtPayload,
  readThoughtFullContent,
} from "./sessionSupervisor/thoughtProjection";
import {
  applyToolBucketDelta,
  deriveTurnStatusFromEvent,
  extractToolCallId,
  extractToolStatus,
  readTurnStatusFromPayload,
  shouldRenderAssistantChunk,
  shouldRenderThoughtChunk,
  summarizeToolPayload,
  toolStatusBucket,
} from "./sessionSupervisor/toolStateProjection";

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};

const readPayloadObject = (
  payload: SessionEvent["payload_json"],
  key: string,
): Record<string, unknown> | null => {
  const value = asRecord(payload)[key];
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
};

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
export type SessionMode = "active" | "archived";
export type SessionLoadState = "pending_hydration" | "live" | "recovering" | "fatal";

export type SessionSupervisorSnapshot = {
  connection: ConnectionStatus;
  sessions: Record<string, SessionCacheEntry>;
};

export type SessionCacheEntry = {
  sessionId: string;
  mode?: SessionMode;
  loadState: SessionLoadState;
  session?: Session;
  acpModels?: any;
  acpModes?: any;
  acpCurrentModelId?: string;
  turns: SessionTurn[];
  turnToolsByTurnId: Record<string, SessionTurnTool[]>;
  turnToolsLoading: string[];
  toolSummaries: SessionTurnToolSummary[];
  toolSummariesReady: boolean;
  hasMoreTurns: boolean;
  events: SessionEvent[];
  messages: Message[];
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
  oldestTurnSeq?: number;
  fetching?: {
    head: boolean;
    history: boolean;
  };
  updatedAtMs: number;
};

type ThoughtCacheEntry = {
  key: string;
  event: SessionEvent;
  updatedAtMs?: number;
};

type OpenOptions = {
  watchDiff?: boolean;
  force?: boolean;
  silent?: boolean;
  mode?: SessionMode;
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
  warmUntilMs: number;
  acpMetaUpdatedAtMs?: number;
  seqSet: Set<number>;
  nextTransientSeq: number;
  startedTurnIds: Set<string>;
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
  thoughtCacheByKey: Record<string, ThoughtCacheEntry>;
  thoughtCacheLoaded: boolean;
  thoughtCacheLoading: boolean;
  thoughtCacheDirty: boolean;
  thoughtCacheTaskId?: string;
  thoughtCacheLoadToken: number;
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
const MODE_RESOLUTION_MAX_ATTEMPTS = 6;
const MODE_RESOLUTION_RETRY_MS = 100;

// The daemon serializes transient events with `seq: null` (see Rust `SessionEvent` Serialize).
// We assign a stable synthetic seq in a negative JS-safe range so sorting never scrambles
// streaming partials (assistant chunks), and these events never look durable (seq >= 0).
const TRANSIENT_SEQ_START = -4503599627370496; // -(2 ** 52)

export class SessionSupervisor {
  private listeners = new Set<() => void>();
  private snapshot: SessionSupervisorSnapshot = { connection: "idle", sessions: {} };
  private entries = new Map<string, InternalEntry>();
  private replica: SessionReplicaBridge;
  private snapshotStore: WorkspaceActiveSnapshotEventSource | null = null;
  private snapshotUnsub: (() => void) | null = null;
  private snapshotStateUnsub: (() => void) | null = null;
  private activeTaskSessionIds: string[] = [];
  private warmSessionIds: string[] = [];
  private subscribedSessionIds: string[] = [];
  private providerOptionsCache = new Map<string, ProviderOptions>();
  private providerOptionsInFlight = new Map<string, Promise<ProviderOptions | undefined>>();
  private taskThoughtCache = new Map<string, PersistedTaskThoughtsV1>();
  private taskThoughtCacheLoading = new Map<string, Promise<PersistedTaskThoughtsV1>>();
  private modeResolutionTimers = new Map<string, ReturnType<typeof globalThis.setTimeout>>();
  private modeResolutionAttempts = new Map<string, number>();
  private modeResolutionOptions = new Map<string, OpenOptions | undefined>();

  constructor() {
    this.replica = new SessionReplicaBridge(this.handleReplicaPatches, {
      eventBufferLimit: EVENT_BUFFER_LIMIT,
      headLimit: HEAD_LIMIT,
    });
  }

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
      for (const sessionId of [...this.modeResolutionTimers.keys()]) {
        this.clearModeResolution(sessionId);
      }
      this.setConnection("disconnected");
      return;
    }
    this.snapshotUnsub = store.subscribeEvents((evt) => this.handleWorkspaceEvent(evt));
    this.snapshotStateUnsub = store.subscribe(() => {
      const state = store.getSnapshot();
      const next = this.mapConnection(state.connection);
      this.setConnection(next);
      this.syncActiveSnapshot(state);
      this.resolvePendingSessionModes(state);
      if (next !== "connected") {
        this.markOpenSessionsRecovering();
      }
    });
    this.setConnection(this.mapConnection(store.getSnapshot().connection));
    this.resolvePendingSessionModes(store.getSnapshot());
    this.refreshSubscriptions();
  }

  openSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.ensureEntry(sessionId);
    entry.refCount += 1;
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    if (opts?.mode) {
      entry.mode = opts.mode;
    }
    if (entry.error) {
      entry.error = undefined;
    }
    this.setSessionLoadState(entry, "pending_hydration");
    const mode = this.resolveSessionMode(sessionId, entry, opts?.mode);
    if (mode) {
      this.clearModeResolution(sessionId);
      this.openSessionWithMode(sessionId, entry, mode, opts);
    } else {
      this.scheduleModeResolution(sessionId, opts);
    }
    this.refreshSubscriptions();
    this.publish();
    return () => this.closeSession(sessionId, opts);
  };

  closeSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.entries.get(sessionId);
    if (!entry) return;
    entry.refCount = Math.max(0, entry.refCount - 1);
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    if (entry.refCount === 0) {
      this.clearModeResolution(sessionId);
    }
    this.replica.dispatch({ type: "close_session", sessionId });
    this.refreshSubscriptions();
    this.publish();
  };

  refreshSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    const mode = this.resolveSessionMode(sessionId, entry, opts?.mode);
    if (!mode) {
      this.scheduleModeResolution(sessionId, opts);
      return;
    }
    this.clearModeResolution(sessionId);
    this.replica.dispatch({
      type: "refresh_session",
      sessionId,
    });
    if (mode === "archived") {
      this.replica.dispatch({
        type: "hydrate_session_head",
        sessionId,
        force: true,
        silent: true,
      });
      this.setSessionLoadState(entry, "pending_hydration");
    }
  };

  loadArtifacts = (sessionId: string, opts?: { force?: boolean }) => {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    void this.ensureArtifacts(entry, opts);
  };

  refreshQueue = (sessionId: string) => {
    this.replica.dispatch({ type: "refresh_session", sessionId });
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
    void this.ensureThoughtCache(entry);
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

  setTurns = (sessionId: string, turns: SessionTurn[], opts?: { replace?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (opts?.replace) {
      entry.turns = [];
    }

    if (turns.length > 0) {
      const byId = new Map<string, SessionTurn>();
      for (const t of entry.turns) {
        const tid = idToString(t.turn_id);
        if (tid) byId.set(tid, t);
      }
      for (const t of turns) {
        const tid = idToString(t.turn_id);
        if (tid) byId.set(tid, t);
      }
      const merged = Array.from(byId.values());
      merged.sort((a, b) => {
        const aSeq = Number(a.start_seq ?? Number.NaN);
        const bSeq = Number(b.start_seq ?? Number.NaN);
        if (Number.isFinite(aSeq) && Number.isFinite(bSeq) && aSeq !== bSeq) return aSeq - bSeq;
        if (Number.isFinite(aSeq) && !Number.isFinite(bSeq)) return -1;
        if (!Number.isFinite(aSeq) && Number.isFinite(bSeq)) return 1;
        const aStart = String(a.started_at ?? "");
        const bStart = String(b.started_at ?? "");
        if (aStart !== bStart) return aStart.localeCompare(bStart);
        return String(a.turn_id ?? "").localeCompare(String(b.turn_id ?? ""));
      });
      entry.turns = merged;
      entry.turnsHydrated = true;
    }

    entry.updatedAtMs = Date.now();
    this.publish();
  };

  setError = (sessionId: string, error: string | null) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (error) {
      this.setFatalError(entry, error);
    } else {
      entry.error = undefined;
      this.setSessionLoadState(entry, "live");
    }
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  dropSessionEntry = (sessionId: string) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    if (!this.entries.has(id)) return;
    this.clearModeResolution(id);
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

  async loadMoreTurns(sessionId: string): Promise<number | null> {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return null;
    if (entry.fetching.history) return null;
    if (!entry.hasMoreTurns) return 0;
    const beforeSeq = entry.oldestTurnSeq;
    if (beforeSeq == null || !Number.isFinite(beforeSeq)) {
      entry.hasMoreTurns = false;
      this.publish();
      return 0;
    }
    entry.fetching.history = true;
    const beforeLen = entry.turns.length;
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
        return entry.turns.length - beforeLen;
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
      return entry.turns.length - beforeLen;
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
    for (const l of this.listeners) l();
  }

  private publish() {
    this.evictIfNeeded();
    const sessions: Record<string, SessionCacheEntry> = {};
    for (const [id, e] of this.entries) {
      sessions[id] = {
        sessionId: e.sessionId,
        mode: e.mode,
        loadState: e.loadState,
        session: e.session,
        acpModels: e.acpModels,
        acpModes: e.acpModes,
        acpCurrentModelId: e.acpCurrentModelId,
        turns: e.turns,
        turnToolsByTurnId: e.turnToolsByTurnId,
        turnToolsLoading: [...e.turnToolsLoadingSet],
        toolSummaries: e.toolSummaries,
        toolSummariesReady: e.toolSummariesReady,
        hasMoreTurns: e.hasMoreTurns,
        events: e.events,
        messages: e.messages,
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
        oldestTurnSeq: e.oldestTurnSeq,
        fetching: { ...e.fetching },
        updatedAtMs: e.updatedAtMs,
      };
    }
    this.snapshot = { connection: this.snapshot.connection, sessions };
    for (const l of this.listeners) l();
  }

  private handleReplicaPatches = (patches: SessionReplicaPatch[]) => {
    if (!patches || patches.length === 0) return;
    let changed = false;
    for (const patch of patches) {
      const sessionId = String(patch.sessionId || "").trim();
      if (!sessionId) continue;
      const entry = this.ensureEntry(sessionId);
      let localOnlyMessages: Message[] = [];
      if (patch.op === "replace") {
        const incomingMessages = Array.isArray(patch.data.messages) ? patch.data.messages : [];
        const incomingMessageIds = new Set(
          incomingMessages
            .map((message) => idToString(message.id))
            .filter((id): id is string => !!id),
        );
        localOnlyMessages = entry.messages.filter((message) => {
          const id = idToString(message.id);
          return id ? !incomingMessageIds.has(id) : false;
        });
        this.resetEntryForGap(entry, { skipPublish: true });
      }
      if (patch.op === "evict") {
        const beforeSeq = patch.data.eventsBeforeSeq;
        if (typeof beforeSeq === "number") {
          entry.events = entry.events.filter(
            (event) => typeof event.seq === "number" && event.seq >= beforeSeq,
          );
          entry.seqSet = new Set(
            entry.events
              .map((event) => (typeof event.seq === "number" ? event.seq : Number.NaN))
              .filter((seq) => Number.isFinite(seq)) as number[],
          );
          entry.updatedAtMs = Date.now();
          changed = true;
        }
        continue;
      }
      const data = patch.data;
      if (data.session) {
        entry.session = data.session;
        if (!entry.mode) {
          const resolvedMode = this.resolveSessionMode(sessionId, entry);
          if (resolvedMode) {
            entry.mode = resolvedMode;
          }
        }
        void this.ensureThoughtCache(entry);
      }
      if (data.turns && data.turns.length > 0) {
        this.mergeTurns(entry, data.turns);
      }
      if (data.messages && data.messages.length > 0) {
        this.mergeMessages(entry, data.messages);
      }
      if (localOnlyMessages.length > 0) {
        this.mergeMessages(entry, localOnlyMessages);
      }
      if (data.events && data.events.length > 0) {
        this.mergeEvents(entry, data.events, { notify: patch.op !== "replace" });
      }
      if (data.toolSummaries && data.toolSummaries.length > 0) {
        this.applyToolSummaries(entry, data.toolSummaries);
      }
      if (data.acpMeta) {
        entry.acpModels = data.acpMeta.models ?? entry.acpModels;
        entry.acpModes = data.acpMeta.modes ?? entry.acpModes;
        entry.acpCurrentModelId = data.acpMeta.currentModelId ?? entry.acpCurrentModelId;
        entry.acpMetaUpdatedAtMs = Date.now();
      }
      if (data.gitStatusSummary !== undefined) {
        entry.gitStatusSummary = data.gitStatusSummary ?? null;
      }
      if (data.artifacts) {
        entry.artifacts = data.artifacts;
        entry.artifactsFetchedAtMs = Date.now();
        entry.artifactsLoaded = true;
        entry.artifactsLoading = false;
      }
      if (data.artifactsLoaded !== undefined) {
        entry.artifactsLoaded = data.artifactsLoaded;
        if (data.artifactsLoaded) {
          entry.artifactsLoading = false;
        }
      }
      if (data.stateLoaded !== undefined) {
        entry.stateLoaded = data.stateLoaded;
      }
      if (data.stateLoading !== undefined) {
        entry.stateLoading = data.stateLoading;
      }
      if (data.stateRev !== undefined) {
        entry.stateRev = data.stateRev;
      }
      if (data.summaryCheckpoint !== undefined) {
        entry.summaryCheckpoint = data.summaryCheckpoint;
      }
      if (data.headWindow !== undefined) {
        entry.headWindow = data.headWindow;
      }
      if (data.lastEventSeq !== undefined) {
        entry.lastEventSeq = data.lastEventSeq;
      }
      if (data.hasMoreTurns !== undefined) {
        entry.hasMoreTurns = data.hasMoreTurns;
      }
      if (data.turnsHydrated !== undefined) {
        entry.turnsHydrated = data.turnsHydrated;
      }
      if (data.loading !== undefined) {
        entry.loading = data.loading;
        if (data.loading && entry.loadState !== "live") {
          this.setSessionLoadState(entry, "pending_hydration");
        }
      }
      const hasRecoveryData =
        data.session !== undefined ||
        (Array.isArray(data.turns) && data.turns.length > 0) ||
        (Array.isArray(data.messages) && data.messages.length > 0) ||
        (Array.isArray(data.events) && data.events.length > 0) ||
        (Array.isArray(data.toolSummaries) && data.toolSummaries.length > 0) ||
        data.lastEventSeq !== undefined ||
        data.stateRev !== undefined ||
        data.summaryCheckpoint !== undefined ||
        data.headWindow !== undefined ||
        data.hasMoreTurns !== undefined ||
        data.turnsHydrated !== undefined;
      if (data.error !== undefined) {
        if (data.error) {
          this.setFatalError(entry, data.error);
        } else {
          entry.error = undefined;
          if (entry.loadState === "fatal") {
            this.setSessionLoadState(entry, "pending_hydration");
          }
        }
      } else if (hasRecoveryData) {
        entry.error = undefined;
        this.setSessionLoadState(entry, "live");
      }
      if (data.subagentNotice) {
        void this.ensureSubagentInvocations(entry, { force: true });
      }
      if (!entry.acpModels || !hasModelList(entry.acpModels)) {
        void this.ensureProviderOptions(entry);
      }
      entry.queue = entry.messages.filter((m) => m.delivery === "queued");
      entry.updatedAtMs = Date.now();
      changed = true;
    }
    if (changed) {
      this.publish();
    }
  };

  private evictIfNeeded() {
    if (this.entries.size <= MAX_CACHED_SESSIONS) return;
    const now = Date.now();
    const candidates = [...this.entries.values()]
      .filter((e) => e.refCount === 0 && now > e.warmUntilMs)
      .sort((a, b) => a.updatedAtMs - b.updatedAtMs);
    for (const c of candidates) {
      if (this.entries.size <= MAX_CACHED_SESSIONS) break;
      this.clearModeResolution(c.sessionId);
      this.entries.delete(c.sessionId);
    }
  }

  private ensureEntry(sessionId: string): InternalEntry {
    const existing = this.entries.get(sessionId);
    if (existing) return existing;
    const entry: InternalEntry = {
      sessionId,
      mode: undefined,
      loadState: "pending_hydration",
      session: undefined,
      acpModels: undefined,
      acpModes: undefined,
      acpCurrentModelId: undefined,
      turns: [],
      turnToolsByTurnId: {},
      turnToolsLoading: [],
      toolSummaries: [],
      toolSummariesReady: false,
      hasMoreTurns: true,
      events: [],
      messages: [],
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
      nextTransientSeq: TRANSIENT_SEQ_START,
      startedTurnIds: new Set<string>(),
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
      thoughtCacheByKey: {},
      thoughtCacheLoaded: false,
      thoughtCacheLoading: false,
      thoughtCacheDirty: false,
      thoughtCacheTaskId: undefined,
      thoughtCacheLoadToken: 0,
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
    const head = this.getActiveSnapshotHead(entry.sessionId);
    if (!head) return false;
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
      const strippedEvents =
        (head.events?.length ?? 0) === 0 && (head.head_window?.event_limit ?? 0) === 0;
      this.applyHead(entry, head as SessionHead, { fromCache: strippedEvents });
      return true;
    };

    return applySnapshot(head);
  }

  private getActiveSnapshotHead(sessionId: string): SessionHeadSnapshot | null {
    const store = this.snapshotStore;
    if (!store) return null;
    const direct = store.getSessionHeadSnapshot(sessionId);
    if (direct) return direct;
    const state = store.getSnapshot();
    for (const taskId of state.activeIds) {
      const item = state.tasksById[taskId];
      const head = item?.primarySessionHead;
      if (!head) continue;
      const headSessionId = idToString(head.session?.id);
      if (!headSessionId || headSessionId !== sessionId) continue;
      return head;
    }
    return null;
  }

  private openSessionWithMode(
    sessionId: string,
    entry: InternalEntry,
    mode: SessionMode,
    opts?: OpenOptions,
  ) {
    entry.mode = mode;
    const seededHead = mode === "active" ? this.seedHeadFromActiveSnapshot(entry) : false;
    this.replica.dispatch({
      type: "open_session",
      sessionId,
      force: opts?.force,
      silent: opts?.silent,
    });
    if (mode === "archived") {
      this.replica.dispatch({
        type: "hydrate_session_head",
        sessionId,
        force: opts?.force,
        silent: opts?.silent,
      });
      this.setSessionLoadState(entry, "pending_hydration");
      return;
    }
    if (seededHead || entry.turnsHydrated || entry.messages.length > 0 || entry.events.length > 0) {
      this.setSessionLoadState(entry, "live");
      return;
    }
    this.setSessionLoadState(entry, "pending_hydration");
  }

  private resolveSessionMode(
    sessionId: string,
    entry?: InternalEntry,
    explicitMode?: SessionMode,
  ): SessionMode | null {
    const id = String(sessionId ?? "").trim();
    if (!id) return null;
    if (explicitMode) {
      if (entry) entry.mode = explicitMode;
      return explicitMode;
    }
    if (entry?.mode) return entry.mode;
    const state = this.snapshotStore?.getSnapshot();
    if (!state) {
      if (entry) {
        entry.mode = "active";
      }
      return "active";
    }
    const mode = this.resolveSessionModeFromState(state, id);
    if (mode && entry) {
      entry.mode = mode;
    }
    return mode;
  }

  private resolveSessionModeFromState(state: WorkspaceActiveSnapshotState, sessionId: string): SessionMode | null {
    const id = String(sessionId ?? "").trim();
    if (!id) return null;
    const isSessionInTask = (taskId: string): boolean => {
      const task = state.tasksById[taskId];
      if (!task) return false;
      if (idToString(task.task.primary_session_id ?? "") === id) return true;
      const primaryHeadId = idToString(task.primarySessionHead?.session?.id ?? "");
      if (primaryHeadId === id) return true;
      for (const summary of task.sessions) {
        if (idToString(summary.session.id) === id) return true;
      }
      return false;
    };

    for (const activeTaskId of state.activeIds) {
      if (isSessionInTask(activeTaskId)) return "active";
    }
    for (const archivedTaskId of state.archivedIds) {
      if (isSessionInTask(archivedTaskId)) return "archived";
    }
    return null;
  }

  private scheduleModeResolution(sessionId: string, opts?: OpenOptions) {
    const id = String(sessionId ?? "").trim();
    if (!id) return;
    const entry = this.entries.get(id);
    if (!entry || entry.refCount <= 0) return;
    this.modeResolutionOptions.set(id, opts ? { ...opts } : undefined);
    this.setSessionLoadState(entry, "pending_hydration");
    if (this.modeResolutionTimers.has(id)) return;

    const run = () => {
      const current = this.entries.get(id);
      if (!current || current.refCount <= 0) {
        this.clearModeResolution(id);
        return;
      }
      const mode = this.resolveSessionMode(id, current);
      if (mode) {
        const resolvedOpts = this.modeResolutionOptions.get(id);
        this.clearModeResolution(id);
        this.openSessionWithMode(id, current, mode, resolvedOpts);
        this.refreshSubscriptions();
        this.publish();
        return;
      }
      const attempts = (this.modeResolutionAttempts.get(id) ?? 0) + 1;
      this.modeResolutionAttempts.set(id, attempts);
      if (attempts >= MODE_RESOLUTION_MAX_ATTEMPTS) {
        this.clearModeResolution(id);
        this.setFatalError(current, `Session not found in workspace snapshot: ${id}`);
        current.updatedAtMs = Date.now();
        this.publish();
        return;
      }
      const timer = globalThis.setTimeout(run, MODE_RESOLUTION_RETRY_MS);
      this.modeResolutionTimers.set(id, timer);
    };

    const timer = globalThis.setTimeout(run, MODE_RESOLUTION_RETRY_MS);
    this.modeResolutionTimers.set(id, timer);
  }

  private resolvePendingSessionModes(state: WorkspaceActiveSnapshotState) {
    if (this.modeResolutionTimers.size === 0) return;
    let changed = false;
    for (const sessionId of [...this.modeResolutionTimers.keys()]) {
      const entry = this.entries.get(sessionId);
      if (!entry || entry.refCount <= 0) {
        this.clearModeResolution(sessionId);
        continue;
      }
      const mode = this.resolveSessionModeFromState(state, sessionId);
      if (!mode) continue;
      const opts = this.modeResolutionOptions.get(sessionId);
      this.clearModeResolution(sessionId);
      this.openSessionWithMode(sessionId, entry, mode, opts);
      changed = true;
    }
    if (changed) {
      this.refreshSubscriptions();
      this.publish();
    }
  }

  private clearModeResolution(sessionId: string) {
    const id = String(sessionId ?? "").trim();
    if (!id) return;
    const timer = this.modeResolutionTimers.get(id);
    if (timer) {
      globalThis.clearTimeout(timer);
    }
    this.modeResolutionTimers.delete(id);
    this.modeResolutionAttempts.delete(id);
    this.modeResolutionOptions.delete(id);
  }

  private setSessionLoadState(entry: InternalEntry, next: SessionLoadState) {
    if (entry.loadState === next) return;
    entry.loadState = next;
  }

  private setFatalError(entry: InternalEntry, message: string) {
    emitUiDiagnostic({
      source: "session_supervisor",
      code: "session.load_fatal",
      severity: "error",
      fatal: true,
      message,
      context: {
        sessionId: entry.sessionId,
        mode: entry.mode ?? null,
      },
    });
    entry.error = message;
    this.setSessionLoadState(entry, "fatal");
  }

  private markOpenSessionsRecovering() {
    let changed = false;
    for (const entry of this.entries.values()) {
      if (entry.refCount <= 0) continue;
      if (entry.loadState === "fatal") continue;
      if (entry.loadState !== "recovering") {
        entry.loadState = "recovering";
        changed = true;
      }
    }
    if (changed) {
      this.publish();
    }
  }

  private applyHead(
    entry: InternalEntry,
    head: SessionHead,
    opts?: { fromCache?: boolean },
  ) {
    entry.headFromCache = Boolean(opts?.fromCache);
    entry.session = head.session;
    if (!entry.mode) {
      const resolvedMode = this.resolveSessionMode(entry.sessionId, entry);
      if (resolvedMode) {
        entry.mode = resolvedMode;
      }
    }
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
    this.mergeEvents(entry, head.events ?? [], { notify: false });
    this.mergeMessages(entry, head.messages ?? []);
    this.applyAcpMetaFromEvents(entry, head.events ?? []);
    this.applyGitStatusSnapshotFromEvents(entry, head.events ?? []);
    if (!entry.acpModels || !hasModelList(entry.acpModels)) {
      void this.ensureProviderOptions(entry);
    }
    const incomingToolSummaries = head.tool_summaries;
    if (Array.isArray(incomingToolSummaries) && incomingToolSummaries.length > 0) {
      entry.toolSummaries = incomingToolSummaries;
    } else if (entry.toolSummaries.length === 0) {
      entry.toolSummaries = incomingToolSummaries ?? [];
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
          input_truncated: summary.input_truncated ?? null,
          input_original_bytes: summary.input_original_bytes ?? null,
          output_truncated: summary.output_truncated ?? null,
          output_original_bytes: summary.output_original_bytes ?? null,
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
    entry.error = undefined;
    this.setSessionLoadState(entry, "live");
    void this.ensureThoughtCache(entry);
    entry.updatedAtMs = Date.now();
    this.publish();
  }

  private applyToolSummaries(entry: InternalEntry, summaries: SessionTurnToolSummary[]) {
    const hydrated = entry.turnToolsHydratedByTurnId;
    let changed = false;
    const nextByTurn: Record<string, SessionTurnTool[]> = {};
    entry.toolSummaries = summaries;

    for (const summary of summaries) {
      const turnId = idToString(summary.turn_id);
      if (!turnId) continue;
      if (hydrated[turnId]) continue;
      const list = nextByTurn[turnId] ?? [];
      list.push({
        session_id: summary.session_id,
        tool_call_id: summary.tool_call_id,
        turn_id: summary.turn_id,
        tool_kind: summary.tool_kind ?? null,
        title: summary.title ?? null,
        status: summary.status ?? null,
        input_json: summary.input_preview ?? null,
        output_text: null,
        input_truncated: summary.input_truncated ?? null,
        input_original_bytes: summary.input_original_bytes ?? null,
        output_truncated: summary.output_truncated ?? null,
        output_original_bytes: summary.output_original_bytes ?? null,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        summary_only: true,
      } as SessionTurnTool & { summary_only: boolean });
      nextByTurn[turnId] = list;
    }

    for (const [turnId, incoming] of Object.entries(nextByTurn)) {
      const existing = entry.turnToolsByTurnId[turnId] ?? [];
      const seen = new Set(existing.map((tool) => String(tool.tool_call_id)));
      const merged = existing.slice();
      for (const tool of incoming) {
        const key = String(tool.tool_call_id);
        if (seen.has(key)) continue;
        seen.add(key);
        merged.push(tool);
      }
      entry.turnToolsByTurnId = {
        ...entry.turnToolsByTurnId,
        [turnId]: merged,
      };
      if (!hydrated[turnId]) hydrated[turnId] = false;
      changed = true;
    }

    if (changed) {
      entry.toolSummariesReady = true;
      entry.updatedAtMs = Date.now();
      this.publish();
    }
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
    const turns = stripTurnPartials(entry.turns);
    const events = stripPartialEvents(entry.events);
    const head = {
      session: entry.session,
      turns,
      events,
      messages: entry.messages,
      tool_summaries: entry.toolSummaries,
      last_event_seq: entry.lastEventSeq ?? 0,
      has_more_turns: entry.hasMoreTurns,
      summary_checkpoint: entry.summaryCheckpoint ?? null,
      head_window: entry.headWindow ?? undefined,
    };
    await saveSessionHeadV1(entry.sessionId, head);
  }

  private async getTaskThoughtCache(taskId: string): Promise<PersistedTaskThoughtsV1> {
    const cached = this.taskThoughtCache.get(taskId);
    if (cached) return cached;
    const inflight = this.taskThoughtCacheLoading.get(taskId);
    if (inflight) return inflight;
    const loader = (async () => {
      const existing = await loadTaskThoughtsV1(taskId);
      return (
        existing ?? {
          v: 1,
          taskId,
          sessions: {},
          updatedAtMs: Date.now(),
        }
      );
    })();
    this.taskThoughtCacheLoading.set(taskId, loader);
    try {
      const resolved = await loader;
      this.taskThoughtCache.set(taskId, resolved);
      return resolved;
    } finally {
      this.taskThoughtCacheLoading.delete(taskId);
    }
  }

  private async ensureThoughtCache(entry: InternalEntry) {
    const taskId = idToString(entry.session?.task_id);
    if (!taskId) return;
    if (entry.thoughtCacheLoaded && entry.thoughtCacheTaskId === taskId) {
      const overlayed = this.overlayThoughtCacheOnEvents(entry, entry.events);
      if (overlayed !== entry.events) {
        entry.events = overlayed;
        entry.seqSet = new Set(
          overlayed
            .map((ev) => (typeof ev.seq === "number" ? ev.seq : Number.NaN))
            .filter((seq) => Number.isFinite(seq)) as number[],
        );
        entry.updatedAtMs = Date.now();
        this.publish();
      }
      if (entry.thoughtCacheDirty) {
        void this.persistThoughtCache(entry);
      }
      return;
    }
    if (entry.thoughtCacheLoading) return;
    entry.thoughtCacheLoading = true;
    const token = (entry.thoughtCacheLoadToken += 1);
    try {
      const cache = await this.getTaskThoughtCache(taskId);
      if (entry.thoughtCacheLoadToken !== token) return;
      const sessionCache = cache.sessions?.[entry.sessionId]?.thoughts ?? {};
      entry.thoughtCacheByKey = {
        ...sessionCache,
        ...entry.thoughtCacheByKey,
      };
      entry.thoughtCacheLoaded = true;
      entry.thoughtCacheTaskId = taskId;
      const overlayed = this.overlayThoughtCacheOnEvents(entry, entry.events);
      if (overlayed !== entry.events) {
        entry.events = overlayed;
        entry.seqSet = new Set(
          overlayed
            .map((ev) => (typeof ev.seq === "number" ? ev.seq : Number.NaN))
            .filter((seq) => Number.isFinite(seq)) as number[],
        );
        entry.updatedAtMs = Date.now();
      }
      if (entry.thoughtCacheDirty) {
        void this.persistThoughtCache(entry);
      }
      this.publish();
    } finally {
      entry.thoughtCacheLoading = false;
    }
  }

  private async persistThoughtCache(entry: InternalEntry) {
    if (!entry.thoughtCacheDirty) return;
    const taskId = idToString(entry.session?.task_id);
    if (!taskId) return;
    const cache = await this.getTaskThoughtCache(taskId);
    const existingSession = cache.sessions?.[entry.sessionId];
    const mergedThoughts = {
      ...(existingSession?.thoughts ?? {}),
      ...entry.thoughtCacheByKey,
    };
    cache.sessions = {
      ...cache.sessions,
      [entry.sessionId]: {
        sessionId: entry.sessionId,
        thoughts: mergedThoughts,
      },
    };
    cache.updatedAtMs = Date.now();
    this.taskThoughtCache.set(taskId, cache);
    entry.thoughtCacheDirty = false;
    await saveTaskThoughtsV1(taskId, { sessions: cache.sessions });
  }

  private async clearTaskThoughts(taskId: string) {
    this.taskThoughtCache.delete(taskId);
    this.taskThoughtCacheLoading.delete(taskId);
    await clearTaskThoughtsV1(taskId);
    let changed = false;
    for (const entry of this.entries.values()) {
      if (idToString(entry.session?.task_id) !== taskId) continue;
      if (Object.keys(entry.thoughtCacheByKey).length > 0) {
        entry.thoughtCacheByKey = {};
        entry.thoughtCacheDirty = false;
        entry.thoughtCacheLoaded = true;
        changed = true;
      }
      if (entry.turns.length > 0) {
        const nextTurns = entry.turns.map((turn) => {
          const current = String(turn.thought_partial ?? "");
          if (!current.trim()) return turn;
          changed = true;
          return { ...turn, thought_partial: "" };
        });
        entry.turns = nextTurns;
      }
      if (entry.events.length > 0) {
        const nextEvents = entry.events.filter((ev) => ev.event_type !== "thought_chunk");
        if (nextEvents.length !== entry.events.length) {
          entry.events = nextEvents;
          entry.seqSet = new Set(
            nextEvents
              .map((ev) => (typeof ev.seq === "number" ? ev.seq : Number.NaN))
              .filter((seq) => Number.isFinite(seq)) as number[],
          );
          changed = true;
        }
      }
    }
    if (changed) {
      this.publish();
    }
  }

  private refreshSubscriptions() {
    const openIds = Array.from(this.entries.values())
      .filter((entry) => entry.refCount > 0)
      .map((entry) => entry.sessionId);
    this.snapshotStore?.setSubscribedSessionIds?.(openIds);
    const combined = mergeOrderedIds(openIds, this.activeTaskSessionIds, this.warmSessionIds);
    const next = combined;
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
    this.publish();
  }

  private overlayThoughtCacheOnEvents(entry: InternalEntry, events: SessionEvent[]): SessionEvent[] {
    if (!entry.thoughtCacheLoaded) return events;
    const cache = entry.thoughtCacheByKey;
    if (!cache || Object.keys(cache).length === 0) return events;
    const bySeq = new Map<number, SessionEvent>();
    for (const ev of events) {
      if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
    }
    let changed = false;
    for (const cached of Object.values(cache)) {
      const ev = cached.event;
      if (!ev || typeof ev.seq !== "number") continue;
      if (bySeq.has(ev.seq)) continue;
      bySeq.set(ev.seq, ev);
      changed = true;
    }
    if (!changed) return events;
    return Array.from(bySeq.values()).sort((a, b) => Number(a.seq ?? 0) - Number(b.seq ?? 0));
  }

  private overlayThoughtCacheOnTurns(entry: InternalEntry, turns: SessionTurn[]): SessionTurn[] {
    if (!entry.thoughtCacheLoaded) return turns;
    const cache = entry.thoughtCacheByKey;
    const keys = cache ? Object.keys(cache) : [];
    if (keys.length === 0) return turns;
    const turnIdsWithThoughts = new Set(keys.map((key) => key.split("|")[0]));
    let changed = false;
    const next = turns.map((turn) => {
      const turnId = idToString(turn.turn_id);
      if (!turnId || !turnIdsWithThoughts.has(turnId)) return turn;
      const current = String(turn.thought_partial ?? "");
      if (!current.trim()) return turn;
      changed = true;
      return { ...turn, thought_partial: "" };
    });
    return changed ? next : turns;
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
      const startSeq = typeof t.start_seq === "number" ? t.start_seq : Number.NaN;
      if (Number.isFinite(startSeq) && startSeq >= 0) {
        entry.startedTurnIds.add(id);
      }
      const prev = byId.get(id);
      byId.set(id, prev ? mergeTurn(prev, t) : t);
    }
    let next = Array.from(byId.values()).sort(this.compareTurnOrder.bind(this));
    const overlayed = this.overlayThoughtCacheOnTurns(entry, next);
    if (overlayed !== next) {
      next = overlayed;
      entry.updatedAtMs = Date.now();
    }
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

  private ensureEventSeq(entry: InternalEntry, event: SessionEvent): SessionEvent {
    if (typeof event.seq === "number") return event;
    const nextSeq = entry.nextTransientSeq;
    entry.nextTransientSeq = nextSeq + 1;
    return { ...event, seq: nextSeq };
  }

  private mergeEvents(entry: InternalEntry, incoming: SessionEvent[], opts?: { notify?: boolean }) {
    if (incoming.length === 0) return;
    const shouldNotify = opts?.notify ?? true;
    const existingSeqs = entry.seqSet;
    const normalizedExisting = entry.events.map((ev) => this.ensureEventSeq(entry, ev));
    const normalizedIncoming = incoming.map((ev) => this.ensureEventSeq(entry, ev));
    if (normalizedExisting !== entry.events) {
      entry.events = normalizedExisting;
    }
    const newEvents: SessionEvent[] = [];
    for (const ev of normalizedIncoming) {
      if (typeof ev.seq === "number") {
        if (!existingSeqs.has(ev.seq)) {
          newEvents.push(ev);
        }
      } else {
        newEvents.push(ev);
      }
    }
    const bySeq = new Map<number, SessionEvent>();
    for (const ev of entry.events) {
      if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
    }
    for (const ev of normalizedIncoming) {
      if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
    }
    const next = Array.from(bySeq.values()).sort((a, b) => Number(a.seq ?? 0) - Number(b.seq ?? 0));
    let trimmed = next.length > EVENT_BUFFER_LIMIT ? next.slice(-EVENT_BUFFER_LIMIT) : next;
    if (entry.thoughtCacheLoaded && Object.keys(entry.thoughtCacheByKey).length > 0) {
      const overlayed = this.overlayThoughtCacheOnEvents(entry, trimmed);
      if (overlayed !== trimmed) {
        trimmed = overlayed;
      }
    }
    entry.events = trimmed;
    entry.seqSet = new Set(
      trimmed
        .map((ev) => (typeof ev.seq === "number" ? ev.seq : Number.NaN))
        .filter((seq) => Number.isFinite(seq)) as number[],
    );
    for (const ev of normalizedIncoming) {
      const turnId = idToString(ev.turn_id);
      if (!turnId) continue;
      const seq = typeof ev.seq === "number" ? ev.seq : Number.NaN;
      if (Number.isFinite(seq) && seq >= 0) {
        entry.startedTurnIds.add(turnId);
      }
    }
    let changed = false;
    for (const ev of newEvents) {
      const turnId = idToString(ev.turn_id);
      if (isPartialEvent(ev) && (!turnId || !entry.startedTurnIds.has(turnId))) {
        continue;
      }
      this.ensureTurnFromEvent(entry, ev);
      if (this.applyEventToTurns(entry, ev, { notify: shouldNotify })) {
        changed = true;
      }
      if (this.applyQueueEvent(entry, ev)) {
        changed = true;
      }
      if (this.applyArtifactsEvent(entry, ev)) {
        changed = true;
      }
      if (this.applyGitStatusSnapshotNotice(entry, ev)) {
        changed = true;
      }
      if (this.applySubagentInvocationNotice(entry, ev)) {
        changed = true;
      }
    }
    if (changed) {
      entry.updatedAtMs = Date.now();
    }
  }

  private compareTurnOrder(a: SessionTurn, b: SessionTurn): number {
    const sa = Number(a.start_seq ?? Number.NaN);
    const sb = Number(b.start_seq ?? Number.NaN);
    if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) {
      return sa - sb;
    }
    return String(a.started_at).localeCompare(String(b.started_at));
  }

  private recordFinalThought(entry: InternalEntry, event: SessionEvent): boolean {
    if (!isFinalThoughtEvent(event)) return false;
    const key = buildThoughtCacheKey(event);
    if (!key) return false;
    const payload = normalizeFinalThoughtPayload(event.payload_json ?? {});
    if (!readThoughtFullContent(payload)) return false;
    const normalizedEvent: SessionEvent = {
      ...event,
      payload_json: payload,
    };
    const existing = entry.thoughtCacheByKey[key];
    if (existing && existing.event.seq === normalizedEvent.seq) return false;
    entry.thoughtCacheByKey = {
      ...entry.thoughtCacheByKey,
      [key]: {
        key,
        event: normalizedEvent,
        updatedAtMs: Date.now(),
      },
    };
    entry.thoughtCacheDirty = true;
    void this.persistThoughtCache(entry);
    return true;
  }

  private ensureTurnFromEvent(entry: InternalEntry, event: SessionEvent): SessionTurn | null {
    const turnId = idToString(event.turn_id);
    if (!turnId) return null;
    const existing = entry.turns.find((t) => idToString(t.turn_id) === turnId);
    if (existing) return existing;
    const createdAt = event.created_at ?? new Date().toISOString();
    const status = deriveTurnStatusFromEvent(event);
    const payload = asRecord(event.payload_json);
    const turn: SessionTurn = {
      turn_id: event.turn_id ?? turnId,
      session_id: event.session_id,
      run_id: event.run_id ?? null,
      user_message_id: readPayloadString(payload, ["user_message_id", "message_id"]) ?? null,
      status,
      start_seq: event.seq ?? null,
      end_seq: null,
      started_at: createdAt,
      updated_at: createdAt,
      assistant_partial: "",
      // Streaming-only placeholder for in-flight thought chunks.
      // Completed thought rows are emitted separately; do not persist this.
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

  private applyEventToTurns(entry: InternalEntry, event: SessionEvent, opts?: { notify?: boolean }): boolean {
    const turnId = idToString(event.turn_id);
    if (!turnId) return false;
    const idx = entry.turns.findIndex((t) => idToString(t.turn_id) === turnId);
    if (idx < 0) return false;

    const turn = entry.turns[idx] as SessionTurn & {
      assistant_partial_provider_message_id?: string | null;
      assistant_last_provider_message_id?: string | null;
      thought_partial_provider_item_id?: string | null;
    };
    const prevStatus = turn.status;
    let changed = false;
    switch (String(event.event_type)) {
      case "assistant_chunk": {
        if (!shouldRenderAssistantChunk(event)) break;
        const fragment = String(event.payload_json?.content_fragment ?? "");
        if (fragment) {
          const providerMessageId = readPayloadString(event.payload_json, ["message_id", "messageId"]);
          if (
            providerMessageId &&
            turn.assistant_partial_provider_message_id &&
            providerMessageId !== turn.assistant_partial_provider_message_id
          ) {
            turn.assistant_partial = fragment;
          } else {
            turn.assistant_partial = appendFragment(turn.assistant_partial, fragment);
          }
          if (providerMessageId) {
            turn.assistant_partial_provider_message_id = providerMessageId;
          }
          changed = true;
        }
        break;
      }
      case "thought_chunk": {
        if (!shouldRenderThoughtChunk(event)) break;
        if (isFinalThoughtEvent(event)) {
          if (this.recordFinalThought(entry, event)) {
            changed = true;
          }
        }
        break;
      }
      case "assistant_message_inserted": {
        turn.assistant_partial = "";
        const providerMessageId = readPayloadString(event.payload_json, [
          "provider_message_id",
          "providerMessageId",
        ]);
        if (providerMessageId) {
          turn.assistant_last_provider_message_id = providerMessageId;
        }
        turn.assistant_partial_provider_message_id = null;
        changed = true;
        break;
      }
      case "assistant_complete": {
        const full =
          event.payload_json?.full_content ??
          event.payload_json?.content ??
          turn.assistant_partial;
        const providerMessageId = readPayloadString(event.payload_json, ["message_id", "messageId"]);
        const shouldUpdatePartial =
          !providerMessageId || providerMessageId !== turn.assistant_last_provider_message_id;
        if (full && shouldUpdatePartial) {
          turn.assistant_partial = String(full);
        }
        if (providerMessageId) {
          turn.assistant_partial_provider_message_id = providerMessageId;
        }
        changed = true;
        break;
      }
      case "turn_queued": {
        turn.status = "queued";
        changed = true;
        break;
      }
      case "turn_started": {
        turn.status = "running";
        changed = true;
        break;
      }
      case "turn_finished": {
        const payloadStatus = readTurnStatusFromPayload(event);
        if (payloadStatus) {
          turn.status = payloadStatus;
        } else if (turn.status !== "interrupted" && turn.status !== "failed") {
          turn.status = "completed";
        }
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
        const contextWindow = readPayloadObject(event.payload_json, "context_window");
        if (contextWindow) {
          turn.metrics_json = contextWindow;
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

    const contextWindow = readPayloadObject(event.payload_json, "context_window");
    if (contextWindow) {
      turn.metrics_json = contextWindow;
      changed = true;
    }

    if (!changed) return false;
    turn.updated_at = event.created_at ?? turn.updated_at;
    entry.turns[idx] = { ...turn };
    if ((opts?.notify ?? true) && prevStatus !== "completed" && turn.status === "completed") {
      this.notifyTurnCompleted(entry);
    }
    return true;
  }

  private notifyTurnCompleted(entry: InternalEntry) {
    if (isAppInForeground()) return;
    if (!getClientSettings().desktopNotifications.turnCompleted) return;
    const title = "Turn completed";
    const body = entry.session?.title ? String(entry.session.title) : undefined;
    void sendDesktopNotification({ title, body });
  }

  private applyQueueEvent(entry: InternalEntry, event: SessionEvent): boolean {
    const messageId = idToString(readPayloadString(event.payload_json, ["message_id"]) ?? "");
    if (!messageId) return false;
    switch (String(event.event_type)) {
      case "message_queue_added": {
        const idx = entry.messages.findIndex((msg) => idToString(msg.id) === messageId);
        if (idx < 0) return false;
        const msg = entry.messages[idx];
        if (msg.delivery === "queued") return false;
        entry.messages[idx] = { ...msg, delivery: "queued" };
        entry.queue = entry.messages.filter((m) => m.delivery === "queued");
        return true;
      }
      case "message_queue_updated": {
        if (!entry.queue.some((msg) => idToString(msg.id) === messageId)) {
          return false;
        }
        entry.queue = entry.messages.filter((m) => m.delivery === "queued");
        return true;
      }
      case "message_queue_removed": {
        const prevMessages = entry.messages.length;
        const prevQueue = entry.queue.length;
        entry.messages = entry.messages.filter((msg) => idToString(msg.id) !== messageId);
        entry.queue = entry.queue.filter((msg) => idToString(msg.id) !== messageId);
        return entry.messages.length !== prevMessages || entry.queue.length !== prevQueue;
      }
      case "message_queue_promoted": {
        let changed = false;
        const idx = entry.messages.findIndex((msg) => idToString(msg.id) === messageId);
        if (idx >= 0) {
          const msg = entry.messages[idx];
          if (msg.delivery === "queued") {
            entry.messages[idx] = { ...msg, delivery: "immediate" };
            changed = true;
          }
        }
        if (entry.queue.some((msg) => idToString(msg.id) === messageId)) {
          entry.queue = entry.queue.filter((msg) => idToString(msg.id) !== messageId);
          changed = true;
        }
        if (changed) {
          entry.queue = entry.messages.filter((m) => m.delivery === "queued");
        }
        return changed;
      }
      default:
        return false;
    }
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
    if (evt.type === "archived_task_upsert") {
      const taskId = idToString(evt.task?.task?.id);
      if (taskId) {
        void this.clearTaskThoughts(taskId);
      }
    } else if (evt.type === "archived_task_delete") {
      const taskId = idToString(evt.task_id);
      if (taskId) {
        void this.clearTaskThoughts(taskId);
      }
    } else if (evt.type === "session_gap") {
      const sessionId = idToString(evt.session_id);
      if (sessionId) {
        const entry = this.entries.get(sessionId);
        if (entry) {
          this.setSessionLoadState(entry, "recovering");
          entry.error = undefined;
          entry.updatedAtMs = Date.now();
          this.publish();
        }
      }
    }
    this.replica.dispatch({ type: "workspace_event", event: evt });
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
    entry.mode = "active";
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
    this.setSessionLoadState(entry, "live");
    return true;
  }

  private resetEntryForGap(entry: InternalEntry, opts?: { skipPublish?: boolean }) {
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
    entry.startedTurnIds.clear();
    entry.turnsHydrated = false;
    entry.toolSummariesReady = false;
    entry.hasMoreTurns = true;
    this.setSessionLoadState(entry, "recovering");
    entry.updatedAtMs = Date.now();
    if (!opts?.skipPublish) {
      this.publish();
    }
  }
}
