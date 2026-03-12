import type { WorkspaceActiveSnapshotEvent } from "@ctx/types";
import { getDaemonClientConfig } from "../../api/client";
import type {
  WorkspaceActiveSnapshotCommand,
  WorkspaceActiveSnapshotPatch,
} from "../workspaceActiveSnapshotProtocol";
import { buildWorkspaceActiveSubscribeMessage } from "./subscriptions";
import type { WorkspaceActiveSnapshotStoreState } from "./storeState";

const FOREGROUND_TASK_DEBOUNCE_MS = 150;

export type WorkspaceActiveSnapshotControlHost = {
  e2eEnabled: boolean;
  e2eDropStreamMessages: boolean;
  worker: Worker | null;
  state: WorkspaceActiveSnapshotStoreState;
  ws: WebSocket | null;
  wsBaseUrlOverride: string | null;
  authTokenOverride: string | null;
  workspaceId: string;
  subscribedSessionIds: string[];
  foregroundTaskId: string | null;
  eventListeners: Set<(event: WorkspaceActiveSnapshotEvent) => void>;
  workerPatchEmitter: ((patch: WorkspaceActiveSnapshotPatch) => void) | null;
  workerPatchPendingEvents: WorkspaceActiveSnapshotEvent[];
  foregroundTaskTimer: ReturnType<typeof globalThis.setTimeout> | null;
  postWorkerCommand(cmd: WorkspaceActiveSnapshotCommand): void;
  publish(): void;
  enqueueStreamMessage(data: unknown): void;
  scheduleSnapshotWarning(reason: string): void;
  scheduleWorkerPatchFlush(): void;
};

export function unwrapEvent(value: unknown): unknown {
  if (!value || typeof value !== "object") return value;
  const rec = value as { type?: string; event?: unknown };
  if (rec.type === "event" && rec.event && typeof rec.event === "object") {
    return rec.event;
  }
  return value;
}

export function notifyEventListeners(
  host: WorkspaceActiveSnapshotControlHost,
  evt: WorkspaceActiveSnapshotEvent,
) {
  for (const listener of host.eventListeners) {
    listener(evt);
  }
  if (host.workerPatchEmitter) {
    host.workerPatchPendingEvents.push(evt);
    host.scheduleWorkerPatchFlush();
  }
}

export function setE2EEnabled(
  host: WorkspaceActiveSnapshotControlHost,
  enabled: boolean,
) {
  host.e2eEnabled = enabled;
  if (!enabled) {
    host.e2eDropStreamMessages = false;
  }
  if (host.worker) {
    host.postWorkerCommand({ type: "e2e_set_enabled", enabled });
  }
}

export function closeActiveSnapshotStream(host: WorkspaceActiveSnapshotControlHost) {
  if (!host.e2eEnabled) return;
  if (host.worker) {
    host.postWorkerCommand({ type: "e2e_close_stream" });
    if (host.state.setConnection("disconnected")) {
      host.publish();
    }
    return;
  }
  try {
    host.ws?.close();
  } catch {
    // ignore
  }
  if (host.state.setConnection("disconnected")) {
    host.publish();
  }
}

export function setDropActiveSnapshotMessages(
  host: WorkspaceActiveSnapshotControlHost,
  drop: boolean,
) {
  if (!host.e2eEnabled) return;
  if (host.worker) {
    host.postWorkerCommand({ type: "e2e_set_drop_messages", drop });
    return;
  }
  host.e2eDropStreamMessages = drop;
}

export function getCanonicalStreamUrl(
  host: WorkspaceActiveSnapshotControlHost,
): string | null {
  if (!host.e2eEnabled) return null;
  const daemonConfig = getDaemonClientConfig();
  const wsBaseUrl = host.wsBaseUrlOverride ?? daemonConfig.wsBaseUrl ?? null;
  const token = host.authTokenOverride ?? daemonConfig.authToken;
  if (!wsBaseUrl) return null;
  const qs = token ? `?token=${encodeURIComponent(token)}` : "";
  return `${wsBaseUrl.replace(/\/+$/, "")}/api/workspaces/${host.workspaceId}/active_snapshot/stream${qs}`;
}

export function setSubscribedSessionIds(
  host: WorkspaceActiveSnapshotControlHost,
  sessionIds: string[],
) {
  const activeSet = new Set(host.state.getActiveSessionIds());
  const next = sessionIds
    .map((id) => String(id || "").trim())
    .filter((id) => id.length > 0 && !activeSet.has(id));
  const deduped = Array.from(new Set(next));
  if (deduped.join("|") === host.subscribedSessionIds.join("|")) return;
  host.subscribedSessionIds = deduped;
  if (host.worker) {
    host.postWorkerCommand({ type: "set_subscribed_session_ids", sessionIds: deduped });
    return;
  }
  flushSubscriptions(host, "session_ids");
}

export function setForegroundTaskId(
  host: WorkspaceActiveSnapshotControlHost,
  taskId: string | null,
) {
  const normalized = typeof taskId === "string" ? taskId.trim() : "";
  const next = normalized ? normalized : null;
  if (next === host.foregroundTaskId) return;
  host.foregroundTaskId = next;
  if (host.worker) {
    host.postWorkerCommand({ type: "set_foreground_task_id", taskId: next });
    return;
  }
  scheduleForegroundTaskFlush(host);
}

export function flushSubscriptions(
  host: WorkspaceActiveSnapshotControlHost,
  reason = "subscribe",
) {
  const ws = host.ws;
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  const { message, requestSnapshot } = buildWorkspaceActiveSubscribeMessage(
    reason,
    host.foregroundTaskId,
    host.subscribedSessionIds,
  );
  if (requestSnapshot) {
    host.scheduleSnapshotWarning(reason);
  }
  try {
    ws.send(JSON.stringify(message));
  } catch {
    // ignore send errors
  }
}

export function scheduleForegroundTaskFlush(host: WorkspaceActiveSnapshotControlHost) {
  if (host.foregroundTaskTimer) {
    globalThis.clearTimeout(host.foregroundTaskTimer);
  }
  host.foregroundTaskTimer = globalThis.setTimeout(() => {
    host.foregroundTaskTimer = null;
    flushSubscriptions(host, "foreground_task");
  }, FOREGROUND_TASK_DEBOUNCE_MS);
}
