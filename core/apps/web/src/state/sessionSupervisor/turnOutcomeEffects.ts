import { idToString, type SessionTurn } from "../../api/client";
import { sendDesktopNotification } from "../../utils/desktopNotifications";
import { isAppInForeground } from "../../utils/windowFocus";
import {
  trackFirstTurnCompleted,
  trackProviderRunCompleted,
  trackTurnCompleted,
} from "../../utils/analytics";
import { markTurnOutcomeTracked } from "../../utils/analytics/turnOutcomeDedup";
import type { AnalyticsSessionKind } from "../../utils/analytics/types";
import { getClientSettings } from "../clientSettings";
import type { ExecutionEnvironment } from "@ctx/types";

type TerminalTurnStatus = Extract<SessionTurn["status"], "completed" | "failed" | "interrupted">;

const isTerminalTurnStatus = (status: SessionTurn["status"] | undefined): status is TerminalTurnStatus =>
  status === "completed" || status === "failed" || status === "interrupted";

type TurnOutcomeEffectInput = {
  sessionId: string;
  turnId?: string;
  providerId?: string;
  modelId?: string;
  reasoningEffort?: string;
  executionEnvironment?: ExecutionEnvironment;
  sessionKind?: AnalyticsSessionKind;
  startedAt?: string;
  completedAt?: string;
  metrics?: unknown;
  title?: string;
  previousStatus?: SessionTurn["status"];
  nextStatus?: SessionTurn["status"];
  notify: boolean;
};

type ReplayTurnOutcomeEffectsInput = {
  sessionId: string;
  providerId?: string;
  modelId?: string;
  reasoningEffort?: string;
  executionEnvironment?: ExecutionEnvironment;
  sessionKind?: AnalyticsSessionKind;
  previousTurns: SessionTurn[];
  nextTurns: SessionTurn[];
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
  reasoningEffort,
  executionEnvironment,
  sessionKind,
  startedAt,
  completedAt,
  metrics,
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
    trackTurnCompleted({
      providerId,
      modelId,
      reasoningEffort,
      executionEnvironment,
      status: nextStatus,
      durationMs,
      sessionKind,
      metrics,
    });
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

export const replayTurnOutcomeEffectsFromTurns = ({
  sessionId,
  providerId,
  modelId,
  reasoningEffort,
  executionEnvironment,
  sessionKind,
  previousTurns,
  nextTurns,
}: ReplayTurnOutcomeEffectsInput): void => {
  const normalizedSessionId = sessionId.trim();
  if (!normalizedSessionId || nextTurns.length === 0) return;

  const previousStatusesByTurnId = new Map<string, SessionTurn["status"]>();
  for (const turn of previousTurns) {
    const turnId = idToString(turn.turn_id);
    if (!turnId) continue;
    previousStatusesByTurnId.set(turnId, turn.status);
  }

  for (const turn of nextTurns) {
    const turnId = idToString(turn.turn_id);
    if (!turnId) continue;
    applyTurnOutcomeEffects({
      notify: false,
      sessionId: normalizedSessionId,
      turnId,
      providerId,
      modelId,
      reasoningEffort,
      executionEnvironment,
      sessionKind,
      startedAt: turn.started_at,
      completedAt: turn.updated_at,
      metrics: turn.metrics_json,
      previousStatus: previousStatusesByTurnId.get(turnId),
      nextStatus: turn.status,
    });
  }
};
