import type {
  Message,
  SessionEvent,
  SessionHead,
  SessionHeadDelta,
  SessionHeadSnapshot,
  SessionTurn,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
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
  reconcileActivityInterruptedFromTurns,
  reconcileLatestTurnInterruptedFromActivity,
} from "./sessionSupervisor/cachePolicy";
import {
  noteFinalDeltaReceived,
  noteGapRecoveryStarted,
  noteProjectionOrSeqRegression,
} from "./foregroundFreshnessTelemetry";
import type {
  SessionReplicaAppendMode,
  SessionReplicaConfig,
  SessionReplicaData,
} from "./sessionReplicaProtocol";
import {
  buildCanonicalReplicaPatch,
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
      noteFinalDeltaReceived({
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
  noteGapRecoveryStarted(
    sessionId,
    typeof (evt as { reason?: unknown }).reason === "string"
      ? String((evt as { reason?: unknown }).reason)
      : null,
  );
  const previousEntry = host.entries.get(sessionId);
  host.gapRepairBaselineBySessionId.set(sessionId, {
    lastEventSeq:
      typeof previousEntry?.lastEventSeq === "number" ? previousEntry.lastEventSeq : null,
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
    mergeReplicaTurnsIntoEntry(entry, turns);
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

  entry.freshness = "authoritative";
  const incomingSeq = typeof delta.last_event_seq === "number" ? delta.last_event_seq : -1;
  const existingSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
  if (incomingSeq >= 0 && existingSeq >= 0 && incomingSeq < existingSeq) {
    noteProjectionOrSeqRegression(sessionId, "last_event_seq", incomingSeq, existingSeq);
  }
  if (typeof delta.projection_rev === "number") {
    if (
      typeof entry.projectionRev === "number" &&
      delta.projection_rev < entry.projectionRev
    ) {
      noteProjectionOrSeqRegression(
        sessionId,
        "projection_rev",
        delta.projection_rev,
        entry.projectionRev,
      );
    }
    entry.projectionRev =
      typeof entry.projectionRev === "number"
        ? Math.max(entry.projectionRev, delta.projection_rev)
        : delta.projection_rev;
  }
  entry.lastEventSeq = Math.max(existingSeq, incomingSeq);
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
  host.emitAppendPatch(sessionId, buildCanonicalReplicaPatch(entry, {
    appendMode: "stream_delta",
  }));
  void host.persistHead(entry);
};
