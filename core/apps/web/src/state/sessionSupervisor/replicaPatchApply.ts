import {
  idToString,
  type Message,
  type SessionEvent,
  type SessionTurn,
  type SessionTurnTool,
} from "../../api/client";
import {
  mergeSessionEvents,
  mergeSessionMessages,
  mergeSessionToolSummaries,
  mergeSessionTurns,
} from "../sessionHeadState";
import { hasModelList } from "./eventHydration";
import { mergeTurnStatus } from "./cachePolicy";
import { resolveTurnAnalyticsMetadata } from "./turnAnalyticsMetadata";
import { replayTurnStartEffectsFromTurns } from "./turnStartEffects";
import { replayTurnOutcomeEffectsFromTurns } from "./turnOutcomeEffects";
import {
  hasSessionReplicaRecoveryData,
  resolveReplicaReadyLoadState,
  shouldReplayReplicaReplace,
} from "./authorityPolicy";
import type { InternalEntry } from "./entryState";
import type { SessionReplicaPatch } from "../sessionReplicaProtocol";
import { adoptLoadedStateRevision } from "./supportLoads";

export type SessionSupervisorReplicaPatchHost = {
  ensureEntry(sessionId: string): InternalEntry;
  resolveSessionMode(
    sessionId: string,
    entry?: InternalEntry,
    explicitMode?: InternalEntry["mode"],
  ): InternalEntry["mode"] | null;
  resetEntryProjectionForReplace(entry: InternalEntry, opts?: { skipPublish?: boolean }): void;
  setSessionLoadState(entry: InternalEntry, next: InternalEntry["loadState"]): void;
  setFatalError(entry: InternalEntry, message: string): void;
  applyAcpMetaFromEvents(entry: InternalEntry, events: SessionEvent[]): boolean;
  applyGitStatusSnapshotFromEvents(entry: InternalEntry, events: SessionEvent[]): boolean;
  syncStateCache(entry: InternalEntry): void;
  clearSupportLoadError(entry: InternalEntry, key: "state" | "artifacts" | "subagentInvocations"): void;
  adoptLoadedSubagentInvocationsRevision(entry: InternalEntry, stateRev: number): void;
  ensureProviderOptions(entry: InternalEntry): Promise<void>;
  ensureSubagentInvocations(entry: InternalEntry, opts?: { force?: boolean }): Promise<void>;
  syncSupportLoadsForOpenSession(entry: InternalEntry): void;
};

function haveSameArrayRefs<T>(previous: readonly T[], next: readonly T[]): boolean {
  if (previous === next) return true;
  if (previous.length !== next.length) return false;
  for (let index = 0; index < previous.length; index += 1) {
    if (previous[index] !== next[index]) return false;
  }
  return true;
}

function haveSameRecordRefs<T>(previous: Record<string, T>, next: Record<string, T>): boolean {
  if (previous === next) return true;
  const previousKeys = Object.keys(previous);
  const nextKeys = Object.keys(next);
  if (previousKeys.length !== nextKeys.length) return false;
  for (const key of previousKeys) {
    if (previous[key] !== next[key]) return false;
  }
  return true;
}

const repairReplaceIsCoveredByEntry = (
  entry: Pick<InternalEntry, "turns" | "messages">,
  data: Pick<Exclude<SessionReplicaPatch, { op: "evict" }>["data"], "turns" | "messages">,
): boolean => {
  const incomingTurns = Array.isArray(data.turns) ? data.turns : [];
  const incomingMessages = Array.isArray(data.messages) ? data.messages : [];

  const entryTurnIds = new Set(entry.turns.map((turn) => idToString(turn.turn_id)).filter(Boolean));
  const entryMessageIds = new Set(entry.messages.map((message) => idToString(message.id)).filter(Boolean));

  return (
    incomingTurns.every((turn) => entryTurnIds.has(idToString(turn.turn_id))) &&
    incomingMessages.every((message) => entryMessageIds.has(idToString(message.id)))
  );
};

