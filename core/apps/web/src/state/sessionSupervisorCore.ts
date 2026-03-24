import {
  idToString,
  type GitStatusSummary,
  type Message,
  type ProviderOptions,
  type Session,
  type SessionEvent,
  type SessionHead,
  type SessionHeadSnapshot,
  type SessionState,
  type SessionTurn,
  type SubagentInvocation,
} from "../api/client";
import type { WorkspaceActiveSnapshotState } from "./workspaceActiveSnapshotStore";
import {
  collectWorkspaceActivePrimarySessionIds,
} from "./workspaceActiveSnapshot/projection";
import {
  type PersistedTaskThoughtsV1,
} from "./uiStateStore";
import { SessionReplicaBridge } from "./sessionReplicaBridge";
import type { SessionReplicaFreshnessState, SessionReplicaPatch } from "./sessionReplicaProtocol";
import { emitUiDiagnostic } from "./diagnosticsChannel";
import type { SessionSubscriptionCursor } from "./sessionSubscription";
import { hasModelList } from "./sessionSupervisor/eventHydration";
import {
  createInternalEntry,
  type ConnectionStatus,
  type InternalEntry,
  type OpenOptions,
  type SessionCacheEntry,
  type SessionLoadState,
  type SessionMode,
  type SessionSupportLoadErrorKey,
  type SessionSupervisorSnapshot,
} from "./sessionSupervisor/entryState";
import {
  mergeEvents,
  mergeMessages,
  mergeTurns,
  ensureTurnFromEvent,
  applyEventToTurns,
} from "./sessionSupervisor/eventProjection";
import {
  applyState,
  applyToolSummaries,
  persistHead,
  resetEntryProjectionForReplace,
  syncStateCache,
} from "./sessionSupervisor/headProjection";
import {
  evictIfNeeded,
  mapConnection,
  publish,
  setConnection,
} from "./sessionSupervisor/snapshotProjection";
import {
  applyAcpMeta,
  applyAcpMetaFromEvents,
  applyGitStatusSnapshotFromEvents,
  ensureArtifacts,
  ensureProviderOptions,
  ensureState,
  ensureSubagentInvocations,
  resolveRequestedStateRev,
} from "./sessionSupervisor/hydration";
import {
  clearTaskThoughts,
  ensureThoughtCache,
  overlayThoughtCacheOnEvents,
  overlayThoughtCacheOnTurns,
  persistThoughtCache,
  resolveEntryWorkspaceOwnerScope,
  resolveWorkspaceOwnerScope,
} from "./sessionSupervisor/thoughtCache";
import { dedupeIds, mergeTurn, sameIdList } from "./sessionSupervisor/cachePolicy";
import {
  addOptimisticQueueRemovalId,
  reconcileOptimisticOverlay,
  removeOptimisticQueuedMessage,
  removeOptimisticQueueRemovalId,
  removeOptimisticThreadMessage,
  upsertOptimisticQueuedMessage,
  upsertOptimisticThreadMessage,
} from "./sessionSupervisor/optimisticOverlay";
import {
  canSeedReplicaFromActiveSnapshot,
  seedReplicaFromActiveSnapshot,
} from "./sessionSupervisor/activeSnapshotSeed";
import {
  buildSubscribedSessions,
  emitSubscribedSessions,
  markOpenSessionsRecovering,
  refreshSubscriptions,
} from "./sessionSupervisor/subscriptions";
import { resolveSessionMode, shouldFailPendingSessionOpen } from "./sessionSupervisor/sessionMode";
import {
  EVENT_BUFFER_LIMIT,
  HEAD_LIMIT,
  MAX_CACHED_SESSIONS,
  TURN_PAGE_LIMIT,
  WARM_TTL_MS,
  isReplicaAuthority,
  shouldSkipBoundedActiveSnapshotSeed,
  toReplicaFreshness,
} from "./sessionSupervisor/config";
import { loadMoreTurnsForEntry, loadTurnToolsForEntry } from "./sessionSupervisor/historySupport";
import {
  adoptLoadedSubagentInvocationsRevision,
  adoptLoadedStateRevision,
  clearSupportLoadError,
  invalidateSupportLoadsWithoutAuthoritativeRevision,
  setSupportLoadError,
  shouldFetchSessionState,
  shouldFetchSubagentInvocations,
  syncSupportLoadsForOpenSession,
} from "./sessionSupervisor/supportLoads";
import type {
  SessionSupervisorSubscribedSessionIdsSink,
  SessionSupervisorWorkspaceEvent,
  SessionSupervisorWorkspaceSessionHeads,
  SessionSupervisorWorkspaceSnapshotState,
} from "./sessionSupervisor/workspaceInputs";

