import {
  applyAssistantChunkToStreaming,
  applyAssistantCompleteToStreaming,
  clearAssistantStreaming,
  reconcileAssistantStreamingWithMessages,
  type AssistantStreamingStore,
} from "../assistantStreaming";
import {
  idToString,
  type GitStatusSummary,
  type Message,
  type Session,
  type SessionEvent,
  type SessionTurn,
  type SubagentInvocation,
} from "../../api/client";
import { compareSessionTurnOrder, mergeSessionMessages } from "../sessionHeadState";
import { appendFragment, isPartialEvent, mergeTurn, mergeTurnStatus } from "./cachePolicy";
import type { InternalEntry } from "./entryState";
import { asRecord, messageFromEvent, readPayloadObject } from "./eventHydration";
import { readPayloadString } from "./eventNormalization";
import { normalizeGitStatusSummaryInput } from "./gitStatusNormalization";
import {
  buildThoughtCacheKey,
  isFinalThoughtEvent,
  normalizeFinalThoughtPayload,
  readThoughtFullContent,
} from "./thoughtProjection";
import {
  applyToolBucketDelta,
  deriveTurnStatusFromEvent,
  extractToolCallId,
  extractToolStatus,
  readTurnStatusFromPayload,
  shouldRenderAssistantChunk,
  shouldRenderThoughtChunk,
  toolStatusBucket,
} from "./toolStateProjection";
import { resolveTurnAnalyticsMetadata } from "./turnAnalyticsMetadata";
import {
  resolveTurnOutcomeNotificationBody,
  resolveTurnOutcomeNotificationTitle,
} from "./turnOutcomeNotificationContent";
import { applyTurnStartEffects } from "./turnStartEffects";
import { applyTurnOutcomeEffects } from "./turnOutcomeEffects";
import type { SessionSupervisorWorkspaceSnapshotState } from "./workspaceInputs";

type SessionSupportLoadErrorKey = "state" | "subagentInvocations";

type SessionSupervisorThoughtCacheRecord = {
  key: string;
  event: SessionEvent;
  updatedAtMs?: number;
};

export type SessionSupervisorEventProjectionEntry = InternalEntry;

export type SessionSupervisorEventProjectionHost = {
  eventBufferLimit: number;
  workspaceSnapshotState: SessionSupervisorWorkspaceSnapshotState;
  subagentInvocationsCacheBySessionId: Map<
    string,
    { invocations: SubagentInvocation[]; stateRev: number }
  >;
  overlayThoughtCacheOnTurns(
    entry: SessionSupervisorEventProjectionEntry,
    turns: SessionTurn[],
  ): SessionTurn[];
  overlayThoughtCacheOnEvents(
    entry: SessionSupervisorEventProjectionEntry,
    events: SessionEvent[],
  ): SessionEvent[];
  persistThoughtCache(entry: SessionSupervisorEventProjectionEntry): Promise<void>;
  ensureSubagentInvocations(
    entry: SessionSupervisorEventProjectionEntry,
    opts?: { force?: boolean },
  ): Promise<void>;
  syncStateCache(entry: SessionSupervisorEventProjectionEntry, stateRev?: number): void;
  clearSupportLoadError(
    entry: SessionSupervisorEventProjectionEntry,
    key: SessionSupportLoadErrorKey,
  ): void;
  bumpTurnsRev(entry: SessionSupervisorEventProjectionEntry): void;
  bumpMessagesRev(entry: SessionSupervisorEventProjectionEntry): void;
  bumpEventsRev(entry: SessionSupervisorEventProjectionEntry): void;
};

type TurnProjectionState = SessionTurn & {
  thought_partial_provider_item_id?: string | null;
};

function isTerminalTurnStatus(
  status: SessionTurn["status"] | null | undefined,
): status is Extract<SessionTurn["status"], "completed" | "failed" | "interrupted"> {
  return status === "completed" || status === "failed" || status === "interrupted";
}

function mergeOrderedTurnStatus(
  previous: SessionTurn["status"] | null | undefined,
  next: SessionTurn["status"] | null | undefined,
): SessionTurn["status"] {
  if (previous === "failed" && next === "interrupted") {
    return "interrupted";
  }
  if (isTerminalTurnStatus(previous)) {
    return previous;
  }
  return mergeTurnStatus(previous, next);
}

