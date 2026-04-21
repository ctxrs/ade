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
import { getClientSettingsState } from "../clientSettings";
import type { ExecutionEnvironment } from "@ctx/types";

type TerminalTurnStatus = Extract<SessionTurn["status"], "completed" | "failed" | "interrupted">;
type NotifiableTurnStatus = Extract<SessionTurn["status"], "completed" | "failed">;

const isTerminalTurnStatus = (status: SessionTurn["status"] | undefined): status is TerminalTurnStatus =>
  status === "completed" || status === "failed" || status === "interrupted";

const isNotifiableTurnStatus = (status: SessionTurn["status"] | undefined): status is NotifiableTurnStatus =>
  status === "completed" || status === "failed";

type TurnOutcomeEffectInput = {
  sessionId: string;
  taskId?: string;
  workspaceId?: string;
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
  notificationBody?: string;
  notificationTitle?: string;
  previousStatus?: SessionTurn["status"];
  nextStatus?: SessionTurn["status"];
  notify: boolean;
};

type ReplayTurnOutcomeEffectsInput = {
  sessionId: string;
  taskId?: string;
  workspaceId?: string;
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

export const shouldNotifyTurnOutcome = (
  previousStatus: SessionTurn["status"] | undefined,
  nextStatus: SessionTurn["status"] | undefined,
): nextStatus is NotifiableTurnStatus => {
  if (!isNotifiableTurnStatus(nextStatus)) return false;
  return nextStatus !== previousStatus;
};

const isNotificationEnabledForStatus = (status: NotifiableTurnStatus): boolean => {
  const state = getClientSettingsState();
  if (!state.loaded) return false;
  const settings = state.settings.desktopNotifications;
  if (status === "completed") return settings.turnCompleted;
  return settings.turnFailed;
};

const notificationTitleForStatus = (status: NotifiableTurnStatus): string =>
  status === "completed" ? "Turn completed" : "Turn failed";

const notificationKindForStatus = (status: NotifiableTurnStatus): "turn_completed" | "turn_failed" =>
  status === "completed" ? "turn_completed" : "turn_failed";

export const applyTurnOutcomeEffects = ({
  sessionId,
  taskId,
  workspaceId,
  turnId,
  providerId,
  modelId,
  reasoningEffort,
  executionEnvironment,
  sessionKind,
  startedAt,
  completedAt,
  metrics,
  notificationBody,
  notificationTitle,
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
  if (sessionKind !== "primary") return;
  if (!shouldNotifyTurnOutcome(previousStatus, nextStatus)) return;
  if (isAppInForeground()) return;
  if (!workspaceId || !taskId) return;
  if (!isNotificationEnabledForStatus(nextStatus)) return;
  const resolvedNotificationTitle =
    String(notificationTitle ?? "").trim() || notificationTitleForStatus(nextStatus);
  const resolvedNotificationBody = String(notificationBody ?? "").trim() || undefined;
  void sendDesktopNotification({
    kind: notificationKindForStatus(nextStatus),
    title: resolvedNotificationTitle,
    body: resolvedNotificationBody,
    workspaceId,
    taskId,
    sessionId: idToString(sessionId),
  });
};

export const replayTurnOutcomeEffectsFromTurns = ({
  sessionId,
  taskId,
  workspaceId,
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
      taskId,
      workspaceId,
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