export type {
  SessionCacheEntry,
  SessionLoadState,
  SessionMode,
  SessionSupportLoadErrorKey,
  SessionSupervisorSnapshot,
} from "./sessionSupervisor/entryState";

// The daemon serializes transient events with `seq: null` (see Rust `SessionEvent` Serialize).
// We assign a stable synthetic seq in a negative JS-safe range so sorting never scrambles
// streaming partials (assistant chunks), and these events never look durable (seq >= 0).
const TRANSIENT_SEQ_START = -4503599627370496; // -(2 ** 52)

export class SessionSupervisor {
  eventBufferLimit = EVENT_BUFFER_LIMIT;
  maxCachedSessions = MAX_CACHED_SESSIONS;
  listeners = new Set<() => void>();
  snapshot: SessionSupervisorSnapshot = { connection: "idle", sessions: {} };
  entries = new Map<string, InternalEntry>();
  private replica: SessionReplicaBridge;
  private activeTaskSessionIds: string[] = [];
  private warmSessionIds: string[] = [];
  private subscribedSessionIds: string[] = [];
  private subscribedSessionIdsSink: SessionSupervisorSubscribedSessionIdsSink = null;
  providerOptionsCache = new Map<string, ProviderOptions>();
  providerOptionsInFlight = new Map<string, Promise<ProviderOptions | undefined>>();
  taskThoughtCache = new Map<string, PersistedTaskThoughtsV1>();
  taskThoughtCacheLoading = new Map<string, Promise<PersistedTaskThoughtsV1>>();
  stateCacheBySessionId = new Map<string, { state: SessionState; stateRev?: number }>();
  stateRequestsInFlight = new Map<string, Promise<void>>();
  subagentInvocationsCacheBySessionId = new Map<
    string,
    { invocations: SubagentInvocation[]; stateRev: number }
  >();
  subagentInvocationsRequestsInFlight = new Map<string, Promise<void>>();
  workspaceSnapshotState: SessionSupervisorWorkspaceSnapshotState = null;
  workspaceSessionHeadsById = new Map<string, SessionHeadSnapshot>();
  private workspaceActivePrimarySessionIds: string[] = [];
  applyAcpMeta = applyAcpMeta;
  applyAcpMetaFromEvents = applyAcpMetaFromEvents;
  applyGitStatusSnapshotFromEvents = applyGitStatusSnapshotFromEvents;
  ensureProviderOptions = ensureProviderOptions;
  ensureState = ensureState;
  resolveRequestedStateRev = resolveRequestedStateRev;
  ensureArtifacts = ensureArtifacts;
  ensureSubagentInvocations = ensureSubagentInvocations;
  resolveWorkspaceOwnerScope = resolveWorkspaceOwnerScope;
  resolveEntryWorkspaceOwnerScope = resolveEntryWorkspaceOwnerScope;
  ensureThoughtCache = ensureThoughtCache;
  persistThoughtCache = persistThoughtCache;
  clearTaskThoughts = clearTaskThoughts;
  overlayThoughtCacheOnEvents = overlayThoughtCacheOnEvents;
  overlayThoughtCacheOnTurns = overlayThoughtCacheOnTurns;
  mergeTurns = mergeTurns;
  mergeMessages = mergeMessages;
  mergeEvents = mergeEvents;
  ensureTurnFromEvent = ensureTurnFromEvent;
  applyEventToTurns = applyEventToTurns;
  applyToolSummaries = applyToolSummaries;
  applyState = applyState;
  syncStateCache = syncStateCache;
  persistHead = persistHead;
  resetEntryProjectionForReplace = resetEntryProjectionForReplace;
  evictIfNeeded = evictIfNeeded;
  mapConnection = mapConnection;
  publish = publish;
  setConnection = setConnection;
  syncSupportLoadsForOpenSession = (entry: InternalEntry) =>
    syncSupportLoadsForOpenSession(entry, {
      resolveRequestedStateRev: (nextEntry) => this.resolveRequestedStateRev(nextEntry),
      ensureState: (nextEntry) => this.ensureState(nextEntry),
      ensureSubagentInvocations: (nextEntry) => this.ensureSubagentInvocations(nextEntry),
    });
  private invalidateSupportLoadsWithoutAuthoritativeRevision = (entry: InternalEntry) =>
    invalidateSupportLoadsWithoutAuthoritativeRevision(entry, {
      resolveRequestedStateRev: (nextEntry) => this.resolveRequestedStateRev(nextEntry),
      subagentInvocationsCacheBySessionId: this.subagentInvocationsCacheBySessionId,
    });
  adoptLoadedSubagentInvocationsRevision = (entry: InternalEntry, stateRev: number) =>
    adoptLoadedSubagentInvocationsRevision(
      entry,
      stateRev,
      this.subagentInvocationsCacheBySessionId,
    );
  clearSupportLoadError = clearSupportLoadError;
  setSupportLoadError = setSupportLoadError;

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

