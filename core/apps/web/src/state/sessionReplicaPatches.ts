import type {
  Message,
  Session,
  SessionActivityState,
  SessionEvent,
  SessionHeadWindow,
  SessionSummaryCheckpoint,
  SessionTurn,
  SessionTurnToolSummary,
} from "@ctx/types";
import type { AssistantStreamingState } from "./assistantStreaming";
import type {
  SessionReplicaAppendMode,
  SessionReplicaData,
  SessionReplicaFreshnessState,
  SessionReplicaReplaceMode,
} from "./sessionReplicaProtocol";

type SessionReplicaPatchEntry = {
  session?: Session;
  activity?: SessionActivityState | null;
  freshness: SessionReplicaFreshnessState;
  turns: SessionTurn[];
  turnsRev: number;
  assistantStreamingByTurnId: Record<string, AssistantStreamingState>;
  assistantStreamingRev: number;
  messages: Message[];
  messagesRev: number;
  events: SessionEvent[];
  eventsRev: number;
  toolSummaries: SessionTurnToolSummary[];
  lastEventSeq?: number;
  projectionRev?: number;
  stateRev?: number;
  hasMoreTurns: boolean;
  summaryCheckpoint?: SessionSummaryCheckpoint | null;
  headWindow?: SessionHeadWindow | null;
  hydrated: boolean;
};

export function buildCanonicalReplicaPatch(
  entry: SessionReplicaPatchEntry,
  opts: {
    appendMode: SessionReplicaAppendMode;
    replaceMode?: SessionReplicaReplaceMode;
  },
): SessionReplicaData & { appendMode: SessionReplicaAppendMode };
export function buildCanonicalReplicaPatch(
  entry: SessionReplicaPatchEntry,
  opts?: {
    replaceMode?: SessionReplicaReplaceMode;
    appendMode?: undefined;
  },
): SessionReplicaData;
export function buildCanonicalReplicaPatch(
  entry: SessionReplicaPatchEntry,
  opts?: {
    replaceMode?: SessionReplicaReplaceMode;
    appendMode?: SessionReplicaAppendMode;
  },
): SessionReplicaData {
  const patch: SessionReplicaData & { appendMode?: SessionReplicaAppendMode } = {
    session: entry.session,
    activity: entry.activity ?? null,
    freshness: entry.freshness,
    turns: entry.turns,
    turnsRev: entry.turnsRev,
    assistantStreamingByTurnId: entry.assistantStreamingByTurnId,
    assistantStreamingRev: entry.assistantStreamingRev,
    messages: entry.messages,
    messagesRev: entry.messagesRev,
    events: entry.events,
    eventsRev: entry.eventsRev,
    toolSummaries: entry.toolSummaries,
    lastEventSeq: entry.lastEventSeq,
    projectionRev: entry.projectionRev,
    hasMoreTurns: entry.hasMoreTurns,
    summaryCheckpoint: entry.summaryCheckpoint ?? null,
    headWindow: entry.headWindow ?? null,
    turnsHydrated: entry.hydrated,
  };
  if (entry.stateRev !== undefined) {
    patch.stateRev = entry.stateRev;
  }
  if (opts?.replaceMode) {
    patch.replaceMode = opts.replaceMode;
  }
  if (opts?.appendMode) {
    patch.appendMode = opts.appendMode;
  }
  return patch;
}

export function buildStreamingOverlayReplicaPatch(
  entry: SessionReplicaPatchEntry,
): SessionReplicaData & { appendMode: "stream_delta" } {
  const patch: SessionReplicaData & { appendMode: "stream_delta" } = {
    assistantStreamingByTurnId: entry.assistantStreamingByTurnId,
    assistantStreamingRev: entry.assistantStreamingRev,
    appendMode: "stream_delta",
  };
  return patch;
}