export function mergeTurns(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  incoming: SessionTurn[],
) {
  if (incoming.length === 0) return;
  const byId = new Map<string, SessionTurn>();
  for (const turn of entry.turns) {
    const turnId = idToString(turn.turn_id);
    if (turnId) byId.set(turnId, turn);
  }
  for (const turn of incoming) {
    const turnId = idToString(turn.turn_id);
    if (!turnId) continue;
    const startSeq = typeof turn.start_seq === "number" ? turn.start_seq : Number.NaN;
    if (Number.isFinite(startSeq) && startSeq >= 0) {
      entry.startedTurnIds.add(turnId);
    }
    const previous = byId.get(turnId);
    byId.set(turnId, previous ? mergeTurn(previous, turn) : turn);
  }
  let next = Array.from(byId.values()).sort(compareSessionTurnOrder);
  const overlayed = this.overlayThoughtCacheOnTurns(entry, next);
  if (overlayed !== next) {
    next = overlayed;
    entry.updatedAtMs = Date.now();
  }
  entry.turns = next;
  this.bumpTurnsRev(entry);
  entry.oldestTurnSeq = next[0]?.start_seq ?? entry.oldestTurnSeq;
}

export function mergeMessages(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  incoming: Message[],
) {
  if (incoming.length === 0) return;
  entry.messages = mergeSessionMessages(entry.messages, incoming);
  reconcileAssistantStreamingWithMessages(entry as AssistantStreamingStore, incoming);
  entry.queue = entry.messages.filter((message) => message.delivery === "queued");
  this.bumpMessagesRev(entry);
}

export function ensureEventSeq(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  event: SessionEvent,
): SessionEvent {
  if (typeof event.seq === "number") return event;
  const nextSeq = entry.nextTransientSeq;
  entry.nextTransientSeq = nextSeq + 1;
  return { ...event, seq: nextSeq };
}

export function mergeEvents(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  incoming: SessionEvent[],
  opts?: { notify?: boolean },
) {
  if (incoming.length === 0) return;
  const shouldNotify = opts?.notify ?? true;
  const existingSeqs = entry.seqSet;
  const normalizedExisting = entry.events.map((event) => ensureEventSeq.call(this, entry, event));
  const normalizedIncoming = incoming.map((event) => ensureEventSeq.call(this, entry, event));
  if (normalizedExisting !== entry.events) {
    entry.events = normalizedExisting;
    this.bumpEventsRev(entry);
  }
  const newEvents: SessionEvent[] = [];
  for (const event of normalizedIncoming) {
    if (typeof event.seq === "number") {
      if (!existingSeqs.has(event.seq)) {
        newEvents.push(event);
      }
    } else {
      newEvents.push(event);
    }
  }
  const bySeq = new Map<number, SessionEvent>();
  for (const event of entry.events) {
    if (typeof event.seq === "number") bySeq.set(event.seq, event);
  }
  for (const event of normalizedIncoming) {
    if (typeof event.seq === "number") bySeq.set(event.seq, event);
  }
  const next = Array.from(bySeq.values()).sort((a, b) => Number(a.seq ?? 0) - Number(b.seq ?? 0));
  let trimmed =
    next.length > this.eventBufferLimit ? next.slice(-this.eventBufferLimit) : next;
  if (entry.thoughtCacheLoaded && Object.keys(entry.thoughtCacheByKey).length > 0) {
    const overlayed = this.overlayThoughtCacheOnEvents(entry, trimmed);
    if (overlayed !== trimmed) {
      trimmed = overlayed;
    }
  }
  entry.events = trimmed;
  this.bumpEventsRev(entry);
  entry.seqSet = new Set(
    trimmed
      .map((event) => (typeof event.seq === "number" ? event.seq : Number.NaN))
      .filter((seq) => Number.isFinite(seq)) as number[],
  );
  for (const event of normalizedIncoming) {
    const turnId = idToString(event.turn_id);
    if (!turnId) continue;
    const seq = typeof event.seq === "number" ? event.seq : Number.NaN;
    if (Number.isFinite(seq) && seq >= 0) {
      entry.startedTurnIds.add(turnId);
    }
  }
  let changed = false;
  for (const event of newEvents) {
    const turnId = idToString(event.turn_id);
    if (isPartialEvent(event) && (!turnId || !entry.startedTurnIds.has(turnId))) {
      continue;
    }
    const message = messageFromEvent(event, entry.session);
    if (message) {
      mergeMessages.call(this, entry, [message]);
      changed = true;
    }
    const previousTurnStatus = turnId
      ? entry.turns.find((turn) => idToString(turn.turn_id) === turnId)?.status
      : undefined;
    ensureTurnFromEvent.call(this, entry, event);
    if (applyEventToTurns.call(this, entry, event, {
      notify: shouldNotify,
      previousStatusOverride: previousTurnStatus,
    })) {
      changed = true;
    }
    if (applyQueueEvent.call(this, entry, event)) {
      changed = true;
    }
    if (applyGitStatusSnapshotNotice.call(this, entry, event)) {
      changed = true;
    }
    if (applySubagentInvocationNotice.call(this, entry, event)) {
      changed = true;
    }
  }
  if (changed) {
    entry.updatedAtMs = Date.now();
  }
}