const rebuildSeqAndStartState = (entry: InternalEntry) => {
  entry.seqSet = new Set(
    entry.events
      .map((event) => (typeof event.seq === "number" ? event.seq : Number.NaN))
      .filter((seq) => Number.isFinite(seq)) as number[],
  );
  entry.startedTurnIds = new Set(
    entry.turns
      .map((turn) => {
        const turnId = idToString(turn.turn_id);
        const startSeq = typeof turn.start_seq === "number" ? turn.start_seq : Number.NaN;
        return turnId && Number.isFinite(startSeq) && startSeq >= 0 ? turnId : "";
      })
      .filter(Boolean),
  );
  for (const event of entry.events) {
    const turnId = idToString(event.turn_id);
    const seq = typeof event.seq === "number" ? event.seq : Number.NaN;
    if (turnId && Number.isFinite(seq) && seq >= 0) {
      entry.startedTurnIds.add(turnId);
    }
  }
};

const summaryOnlyTool = (
  summary: InternalEntry["toolSummaries"][number],
): SessionTurnTool & { summary_only: boolean } => ({
  session_id: summary.session_id,
  tool_call_id: summary.tool_call_id,
  turn_id: summary.turn_id,
  tool_kind: summary.tool_kind ?? null,
  provider_tool_name: summary.provider_tool_name ?? null,
  title: summary.title ?? null,
  subtitle: summary.subtitle ?? null,
  status: summary.status ?? null,
  input_json: summary.input_preview ?? null,
  output_text: null,
  order_seq: summary.order_seq,
  input_truncated: summary.input_truncated ?? null,
  input_original_bytes: summary.input_original_bytes ?? null,
  output_truncated: summary.output_truncated ?? null,
  output_original_bytes: summary.output_original_bytes ?? null,
  first_event_seq: summary.first_event_seq ?? null,
  created_at: summary.created_at,
  updated_at: summary.updated_at,
  summary_only: true,
});

const applyCanonicalToolSummaries = (
  entry: InternalEntry,
  summaries: InternalEntry["toolSummaries"],
  opts?: { resetByTurn?: boolean },
) => {
  const support = entry.support;
  let changed = entry.toolSummaries !== summaries;
  entry.toolSummaries = summaries;
  support.toolSummariesReady = true;
  const resetByTurn = opts?.resetByTurn === true;
  const currentByTurn = support.turnToolsByTurnId;
  let nextByTurn: Record<string, SessionTurnTool[]> = resetByTurn ? {} : currentByTurn;
  let toolsByTurnChanged = resetByTurn && Object.keys(currentByTurn).length > 0;

  for (const summary of summaries) {
    const turnId = idToString(summary.turn_id);
    if (!turnId) continue;
    if (support.turnToolsHydratedByTurnId[turnId]) continue;
    const existing = nextByTurn[turnId] ?? [];
    const key = String(summary.tool_call_id ?? "").trim();
    if (!key) continue;
    if (existing.some((tool) => String(tool.tool_call_id ?? "").trim() === key)) continue;
    if (!resetByTurn && nextByTurn === currentByTurn) {
      nextByTurn = { ...currentByTurn };
    }
    nextByTurn[turnId] = [...existing, summaryOnlyTool(summary)];
    toolsByTurnChanged = true;
    if (support.turnToolsHydratedByTurnId[turnId] === undefined) {
      support.turnToolsHydratedByTurnId[turnId] = false;
    }
  }

  if (toolsByTurnChanged) {
    support.turnToolsByTurnId = nextByTurn;
    changed = true;
  }
  return changed;
};

const preserveLocalQueuedMessages = (
  currentMessages: Message[],
  incomingMessages: Message[],
): Message[] => {
  const incomingIds = new Set(
    incomingMessages.map((message) => idToString(message.id)).filter((id): id is string => Boolean(id)),
  );
  return currentMessages.filter((message) => {
    const messageId = idToString(message.id);
    if (!messageId || incomingIds.has(messageId)) return false;
    return message.delivery === "queued";
  });
};

