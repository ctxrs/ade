import type { SessionSnapshotSummary, WorktreeVcsSnapshot } from "@ctx/types";
import { idToString } from "../../api/client";

export const sortSessionSummaries = (summaries: SessionSnapshotSummary[]): SessionSnapshotSummary[] => {
  return summaries
    .slice()
    .sort((a, b) => String(a.session.created_at ?? "").localeCompare(String(b.session.created_at ?? "")));
};

export const mapWorktreeVcsSnapshots = (
  snapshots?: WorktreeVcsSnapshot[] | null,
): Record<string, WorktreeVcsSnapshot> => {
  const out: Record<string, WorktreeVcsSnapshot> = {};
  if (!Array.isArray(snapshots)) return out;
  for (const snapshot of snapshots) {
    const id = idToString(snapshot?.worktree_id ?? "");
    if (!id) continue;
    out[id] = snapshot;
  }
  return out;
};

export const hasOwnProperty = (value: unknown, key: string): boolean => {
  if (!value || typeof value !== "object") return false;
  return Object.prototype.hasOwnProperty.call(value, key);
};

export const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

export const readString = (value: unknown): string | null => {
  if (typeof value !== "string") return null;
  return value;
};
