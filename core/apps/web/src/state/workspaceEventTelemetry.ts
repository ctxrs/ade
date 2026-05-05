import type { WorkspaceActiveSnapshotEvent } from "@ctx/types";

const receivedAtByEvent = new WeakMap<object, number>();

export const markWorkspaceEventReceivedAt = (
  event: WorkspaceActiveSnapshotEvent,
  receivedAtMs: number,
): void => {
  if (!Number.isFinite(receivedAtMs)) return;
  receivedAtByEvent.set(event, receivedAtMs);
};

export const readWorkspaceEventReceivedAt = (
  event: WorkspaceActiveSnapshotEvent,
): number | null => {
  const value = receivedAtByEvent.get(event);
  return typeof value === "number" && Number.isFinite(value) ? value : null;
};