const preserveLocalUserMessageAnchors = (
  previousTurns: SessionTurn[],
  previousMessages: Message[],
  nextTurns: SessionTurn[],
  nextMessages: Message[],
): { turns: SessionTurn[]; messages: Message[] } => {
  if (previousTurns.length === 0 || previousMessages.length === 0 || nextTurns.length === 0) {
    return { turns: nextTurns, messages: nextMessages };
  }

  const previousTurnById = new Map(
    previousTurns
      .map((turn) => {
        const turnId = idToString(turn.turn_id);
        return turnId ? ([turnId, turn] as const) : null;
      })
      .filter((item): item is readonly [string, SessionTurn] => item !== null),
  );
  const previousMessageById = new Map(
    previousMessages
      .map((message) => {
        const messageId = idToString(message.id);
        return messageId ? ([messageId, message] as const) : null;
      })
      .filter((item): item is readonly [string, Message] => item !== null),
  );
  const nextMessageIds = new Set(
    nextMessages.map((message) => idToString(message.id)).filter((messageId): messageId is string => Boolean(messageId)),
  );

  let repairedTurns = nextTurns;
  let repairedMessages = nextMessages;
  const preservedMessages: Message[] = [];

  nextTurns.forEach((turn, index) => {
    const turnId = idToString(turn.turn_id);
    if (!turnId) return;
    const nextUserMessageId = idToString(turn.user_message_id ?? "");
    if (nextUserMessageId && nextMessageIds.has(nextUserMessageId)) return;

    const previousTurn = previousTurnById.get(turnId);
    const previousUserMessageId = idToString(previousTurn?.user_message_id ?? "");
    if (!previousTurn || !previousUserMessageId) return;

    const previousUserMessage = previousMessageById.get(previousUserMessageId);
    if (!previousUserMessage || previousUserMessage.role !== "user") return;
    if (idToString(previousUserMessage.turn_id ?? "") !== turnId) return;

    if (repairedTurns === nextTurns) {
      repairedTurns = nextTurns.slice();
    }
    repairedTurns[index] =
      repairedTurns[index]?.user_message_id === previousTurn.user_message_id
        ? repairedTurns[index]!
        : { ...turn, user_message_id: previousTurn.user_message_id };

    if (!nextMessageIds.has(previousUserMessageId)) {
      nextMessageIds.add(previousUserMessageId);
      preservedMessages.push(previousUserMessage);
    }
  });

  if (preservedMessages.length > 0) {
    repairedMessages = mergeSessionMessages(repairedMessages, preservedMessages);
  }

  return {
    turns: repairedTurns,
    messages: repairedMessages,
  };
};

const preserveMonotonicTurns = (
  previousTurns: SessionTurn[],
  nextTurns: SessionTurn[],
): SessionTurn[] => {
  if (previousTurns.length === 0 || nextTurns.length === 0) return nextTurns;
  const previousById = new Map(
    previousTurns
      .map((turn) => {
        const turnId = idToString(turn.turn_id);
        return turnId ? ([turnId, turn] as const) : null;
      })
      .filter((item): item is readonly [string, SessionTurn] => item !== null),
  );
  let changed = false;
  const merged = nextTurns.map((turn) => {
    const turnId = idToString(turn.turn_id);
    if (!turnId) return turn;
    const previous = previousById.get(turnId);
    if (!previous) return turn;
    const nextStatus = mergeTurnStatus(previous.status, turn.status);
    const nextTurn: SessionTurn = {
      ...turn,
      status: nextStatus,
      end_seq: turn.end_seq ?? previous.end_seq,
      tool_total: Math.max(previous.tool_total ?? 0, turn.tool_total ?? 0),
      // `tool_pending`/`tool_running` are live counters, not cumulative totals.
      // Authoritative replace/repair patches must be able to clear them.
      tool_pending: turn.tool_pending,
      tool_running: turn.tool_running,
      tool_completed: Math.max(previous.tool_completed ?? 0, turn.tool_completed ?? 0),
      tool_failed: Math.max(previous.tool_failed ?? 0, turn.tool_failed ?? 0),
      metrics_json: turn.metrics_json ?? previous.metrics_json,
    };
    changed =
      changed ||
      nextTurn.status !== turn.status ||
      nextTurn.end_seq !== turn.end_seq ||
      nextTurn.tool_total !== turn.tool_total ||
      nextTurn.tool_pending !== turn.tool_pending ||
      nextTurn.tool_running !== turn.tool_running ||
      nextTurn.tool_completed !== turn.tool_completed ||
      nextTurn.tool_failed !== turn.tool_failed ||
      nextTurn.metrics_json !== turn.metrics_json;
    return nextTurn;
  });
  return changed ? merged : nextTurns;
};

