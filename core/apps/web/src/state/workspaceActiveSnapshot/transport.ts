import type {
  SessionHeadDelta,
  SessionHeadSnapshot,
  WorkspaceActiveSnapshot,
} from "@ctx/types";

const isWorkspaceActiveSnapshot = (value: unknown): value is WorkspaceActiveSnapshot => {
  if (!value || typeof value !== "object") return false;
  const active = (value as WorkspaceActiveSnapshot).active as { tasks?: unknown } | undefined;
  if (!active || typeof active !== "object") return false;
  return Array.isArray(active.tasks);
};

export const shouldRequestWorkspaceSnapshot = (reason: string): boolean => {
  switch (reason) {
    case "ws_open":
    case "reset_required":
    case "active_task_upsert":
    case "session_gap":
    case "snapshot_rev_reset":
    case "session_ids":
    case "stream_seq_gap":
    case "stream_seq_reset":
    case "foreground_session":
      return true;
    default:
      return false;
  }
};

export const toWorkspaceHttpBaseUrl = (base: string): string => {
  const trimmed = base.replace(/\/+$/, "");
  if (trimmed.startsWith("ws://")) return trimmed.replace(/^ws:\/\//, "http://");
  if (trimmed.startsWith("wss://")) return trimmed.replace(/^wss:\/\//, "https://");
  return trimmed;
};

export const readWorkspaceStreamRev = (value: unknown): number | null => {
  if (!value || typeof value !== "object") return null;
  const rec = value as { rev?: unknown };
  return typeof rec.rev === "number" ? rec.rev : null;
};

export const readWorkspaceSnapshotPayload = (
  value: unknown,
): { snapshot: WorkspaceActiveSnapshot; heads: SessionHeadSnapshot[] } | null => {
  if (!value || typeof value !== "object") return null;
  const rec = value as Record<string, unknown>;
  const directSnapshot =
    rec.type === "snapshot" && isWorkspaceActiveSnapshot(value) ? (value as WorkspaceActiveSnapshot) : null;
  const candidate =
    (rec.snapshot as WorkspaceActiveSnapshot | undefined) ??
    (rec.active_snapshot as WorkspaceActiveSnapshot | undefined) ??
    (rec.activeSnapshot as WorkspaceActiveSnapshot | undefined) ??
    directSnapshot ??
    null;
  if (!isWorkspaceActiveSnapshot(candidate)) return null;
  const headsPayload =
    (rec.heads as unknown) ??
    (rec.active_heads as unknown) ??
    (rec.activeHeads as unknown) ??
    [];
  const headsArray = Array.isArray(headsPayload)
    ? headsPayload
    : Array.isArray((headsPayload as { heads?: unknown }).heads)
      ? (headsPayload as { heads: SessionHeadSnapshot[] }).heads
      : [];
  return { snapshot: candidate, heads: headsArray };
};

export const readWorkspaceHeadsBatchPayload = (
  value: unknown,
): { snapshotRev: number; deltas: SessionHeadDelta[] } | null => {
  if (!value || typeof value !== "object") return null;
  const rec = value as Record<string, unknown>;
  if (rec.type !== "heads_batch") return null;
  const snapshotRev =
    (rec.snapshot_rev as number | undefined) ?? (rec.snapshotRev as number | undefined) ?? 0;
  const deltas = Array.isArray(rec.deltas) ? (rec.deltas as SessionHeadDelta[]) : [];
  return { snapshotRev, deltas };
};