export function recordFinalThought(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  event: SessionEvent,
): boolean {
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

export function ensureTurnFromEvent(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  event: SessionEvent,
): SessionTurn | null {
  const turnId = idToString(event.turn_id);
  if (!turnId) return null;
  const existing = entry.turns.find((turn) => idToString(turn.turn_id) === turnId);
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
    assistant_partial: null,
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
  entry.turns = [...entry.turns, turn].sort(compareSessionTurnOrder);
  this.bumpTurnsRev(entry);
  entry.oldestTurnSeq = entry.turns[0]?.start_seq ?? entry.oldestTurnSeq;
  return turn;
}

export function applyEventToTurns(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  event: SessionEvent,
  opts?: { notify?: boolean; previousStatusOverride?: SessionTurn["status"] },
): boolean {
  const turnId = idToString(event.turn_id);
  if (!turnId) return false;
  const turnIndex = entry.turns.findIndex((turn) => idToString(turn.turn_id) === turnId);
  if (turnIndex < 0) return false;

  const turn = entry.turns[turnIndex] as TurnProjectionState;
  const previousStatus = opts?.previousStatusOverride ?? turn.status;
  let changed = false;
  switch (String(event.event_type)) {
    case "assistant_chunk": {
      if (!shouldRenderAssistantChunk(event)) break;
      const fragment = String(event.payload_json?.content_fragment ?? "");
      if (fragment) {
        const providerMessageId = readPayloadString(event.payload_json, ["message_id", "messageId"]);
        changed =
          applyAssistantChunkToStreaming(entry as AssistantStreamingStore, turnId, fragment, providerMessageId) ||
          changed;
      }
      break;
    }
    case "thought_chunk": {
      if (!shouldRenderThoughtChunk(event)) break;
      if (isFinalThoughtEvent(event)) {
        if (recordFinalThought.call(this, entry, event)) {
          changed = true;
        }
      }
      break;
    }
    case "assistant_message_inserted": {
      changed = clearAssistantStreaming(entry as AssistantStreamingStore, turnId) || changed;
      break;
    }
    case "assistant_complete": {
      const full =
        event.payload_json?.full_content ??
        event.payload_json?.content ??
        entry.assistantStreamingByTurnId[turnId]?.content;
      const providerMessageId = readPayloadString(event.payload_json, ["message_id", "messageId"]);
      changed =
        applyAssistantCompleteToStreaming(
          entry as AssistantStreamingStore,
          turnId,
          String(full ?? ""),
          providerMessageId,
        ) || changed;
      break;
    }
    case "turn_queued": {
      turn.status = mergeOrderedTurnStatus(turn.status, "queued");
      changed = true;
      break;
    }
    case "turn_started": {
      turn.status = mergeOrderedTurnStatus(turn.status, "running");
      changed = true;
      break;
    }
    case "turn_finished": {
      const payloadStatus = readTurnStatusFromPayload(event);
      if (payloadStatus) {
        turn.status = mergeOrderedTurnStatus(turn.status, payloadStatus);
      } else if (turn.status !== "interrupted" && turn.status !== "failed") {
        turn.status = mergeOrderedTurnStatus(turn.status, "completed");
      }
      changed = true;
      break;
    }
    case "turn_interrupted": {
      turn.status = mergeOrderedTurnStatus(turn.status, "interrupted");
      changed = true;
      break;
    }
    case "error": {
      turn.status = mergeOrderedTurnStatus(turn.status, "failed");
      changed = true;
      break;
    }
    case "done": {
      turn.status = mergeOrderedTurnStatus(turn.status, "completed");
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
      if (applyToolEventToTurn.call(this, entry, turnId, turn, event)) {
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
  entry.turns[turnIndex] = { ...turn };
  this.bumpTurnsRev(entry);
  const analytics = resolveTurnAnalyticsMetadata(entry.session, turn.session_id ?? entry.sessionId);
  const notificationTitle = resolveTurnOutcomeNotificationTitle({
    session: entry.session,
    workspaceSnapshotState: this.workspaceSnapshotState,
  });
  const notificationBody = resolveTurnOutcomeNotificationBody({
    events: entry.events,
    turnId,
    messages: entry.messages,
    status: turn.status,
  });
  applyTurnStartEffects({
    ...analytics,
    turnId,
    previousStatus,
    nextStatus: turn.status,
  });
  applyTurnOutcomeEffects({
    notify: opts?.notify ?? true,
    ...analytics,
    turnId,
    startedAt: turn.started_at,
    completedAt: turn.updated_at,
    metrics: turn.metrics_json,
    notificationBody,
    notificationTitle,
    previousStatus,
    nextStatus: turn.status,
  });
  return true;
}

export function applyQueueEvent(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  event: SessionEvent,
): boolean {
  const messageId = idToString(readPayloadString(event.payload_json, ["message_id"]) ?? "");
  if (!messageId) return false;
  switch (String(event.event_type)) {
    case "message_queue_added": {
      const messageIndex = entry.messages.findIndex((message) => idToString(message.id) === messageId);
      if (messageIndex < 0) return false;
      const message = entry.messages[messageIndex];
      if (message.delivery === "queued") return false;
      entry.messages[messageIndex] = { ...message, delivery: "queued" };
      entry.queue = entry.messages.filter((current) => current.delivery === "queued");
      this.bumpMessagesRev(entry);
      return true;
    }
    case "message_queue_updated": {
      if (!entry.queue.some((message) => idToString(message.id) === messageId)) {
        return false;
      }
      entry.queue = entry.messages.filter((message) => message.delivery === "queued");
      this.bumpMessagesRev(entry);
      return true;
    }
    case "message_queue_removed": {
      const previousMessages = entry.messages.length;
      const previousQueue = entry.queue.length;
      entry.messages = entry.messages.filter((message) => idToString(message.id) !== messageId);
      entry.queue = entry.queue.filter((message) => idToString(message.id) !== messageId);
      if (entry.messages.length !== previousMessages || entry.queue.length !== previousQueue) {
        this.bumpMessagesRev(entry);
      }
      return entry.messages.length !== previousMessages || entry.queue.length !== previousQueue;
    }
    case "message_queue_promoted": {
      let changed = false;
      const messageIndex = entry.messages.findIndex((message) => idToString(message.id) === messageId);
      if (messageIndex >= 0) {
        const message = entry.messages[messageIndex];
        if (message.delivery === "queued") {
          entry.messages[messageIndex] = { ...message, delivery: "immediate" };
          changed = true;
        }
      }
      if (entry.queue.some((message) => idToString(message.id) === messageId)) {
        entry.queue = entry.queue.filter((message) => idToString(message.id) !== messageId);
        changed = true;
      }
      if (changed) {
        entry.queue = entry.messages.filter((message) => message.delivery === "queued");
        this.bumpMessagesRev(entry);
      }
      return changed;
    }
    default:
      return false;
  }
}

export function applyGitStatusSnapshotNotice(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  event: SessionEvent,
): boolean {
  if (String(event.event_type) !== "notice") return false;
  const payload = event.payload_json;
  if (payload?.kind !== "git_status_snapshot") return false;
  const partial = normalizeGitStatusSummaryInput(payload.summary, payload.entries);
  if (Object.keys(partial).length === 0) return false;
  entry.support.gitStatusSummary = { ...(entry.support.gitStatusSummary ?? {}), ...partial };
  this.syncStateCache(entry);
  return true;
}

export function applySubagentInvocationNotice(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
  event: SessionEvent,
): boolean {
  if (String(event.event_type) !== "notice") return false;
  const kind = event.payload_json?.kind;
  if (kind !== "subagent_invocation_created" && kind !== "subagent_invocation_updated") {
    return false;
  }
  this.subagentInvocationsCacheBySessionId.delete(entry.sessionId);
  void this.ensureSubagentInvocations(entry, { force: true });
  return false;
}

export function applyToolEventToTurn(
  this: SessionSupervisorEventProjectionHost,
  entry: SessionSupervisorEventProjectionEntry,
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
  const previousStatus = entry.toolStatusByKey.get(key);
  if (!entry.toolIdsByTurn.has(turnId)) {
    if ((turn.tool_total ?? 0) > 0) {
      return false;
    }
    entry.toolIdsByTurn.set(turnId, new Set());
  }
  const turnToolIds = entry.toolIdsByTurn.get(turnId);
  if (!turnToolIds) return false;
  const isNew = !turnToolIds.has(toolCallId);
  if (isNew) {
    turnToolIds.add(toolCallId);
    turn.tool_total = (turn.tool_total ?? 0) + 1;
  }

  const previousBucket = toolStatusBucket(previousStatus);
  const nextBucket = toolStatusBucket(nextStatus);
  if (previousBucket !== nextBucket) {
    applyToolBucketDelta(turn, previousBucket, -1);
    applyToolBucketDelta(turn, nextBucket, 1);
  }
  entry.toolStatusByKey.set(key, nextStatus);
  return true;
}
