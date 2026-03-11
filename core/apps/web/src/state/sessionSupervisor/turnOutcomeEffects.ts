import type { SessionTurn } from "../../api/client";
import { sendDesktopNotification } from "../../utils/desktopNotifications";
import { isAppInForeground } from "../../utils/windowFocus";
import { trackFirstTurnCompleted, trackProviderRunCompleted } from "../../utils/analytics";
import { markTurnOutcomeTracked } from "../../utils/analytics/turnOutcomeDedup";
import { getClientSettings } from "../clientSettings";

type TerminalTurnStatus = Extract<SessionTurn["status"], "completed" | "failed" | "interrupted">;

const isTerminalTurnStatus = (status: SessionTurn["status"] | undefined): status is TerminalTurnStatus =>
  status === "completed" || status === "failed" || status === "interrupted";

type TurnOutcomeEffectInput = {
  sessionId: string;
  turnId?: string;
  providerId?: string;
  modelId?: string;
  sessionKind?: "primary" | "subagent";
  startedAt?: string;
  completedAt?: string;
  title?: string;
  previousStatus?: SessionTurn["status"];
  nextStatus?: SessionTurn["status"];
  notify: boolean;
};

const parseTimestampMs = (value: string | undefined): number | null => {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
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
  turnId,
  providerId,
  modelId,
  sessionKind,
  startedAt,
  completedAt,
  title,
  previousStatus,
  nextStatus,
  notify,
}: TurnOutcomeEffectInput): void => {
  const startedAtMs = parseTimestampMs(startedAt);
  const completedAtMs = parseTimestampMs(completedAt);
  const durationMs = startedAtMs !== null && completedAtMs !== null && completedAtMs >= startedAtMs
    ? completedAtMs - startedAtMs
    : undefined;
  if (
    shouldTrackTurnOutcome(previousStatus, nextStatus)
      && (!turnId || markTurnOutcomeTracked(sessionId, turnId, nextStatus))
  ) {
    trackProviderRunCompleted({
      providerId,
      modelId,
      status: nextStatus,
      durationMs,
      sessionKind,
    });
    trackFirstTurnCompleted({
      sessionId,
      providerId,
      status: nextStatus,
      sessionKind,
    });
  }
  if (!notify) return;
  if (!shouldNotifyTurnCompleted(previousStatus, nextStatus)) return;
  if (isAppInForeground()) return;
  if (!getClientSettings().desktopNotifications.turnCompleted) return;
  void sendDesktopNotification({
    title: "Turn completed",
    body: title || undefined,
  });
};