  setSubscribedSessionIdsSink = (sink: SessionSupervisorSubscribedSessionIdsSink) => {
    this.subscribedSessionIdsSink = sink;
    this.emitSubscribedSessions();
  };

  setWorkspaceSnapshotState = (state: SessionSupervisorWorkspaceSnapshotState) => {
    this.workspaceSnapshotState = state;
    if (!state) {
      this.workspaceActivePrimarySessionIds = [];
      this.setConnection("disconnected");
      return;
    }
    const nextWorkspaceActivePrimarySessionIds = collectWorkspaceActivePrimarySessionIds(state);
    const activePrimaryMembershipChanged = !sameIdList(
      nextWorkspaceActivePrimarySessionIds,
      this.workspaceActivePrimarySessionIds,
    );
    this.workspaceActivePrimarySessionIds = nextWorkspaceActivePrimarySessionIds;
    const next = this.mapConnection(state.connection);
    this.setConnection(next);
    this.syncActiveSnapshot(state);
    if (next !== "connected") {
      this.markOpenSessionsRecovering();
    }
    this.refreshSubscriptions({ emitIfUnchanged: activePrimaryMembershipChanged });
  };

  setWorkspaceSessionHeads = (heads: SessionSupervisorWorkspaceSessionHeads) => {
    this.workspaceSessionHeadsById = new Map(Object.entries(heads));
    for (const entry of this.entries.values()) {
      this.syncSupportLoadsForOpenSession(entry);
    }
    this.emitSubscribedSessions();
  };

  handleWorkspaceEvent = (evt: SessionSupervisorWorkspaceEvent) => {
    this.ingestWorkspaceEvent(evt);
  };

  private beginSessionOpenEntry(sessionId: string, opts?: OpenOptions): InternalEntry | null {
    const id = String(sessionId ?? "").trim();
    if (!id) return null;
    const entry = this.ensureEntry(id);
    const reopeningSession = entry.refCount === 0;
    entry.refCount += 1;
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    if (opts?.mode) {
      entry.mode = opts.mode;
    }
    if (entry.error) {
      entry.error = undefined;
    }
    if (reopeningSession) {
      this.invalidateSupportLoadsWithoutAuthoritativeRevision(entry);
      const requestedStateRev = this.resolveRequestedStateRev(entry);
      if (shouldFetchSessionState(entry)) {
        entry.stateAutoLoadKey = undefined;
      }
      if (shouldFetchSubagentInvocations(entry, requestedStateRev)) {
        entry.subagentAutoLoadKey = undefined;
      }
    }
    this.setSessionLoadState(entry, "pending_hydration");
    return entry;
  }

