import { useRef } from "react";
import type { Message, SessionEvent, SessionTurn, SessionTurnTool } from "../../api/client";

export type InitialThreadProjection = {
  sessionId: string;
  turnsStamp: string;
  turns: SessionTurn[];
  messagesStamp: string;
  messages: Message[];
  eventsStamp: string;
  events: SessionEvent[];
  toolsByTurnId: Record<string, SessionTurnTool[]>;
  toolSummariesReady: boolean;
};

export function shouldReleaseInitialThreadProjection(params: {
  loaded: boolean;
  sessionId: string;
  settledSessionId: string | null;
  releasedSessionId: string | null;
}): boolean {
  const { loaded, sessionId, settledSessionId, releasedSessionId } = params;
  return loaded && settledSessionId === sessionId && releasedSessionId !== sessionId;
}

export function shouldFreezeInitialThreadProjection(params: {
  loaded: boolean;
  sessionId: string;
  releasedSessionId: string | null;
}): boolean {
  const { loaded, sessionId, releasedSessionId } = params;
  return loaded && releasedSessionId !== sessionId;
}

export function shouldRestoreBottomAnchorAfterProjectionRelease(params: {
  previousSource: "initial" | "coalesced";
  nextSource: "initial" | "coalesced";
  wasAtBottom: boolean;
}): boolean {
  const { previousSource, nextSource, wasAtBottom } = params;
  return wasAtBottom && previousSource === "initial" && nextSource === "coalesced";
}

type InitialThreadProjectionOptions = {
  releaseToLive: boolean;
};

type FrozenProjectionState = {
  sessionId: string;
  frozen: InitialThreadProjection;
  capturedVisibleContent: boolean;
};

const hasProjectionContent = (projection: InitialThreadProjection): boolean =>
  projection.turns.length > 0 ||
  projection.messages.length > 0 ||
  projection.events.length > 0 ||
  Object.keys(projection.toolsByTurnId).length > 0;

export function useInitialThreadProjection(
  projection: InitialThreadProjection,
  options: InitialThreadProjectionOptions,
): InitialThreadProjection {
  const stateRef = useRef<FrozenProjectionState | null>(null);

  if (!stateRef.current || stateRef.current.sessionId !== projection.sessionId) {
    stateRef.current = {
      sessionId: projection.sessionId,
      frozen: projection,
      capturedVisibleContent: hasProjectionContent(projection),
    };
  }

  const state = stateRef.current;
  if (!options.releaseToLive) {
    if (!state.capturedVisibleContent && hasProjectionContent(projection)) {
      state.frozen = projection;
      state.capturedVisibleContent = true;
    }
    return state.frozen;
  }

  return projection;
}
