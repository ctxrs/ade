import type {
  WorkspaceActiveSnapshotClientMessage,
  WorkspaceActiveSnapshotSessionReplay,
} from "../../api/client";
import type { SessionSubscriptionCursor } from "../sessionSubscription";
import { shouldRequestWorkspaceSnapshot } from "./transport";

const toWorkspaceReplay = (
  replay: SessionSubscriptionCursor["replay"],
): WorkspaceActiveSnapshotSessionReplay => {
  switch (replay.kind) {
    case "reset":
      return { mode: "reset" };
    case "resume":
      return { mode: "resume", after_seq: replay.afterSeq };
    default:
      return { mode: "auto" };
  }
};

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
      replay: toWorkspaceReplay(session.replay),
    }));
  }
  return { message, requestSnapshot };
}
