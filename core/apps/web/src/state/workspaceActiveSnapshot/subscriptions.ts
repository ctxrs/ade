import type { WorkspaceActiveSnapshotClientMessage } from "../../api/client";
import { shouldRequestWorkspaceSnapshot } from "./transport";

export function buildWorkspaceActiveSubscribeMessage(
  reason: string,
  foregroundTaskId: string | null,
  subscribedSessionIds: string[],
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
  if (subscribedSessionIds.length > 0) {
    message.session_ids = subscribedSessionIds.slice();
  }
  return { message, requestSnapshot };
}
