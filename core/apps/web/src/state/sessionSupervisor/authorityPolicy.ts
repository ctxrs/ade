import type { SessionReplicaData, SessionReplicaPatch } from "../sessionReplicaProtocol";
import { isReplicaAuthority } from "./config";
import type { InternalEntry, SessionLoadState } from "./entryState";

const hasDurableSeq = (value: number | undefined): value is number =>
  typeof value === "number" && value >= 0;

export const hasVisibleBootstrapOnlyTranscript = (
  entry: Pick<InternalEntry, "mode" | "freshness">,
): boolean => entry.mode === "active" && !isReplicaAuthority(entry.freshness);

export const resolveReplicaReadyLoadState = (
  entry: Pick<InternalEntry, "mode" | "freshness">,
): SessionLoadState => (hasVisibleBootstrapOnlyTranscript(entry) ? "pending_hydration" : "live");

export const hasSessionReplicaRecoveryData = (data: SessionReplicaData): boolean =>
  data.session !== undefined ||
  (Array.isArray(data.turns) && data.turns.length > 0) ||
  (Array.isArray(data.messages) && data.messages.length > 0) ||
  (Array.isArray(data.events) && data.events.length > 0) ||
  (Array.isArray(data.toolSummaries) && data.toolSummaries.length > 0) ||
  data.lastEventSeq !== undefined ||
  data.projectionRev !== undefined ||
  data.stateRev !== undefined ||
  data.summaryCheckpoint !== undefined ||
  data.headWindow !== undefined ||
  data.hasMoreTurns !== undefined ||
  data.turnsHydrated !== undefined;

export const shouldReplayReplicaReplace = ({
  entry,
  patch,
  normalizedFreshness,
}: {
  entry: Pick<InternalEntry, "freshness" | "projectionRev" | "lastEventSeq">;
  patch: SessionReplicaPatch;
  normalizedFreshness?: InternalEntry["freshness"];
}): boolean => {
  if (patch.op !== "replace") return true;
  if (!isReplicaAuthority(entry.freshness)) return true;
  if (normalizedFreshness === "recovering") return true;
  if (patch.data.forceReplace !== true || normalizedFreshness !== "replica") return false;

  const incomingProjectionRev =
    typeof patch.data.projectionRev === "number" ? patch.data.projectionRev : undefined;
  const currentProjectionRev =
    typeof entry.projectionRev === "number" ? entry.projectionRev : undefined;
  if (hasDurableSeq(incomingProjectionRev) && !hasDurableSeq(currentProjectionRev)) return true;
  if (
    hasDurableSeq(incomingProjectionRev) &&
    hasDurableSeq(currentProjectionRev) &&
    incomingProjectionRev > currentProjectionRev
  ) {
    return true;
  }

  const incomingLastEventSeq =
    typeof patch.data.lastEventSeq === "number" ? patch.data.lastEventSeq : undefined;
  const currentLastEventSeq =
    typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : undefined;
  if (hasDurableSeq(incomingLastEventSeq) && !hasDurableSeq(currentLastEventSeq)) return true;
  if (
    hasDurableSeq(incomingLastEventSeq) &&
    hasDurableSeq(currentLastEventSeq) &&
    incomingLastEventSeq > currentLastEventSeq
  ) {
    return true;
  }

  return false;
};
