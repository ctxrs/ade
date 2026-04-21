import type { SessionEvent, SessionTurn } from "../../api/client";
import { readTurnStatusFromPayload } from "./toolStateProjection";

export const isTerminalTurnStatus = (
  status: SessionTurn["status"] | null | undefined,
): status is Extract<SessionTurn["status"], "completed" | "failed" | "interrupted"> =>
  status === "completed" || status === "failed" || status === "interrupted";

export const mergeOrderedTurnStatus = (
  previous: SessionTurn["status"] | null | undefined,
  next: SessionTurn["status"] | null | undefined,
): SessionTurn["status"] => {
  if (previous === "failed" && next === "interrupted") {
    return "interrupted";
  }
  if (isTerminalTurnStatus(previous)) {
    return previous;
  }
  if (!previous) return next ?? "queued";
  return next ?? previous;
};

export const resolveTurnStatusFromLifecycleEvent = (
  previousStatus: SessionTurn["status"] | null | undefined,
  event: SessionEvent,
): SessionTurn["status"] | null => {
  switch (String(event.event_type)) {
    case "turn_queued":
      return mergeOrderedTurnStatus(previousStatus, "queued");
    case "turn_started":
      return mergeOrderedTurnStatus(previousStatus, "running");
    case "turn_finished": {
      const payloadStatus = readTurnStatusFromPayload(event);
      if (payloadStatus) {
        return mergeOrderedTurnStatus(previousStatus, payloadStatus);
      }
      if (previousStatus === "interrupted" || previousStatus === "failed") {
        return previousStatus;
      }
      return mergeOrderedTurnStatus(previousStatus, "completed");
    }
    case "turn_interrupted":
      return mergeOrderedTurnStatus(previousStatus, "interrupted");
    case "error":
      return mergeOrderedTurnStatus(previousStatus, "failed");
    case "done":
      return mergeOrderedTurnStatus(previousStatus, "completed");
    default:
      return null;
  }
};
