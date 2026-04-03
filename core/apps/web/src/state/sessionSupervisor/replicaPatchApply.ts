import {
  idToString,
  type Message,
  type SessionEvent,
  type SessionTurn,
  type SessionTurnTool,
} from "../../api/client";
import { mergeSessionMessages } from "../sessionHeadState";
import { hasModelList } from "./eventHydration";
import { mergeTurnStatus } from "./cachePolicy";
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
  entry.toolSummaries = summaries;
  support.toolSummariesReady = true;
  if (opts?.resetByTurn) {
    support.turnToolsByTurnId = {};
  }

  const nextByTurn: Record<string, SessionTurnTool[]> = opts?.resetByTurn
    ? {}
    : { ...support.turnToolsByTurnId };

  for (const summary of summaries) {
    const turnId = idToString(summary.turn_id);
    if (!turnId) continue;
    if (support.turnToolsHydratedByTurnId[turnId]) continue;
    const existing = nextByTurn[turnId] ?? [];
    const key = String(summary.tool_call_id ?? "").trim();
    if (!key) continue;
    if (existing.some((tool) => String(tool.tool_call_id ?? "").trim() === key)) continue;
    nextByTurn[turnId] = [...existing, summaryOnlyTool(summary)];
    if (support.turnToolsHydratedByTurnId[turnId] === undefined) {
      support.turnToolsHydratedByTurnId[turnId] = false;
    }
  }

  support.turnToolsByTurnId = nextByTurn;
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
      tool_pending: Math.max(previous.tool_pending ?? 0, turn.tool_pending ?? 0),
      tool_running: Math.max(previous.tool_running ?? 0, turn.tool_running ?? 0),
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
) => {
  const data = patch.data;
  const support = entry.support;
  const replaceMode = patch.op === "replace" ? data.replaceMode ?? null : null;
  const shouldApplyReplace = patch.op !== "replace" || shouldReplayReplicaReplace({
    entry,
    patch,
    normalizedFreshness,
  });
  const preserveCoveredHistoryOnRepair =
    replaceMode === "repair_replace" && repairReplaceIsCoveredByEntry(entry, data);
  const localQueuedMessages =
    Array.isArray(data.messages) ? preserveLocalQueuedMessages(entry.messages, data.messages) : [];
  const previousTurns = entry.turns;

  if (patch.op === "replace" && shouldApplyReplace && !preserveCoveredHistoryOnRepair) {
    host.resetEntryProjectionForReplace(entry, { skipPublish: true });
  }

  const shouldCopyCanonicalTranscript = patch.op !== "replace" || shouldApplyReplace;

  if (shouldCopyCanonicalTranscript && !preserveCoveredHistoryOnRepair) {
    if (Array.isArray(data.turns)) {
      entry.turns = preserveMonotonicTurns(previousTurns, data.turns);
      entry.turnsRev = data.turnsRev ?? (entry.turnsRev + 1);
    }
    if (Array.isArray(data.messages)) {
      entry.messages = mergeSessionMessages(data.messages, localQueuedMessages);
      entry.messagesRev = data.messagesRev ?? (entry.messagesRev + 1);
      entry.queue = entry.messages.filter((message) => message.delivery === "queued");
    }
    if (Array.isArray(data.events)) {
      entry.events = data.events;
      entry.eventsRev = data.eventsRev ?? (entry.eventsRev + 1);
    }
    rebuildSeqAndStartState(entry);
    if (data.turnsHydrated !== undefined) {
      entry.turnsHydrated = data.turnsHydrated;
    } else if (Array.isArray(data.turns) || Array.isArray(data.messages) || Array.isArray(data.events)) {
      entry.turnsHydrated = true;
    }
    if (Array.isArray(data.toolSummaries)) {
      applyCanonicalToolSummaries(entry, data.toolSummaries, {
        resetByTurn: patch.op === "replace",
      });
    }
  }
  if (data.assistantStreamingByTurnId) {
    entry.assistantStreamingByTurnId = data.assistantStreamingByTurnId;
    entry.assistantStreamingRev = data.assistantStreamingRev ?? (entry.assistantStreamingRev + 1);
  }

  if (data.session) {
    entry.session = data.session;
    if (!entry.mode) {
      const resolvedMode = host.resolveSessionMode(entry.sessionId, entry);
      if (resolvedMode) {
        entry.mode = resolvedMode;
      }
    }
  }
  if (data.activity !== undefined) {
    entry.activity = data.activity ?? null;
  }
  if (normalizedFreshness !== undefined) {
    entry.freshness = normalizedFreshness;
  }
  if (data.projectionRev !== undefined) {
    entry.projectionRev = data.projectionRev;
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
    entry.summaryCheckpoint = data.summaryCheckpoint;
  }
  if (data.headWindow !== undefined) {
    entry.headWindow = data.headWindow;
  }
  if (data.lastEventSeq !== undefined) {
    entry.lastEventSeq = data.lastEventSeq;
  }
  if (data.hasMoreTurns !== undefined) {
    const preserveHasMoreHistory =
      patch.op === "replace" && data.hasMoreTurns === false && entry.historyExtended;
    entry.hasMoreTurns = preserveHasMoreHistory ? true : data.hasMoreTurns;
    if (preserveHasMoreHistory) {
      entry.historyExtended = true;
    }
  }
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

    applyCanonicalTranscriptPatch(host, entry, patch, normalizedFreshness);

    const data = patch.data;
    if (Array.isArray(data.events) && data.events.length > 0) {
      host.applyAcpMetaFromEvents(entry, data.events);
      host.applyGitStatusSnapshotFromEvents(entry, data.events);
    }
    if (data.gitStatusSummary !== undefined) {
      entry.support.gitStatusSummary = data.gitStatusSummary ?? null;
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
      entry.support.stateLoading = data.stateLoading;
    }
    if (data.loading !== undefined) {
      entry.loading = data.loading;
      if (data.loading && entry.loadState !== "live") {
        host.setSessionLoadState(entry, "pending_hydration");
      }
    }
    if (data.error !== undefined) {
      if (data.error) {
        host.setFatalError(entry, data.error);
      } else {
        entry.error = undefined;
        if (entry.loadState === "fatal") {
          host.setSessionLoadState(entry, "pending_hydration");
        }
      }
    } else if (hasSessionReplicaRecoveryData(data)) {
      entry.error = undefined;
      host.setSessionLoadState(entry, resolveReplicaReadyLoadState(entry));
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
    }

    if (data.lastEventSeq !== undefined && entry.subscribed) {
      subscriptionCursorsChanged = true;
    }
    host.syncSupportLoadsForOpenSession(entry);
    entry.updatedAtMs = Date.now();
    changed = true;
  }

  return { changed, subscriptionCursorsChanged };
};
