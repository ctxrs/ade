import type { SessionHeadSnapshot } from "../../api/client";
import { findWorkspaceSessionHead } from "../workspaceActiveSnapshot/projection";
import { isReplicaAuthority, shouldSkipBoundedActiveSnapshotSeed } from "./config";
import type { InternalEntry } from "./entryState";
import type { SessionSupervisorWorkspaceSnapshotState } from "./workspaceInputs";

type ActiveSnapshotSeedHost = {
  workspaceSnapshotState: SessionSupervisorWorkspaceSnapshotState;
  workspaceSessionHeadsById: Map<string, SessionHeadSnapshot>;
  dispatchSeedHead(cmd: { type: "seed_head"; sessionId: string; head: SessionHeadSnapshot }): void;
};

export function canSeedReplicaFromActiveSnapshot(
  entry: InternalEntry,
  opts?: { allowRecoveringRefresh?: boolean },
): boolean {
  if (opts?.allowRecoveringRefresh && entry.freshness === "recovering") {
    return true;
  }
  return (
    !isReplicaAuthority(entry.freshness) &&
    !entry.turnsHydrated &&
    entry.messages.length === 0 &&
    entry.events.length === 0
  );
}

export function seedReplicaFromActiveSnapshot(
  host: ActiveSnapshotSeedHost,
  sessionId: string,
  entry: InternalEntry,
): boolean {
  if (!canSeedReplicaFromActiveSnapshot(entry)) return false;
  const head = findWorkspaceSessionHead(
    host.workspaceSnapshotState,
    host.workspaceSessionHeadsById,
    sessionId,
  );
  if (!head) return false;
  if (shouldSkipBoundedActiveSnapshotSeed(entry, head)) return false;
  host.dispatchSeedHead({ type: "seed_head", sessionId, head });
  return true;
}