  beginSessionOpen = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.beginSessionOpenEntry(sessionId, opts);
    if (!entry) return;
    this.refreshSubscriptions();
    this.publish();
  };
  commitSessionOpenMode = (sessionId: string, mode: SessionMode, opts?: OpenOptions) => {
    const id = String(sessionId ?? "").trim();
    if (!id) return;
    const entry = this.entries.get(id);
    if (!entry || entry.refCount <= 0) return;
    this.openSessionWithMode(id, entry, mode, opts);
    this.refreshSubscriptions();
    this.publish();
  };

  failPendingSessionOpen = (sessionId: string, message?: string) => {
    const id = String(sessionId ?? "").trim();
    if (!id) return;
    const entry = this.entries.get(id);
    if (!entry || entry.refCount <= 0) return;
    this.setFatalError(entry, message ?? `Session not found in workspace snapshot: ${id}`);
    entry.updatedAtMs = Date.now();
    this.refreshSubscriptions();
    this.publish();
  };
  openSession = (sessionId: string, opts?: OpenOptions) => {
    const id = String(sessionId ?? "").trim();
    if (!id) return () => {};
    const entry = this.beginSessionOpenEntry(id, opts);
    if (!entry) return () => this.closeSession(id, opts);
    const mode = this.resolveSessionMode(id, entry, opts?.mode);
    if (mode) {
      this.openSessionWithMode(id, entry, mode, opts);
    } else if (this.shouldFailPendingSessionOpen()) {
      this.setFatalError(entry, `Session not found in workspace snapshot: ${id}`);
      entry.updatedAtMs = Date.now();
    }
    this.refreshSubscriptions();
    this.publish();
    return () => this.closeSession(id, opts);
  };
  closeSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.entries.get(sessionId);
    if (!entry) return;
    entry.refCount = Math.max(0, entry.refCount - 1);
    entry.warmUntilMs = Date.now() + WARM_TTL_MS;
    this.replica.dispatch({ type: "close_session", sessionId });
    this.refreshSubscriptions();
    this.publish();
  };
  refreshSession = (sessionId: string, opts?: OpenOptions) => {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    const mode = this.resolveSessionMode(sessionId, entry, opts?.mode);
    if (!mode) {
      if (this.shouldFailPendingSessionOpen()) {
        this.failPendingSessionOpen(sessionId);
      }
      return;
    }
    this.replica.dispatch({
      type: "refresh_session",
      sessionId,
    });
    if (mode === "archived" || !isReplicaAuthority(entry.freshness)) {
      this.setSessionLoadState(entry, "pending_hydration");
    }
  };

  loadSessionState = (sessionId: string, opts?: { force?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    void this.ensureState(entry, opts);
  };
  loadArtifacts = (sessionId: string, opts?: { force?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    void this.ensureArtifacts(entry, opts);
  };
  loadSubagentInvocations = (sessionId: string, opts?: { force?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    void this.ensureSubagentInvocations(entry, opts);
  };
  refreshQueue = (sessionId: string) => {
    this.replica.dispatch({ type: "refresh_session", sessionId });
  };
  getSubscribedSessionIds = (): string[] => this.subscribedSessionIds.slice();

  private buildSubscribedSessions(): SessionSubscriptionCursor[] {
    return buildSubscribedSessions(this.subscribedSessionIds, this.entries, this.workspaceSessionHeadsById);
  }

  setActiveTaskSessionIds = (sessionIds: string[]) => {
    const next = dedupeIds(sessionIds);
    if (sameIdList(next, this.activeTaskSessionIds)) return;
    this.activeTaskSessionIds = next;
    this.refreshSubscriptions({ emitIfUnchanged: true });
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
    this.ensureEntry(sessionId);
    this.replica.dispatch({ type: "set_session", session });
  };
  setMessages = (sessionId: string, messages: Message[], opts?: { replace?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (opts?.replace) {
      entry.messages = [];
      entry.queue = [];
      this.bumpMessagesRev(entry);
    }
    this.mergeMessages(entry, messages);
    reconcileOptimisticOverlay(entry);
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  setTurns = (sessionId: string, turns: SessionTurn[], opts?: { replace?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (opts?.replace) {
      entry.turns = [];
      this.bumpTurnsRev(entry);
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
      this.bumpTurnsRev(entry);
      entry.turnsHydrated = true;
    }

    entry.updatedAtMs = Date.now();
    this.publish();
  };

  upsertOptimisticThreadMessage = (sessionId: string, message: Message) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (!upsertOptimisticThreadMessage(entry, message)) return;
    this.publish();
  };

  removeOptimisticThreadMessage = (sessionId: string, messageId: string) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (!removeOptimisticThreadMessage(entry, messageId)) return;
    this.publish();
  };

  upsertOptimisticQueuedMessage = (sessionId: string, message: Message) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (!upsertOptimisticQueuedMessage(entry, message)) return;
    this.publish();
  };

  removeOptimisticQueuedMessage = (sessionId: string, messageId: string) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (!removeOptimisticQueuedMessage(entry, messageId)) return;
    this.publish();
  };

  addOptimisticQueueRemovalId = (sessionId: string, messageId: string) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (!addOptimisticQueueRemovalId(entry, messageId)) return;
    this.publish();
  };

  removeOptimisticQueueRemovalId = (sessionId: string, messageId: string) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (!removeOptimisticQueueRemovalId(entry, messageId)) return;
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
    return loadMoreTurnsForEntry({
      sessionId,
      entry,
      turnPageLimit: TURN_PAGE_LIMIT,
      resolveEntryWorkspaceOwnerScope: (nextEntry) => this.resolveEntryWorkspaceOwnerScope(nextEntry),
      mergeTurns: (nextEntry, turns) => this.mergeTurns(nextEntry, turns),
      mergeMessages: (nextEntry, messages) => this.mergeMessages(nextEntry, messages),
      publish: () => this.publish(),
      persistHead: (nextEntry) => this.persistHead(nextEntry),
    });
  }

  async loadTurnTools(sessionId: string, turnId: string) {
    const entry = this.entries.get(String(sessionId));
    if (!entry) return;
    return loadTurnToolsForEntry({
      sessionId,
      turnId,
      entry,
      publish: () => this.publish(),
    });
  }

  private handleReplicaPatches = (patches: SessionReplicaPatch[]) => {
    if (!patches || patches.length === 0) return;
    let changed = false;
    let subscriptionCursorsChanged = false;
    for (const patch of patches) {
      const sessionId = String(patch.sessionId || "").trim();
      if (!sessionId) continue;
      const entry = this.ensureEntry(sessionId);
      const priorHistoryExtended = entry.historyExtended;
      let localOnlyMessages: Message[] = [];
      if (patch.op === "evict") {
        const beforeSeq = patch.data.eventsBeforeSeq;
        if (typeof beforeSeq === "number") {
          entry.events = entry.events.filter(
            (event) => typeof event.seq === "number" && event.seq >= beforeSeq,
          );
          this.bumpEventsRev(entry);
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
      const normalizedFreshness =
        patch.data.freshness === undefined ? undefined : toReplicaFreshness(patch.data.freshness);
      const shouldReplaceReplay =
        patch.op !== "replace" ||
        !isReplicaAuthority(entry.freshness) ||
        normalizedFreshness === "recovering";
      const preservedTurnsById =
        patch.op === "replace" && shouldReplaceReplay
          ? new Map(
              entry.turns
                .map((turn) => {
                  const turnId = idToString(turn.turn_id);
                  return turnId ? ([turnId, turn] as const) : null;
                })
                .filter((item): item is readonly [string, SessionTurn] => item !== null),
            )
          : null;
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
        if (shouldReplaceReplay) {
          this.resetEntryProjectionForReplace(entry, { skipPublish: true });
        }
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
      if (data.activity !== undefined) {
        entry.activity = data.activity ?? null;
      }
      if (normalizedFreshness !== undefined) {
        entry.freshness = normalizedFreshness;
      }
      if (shouldReplaceReplay && data.turns && data.turns.length > 0) {
        this.mergeTurns(entry, data.turns);
      }
      if (shouldReplaceReplay && data.messages && data.messages.length > 0) {
        this.mergeMessages(entry, data.messages);
      }
      if (localOnlyMessages.length > 0) {
        this.mergeMessages(entry, localOnlyMessages);
      }
      reconcileOptimisticOverlay(entry);
      if (shouldReplaceReplay && data.events && data.events.length > 0) {
        this.mergeEvents(entry, data.events, { notify: patch.op !== "replace" });
        this.applyAcpMetaFromEvents(entry, data.events);
      }
      if (preservedTurnsById && preservedTurnsById.size > 0 && entry.turns.length > 0) {
        let reapplied = false;
        const nextTurns = entry.turns.map((turn) => {
          const turnId = idToString(turn.turn_id);
          if (!turnId) return turn;
          const preserved = preservedTurnsById.get(turnId);
          if (!preserved) return turn;
          reapplied = true;
          return mergeTurn(preserved, turn);
        });
        if (reapplied) {
          entry.turns = nextTurns;
          this.bumpTurnsRev(entry);
        }
      }
      if (data.toolSummaries && data.toolSummaries.length > 0) {
        this.applyToolSummaries(entry, data.toolSummaries);
      }
      if (data.acpMeta) {
        this.applyAcpMeta(entry, data.acpMeta, { persist: false });
      }
      if (data.gitStatusSummary !== undefined) {
        entry.gitStatusSummary = data.gitStatusSummary ?? null;
        this.syncStateCache(entry);
      }
      if (data.artifacts) {
        entry.artifacts = data.artifacts;
        entry.artifactsFetchedAtMs = Date.now();
        entry.artifactsLoaded = true;
        entry.artifactsLoading = false;
        this.clearSupportLoadError(entry, "artifacts");
        this.syncStateCache(entry);
      }
      if (data.artifactsLoaded !== undefined) {
        entry.artifactsLoaded = data.artifactsLoaded;
        if (data.artifactsLoaded) {
          entry.artifactsLoading = false;
          this.clearSupportLoadError(entry, "artifacts");
        }
      }
      if (data.stateLoaded !== undefined) {
        entry.stateLoaded = data.stateLoaded;
        if (data.stateLoaded) {
          this.clearSupportLoadError(entry, "state");
        }
      }
      if (data.stateLoading !== undefined) {
        entry.stateLoading = data.stateLoading;
      }
      if (data.projectionRev !== undefined) {
        entry.projectionRev = data.projectionRev;
      }
      if (data.stateRev !== undefined) {
        entry.stateRev = data.stateRev;
        entry.stateAppliedRev = adoptLoadedStateRevision(
          entry.stateLoaded,
          entry.stateAppliedRev,
          data.stateRev,
        );
        this.adoptLoadedSubagentInvocationsRevision(entry, data.stateRev);
      }
      if (data.summaryCheckpoint !== undefined) {
        entry.summaryCheckpoint = data.summaryCheckpoint;
      }
      if (data.headWindow !== undefined) {
        entry.headWindow = data.headWindow;
      }
      if (data.lastEventSeq !== undefined) {
        if (entry.lastEventSeq !== data.lastEventSeq) {
          entry.lastEventSeq = data.lastEventSeq;
          if (entry.subscribed) {
            subscriptionCursorsChanged = true;
          }
        }
      }
      if (data.hasMoreTurns !== undefined) {
        const preserveHasMoreHistory = patch.op === "replace" && data.hasMoreTurns === false && priorHistoryExtended;
        if (!preserveHasMoreHistory) {
          entry.hasMoreTurns = data.hasMoreTurns;
        } else {
          entry.hasMoreTurns = true;
          entry.historyExtended = true;
        }
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
        data.projectionRev !== undefined ||
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
      reconcileOptimisticOverlay(entry);
      this.syncSupportLoadsForOpenSession(entry);
      entry.updatedAtMs = Date.now();
      changed = true;
    }
    if (changed) {
      this.publish();
    }
    if (subscriptionCursorsChanged) {
      this.emitSubscribedSessions();
    }
  };

  private ensureEntry(sessionId: string): InternalEntry {
    const existing = this.entries.get(sessionId);
    if (existing) return existing;
    const entry = createInternalEntry(sessionId, {
      transientSeqStart: TRANSIENT_SEQ_START,
      warmTtlMs: WARM_TTL_MS,
    });
    this.entries.set(sessionId, entry);
    return entry;
  }

  private openSessionWithMode(
    sessionId: string,
    entry: InternalEntry,
    mode: SessionMode,
    opts?: OpenOptions,
  ) {
    entry.mode = mode;
    const seededHead =
      mode === "active"
        ? seedReplicaFromActiveSnapshot(
            {
              workspaceSnapshotState: this.workspaceSnapshotState,
              workspaceSessionHeadsById: this.workspaceSessionHeadsById,
              dispatchSeedHead: (cmd) => this.replica.dispatch(cmd),
            },
            sessionId,
            entry,
          )
        : false;
    const shouldSkipCache =
      (entry.turnsHydrated ||
        entry.messages.length > 0 ||
        entry.events.length > 0 ||
        typeof entry.lastEventSeq === "number") ||
      entry.freshness !== "bootstrap";
    this.replica.dispatch({
      type: "open_session",
      sessionId,
      force: opts?.force,
      silent: opts?.silent,
      skipCache: shouldSkipCache,
      skipBoundedBootstrapCache: mode === "active",
      forceHydrate: entry.freshness === "recovering" || entry.loadState === "recovering",
      hydrateIfNeeded:
        mode === "archived" ||
        !isReplicaAuthority(entry.freshness) ||
        entry.loadState === "recovering",
    });
    if (mode === "archived") {
      this.setSessionLoadState(entry, "pending_hydration");
      this.syncSupportLoadsForOpenSession(entry);
      return;
    }
    if (seededHead || entry.turnsHydrated || entry.messages.length > 0 || entry.events.length > 0) {
      this.setSessionLoadState(entry, "live");
      this.syncSupportLoadsForOpenSession(entry);
      return;
    }
    this.setSessionLoadState(entry, "pending_hydration");
    this.syncSupportLoadsForOpenSession(entry);
  }

  resolveSessionMode(
    sessionId: string,
    entry?: InternalEntry,
    explicitMode?: SessionMode,
  ): SessionMode | null {
    return resolveSessionMode.call(this, sessionId, entry, explicitMode);
  }

  private shouldFailPendingSessionOpen() {
    return shouldFailPendingSessionOpen(this.workspaceSnapshotState);
  }

  setSessionLoadState(entry: InternalEntry, next: SessionLoadState) {
    if (entry.loadState === next) return;
    entry.loadState = next;
  }

  bumpTurnsRev(entry: InternalEntry) {
    entry.turnsRev += 1;
  }

  bumpMessagesRev(entry: InternalEntry) {
    entry.messagesRev += 1;
  }

  bumpEventsRev(entry: InternalEntry) {
    entry.eventsRev += 1;
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
    markOpenSessionsRecovering({ entries: this.entries, emitSubscribedSessions: () => this.emitSubscribedSessions(), publish: () => this.publish() });
  }

  private refreshSubscriptions(opts?: { emitIfUnchanged?: boolean }) {
    refreshSubscriptions({
      entries: this.entries,
      activeTaskSessionIds: this.activeTaskSessionIds,
      warmSessionIds: this.warmSessionIds,
      subscribedSessionIds: this.subscribedSessionIds,
      setSubscribedSessionIds: (next) => { this.subscribedSessionIds = next; },
      emitSubscribedSessions: () => this.emitSubscribedSessions(),
      ensureEntry: (sessionId) => this.ensureEntry(sessionId),
      publish: () => this.publish(),
    }, opts);
  }

  private emitSubscribedSessions() {
    emitSubscribedSessions(this.subscribedSessionIdsSink, this.buildSubscribedSessions());
  }

  private ingestWorkspaceEvent(evt: SessionSupervisorWorkspaceEvent) {
    let changed = false;
    let subscriptionCursorsChanged = false;
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
          changed = true;
          if (entry.subscribed) {
            subscriptionCursorsChanged = true;
          }
        }
      }
    }
    if (changed) {
      this.publish();
    }
    if (subscriptionCursorsChanged) {
      this.emitSubscribedSessions();
    }
    this.replica.dispatch({ type: "workspace_event", event: evt });
  }

  private syncActiveSnapshot(state: WorkspaceActiveSnapshotState) {
    for (const taskId of state.activeIds) {
      const item = state.tasksById[taskId];
      const head = item?.primarySessionHead;
      if (!head) continue;
      const sessionId = idToString(head.session?.id);
      if (!sessionId) continue;
      const entry = this.ensureEntry(sessionId);
      if (!canSeedReplicaFromActiveSnapshot(entry, { allowRecoveringRefresh: true })) continue;
      if (shouldSkipBoundedActiveSnapshotSeed(entry, head)) continue;
      this.replica.dispatch({ type: "seed_head", sessionId, head });
    }
  }
}
