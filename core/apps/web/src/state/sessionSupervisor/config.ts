import type { SessionHeadSnapshot } from "../../api/client";
import type { InternalEntry } from "./entryState";
import type { SessionReplicaFreshnessState } from "../sessionReplicaProtocol";

const readTunableInt = (key: string, fallback: number) => {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return fallback;
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
    return parsed;
  } catch {
    return fallback;
  }
};

export const TURN_PAGE_LIMIT = readTunableInt("contextTurnPageLimit", 60);
export const WARM_SESSION_BUDGET = readTunableInt("contextWarmSessionBudget", 12);
export const EVENT_BUFFER_LIMIT = readTunableInt("contextEventBufferLimit", 800);
export const MAX_CACHED_SESSIONS = readTunableInt(
  "contextMaxCachedSessions",
  Math.max(30, WARM_SESSION_BUDGET * 3),
);
export const WARM_TTL_MS = readTunableInt("contextWarmSessionTtlMs", 10 * 60 * 1000);
export const HEAD_LIMIT = readTunableInt("contextSessionHeadLimit", TURN_PAGE_LIMIT);

export const isReplicaAuthority = (freshness: InternalEntry["freshness"]) =>
  freshness === "replica" || freshness === "authoritative";

export const toReplicaFreshness = (
  freshness: SessionReplicaFreshnessState,
): InternalEntry["freshness"] => (freshness === "authoritative" ? "replica" : freshness);

const isBoundedHeadSeed = (head: SessionHeadSnapshot): boolean =>
  typeof head.head_window?.turn_limit === "number" && head.head_window.turn_limit > 0;

export const shouldSkipBoundedBootstrapSeed = (
  entry: InternalEntry,
  head: SessionHeadSnapshot,
): boolean => {
  const freshBootstrapOpen =
    entry.freshness === "bootstrap" &&
    !entry.turnsHydrated &&
    entry.turns.length === 0 &&
    entry.messages.length === 0 &&
    entry.events.length === 0;
  return freshBootstrapOpen && isBoundedHeadSeed(head);
};