const applyCanonicalTranscriptPatch = (
  host: SessionSupervisorReplicaPatchHost,
  entry: InternalEntry,
  patch: Exclude<SessionReplicaPatch, { op: "evict" }>,
  normalizedFreshness: InternalEntry["freshness"] | undefined,
): boolean => {
  const data = patch.data;
  const support = entry.support;
  const replaceMode = patch.op === "replace" ? data.replaceMode ?? null : null;
  const shouldApplyReplace = patch.op !== "replace" || shouldReplayReplicaReplace({
    entry,
    patch,
    normalizedFreshness,
  });
  const preserveCoveredHistoryOnReplace =
    patch.op === "replace" &&
    repairReplaceIsCoveredByEntry(entry, data) &&
    (entry.historyExtended || replaceMode === "repair_replace");
  const localQueuedMessages =
    Array.isArray(data.messages) ? preserveLocalQueuedMessages(entry.messages, data.messages) : [];
  const previousTurns = entry.turns;
  const previousMessages = entry.messages;
  let nextTurnsForAnalytics: SessionTurn[] | null = null;
  let changed = false;

  if (patch.op === "replace" && shouldApplyReplace && !preserveCoveredHistoryOnReplace) {
    host.resetEntryProjectionForReplace(entry, { skipPublish: true });
    changed = true;
  }

  const shouldCopyCanonicalTranscript = patch.op !== "replace" || shouldApplyReplace;

  if (shouldCopyCanonicalTranscript && !preserveCoveredHistoryOnReplace) {
    let nextTurns = entry.turns;
    if (Array.isArray(data.turns)) {
      nextTurns = preserveMonotonicTurns(previousTurns, data.turns);
    }
    let nextMessages = entry.messages;
    if (Array.isArray(data.messages)) {
      nextMessages = mergeSessionMessages(data.messages, localQueuedMessages);
    }
    if (Array.isArray(data.turns) || Array.isArray(data.messages)) {
      const repaired = preserveLocalUserMessageAnchors(previousTurns, previousMessages, nextTurns, nextMessages);
      nextTurns = repaired.turns;
      nextMessages = repaired.messages;
    }
    if (!haveSameArrayRefs(entry.turns, nextTurns)) {
      entry.turns = nextTurns;
      entry.turnsRev = data.turnsRev ?? (entry.turnsRev + 1);
      nextTurnsForAnalytics = entry.turns;
      changed = true;
    }
    if (!haveSameArrayRefs(entry.messages, nextMessages)) {
      entry.messages = nextMessages;
      entry.messagesRev = data.messagesRev ?? (entry.messagesRev + 1);
      entry.queue = entry.messages.filter((message) => message.delivery === "queued");
      changed = true;
    }
    if (Array.isArray(data.events)) {
      if (!haveSameArrayRefs(entry.events, data.events)) {
        entry.events = data.events;
        entry.eventsRev = data.eventsRev ?? (entry.eventsRev + 1);
        changed = true;
      }
    }
    if (changed) {
      rebuildSeqAndStartState(entry);
    }
    if (data.turnsHydrated !== undefined) {
      if (entry.turnsHydrated !== data.turnsHydrated) {
        entry.turnsHydrated = data.turnsHydrated;
        changed = true;
      }
    } else if ((Array.isArray(data.turns) || Array.isArray(data.messages) || Array.isArray(data.events)) && !entry.turnsHydrated) {
      entry.turnsHydrated = true;
      changed = true;
    }
    if (Array.isArray(data.toolSummaries)) {
      changed =
        applyCanonicalToolSummaries(entry, data.toolSummaries, {
          resetByTurn: patch.op === "replace",
        }) || changed;
    }
  } else if (shouldCopyCanonicalTranscript && preserveCoveredHistoryOnReplace) {
    let nextTurns = entry.turns;
    if (Array.isArray(data.turns)) {
      nextTurns = preserveMonotonicTurns(entry.turns, mergeSessionTurns(entry.turns, data.turns));
    }
    let nextMessages = entry.messages;
    if (Array.isArray(data.messages)) {
      nextMessages = mergeSessionMessages(entry.messages, data.messages);
    }
    if (Array.isArray(data.turns) || Array.isArray(data.messages)) {
      const repaired = preserveLocalUserMessageAnchors(previousTurns, previousMessages, nextTurns, nextMessages);
      nextTurns = repaired.turns;
      nextMessages = repaired.messages;
    }
    if (!haveSameArrayRefs(entry.turns, nextTurns)) {
      entry.turns = nextTurns;
      entry.turnsRev = data.turnsRev ?? (entry.turnsRev + 1);
      nextTurnsForAnalytics = entry.turns;
      changed = true;
    }
    if (!haveSameArrayRefs(entry.messages, nextMessages)) {
      entry.messages = nextMessages;
      entry.messagesRev = data.messagesRev ?? (entry.messagesRev + 1);
      entry.queue = entry.messages.filter((message) => message.delivery === "queued");
      changed = true;
    }
    if (Array.isArray(data.events)) {
      const nextEvents = mergeSessionEvents(entry.events, data.events);
      if (!haveSameArrayRefs(entry.events, nextEvents)) {
        entry.events = nextEvents;
        entry.eventsRev = data.eventsRev ?? (entry.eventsRev + 1);
        changed = true;
      }
    }
    if (changed) {
      rebuildSeqAndStartState(entry);
    }
    if (data.turnsHydrated !== undefined) {
      if (entry.turnsHydrated !== data.turnsHydrated) {
        entry.turnsHydrated = data.turnsHydrated;
        changed = true;
      }
    } else if ((Array.isArray(data.turns) || Array.isArray(data.messages) || Array.isArray(data.events)) && !entry.turnsHydrated) {
      entry.turnsHydrated = true;
      changed = true;
    }
    if (Array.isArray(data.toolSummaries)) {
      const nextSummaries = mergeSessionToolSummaries(entry.toolSummaries, data.toolSummaries, entry.turns);
      changed = applyCanonicalToolSummaries(entry, nextSummaries) || changed;
    }
  }
  if (data.assistantStreamingByTurnId) {
    if (!haveSameRecordRefs(entry.assistantStreamingByTurnId, data.assistantStreamingByTurnId)) {
      entry.assistantStreamingByTurnId = data.assistantStreamingByTurnId;
      entry.assistantStreamingRev = data.assistantStreamingRev ?? (entry.assistantStreamingRev + 1);
      changed = true;
    }
  }

  if (data.session) {
    if (entry.session !== data.session) {
      entry.session = data.session;
      changed = true;
    }
    if (!entry.mode) {
      const resolvedMode = host.resolveSessionMode(entry.sessionId, entry);
      if (resolvedMode) {
        entry.mode = resolvedMode;
        changed = true;
      }
    }
  }
  if (data.activity !== undefined) {
    const nextActivity = data.activity ?? null;
    if (entry.activity !== nextActivity) {
      entry.activity = nextActivity;
      changed = true;
    }
  }
  if (normalizedFreshness !== undefined) {
    if (entry.freshness !== normalizedFreshness) {
      entry.freshness = normalizedFreshness;
      changed = true;
    }
  }
  if (data.projectionRev !== undefined) {
    if (entry.projectionRev !== data.projectionRev) {
      entry.projectionRev = data.projectionRev;
      changed = true;
    }
  }
  if (data.stateRev !== undefined) {
    entry.stateRev = data.stateRev;
    support.stateAppliedRev = adoptLoadedStateRevision(
      support.stateLoaded,
      support.stateAppliedRev,
      data.stateRev,
    );
    host.adoptLoadedSubagentInvocationsRevision(entry, data.stateRev);
  }
  if (data.summaryCheckpoint !== undefined) {
    if (entry.summaryCheckpoint !== data.summaryCheckpoint) {
      entry.summaryCheckpoint = data.summaryCheckpoint;
      changed = true;
    }
  }
  if (data.headWindow !== undefined) {
    if (entry.headWindow !== data.headWindow) {
      entry.headWindow = data.headWindow;
      changed = true;
    }
  }
  if (data.lastEventSeq !== undefined) {
    if (entry.lastEventSeq !== data.lastEventSeq) {
      entry.lastEventSeq = data.lastEventSeq;
      changed = true;
    }
  }
  if (data.hasMoreTurns !== undefined) {
    const preserveHasMoreHistory =
      patch.op === "replace" && data.hasMoreTurns === false && entry.historyExtended;
    const nextHasMoreTurns = preserveHasMoreHistory ? true : data.hasMoreTurns;
    if (entry.hasMoreTurns !== nextHasMoreTurns) {
      entry.hasMoreTurns = nextHasMoreTurns;
      changed = true;
    }
    if (preserveHasMoreHistory) {
      entry.historyExtended = true;
    }
  }
  if (Array.isArray(data.turns) && entry.turns.length > 0) {
    const nextOldestTurnSeq = entry.turns[0]?.start_seq;
    if (typeof nextOldestTurnSeq === "number" && entry.oldestTurnSeq !== nextOldestTurnSeq) {
      entry.oldestTurnSeq = nextOldestTurnSeq;
      changed = true;
    }
  }
  // Only emit analytics for live incoming deltas (append). Historical replaces
  // are for hydration/backfill and must not count toward current-day analytics.
  if (nextTurnsForAnalytics && patch.op === "append") {
    const analytics = resolveTurnAnalyticsMetadata(entry.session, entry.sessionId);
    replayTurnStartEffectsFromTurns({
      ...analytics,
      previousTurns,
      nextTurns: nextTurnsForAnalytics,
    });
    replayTurnOutcomeEffectsFromTurns({
      ...analytics,
      previousTurns,
      nextTurns: nextTurnsForAnalytics,
    });
  }
  return changed;
};

