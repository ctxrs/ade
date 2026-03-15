import type { SessionHeadSnapshot } from "../../api/client";
import type { SessionSubscriptionCursor } from "../sessionSubscription";
import type { InternalEntry } from "./entryState";
import { buildSessionSubscriptionPlan } from "./sessionSubscriptionPlan";

export function buildSubscribedSessions(
  subscribedSessionIds: string[],
  entries: Map<string, InternalEntry>,
  workspaceSessionHeadsById: Map<string, SessionHeadSnapshot>,
): SessionSubscriptionCursor[] {
  return subscribedSessionIds.map((sessionId) => {
    const entry = entries.get(sessionId);
    const headSeq = workspaceSessionHeadsById.get(sessionId)?.last_event_seq;
    if (entry?.freshness === "recovering") {
      return {
        sessionId,
        replay: { kind: "reset" },
      };
    }
    const afterSeq =
      typeof entry?.lastEventSeq === "number"
        ? entry.lastEventSeq
        : typeof headSeq === "number"
          ? headSeq
          : null;
    return {
      sessionId,
      replay: typeof afterSeq === "number" ? { kind: "resume", afterSeq } : { kind: "auto" },
    };
  });
}

export function applySessionActivityUpdate(
  entries: Map<string, InternalEntry>,
  sessionId: string,
  activity: InternalEntry["activity"],
  version?: { lastEventSeq?: number | null; stateRev?: number | null },
): boolean {
  const entry = entries.get(sessionId);
  if (!entry || activity === undefined) return false;
  const incomingStateRev = typeof version?.stateRev === "number" ? version.stateRev : null;
  const incomingLastEventSeq =
    typeof version?.lastEventSeq === "number" ? version.lastEventSeq : null;
  if (
    incomingStateRev === null &&
    incomingLastEventSeq === null &&
    (typeof entry.stateRev === "number" || typeof entry.lastEventSeq === "number")
  ) {
    return false;
  }
  if (
    incomingStateRev !== null &&
    typeof entry.stateRev === "number" &&
    incomingStateRev < entry.stateRev
  ) {
    return false;
  }
  if (
    incomingLastEventSeq !== null &&
    typeof entry.lastEventSeq === "number" &&
    incomingLastEventSeq < entry.lastEventSeq
  ) {
    return false;
  }
  if (
    (entry.activity?.is_working ?? false) === (activity?.is_working ?? false) &&
    (entry.activity?.last_turn_status ?? null) === (activity?.last_turn_status ?? null)
  ) {
    return false;
  }
  entry.activity = activity ?? null;
  entry.updatedAtMs = Date.now();
  return true;
}

export type SessionSupervisorSubscriptionHost = {
  entries: Map<string, InternalEntry>;
  activeTaskSessionIds: string[];
  warmSessionIds: string[];
  subscribedSessionIds: string[];
  setSubscribedSessionIds(next: string[]): void;
  emitSubscribedSessions(): void;
  ensureEntry(sessionId: string): InternalEntry;
  publish(): void;
};

export function refreshSubscriptions(
  host: SessionSupervisorSubscriptionHost,
  opts?: { emitIfUnchanged?: boolean },
) {
  const openSessionIds = Array.from(host.entries.values())
    .filter((entry) => entry.refCount > 0)
    .map((entry) => entry.sessionId);
  const plan = buildSessionSubscriptionPlan({
    openSessionIds,
    activeTaskSessionIds: host.activeTaskSessionIds,
    warmSessionIds: host.warmSessionIds,
    previousSubscribedSessionIds: host.subscribedSessionIds,
  });
  if (!plan.changed) {
    if (opts?.emitIfUnchanged) {
      host.emitSubscribedSessions();
    }
    return;
  }
  const nextSet = new Set(plan.nextSubscribedSessionIds);
  host.setSubscribedSessionIds(plan.nextSubscribedSessionIds);
  for (const entry of host.entries.values()) {
    entry.subscribed = nextSet.has(entry.sessionId);
  }
  for (const sessionId of plan.addedSessionIds) {
    const entry = host.ensureEntry(sessionId);
    entry.subscribed = true;
  }
  host.emitSubscribedSessions();
  host.publish();
}

export type SessionSupervisorRecoveryHost = {
  entries: Map<string, InternalEntry>;
  emitSubscribedSessions(): void;
  publish(): void;
};

export function markOpenSessionsRecovering(host: SessionSupervisorRecoveryHost) {
  let changed = false;
  for (const entry of host.entries.values()) {
    if (entry.refCount <= 0) continue;
    if (entry.loadState === "fatal") continue;
    if (entry.loadState !== "recovering") {
      entry.loadState = "recovering";
      changed = true;
    }
    if (entry.freshness !== "recovering") {
      entry.freshness = "recovering";
      changed = true;
    }
    entry.updatedAtMs = Date.now();
  }
  if (changed) {
    host.publish();
    host.emitSubscribedSessions();
  }
}

export function emitSubscribedSessions(
  sink: ((sessions: SessionSubscriptionCursor[]) => void) | null,
  sessions: SessionSubscriptionCursor[],
) {
  sink?.(sessions);
}
