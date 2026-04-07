import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { SessionSupervisorSnapshot } from "../../state/sessionSupervisor";
import type { WorkspaceActiveSnapshotState } from "../../state/workspaceActiveSnapshotStore";
import { collectWorkspaceActivePrimarySessionIds } from "../../state/workspaceActiveSnapshot/projection";
import { selectSessionThreadProjection } from "../../state/sessionThreadProjection/selectors";
import { collectAskUserQuestionAnswers } from "../SessionPage.workbenchViewModel";
import { primeWarmWorkbenchThreadViewModel } from "../workbenchThreadViewModelWarmCache";
import {
  createDefaultSessionTranscriptUiState,
  getOrCreateSessionPretextRuntime,
  primeSessionPretextRuntime,
} from "../sessionThread/pretextSessionRuntimeCache";
import {
  getSessionTranscriptWarmState,
  subscribeSessionTranscriptWarmState,
} from "../sessionThread/sessionTranscriptWarmState";

type IdleHandle = number;

const requestIdle = (callback: () => void): IdleHandle => {
  const idleWindow = window as Window & {
    requestIdleCallback?: (cb: () => void) => number;
  };
  if (typeof idleWindow.requestIdleCallback === "function") {
    return idleWindow.requestIdleCallback(() => callback());
  }
  return window.setTimeout(callback, 16);
};

const cancelIdle = (handle: IdleHandle) => {
  const idleWindow = window as Window & {
    cancelIdleCallback?: (handle: number) => void;
  };
  if (typeof idleWindow.cancelIdleCallback === "function") {
    idleWindow.cancelIdleCallback(handle);
    return;
  }
  window.clearTimeout(handle);
};

export function useWarmSessionTranscriptRuntimes({
  workspaceSnapshot,
  sessionSnap,
  activeSessionId,
}: {
  workspaceSnapshot: WorkspaceActiveSnapshotState;
  sessionSnap: SessionSupervisorSnapshot;
  activeSessionId: string | null;
}) {
  const activePrimarySessionIds = useMemo(
    () => collectWorkspaceActivePrimarySessionIds(workspaceSnapshot),
    [workspaceSnapshot],
  );
  const warmState = useSyncExternalStore(
    subscribeSessionTranscriptWarmState,
    getSessionTranscriptWarmState,
    getSessionTranscriptWarmState,
  );

  useEffect(() => {
    if (warmState.viewportWidth <= 0) return;

    const sessionIds = activePrimarySessionIds.filter((sessionId) => sessionId !== activeSessionId);
    if (sessionIds.length === 0) return;

    let cancelled = false;
    let idleHandle: IdleHandle | null = null;
    let index = 0;

    const warmNext = () => {
      if (cancelled) return;
      while (index < sessionIds.length) {
        const sessionId = sessionIds[index]!;
        index += 1;

        const entry = sessionSnap.sessions[sessionId];
        if (!entry) continue;
        const threadProjection = selectSessionThreadProjection(entry);
        const hasProjectionData =
          threadProjection.loaded ||
          threadProjection.turns.length > 0 ||
          threadProjection.messages.length > 0 ||
          threadProjection.events.length > 0;
        if (!hasProjectionData) continue;

        const askUserQuestionAnswers = collectAskUserQuestionAnswers(threadProjection.events, {});
        const warmedViewModel = primeWarmWorkbenchThreadViewModel({
          sessionId,
          projectionRev: threadProjection.projectionRev,
          turnsStamp: threadProjection.turnsStamp,
          messagesStamp: threadProjection.messagesStamp,
          eventsStamp: threadProjection.eventsStamp,
          verbosity: warmState.verbosity,
          turns: threadProjection.turns,
          assistantStreamingByTurnId: threadProjection.assistantStreamingByTurnId,
          messages: threadProjection.messages,
          events: threadProjection.events,
          toolsByTurnId: threadProjection.toolsByTurnId,
          toolSummariesReady: threadProjection.toolSummariesReady,
          askUserQuestionAnswers,
          enableDebugEvents: false,
        });

        const existingRuntime = getOrCreateSessionPretextRuntime(sessionId);
        const runtimeUiState = existingRuntime.hasVisibleMount
          ? {
              ...existingRuntime.uiState,
              turnToolsLoading: entry.turnToolsLoading,
              verbosity: warmState.verbosity,
            }
          : createDefaultSessionTranscriptUiState(warmState.verbosity, entry.turnToolsLoading);

        primeSessionPretextRuntime({
          sessionId,
          listItems: warmedViewModel.listItems,
          uiState: runtimeUiState,
          viewportWidth: warmState.viewportWidth,
          viewportHeight: warmState.viewportHeight,
        });
        break;
      }
      if (index < sessionIds.length) {
        schedule();
      }
    };

    const schedule = () => {
      idleHandle = requestIdle(() => {
        idleHandle = null;
        warmNext();
      });
    };

    schedule();
    return () => {
      cancelled = true;
      if (idleHandle != null) {
        cancelIdle(idleHandle);
      }
    };
  }, [activePrimarySessionIds, activeSessionId, sessionSnap, warmState]);
}