export const applyReplicaPatches = (
  host: SessionSupervisorReplicaPatchHost,
  patches: SessionReplicaPatch[],
): { changed: boolean; subscriptionCursorsChanged: boolean } => {
  if (!patches || patches.length === 0) {
    return { changed: false, subscriptionCursorsChanged: false };
  }

  let changed = false;
  let subscriptionCursorsChanged = false;
  for (const patch of patches) {
    const sessionId = String(patch.sessionId || "").trim();
    if (!sessionId) continue;
    const entry = host.ensureEntry(sessionId);
    const priorHistoryExtended = entry.historyExtended;

    if (patch.op === "evict") {
      const beforeSeq = patch.data.eventsBeforeSeq;
      if (typeof beforeSeq === "number") {
        entry.events = entry.events.filter(
          (event) => typeof event.seq === "number" && event.seq >= beforeSeq,
        );
        rebuildSeqAndStartState(entry);
        entry.eventsRev += 1;
        entry.updatedAtMs = Date.now();
        changed = true;
      }
      continue;
    }

    const normalizedFreshness =
      patch.data.freshness === undefined ? undefined : patch.data.freshness === "authoritative"
        ? "replica"
        : patch.data.freshness;

    let entryChanged = applyCanonicalTranscriptPatch(host, entry, patch, normalizedFreshness);

    const data = patch.data;
    if (Array.isArray(data.events) && data.events.length > 0) {
      host.applyAcpMetaFromEvents(entry, data.events);
      host.applyGitStatusSnapshotFromEvents(entry, data.events);
    }
    if (data.gitStatusSummary !== undefined) {
      if (entry.support.gitStatusSummary !== (data.gitStatusSummary ?? null)) {
        entry.support.gitStatusSummary = data.gitStatusSummary ?? null;
        entryChanged = true;
      }
      host.syncStateCache(entry);
    }
    if (data.artifacts) {
      entry.support.artifacts = data.artifacts;
      entry.support.artifactsFetchedAtMs = Date.now();
      entry.support.artifactsLoaded = true;
      entry.support.artifactsLoading = false;
      host.clearSupportLoadError(entry, "artifacts");
      host.syncStateCache(entry);
    }
    if (data.artifactsLoaded !== undefined) {
      entry.support.artifactsLoaded = data.artifactsLoaded;
      if (data.artifactsLoaded) {
        entry.support.artifactsLoading = false;
        host.clearSupportLoadError(entry, "artifacts");
      }
    }
    if (data.stateLoaded !== undefined) {
      entry.support.stateLoaded = data.stateLoaded;
      if (data.stateLoaded) {
        host.clearSupportLoadError(entry, "state");
      }
    }
    if (data.stateLoading !== undefined) {
      if (entry.support.stateLoading !== data.stateLoading) {
        entry.support.stateLoading = data.stateLoading;
        entryChanged = true;
      }
    }
    if (data.loading !== undefined) {
      const prevLoading = entry.loading;
      const prevLoadState = entry.loadState;
      entry.loading = data.loading;
      if (data.loading && entry.loadState !== "live") {
        host.setSessionLoadState(entry, "pending_hydration");
      }
      if (entry.loading !== prevLoading || entry.loadState !== prevLoadState) {
        entryChanged = true;
      }
    }
    if (data.error !== undefined) {
      const prevError = entry.error;
      const prevLoadState = entry.loadState;
      if (data.error) {
        host.setFatalError(entry, data.error);
      } else {
        entry.error = undefined;
        if (entry.loadState === "fatal") {
          host.setSessionLoadState(entry, "pending_hydration");
        }
      }
      if (entry.error !== prevError || entry.loadState !== prevLoadState) {
        entryChanged = true;
      }
    } else if (hasSessionReplicaRecoveryData(data)) {
      const prevLoadState = entry.loadState;
      entry.error = undefined;
      host.setSessionLoadState(entry, resolveReplicaReadyLoadState(entry));
      if (entry.loadState !== prevLoadState) {
        entryChanged = true;
      }
    }
    if (data.subagentNotice) {
      void host.ensureSubagentInvocations(entry, { force: true });
    }
    if (!entry.acpModels || !hasModelList(entry.acpModels)) {
      void host.ensureProviderOptions(entry);
    }

    if (patch.op === "replace" && priorHistoryExtended && data.hasMoreTurns === false) {
      entry.hasMoreTurns = true;
      entry.historyExtended = true;
      entryChanged = true;
    }

    if (data.lastEventSeq !== undefined && entry.subscribed) {
      subscriptionCursorsChanged = true;
    }
    host.syncSupportLoadsForOpenSession(entry);
    if (entryChanged) {
      entry.updatedAtMs = Date.now();
      changed = true;
    }
  }

  return { changed, subscriptionCursorsChanged };
};
