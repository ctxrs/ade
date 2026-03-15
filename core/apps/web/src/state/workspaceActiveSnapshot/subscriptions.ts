import type { WorkspaceActiveSnapshotClientMessage } from "../../api/client";
import type { SessionSubscriptionCursor } from "../sessionSubscription";
import { shouldRequestWorkspaceSnapshot } from "./transport";

export function buildWorkspaceActiveSubscribeMessage(
  reason: string,
  foregroundTaskId: string | null,
  subscribedSessions: SessionSubscriptionCursor[],
): {
  message: WorkspaceActiveSnapshotClientMessage;
  requestSnapshot: boolean;
} {
  const requestSnapshot = shouldRequestWorkspaceSnapshot(reason);
  const message: WorkspaceActiveSnapshotClientMessage = {
    type: "subscribe",
    scope: "active",
    include_active_heads: requestSnapshot,
  };
  if (foregroundTaskId) {
    message.foreground_task_id = foregroundTaskId;
  }
  if (subscribedSessions.length > 0) {
    message.session_ids = subscribedSessions.map((session) => session.sessionId);
    message.sessions = subscribedSessions.map((session) => ({
      session_id: session.sessionId,
      after_seq: session.afterSeq,
    }));
  }
  return { message, requestSnapshot };
}
