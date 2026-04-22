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
import { type PersistedTaskThoughtsV1 } from "./uiStateStore";
import { SessionReplicaBridge } from "./sessionReplicaBridge";
import type {
  SessionReplicaCommand,
  SessionReplicaFreshnessState,
  SessionReplicaPatch,
} from "./sessionReplicaProtocol";
import { emitUiDiagnostic } from "./diagnosticsChannel";
import type { SessionSubscriptionCursor } from "./sessionSubscription";
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
import {
  buildThoughtCacheKey,
  isFinalThoughtEvent,
  normalizeFinalThoughtPayload,
  readThoughtFullContent,
} from "./sessionSupervisor/thoughtProjection";
import {
  dedupeIds,
  reconcileActivityFromTurns,
  reconcileLatestTurnInterruptedFromActivity,
  sameIdList,
} from "./sessionSupervisor/cachePolicy";
import {
  addOptimisticQueueRemovalId,
  reconcileOptimisticOverlay,
  removeOptimisticQueuedMessage,
  removeOptimisticQueueRemovalId,
  removeOptimisticThreadMessage,
  upsertOptimisticQueuedMessage,
  upsertOptimisticThreadMessage,
} from "./sessionSupervisor/optimisticOverlay";
import { applyReplicaPatches } from "./sessionSupervisor/replicaPatchApply";
import {
  buildSubscribedSessions,
  emitSubscribedSessions,
  markOpenSessionsRecovering,
  refreshSubscriptions,
} from "./sessionSupervisor/subscriptions";
import type { SessionActivityState } from "@ctx/types";
import {
  beginSessionOpen as beginSessionLifecycleOpen,
  closeSession as closeSessionLifecycle,
  commitSessionOpenMode as commitSessionLifecycleOpenMode,
  dropSessionEntry as dropSessionLifecycleEntry,
  failPendingSessionOpen as failPendingLifecycleOpen,
  openSession as openSessionLifecycle,
  refreshSession as refreshSessionLifecycle,
} from "./sessionSupervisor/sessionLifecycle";
import { seedReplicaFromActiveSnapshot } from "./sessionSupervisor/activeSnapshotSeed";
import { resolveSessionMode, shouldFailPendingSessionOpen } from "./sessionSupervisor/sessionMode";
import {
  EVENT_BUFFER_LIMIT,
  HEAD_LIMIT,
  MAX_CACHED_SESSIONS,
  TURN_PAGE_LIMIT,
  WARM_TTL_MS,
} from "./sessionSupervisor/config";
import { loadMoreTurnsForEntry, loadTurnToolsForEntry } from "./sessionSupervisor/historySupport";
import {
  adoptLoadedSubagentInvocationsRevision,
  clearSupportLoadError,
  invalidateSupportLoadsWithoutAuthoritativeRevision,
  setSupportLoadError,
  syncSupportLoadsForOpenSession,
} from "./sessionSupervisor/supportLoads";
import type {
  SessionSupervisorSubscribedSessionIdsSink,
  SessionSupervisorWorkspaceEvent,
  SessionSupervisorWorkspaceSessionHeads,
  SessionSupervisorWorkspaceSnapshotState,
} from "./sessionSupervisor/workspaceInputs";
import {
  ingestWorkspaceEvent as ingestWorkspaceAuthorityEvent,
  setWorkspaceSessionHeads as setWorkspaceAuthoritySessionHeads,
  setWorkspaceSnapshotState as setWorkspaceAuthoritySnapshotState,
  syncActiveSnapshot as syncWorkspaceAuthorityActiveSnapshot,
  upsertWorkspaceSessionHead as upsertWorkspaceAuthoritySessionHead,
} from "./sessionSupervisor/workspaceAuthority";

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
  replicaDispatch = (cmd: SessionReplicaCommand) => this.replica.dispatch(cmd);
  applyAcpMeta = applyAcpMeta;
  applyAcpMetaFromEvents = applyAcpMetaFromEvents;
  applyGitStatusSnapshotFromEvents = applyGitStatusSnapshotFromEvents;
  ensureProviderOptions = ensureProviderOptions;
  ensureState = ensureState;
  resolveRequestedStateRev = resolveRequestedStateRev;
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
      invalidateStateRequest: (nextEntry) => {
        nextEntry.support.stateFetchToken += 1;
        nextEntry.support.stateLoading = false;
        this.stateRequestsInFlight.delete(nextEntry.sessionId);
      },
      invalidateSubagentInvocationsRequest: (nextEntry) => {
        nextEntry.support.subagentInvocationsFetchToken += 1;
        nextEntry.support.subagentInvocationsLoading = false;
        this.subagentInvocationsRequestsInFlight.delete(nextEntry.sessionId);
      },
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
    setWorkspaceAuthoritySnapshotState(this.createWorkspaceAuthorityHost(), state);
  };

  setWorkspaceSessionHeads = (heads: SessionSupervisorWorkspaceSessionHeads) => {
    setWorkspaceAuthoritySessionHeads(this.createWorkspaceAuthorityHost(), heads);
  };

  upsertWorkspaceSessionHead = (sessionId: string, head: SessionHeadSnapshot) => {
    upsertWorkspaceAuthoritySessionHead(this.createWorkspaceAuthorityHost(), sessionId, head);
  };

  handleWorkspaceEvent = (evt: SessionSupervisorWorkspaceEvent) => {
    ingestWorkspaceAuthorityEvent(this.createWorkspaceAuthorityHost(), evt);
  };

  beginSessionOpen = (sessionId: string, opts?: OpenOptions) => {
    beginSessionLifecycleOpen(this.createSessionLifecycleHost(), sessionId, opts);
  };
  commitSessionOpenMode = (sessionId: string, mode: SessionMode, opts?: OpenOptions) => {
    commitSessionLifecycleOpenMode(this.createSessionLifecycleHost(), sessionId, mode, opts);
  };

  failPendingSessionOpen = (sessionId: string, message?: string) => {
    failPendingLifecycleOpen(this.createSessionLifecycleHost(), sessionId, message);
  };
  openSession = (sessionId: string, opts?: OpenOptions) => {
    return openSessionLifecycle(this.createSessionLifecycleHost(), sessionId, opts);
  };
  closeSession = (sessionId: string, opts?: OpenOptions) => {
    closeSessionLifecycle(this.createSessionLifecycleHost(), sessionId, opts);
  };
  refreshSession = (sessionId: string, opts?: OpenOptions) => {
    refreshSessionLifecycle(this.createSessionLifecycleHost(), sessionId, opts);
  };

  loadSessionState = (sessionId: string, opts?: { force?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    void this.ensureState(entry, {
      ...opts,
      allowEntryStateRevFallback: entry.refCount <= 0,
    });
  };
  loadSubagentInvocations = (sessionId: string, opts?: { force?: boolean }) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    void this.ensureSubagentInvocations(entry, {
      ...opts,
      allowEntryStateRevFallback: entry.refCount <= 0,
    });
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
    const previous = this.activeTaskSessionIds;
    this.activeTaskSessionIds = next;
    for (const sessionId of next) {
      if (previous.includes(sessionId)) continue;
      const entry = this.ensureEntry(sessionId);
      seedReplicaFromActiveSnapshot(
        {
          workspaceSnapshotState: this.workspaceSnapshotState,
          workspaceSessionHeadsById: this.workspaceSessionHeadsById,
          dispatchSeedHead: (cmd) => this.replicaDispatch(cmd),
        },
        sessionId,
        entry,
        { allowRecoveringRefresh: true, allowRepairReplace: true },
      );
    }
    this.refreshSubscriptions({ emitIfUnchanged: true });
  };
  setWarmSessionIds = (sessionIds: string[]) => {
    const next = dedupeIds(sessionIds);
    if (sameIdList(next, this.warmSessionIds)) return;
    this.warmSessionIds = next;
    for (const sessionId of next) {
      const entry = this.ensureEntry(sessionId);
      seedReplicaFromActiveSnapshot(
        {
          workspaceSnapshotState: this.workspaceSnapshotState,
          workspaceSessionHeadsById: this.workspaceSessionHeadsById,
          dispatchSeedHead: (cmd) => this.replicaDispatch(cmd),
        },
        sessionId,
        entry,
        { allowRecoveringRefresh: true, allowRepairReplace: true },
      );
    }
    this.refreshSubscriptions({ emitIfUnchanged: true });
  };
  setSession = (session: Session) => {
    const sessionId = idToString(session.id);
    if (!sessionId) return;
    this.ensureEntry(sessionId);
    this.replica.dispatch({ type: "set_session", session });
  };
  setSessionActivity = (sessionId: string, activity: SessionActivityState | null) => {
    const id = String(sessionId || "").trim();
    if (!id) return;
    const entry = this.ensureEntry(id);
    const nextActivity = activity ?? null;
    const normalizedActivity = reconcileActivityFromTurns(nextActivity, entry.turns);
    if (entry.activity === normalizedActivity) {
      return;
    }
    entry.activity = normalizedActivity;
    if (reconcileLatestTurnInterruptedFromActivity(entry.turns, nextActivity)) {
      this.bumpTurnsRev(entry);
      const normalizedAfterInterrupt = reconcileActivityFromTurns(entry.activity, entry.turns);
      if (normalizedAfterInterrupt !== entry.activity) {
        entry.activity = normalizedAfterInterrupt;
      }
    }
    entry.activity = reconcileActivityFromTurns(entry.activity, entry.turns);
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
      if (turns.length === 0) {
        entry.activity = null;
      }
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

    const normalizedActivity = reconcileActivityFromTurns(entry.activity, entry.turns);
    if (normalizedActivity !== entry.activity) {
      entry.activity = normalizedActivity;
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
    dropSessionLifecycleEntry(this.createSessionLifecycleHost(), sessionId);
  };

  setDiff = (sessionId: string, diff: string) => {
    const entry = this.ensureEntry(sessionId);
    entry.support.diff = diff;
    entry.updatedAtMs = Date.now();
    this.publish();
  };

  setGitStatusSummary = (sessionId: string, summary: GitStatusSummary | null) => {
    const entry = this.ensureEntry(sessionId);
    entry.support.gitStatusSummary = summary;
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
      normalizeActivity: (nextEntry) => {
        const normalizedActivity = reconcileActivityFromTurns(nextEntry.activity, nextEntry.turns);
        if (normalizedActivity !== nextEntry.activity) {
          nextEntry.activity = normalizedActivity;
        }
      },
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
    const { changed, subscriptionCursorsChanged } = applyReplicaPatches(
      {
        workspaceSnapshotState: this.workspaceSnapshotState,
        getEntry: (sessionId) => this.entries.get(sessionId),
        ensureEntry: (sessionId) => this.ensureEntry(sessionId),
        resolveSessionMode: (sessionId, entry, explicitMode) =>
          this.resolveSessionMode(sessionId, entry, explicitMode),
        resetEntryProjectionForReplace: (entry, opts) => this.resetEntryProjectionForReplace(entry, opts),
        setSessionLoadState: (entry, next) => this.setSessionLoadState(entry, next),
        setFatalError: (entry, message) => this.setFatalError(entry, message),
        applyAcpMetaFromEvents: (entry, events) => this.applyAcpMetaFromEvents(entry, events),
        applyGitStatusSnapshotFromEvents: (entry, events) =>
          this.applyGitStatusSnapshotFromEvents(entry, events),
        syncStateCache: (entry) => this.syncStateCache(entry),
        clearSupportLoadError: (entry, key) => this.clearSupportLoadError(entry, key),
        adoptLoadedSubagentInvocationsRevision: (entry, stateRev) =>
          this.adoptLoadedSubagentInvocationsRevision(entry, stateRev),
        ensureProviderOptions: (entry) => this.ensureProviderOptions(entry),
        ensureSubagentInvocations: (entry, opts) => this.ensureSubagentInvocations(entry, opts),
        syncSupportLoadsForOpenSession: (entry) => this.syncSupportLoadsForOpenSession(entry),
        bumpTurnsRev: (entry) => this.bumpTurnsRev(entry),
      },
      patches,
    );
    for (const patch of patches) {
      if (patch.op === "evict") continue;
      const sessionId = String(patch.sessionId || "").trim();
      if (!sessionId) continue;
      const entry = this.entries.get(sessionId);
      if (!entry) continue;
      if (patch.data.session) {
        void this.ensureThoughtCache(entry);
      }
      if (Array.isArray(patch.data.events) && patch.data.events.length > 0) {
        let thoughtChanged = false;
        for (const event of patch.data.events) {
          if (!isFinalThoughtEvent(event)) continue;
          const key = buildThoughtCacheKey(event);
          if (!key) continue;
          const payload = normalizeFinalThoughtPayload(event.payload_json ?? {});
          if (!readThoughtFullContent(payload)) continue;
          const normalizedEvent: SessionEvent = {
            ...event,
            payload_json: payload,
          };
          const existing = entry.thoughtCacheByKey[key];
          if (existing && existing.event.seq === normalizedEvent.seq) continue;
          entry.thoughtCacheByKey = {
            ...entry.thoughtCacheByKey,
            [key]: {
              key,
              event: normalizedEvent,
              updatedAtMs: Date.now(),
            },
          };
          entry.thoughtCacheDirty = true;
          thoughtChanged = true;
        }
        if (thoughtChanged) {
          void this.persistThoughtCache(entry);
        }
      }
      reconcileOptimisticOverlay(entry);
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

  private createSessionLifecycleHost() {
    return {
      entries: this.entries,
      getWorkspaceSnapshotState: () => this.workspaceSnapshotState,
      getWorkspaceSessionHeadsById: () => this.workspaceSessionHeadsById,
      getActiveTaskSessionIds: () => this.activeTaskSessionIds,
      setActiveTaskSessionIds: (sessionIds: string[]) => {
        this.activeTaskSessionIds = sessionIds;
      },
      getWarmSessionIds: () => this.warmSessionIds,
      setWarmSessionIds: (sessionIds: string[]) => {
        this.warmSessionIds = sessionIds;
      },
      getSubscribedSessionIds: () => this.subscribedSessionIds,
      setSubscribedSessionIds: (sessionIds: string[]) => {
        this.subscribedSessionIds = sessionIds;
      },
      ensureEntry: (sessionId: string) => this.ensureEntry(sessionId),
      invalidateSupportLoadsWithoutAuthoritativeRevision: (entry: InternalEntry) =>
        this.invalidateSupportLoadsWithoutAuthoritativeRevision(entry),
      resolveRequestedStateRev: (entry: InternalEntry) => this.resolveRequestedStateRev(entry),
      setSessionLoadState: (entry: InternalEntry, next: SessionLoadState) =>
        this.setSessionLoadState(entry, next),
      setFatalError: (entry: InternalEntry, message: string) => this.setFatalError(entry, message),
      syncSupportLoadsForOpenSession: (entry: InternalEntry) => this.syncSupportLoadsForOpenSession(entry),
      resolveSessionMode: (sessionId: string, entry?: InternalEntry, explicitMode?: SessionMode) =>
        this.resolveSessionMode(sessionId, entry, explicitMode),
      shouldFailPendingSessionOpen: () => this.shouldFailPendingSessionOpen(),
      refreshSubscriptions: (opts?: { emitIfUnchanged?: boolean }) => this.refreshSubscriptions(opts),
      publish: () => this.publish(),
      replicaDispatch: (cmd: SessionReplicaCommand) => this.replicaDispatch(cmd),
    };
  }

  private createWorkspaceAuthorityHost() {
    return {
      getWorkspaceSnapshotState: () => this.workspaceSnapshotState,
      setWorkspaceSnapshotState: (state: SessionSupervisorWorkspaceSnapshotState) => {
        this.workspaceSnapshotState = state;
      },
      getWorkspaceSessionHeadsById: () => this.workspaceSessionHeadsById,
      setWorkspaceSessionHeadsById: (heads: Map<string, SessionHeadSnapshot>) => {
        this.workspaceSessionHeadsById = heads;
      },
      getWorkspaceActivePrimarySessionIds: () => this.workspaceActivePrimarySessionIds,
      setWorkspaceActivePrimarySessionIds: (sessionIds: string[]) => {
        this.workspaceActivePrimarySessionIds = sessionIds;
      },
      mapConnection: (connection: WorkspaceActiveSnapshotState["connection"]) =>
        this.mapConnection(connection),
      setConnection: (next: ConnectionStatus) => this.setConnection(next),
      syncActiveSnapshot: (state: WorkspaceActiveSnapshotState) => this.syncActiveSnapshot(state),
      markOpenSessionsRecovering: () => this.markOpenSessionsRecovering(),
      rehydrateRecoveringOpenSessions: () => this.rehydrateRecoveringOpenSessions(),
      refreshSubscriptions: (opts?: { emitIfUnchanged?: boolean }) => this.refreshSubscriptions(opts),
      emitSubscribedSessions: () => this.emitSubscribedSessions(),
      clearTaskThoughts: (taskId: string) => this.clearTaskThoughts(taskId),
      publish: () => this.publish(),
      syncSupportLoadsForOpenSession: (entry: InternalEntry) => this.syncSupportLoadsForOpenSession(entry),
      replicaDispatch: (cmd: SessionReplicaCommand) => this.replicaDispatch(cmd),
      entries: this.entries,
      ensureEntry: (sessionId: string) => this.ensureEntry(sessionId),
      setSessionLoadState: (entry: InternalEntry, next: SessionLoadState) =>
        this.setSessionLoadState(entry, next),
    };
  }

  private createWorkspaceActiveSyncHost() {
    return {
      ensureEntry: (sessionId: string) => this.ensureEntry(sessionId),
      getWorkspaceSessionHeadsById: () => this.workspaceSessionHeadsById,
      replicaDispatch: (cmd: SessionReplicaCommand) => this.replicaDispatch(cmd),
    };
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

  private rehydrateRecoveringOpenSessions() {
    let changed = false;
    for (const entry of this.entries.values()) {
      if (entry.refCount <= 0) continue;
      if (entry.loadState !== "recovering" && entry.freshness !== "recovering") continue;
      entry.error = undefined;
      entry.updatedAtMs = Date.now();
      changed = true;
      this.replicaDispatch({
        type: "hydrate_session_head",
        sessionId: entry.sessionId,
        force: true,
        silent: true,
      });
    }
    if (changed) {
      this.publish();
    }
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

  onEvictSession = (sessionId: string) => {
    this.replicaDispatch({ type: "drop_session", sessionId });
  };

  private syncActiveSnapshot(state: WorkspaceActiveSnapshotState) {
    syncWorkspaceAuthorityActiveSnapshot(this.createWorkspaceActiveSyncHost(), state);
  }
}
