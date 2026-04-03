import type { Message, SessionEvent, SessionTurn, SessionTurnTool } from "../../api/client";
import {
  deriveAssistantStreamingKey,
  deriveMessagesKey,
  deriveTurnsKey,
} from "../../pages/workbenchViewModel/messageKeys";
import { deriveSessionThreadEventsStamp } from "./applyEvents";
import type { SessionThreadProjection } from "./types";
import type { AssistantStreamingState } from "../assistantStreaming";

type SessionThreadProjectionSource = {
  stateLoaded?: boolean;
  turns?: SessionTurn[];
  turnsRev?: number;
  assistantStreamingByTurnId?: Record<string, AssistantStreamingState>;
  assistantStreamingRev?: number;
  messages?: Message[];
  messagesRev?: number;
  events?: SessionEvent[];
  eventsRev?: number;
  turnToolsByTurnId?: Record<string, SessionTurnTool[]>;
  toolSummariesReady?: boolean;
  projectionRev?: number;
};

export function buildSessionThreadProjectionFromSnapshot(
  source: SessionThreadProjectionSource,
): SessionThreadProjection {
  const turns = source.turns ?? [];
  const assistantStreamingByTurnId = source.assistantStreamingByTurnId ?? {};
  const assistantStreamingStamp = `${source.assistantStreamingRev ?? 0}:${deriveAssistantStreamingKey(
    assistantStreamingByTurnId,
  )}`;
  const messages = source.messages ?? [];
  const events = source.events ?? [];
  return {
    loaded: Boolean(source.stateLoaded),
    turns,
    turnsStamp: `${source.turnsRev ?? 0}:${deriveTurnsKey(turns)}:${assistantStreamingStamp}`,
    assistantStreamingByTurnId,
    assistantStreamingStamp,
    messages,
    messagesStamp: `${source.messagesRev ?? 0}:${deriveMessagesKey(messages)}`,
    events,
    eventsStamp: deriveSessionThreadEventsStamp(events, source.eventsRev),
    toolsByTurnId: source.turnToolsByTurnId ?? {},
    toolSummariesReady: Boolean(source.toolSummariesReady),
    projectionRev: source.projectionRev ?? 0,
  };
}
