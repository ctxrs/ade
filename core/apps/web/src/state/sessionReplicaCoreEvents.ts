import type {
  Message,
  SessionEvent,
  SessionHead,
  SessionHeadDelta,
  SessionHeadSnapshot,
  SessionTurn,
  SessionTurnToolSummary,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import { idToString } from "../api/client";
import { clearSessionHeadV1, clearSessionHistoryPagesV1 } from "./uiStateStore";
import {
  applyReplicaTranscriptEvent,
  ensureReplicaEventSeq,
  isStreamOnlyAssistantChunk,
  mergeReplicaEventsIntoEntry,
  mergeReplicaMessagesIntoEntry,
  mergeReplicaTurnsIntoEntry,
  rebuildReplicaTranscriptAuxState,
} from "./sessionReplicaTranscript";
import { isBoundedSessionHead, shouldRepairSessionHeadReplace } from "./sessionHeadRepair";
import {
  isTerminalTurnStatus,
  reconcileActivityInterruptedFromTurns,
  reconcileLatestTurnInterruptedFromActivity,
} from "./sessionSupervisor/cachePolicy";
import type {
  SessionReplicaAppendMode,
  SessionReplicaConfig,
  SessionReplicaData,
  SessionReplicaFreshnessEvent,
} from "./sessionReplicaProtocol";
import {
  buildCanonicalReplicaPatch,
  buildStreamDeltaReplicaPatch,
  buildStreamingOverlayReplicaPatch,
} from "./sessionReplicaPatches";
import type {
  SessionReplicaApplyHeadOptions,
  SessionReplicaEntry,
} from "./sessionReplicaCoreSupport";
import {
  normalizeReplicaId,
  resolveFinalReplicaDeltaTurnId,
  SHOULD_EMIT_REPLICA_DEV_DIAGNOSTICS,
} from "./sessionReplicaCoreSupport";

type SessionReplicaHydrateOptions = {
  force?: boolean;
  silent?: boolean;
  emitOp?: "append" | "replace";
};

const changedItemsById = <T>(
  previous: readonly T[],
  next: readonly T[],
  getId: (item: T) => string,
): T[] => {
  const previousById = new Map<string, T>();
  for (const item of previous) {
    const id = getId(item);
    if (id) previousById.set(id, item);
  }
  return next.filter((item) => {
    const id = getId(item);
    return !id || previousById.get(id) !== item;
  });
};

const changedTurnsById = (
  previous: readonly SessionTurn[],
  next: readonly SessionTurn[],
): SessionTurn[] => changedItemsById(previous, next, (turn) => idToString(turn.turn_id));

const changedMessagesById = (
  previous: readonly Message[],
  next: readonly Message[],
): Message[] => changedItemsById(previous, next, (message) => idToString(message.id));

const removedMessageIdsById = (
  previous: readonly Message[],
  next: readonly Message[],
): string[] => {
  const nextIds = new Set(next.map((message) => idToString(message.id)).filter(Boolean));
  return previous
    .map((message) => idToString(message.id))
    .filter((id) => id && !nextIds.has(id));
};

const changedToolSummariesById = (
  previous: readonly SessionTurnToolSummary[],
  next: readonly SessionTurnToolSummary[],
): SessionTurnToolSummary[] =>
  changedItemsById(previous, next, (summary) => String(summary.tool_call_id ?? "").trim());

const TERMINAL_VISIBLE_EVENT_TYPES = new Set([
  "assistant_complete",
  "assistant_message_inserted",
  "done",
  "error",
  "turn_finished",
  "turn_interrupted",
]);

const findReplicaTurn = (
  turns: readonly SessionTurn[],
  turnId: string,
): SessionTurn | null => turns.find((turn) => idToString(turn.turn_id) === turnId) ?? null;

const hasReplicaEventSeq = (
  events: readonly SessionEvent[],
  seq: number | null,
): boolean => seq !== null && events.some((event) => event.seq === seq);

const hasReplicaMessage = (
  messages: readonly Message[],
  messageId: string,
): boolean => Boolean(messageId) && messages.some((message) => idToString(message.id) === messageId);

const hasReplicaToolSummary = (
  summaries: readonly SessionTurnToolSummary[],
  toolCallId: string,
): boolean =>
  Boolean(toolCallId) &&
  summaries.some((summary) => String(summary.tool_call_id ?? "").trim() === toolCallId);

const readEventPayloadString = (
  payload: SessionEvent["payload_json"],
  keys: readonly string[],
): string => {
  if (!payload) return "";
  for (const key of keys) {
    const value = payload[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return "";
};

const staleDeltaHasVisibleForwardProgress = (
  entry: SessionReplicaEntry,
  delta: SessionHeadDelta,
  event: SessionEvent | null,
  toolSummaries: readonly SessionTurnToolSummary[],
): boolean => {
  const messageId = normalizeReplicaId(delta.message?.id ?? "");
  if (messageId && !hasReplicaMessage(entry.messages, messageId)) return true;

  const deltaTurnId = normalizeReplicaId(delta.turn?.turn_id ?? "");
  if (delta.turn && deltaTurnId && isTerminalTurnStatus(delta.turn.status)) {
    const existingTurn = findReplicaTurn(entry.turns, deltaTurnId);
    if (!existingTurn || !isTerminalTurnStatus(existingTurn.status)) return true;
  }

  const eventType = String(event?.event_type ?? "");
  const eventSeq = typeof event?.seq === "number" ? event.seq : null;
  if (event && TERMINAL_VISIBLE_EVENT_TYPES.has(eventType) && !hasReplicaEventSeq(entry.events, eventSeq)) {
    const eventTurnId = normalizeReplicaId(event.turn_id ?? "");
    const existingTurn = eventTurnId ? findReplicaTurn(entry.turns, eventTurnId) : null;
    if (eventType === "assistant_message_inserted") {
      const eventMessageId = normalizeReplicaId(
        readEventPayloadString(event.payload_json, ["message_id", "messageId"]),
      );
      if (eventMessageId && !hasReplicaMessage(entry.messages, eventMessageId)) return true;
    } else if (eventType === "assistant_complete") {
      if (
        eventTurnId &&
        (!existingTurn ||
          !isTerminalTurnStatus(existingTurn.status) ||
          Boolean(entry.assistantStreamingByTurnId[eventTurnId]))
      ) {
        return true;
      }
    } else if (!existingTurn || !isTerminalTurnStatus(existingTurn.status)) {
      return true;
    }
  }

  return toolSummaries.some((summary) => {
    const toolCallId = String(summary.tool_call_id ?? "").trim();
    if (!toolCallId || hasReplicaToolSummary(entry.toolSummaries, toolCallId)) return false;
    const turn = findReplicaTurn(entry.turns, normalizeReplicaId(summary.turn_id ?? ""));
    return Boolean(turn && !isTerminalTurnStatus(turn.status));
  });
};

export type SessionReplicaEventHost = {
  entries: Map<string, SessionReplicaEntry>;
  config: SessionReplicaConfig;
  gapAlertedSessionIds: Set<string>;
  gapRepairBaselineBySessionId: Map<string, { lastEventSeq: number | null }>;
  ensureEntry(sessionId: string): SessionReplicaEntry;
  applyHead(
    entry: SessionReplicaEntry,
    head: SessionHead | SessionHeadSnapshot,
    emitOp?: "append" | "replace",
    opts?: SessionReplicaApplyHeadOptions,
  ): void;
  emitAppendPatch(
    sessionId: string,
    data: SessionReplicaData & { appendMode: SessionReplicaAppendMode },
  ): void;
  emitEvictPatch(sessionId: string, data: { eventsBeforeSeq?: number }): void;
  hydrateSessionHead(sessionId: string, opts?: SessionReplicaHydrateOptions): Promise<void>;
  persistHead(entry: SessionReplicaEntry): Promise<void>;
  emitFreshnessEvent(event: SessionReplicaFreshnessEvent): void;
};

export const handleSessionReplicaWorkspaceEvent = (
  host: SessionReplicaEventHost,
  evt: WorkspaceActiveSnapshotEvent,
): void => {
  const evtType = (evt as { type?: string }).type;
  if (evtType === "session_head_delta" || evtType === "session_delta") {
    const delta = (evt as { delta?: SessionHeadDelta }).delta;
    if (!delta) return;
    const turnId = resolveFinalReplicaDeltaTurnId(delta);
    if (turnId) {
      host.emitFreshnessEvent({
        type: "final_delta_received",
        sessionId: normalizeReplicaId(delta.session_id),
        turnId,
        emittedAtMs:
          typeof delta.emitted_at_ms === "number" && Number.isFinite(delta.emitted_at_ms)
            ? delta.emitted_at_ms
            : null,
        lastEventSeq: delta.last_event_seq,
      });
    }
    applySessionReplicaHeadDelta(host, delta);
    return;
  }

  if (evtType === "session_head_seed") {
    const head = (evt as { head?: SessionHeadSnapshot }).head;
    const sessionId = normalizeReplicaId(head?.session?.id ?? "");
    if (!head || !sessionId) return;
    const entry = host.ensureEntry(sessionId);
    host.applyHead(entry, head, "replace", {
      replaceMode:
        isBoundedSessionHead(head) || shouldRepairSessionHeadReplace(entry, head)
          ? "repair_replace"
          : "authoritative_replace",
      freshness: "authoritative",
    });
    return;
  }

  if (evtType !== "session_gap") return;
  const sessionId = normalizeReplicaId((evt as { session_id?: unknown }).session_id);
  const afterSeq =
    typeof (evt as { after_seq?: number }).after_seq === "number"
      ? (evt as { after_seq?: number }).after_seq
      : undefined;
  if (!sessionId) return;
  host.emitFreshnessEvent({
    type: "gap_recovery_started",
    sessionId,
    reason:
      typeof (evt as { reason?: unknown }).reason === "string"
        ? String((evt as { reason?: unknown }).reason)
        : null,
  });
  const previousEntry = host.entries.get(sessionId);
  const previousLastEventSeq =
    typeof previousEntry?.lastEventSeq === "number" ? previousEntry.lastEventSeq : null;
  const gapAfterSeq = typeof afterSeq === "number" ? afterSeq : null;
  const requiredSeqCandidates = [previousLastEventSeq, gapAfterSeq].filter(
    (value): value is number => typeof value === "number" && Number.isFinite(value),
  );
  const requiredLastEventSeq =
    requiredSeqCandidates.length > 0 ? Math.max(...requiredSeqCandidates) : null;
  host.gapRepairBaselineBySessionId.set(sessionId, {
    lastEventSeq: requiredLastEventSeq,
  });

  if (typeof window !== "undefined" && SHOULD_EMIT_REPLICA_DEV_DIAGNOSTICS) {
    const prevSeq = host.entries.get(sessionId)?.lastEventSeq;
    const message = [
      "ctx session_gap detected.",
      `session_id=${sessionId}`,
      `after_seq=${afterSeq ?? "unknown"}`,
      `last_event_seq=${prevSeq ?? "unknown"}`,
    ].join("\n");
    if (!host.gapAlertedSessionIds.has(sessionId)) {
      host.gapAlertedSessionIds.add(sessionId);
      try {
        window.alert(message);
      } catch {
        // ignore
      }
    }
    // eslint-disable-next-line no-console
    console.warn(message);
  }

  void clearSessionHeadV1(sessionId).catch(() => {});
  void clearSessionHistoryPagesV1(sessionId).catch(() => {});
  const entry = host.entries.get(sessionId);
  if (!entry) return;
  entry.hydrated = false;
  entry.freshness = "recovering";
  host.emitAppendPatch(sessionId, {
    freshness: "recovering",
    error: null,
    appendMode: "metadata_update",
  });
  void host.hydrateSessionHead(sessionId, {
    force: true,
    emitOp: "replace",
  }).catch(() => {});
};

const applySessionReplicaHeadDelta = (
  host: SessionReplicaEventHost,
  delta: SessionHeadDelta,
): void => {
  const sessionId = normalizeReplicaId(delta.session_id);
  if (!sessionId) return;

  const rawEvent = delta.event ?? null;
  const rawStreamOnlyAssistantChunk = rawEvent ? isStreamOnlyAssistantChunk(rawEvent) : false;
  const toolSummaries = Array.isArray(delta.tool_summaries) ? delta.tool_summaries : [];
  const streamOnlyCandidate =
    rawStreamOnlyAssistantChunk &&
    !delta.turn &&
    !delta.message &&
    toolSummaries.length === 0;
  const existingEntry = host.entries.get(sessionId);
  if (streamOnlyCandidate && !existingEntry) return;

  const entry = existingEntry ?? host.ensureEntry(sessionId);
  const incomingSeq =
    typeof delta.last_event_seq === "number"
      ? delta.last_event_seq
      : typeof rawEvent?.seq === "number"
        ? rawEvent.seq
        : null;
  const existingSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : null;
  const incomingProjectionRev =
    typeof delta.projection_rev === "number" ? delta.projection_rev : null;
  const existingProjectionRev =
    typeof entry.projectionRev === "number" ? entry.projectionRev : null;
  let staleDelta = false;
  if (incomingSeq !== null && existingSeq !== null && incomingSeq < existingSeq) {
    host.emitFreshnessEvent({
      type: "projection_or_seq_regression",
      sessionId,
      dimension: "last_event_seq",
      incoming: incomingSeq,
      existing: existingSeq,
    });
    staleDelta = true;
  }
  if (
    incomingProjectionRev !== null &&
    existingProjectionRev !== null &&
    incomingProjectionRev < existingProjectionRev &&
    (incomingSeq === null || existingSeq === null || incomingSeq < existingSeq)
  ) {
    host.emitFreshnessEvent({
      type: "projection_or_seq_regression",
      sessionId,
      dimension: "projection_rev",
      incoming: incomingProjectionRev,
      existing: existingProjectionRev,
    });
    staleDelta = true;
  }
  if (staleDelta && !staleDeltaHasVisibleForwardProgress(entry, delta, rawEvent, toolSummaries)) {
    return;
  }

  const previousSession = entry.session;
  const previousActivity = entry.activity;
  const previousTurns = entry.turns.slice();
  const previousMessages = entry.messages.slice();
  const previousToolSummaries = entry.toolSummaries.slice();
  const previousAssistantStreamingRev = entry.assistantStreamingRev;
  const turns: SessionTurn[] = [];
  const messages: Message[] = [];
  const events: SessionEvent[] = [];
  if (delta.turn) turns.push(delta.turn);
  if (delta.message) messages.push(delta.message);

  const event =
    rawEvent && !rawStreamOnlyAssistantChunk
      ? ensureReplicaEventSeq(entry, rawEvent)
      : rawEvent;
  const streamOnlyAssistantChunk = event ? isStreamOnlyAssistantChunk(event) : false;
  if (event && !streamOnlyAssistantChunk) events.push(event);

  if (turns.length > 0) {
    mergeReplicaTurnsIntoEntry(entry, turns, { authoritative: true });
  }
  if (messages.length > 0) {
    mergeReplicaMessagesIntoEntry(entry, messages);
  }
  const { newEvents, evictedBeforeSeq } =
    events.length > 0
      ? mergeReplicaEventsIntoEntry(entry, events, host.config.eventBufferLimit)
      : { newEvents: [] as SessionEvent[] };
  for (const nextEvent of newEvents) {
    applyReplicaTranscriptEvent(entry, nextEvent);
  }
  if (event && streamOnlyAssistantChunk) {
    applyReplicaTranscriptEvent(entry, event);
  }
  if (toolSummaries.length > 0) {
    const byId = new Map(entry.toolSummaries.map((summary) => [String(summary.tool_call_id), summary]));
    for (const summary of toolSummaries) {
      byId.set(String(summary.tool_call_id), summary);
    }
    entry.toolSummaries = Array.from(byId.values());
    rebuildReplicaTranscriptAuxState(entry);
  }
  if (typeof evictedBeforeSeq === "number") {
    host.emitEvictPatch(sessionId, { eventsBeforeSeq: evictedBeforeSeq });
  }

  const streamOnlyDelta =
    streamOnlyCandidate &&
    streamOnlyAssistantChunk &&
    turns.length === 0 &&
    messages.length === 0 &&
    events.length === 0 &&
    toolSummaries.length === 0;
  if (streamOnlyDelta) {
    if (entry.assistantStreamingRev === previousAssistantStreamingRev) return;
    host.emitAppendPatch(sessionId, buildStreamingOverlayReplicaPatch(entry));
    return;
  }

  const previousFreshness = entry.freshness;
  if (previousFreshness !== "recovering") {
    entry.freshness = "authoritative";
  }
  if (typeof delta.projection_rev === "number") {
    entry.projectionRev =
      typeof entry.projectionRev === "number"
        ? Math.max(entry.projectionRev, delta.projection_rev)
        : delta.projection_rev;
  }
  entry.lastEventSeq = Math.max(existingSeq ?? -1, incomingSeq ?? -1);
  if (typeof delta.state_rev === "number") {
    entry.stateRev =
      typeof entry.stateRev === "number" ? Math.max(entry.stateRev, delta.state_rev) : delta.state_rev;
  }
  if (delta.session) {
    entry.session = delta.session;
  }
  if (delta.activity !== undefined && delta.activity !== null) {
    entry.activity = delta.activity;
    if (typeof delta.last_event_seq === "number") {
      entry.activityLastEventSeq =
        typeof entry.activityLastEventSeq === "number"
          ? Math.max(entry.activityLastEventSeq, delta.last_event_seq)
          : delta.last_event_seq;
    }
    if (typeof delta.projection_rev === "number") {
      entry.activityProjectionRev =
        typeof entry.activityProjectionRev === "number"
          ? Math.max(entry.activityProjectionRev, delta.projection_rev)
          : delta.projection_rev;
    }
  }
  if (reconcileLatestTurnInterruptedFromActivity(entry.turns, entry.activity)) {
    entry.turnsRev += 1;
  }
  entry.activity = reconcileActivityInterruptedFromTurns(entry.activity, entry.turns);
  entry.hydrated = true;
  host.emitAppendPatch(sessionId, buildStreamDeltaReplicaPatch(entry, {
    turns: changedTurnsById(previousTurns, entry.turns),
    messages: changedMessagesById(previousMessages, entry.messages),
    removedMessageIds: removedMessageIdsById(previousMessages, entry.messages),
    events: newEvents,
    toolSummaries: changedToolSummariesById(previousToolSummaries, entry.toolSummaries),
    includeSession: Boolean(delta.session && entry.session !== previousSession),
    includeActivity: delta.activity !== undefined || entry.activity !== previousActivity,
    includeAssistantStreaming: entry.assistantStreamingRev !== previousAssistantStreamingRev,
  }));
  void host.persistHead(entry);
};
