import type { SessionHeadSnapshot } from "../../api/client";
import type { SessionReplicaHeadSeedMode } from "../sessionReplicaProtocol";
import { findWorkspaceSessionHead } from "../workspaceActiveSnapshot/projection";
import { isReplicaAuthority, shouldSkipBoundedActiveSnapshotSeed } from "./config";
import type { InternalEntry } from "./entryState";
import type { SessionSupervisorWorkspaceSnapshotState } from "./workspaceInputs";

type ActiveSnapshotSeedHost = {
  workspaceSnapshotState: SessionSupervisorWorkspaceSnapshotState;
  workspaceSessionHeadsById: Map<string, SessionHeadSnapshot>;
  dispatchSeedHead(cmd: {
    type: "seed_head";
    sessionId: string;
    head: SessionHeadSnapshot;
    mode: SessionReplicaHeadSeedMode;
  }): void;
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

const readVersion = (value: number | null | undefined): number =>
  typeof value === "number" && Number.isFinite(value) ? value : -1;

const coversTurns = (entry: InternalEntry, head: SessionHeadSnapshot): boolean => {
  const headTurns = Array.isArray(head.turns) ? head.turns : [];
  if (headTurns.length === 0) return true;
  const entryTurnIds = new Set(entry.turns.map((turn) => String(turn.turn_id ?? "").trim()).filter(Boolean));
  return headTurns.every((turn) => entryTurnIds.has(String(turn.turn_id ?? "").trim()));
};

const coversMessages = (entry: InternalEntry, head: SessionHeadSnapshot): boolean => {
  const headMessages = Array.isArray(head.messages) ? head.messages : [];
  if (headMessages.length === 0) return true;
  const entryMessageIds = new Set(entry.messages.map((message) => String(message.id ?? "").trim()).filter(Boolean));
  return headMessages.every((message) => entryMessageIds.has(String(message.id ?? "").trim()));
};

export function shouldRepairReplicaFromActiveSnapshot(
  entry: InternalEntry,
  head: SessionHeadSnapshot,
): boolean {
  const headLastEventSeq = readVersion(head.last_event_seq);
  const entryLastEventSeq = readVersion(entry.lastEventSeq);
  const headProjectionRev = readVersion(head.projection_rev);
  const entryProjectionRev = readVersion(entry.projectionRev);

  const missingTranscript =
    !entry.turnsHydrated &&
    entry.turns.length === 0 &&
    entry.messages.length === 0 &&
    entry.events.length === 0;
  if (missingTranscript) {
    return false;
  }

  const recovering = entry.freshness === "recovering" || entry.loadState === "recovering";
  const versionsNotOlder =
    (headLastEventSeq < 0 || entryLastEventSeq < 0 || headLastEventSeq >= entryLastEventSeq) &&
    (headProjectionRev < 0 || entryProjectionRev < 0 || headProjectionRev >= entryProjectionRev);
  if (recovering && versionsNotOlder && coversTurns(entry, head) && coversMessages(entry, head)) {
    return true;
  }

  if (!coversTurns(entry, head)) {
    return true;
  }

  if (!coversMessages(entry, head)) {
    return true;
  }

  if (headLastEventSeq > entryLastEventSeq) {
    return true;
  }

  if (headProjectionRev > entryProjectionRev) {
    return true;
  }

  const headWorking = Boolean(head.activity?.is_working);
  const entryWorking = Boolean(entry.activity?.is_working);
  const headStatus = head.activity?.last_turn_status ?? null;
  const entryStatus = entry.activity?.last_turn_status ?? null;
  return headWorking !== entryWorking || headStatus !== entryStatus;
}

export function classifyActiveSnapshotSeedMode(
  entry: InternalEntry,
  head: SessionHeadSnapshot,
  opts?: { allowRecoveringRefresh?: boolean },
): SessionReplicaHeadSeedMode | null {
  if (shouldRepairReplicaFromActiveSnapshot(entry, head)) {
    return "repair_replace";
  }
  if (canSeedReplicaFromActiveSnapshot(entry, opts)) {
    if (!shouldSkipBoundedActiveSnapshotSeed(entry, head)) {
      return "bootstrap_seed";
    }
    return null;
  }
  return null;
}

export function seedReplicaFromActiveSnapshot(
  host: ActiveSnapshotSeedHost,
  sessionId: string,
  entry: InternalEntry,
): boolean {
  const head = findWorkspaceSessionHead(
    host.workspaceSnapshotState,
    host.workspaceSessionHeadsById,
    sessionId,
  );
  if (!head) return false;
  const recoveringBootstrap =
    (entry.freshness === "recovering" || entry.loadState === "recovering") &&
    !shouldSkipBoundedActiveSnapshotSeed(entry, head);
  if (recoveringBootstrap) {
    host.dispatchSeedHead({ type: "seed_head", sessionId, head, mode: "bootstrap_seed" });
    return true;
  }
  if (classifyActiveSnapshotSeedMode(entry, head) !== "bootstrap_seed") return false;
  host.dispatchSeedHead({ type: "seed_head", sessionId, head, mode: "bootstrap_seed" });
  return true;
}
