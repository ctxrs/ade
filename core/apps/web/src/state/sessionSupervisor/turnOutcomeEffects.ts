import type { SessionTurn } from "../../api/client";
import { sendDesktopNotification } from "../../utils/desktopNotifications";
import { isAppInForeground } from "../../utils/windowFocus";
import { trackFirstTurnCompleted, trackProviderRunCompleted } from "../../utils/analytics";
import { getClientSettings } from "../clientSettings";

type TerminalTurnStatus = Extract<SessionTurn["status"], "completed" | "failed" | "interrupted">;

const isTerminalTurnStatus = (status: SessionTurn["status"] | undefined): status is TerminalTurnStatus =>
  status === "completed" || status === "failed" || status === "interrupted";

type TurnOutcomeEffectInput = {
  sessionId: string;
  providerId?: string;
  modelId?: string;
  title?: string;
  previousStatus?: SessionTurn["status"];
  nextStatus?: SessionTurn["status"];
  notify: boolean;
};

export const shouldTrackTurnOutcome = (
  previousStatus: SessionTurn["status"] | undefined,
  nextStatus: SessionTurn["status"] | undefined,
): nextStatus is TerminalTurnStatus => {
  if (!isTerminalTurnStatus(nextStatus)) return false;
  return nextStatus !== previousStatus;
};

export const shouldNotifyTurnCompleted = (
  previousStatus: SessionTurn["status"] | undefined,
  nextStatus: SessionTurn["status"] | undefined,
): boolean => previousStatus !== "completed" && nextStatus === "completed";

export const applyTurnOutcomeEffects = ({
  sessionId,
  providerId,
  modelId,
  title,
  previousStatus,
  nextStatus,
  notify,
}: TurnOutcomeEffectInput): void => {
  if (!notify) return;
  if (shouldTrackTurnOutcome(previousStatus, nextStatus)) {
    trackProviderRunCompleted({
      providerId,
      modelId,
      status: nextStatus,
    });
    trackFirstTurnCompleted({
      sessionId,
      providerId,
      status: nextStatus,
    });
  }
  if (!shouldNotifyTurnCompleted(previousStatus, nextStatus)) return;
  if (isAppInForeground()) return;
  if (!getClientSettings().desktopNotifications.turnCompleted) return;
  void sendDesktopNotification({
    title: "Turn completed",
    body: title || undefined,
  });
};
