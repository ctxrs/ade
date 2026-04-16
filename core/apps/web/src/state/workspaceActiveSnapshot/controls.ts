import type { WorkspaceActiveSnapshotEvent } from "@ctx/types";
import { getDaemonClientConfig } from "../../api/client";
import type {
  WorkspaceActiveSnapshotCommand,
  WorkspaceActiveSnapshotPatch,
} from "../workspaceActiveSnapshotProtocol";
import type { SessionSubscriptionCursor } from "../sessionSubscription";
import {
  normalizeSessionSubscriptionCursors,
  sameSessionSubscriptionCursorIds,
  sameSessionSubscriptionCursors,
  type SessionSubscriptionReplay,
} from "../sessionSubscription";
import { buildWorkspaceActiveSubscribeMessage } from "./subscriptions";
import type { WorkspaceActiveSnapshotStoreState } from "./storeState";

export type WorkspaceActiveSnapshotControlHost = {
  e2eEnabled: boolean;
  e2eDropStreamMessages: boolean;
  worker: Worker | null;
  state: WorkspaceActiveSnapshotStoreState;
  ws: WebSocket | null;
  wsBaseUrlOverride: string | null;
  authTokenOverride: string | null;
  workspaceId: string;
  subscribedSessions: SessionSubscriptionCursor[];
  foregroundSessionId: string | null;
  eventListeners: Set<(event: WorkspaceActiveSnapshotEvent) => void>;
  workerPatchEmitter: ((patch: WorkspaceActiveSnapshotPatch) => void) | null;
  workerPatchPendingEvents: WorkspaceActiveSnapshotEvent[];
  postWorkerCommand(cmd: WorkspaceActiveSnapshotCommand): void;
  publish(): void;
  enqueueStreamMessage(data: unknown): void;
  scheduleSnapshotWarning(reason: string): void;
  scheduleWorkerPatchFlush(): void;
  flushWorkerPatchNow(): void;
};

const isImmediateWorkerPatchEvent = (
  host: WorkspaceActiveSnapshotControlHost,
  evt: WorkspaceActiveSnapshotEvent,
): boolean => {
  const normalizeId = (value: string | null | undefined): string =>
    typeof value === "string" ? value.trim() : "";
  const foregroundSessionId = normalizeId(host.foregroundSessionId);
  if (!foregroundSessionId) return false;
  const isForegroundSession = (sessionId: string | null | undefined): boolean =>
    normalizeId(sessionId) === foregroundSessionId;

  switch (evt.type) {
    case "session_gap":
      return isForegroundSession(evt.session_id);
    case "session_head_seed":
      return isForegroundSession(evt.head.session.id);
    case "session_summary":
      return isForegroundSession(evt.summary.session.id);
    case "session_summary_delta":
      return isForegroundSession(evt.delta.session_id);
    case "session_head_delta": {
      if (!isForegroundSession(evt.delta.session_id)) return false;
      if (evt.delta.message) return true;
      const eventType = String(evt.delta.event?.event_type ?? "");
      if (!eventType) return false;
      return (
        eventType !== "assistant_chunk" &&
        eventType !== "thought_chunk" &&
        eventType !== "context_window_update"
      );
    }
    default:
      return false;
  }
};

const compareResumeReplay = (
  left: Extract<SessionSubscriptionReplay, { kind: "resume" }>,
  right: Extract<SessionSubscriptionReplay, { kind: "resume" }>,
): number => {
  if (left.afterSeq !== right.afterSeq) {
    return left.afterSeq - right.afterSeq;
  }
  return (left.afterProjectionRev ?? 0) - (right.afterProjectionRev ?? 0);
};

const replayControlChanged = (
  previous: SessionSubscriptionReplay,
  next: SessionSubscriptionReplay,
): boolean => {
  if (previous.kind !== next.kind) {
    return previous.kind === "reset" || next.kind === "reset";
  }
  if (previous.kind !== "resume" || next.kind !== "resume") {
    return false;
  }
  return compareResumeReplay(next, previous) < 0;
};

const shouldFlushLiveSubscriptionUpdate = (
  previous: SessionSubscriptionCursor[],
  next: SessionSubscriptionCursor[],
): boolean => {
  if (!sameSessionSubscriptionCursorIds(previous, next)) {
    return true;
  }
  for (let index = 0; index < previous.length; index += 1) {
    const prior = previous[index];
    const current = next[index];
    if (!prior || !current) return true;
    if (replayControlChanged(prior.replay, current.replay)) {
      return true;
    }
  }
  return false;
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
    if (isImmediateWorkerPatchEvent(host, evt)) {
      host.flushWorkerPatchNow();
      return;
    }
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

export function setSubscribedSessions(
  host: WorkspaceActiveSnapshotControlHost,
  sessions: SessionSubscriptionCursor[],
) {
  const deduped = normalizeSessionSubscriptionCursors(sessions);
  if (sameSessionSubscriptionCursors(deduped, host.subscribedSessions)) return;
  const previous = host.subscribedSessions;
  const idsChanged = !sameSessionSubscriptionCursorIds(deduped, previous);
  host.subscribedSessions = deduped;
  if (host.worker) {
    host.postWorkerCommand({ type: "set_subscribed_sessions", sessions: deduped });
    return;
  }
  if (!shouldFlushLiveSubscriptionUpdate(previous, deduped)) {
    return;
  }
  flushSubscriptions(host, idsChanged ? "session_ids" : "session_cursors");
}

export function setForegroundSessionId(
  host: WorkspaceActiveSnapshotControlHost,
  sessionId: string | null,
) {
  const normalized = typeof sessionId === "string" ? sessionId.trim() : "";
  const next = normalized ? normalized : null;
  if (next === host.foregroundSessionId) return;
  host.foregroundSessionId = next;
  if (host.worker) {
    host.postWorkerCommand({ type: "set_foreground_session_id", sessionId: next });
    return;
  }
  flushSubscriptions(host, "foreground_session");
}

export function flushSubscriptions(
  host: WorkspaceActiveSnapshotControlHost,
  reason = "subscribe",
) {
  const ws = host.ws;
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  const { message, requestSnapshot } = buildWorkspaceActiveSubscribeMessage(
    reason,
    host.foregroundSessionId,
    host.subscribedSessions,
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
