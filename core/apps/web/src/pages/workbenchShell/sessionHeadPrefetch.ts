import type { SessionHead, SessionHeadSnapshot, WorkspaceActiveSnapshotEvent } from "@ctx/types";
import { idToString } from "../../api/client";
import { SessionHeadBootstrapCache } from "../../state/sessionHeadBootstrapCache";
import { loadSessionHeadV1 } from "../../state/uiStateStore";
import { shouldReplaceSessionHead } from "../../state/workspaceActiveSnapshot/summaryHelpers";
import {
  type WorkspaceActiveSnapshotEventSource,
  type WorkspaceActiveSnapshotState,
} from "../../state/workspaceActiveSnapshotStore";

type SessionHeadStoreReader = Pick<WorkspaceActiveSnapshotEventSource, "getSessionHeadSnapshot"> & {
  getSessionHeadsSnapshot?: () => Record<string, SessionHeadSnapshot>;
};

const getPrimarySessionIdForTask = (
  snapshot: WorkspaceActiveSnapshotState,
  taskId: string,
): string | null => {
  const item = snapshot.tasksById[taskId];
  if (!item) return null;

  return (
    item.primarySessionId ||
    idToString(item.task.primary_session_id ?? "") ||
    idToString(item.primarySessionHead?.session?.id ?? "")
  );
};

export const collectWorkspaceSessionHeadIds = (
  snapshot: WorkspaceActiveSnapshotState,
): string[] => {
  const ids = new Set<string>();
  for (const taskId of snapshot.activeIds) {
    const item = snapshot.tasksById[taskId];
    const primaryId = getPrimarySessionIdForTask(snapshot, taskId);
    if (primaryId) ids.add(primaryId);
    for (const summary of item?.sessions ?? []) {
      const sessionId = idToString(summary.session?.id ?? "");
      if (sessionId) ids.add(sessionId);
    }
  }
  return Array.from(ids);
};

export const collectSessionHeadsForSupervisor = (
  snapshot: WorkspaceActiveSnapshotState,
  store: SessionHeadStoreReader,
  bootstrapCache: SessionHeadBootstrapCache,
): Record<string, SessionHeadSnapshot> => {
  const batchHeads = store.getSessionHeadsSnapshot?.();
  const out: Record<string, SessionHeadSnapshot> = batchHeads ? { ...batchHeads } : {};

  for (const sessionId of collectWorkspaceSessionHeadIds(snapshot)) {
    const head = store.getSessionHeadSnapshot(sessionId);
    if (head) {
      out[sessionId] = head;
    }
  }

  for (const [sessionId, head] of Object.entries(bootstrapCache.snapshot())) {
    if (shouldReplaceSessionHead(out[sessionId], head)) {
      out[sessionId] = head;
    }
  }

  return out;
};

const persistedHeadToSnapshot = (head: SessionHead): SessionHeadSnapshot => {
  const headWithOptionalStateRev = head as SessionHead & { state_rev?: number };
  return {
    ...head,
    state_rev:
      typeof headWithOptionalStateRev.state_rev === "number"
        ? headWithOptionalStateRev.state_rev
        : undefined,
    has_more_history: head.has_more_turns,
    history_cursor: head.has_more_turns ? null : null,
  };
};

export const primePersistedSessionHeads = async (
  snapshot: WorkspaceActiveSnapshotState,
  store: SessionHeadStoreReader,
  bootstrapCache: SessionHeadBootstrapCache,
): Promise<boolean> => {
  const batchHeads = store.getSessionHeadsSnapshot?.() ?? {};
  let changed = false;

  await Promise.all(
    collectWorkspaceSessionHeadIds(snapshot).map(async (sessionId) => {
      if (!bootstrapCache.beginPersistedPrefetch(sessionId)) return;
      const persisted = await loadSessionHeadV1(sessionId).catch(() => null);
      if (!persisted?.head) return;
      const persistedHead = persistedHeadToSnapshot(persisted.head);
      const directHead = batchHeads[sessionId] ?? store.getSessionHeadSnapshot(sessionId);
      if (directHead && !shouldReplaceSessionHead(directHead, persistedHead)) {
        return;
      }
      if (bootstrapCache.upsert(persistedHead)) {
        changed = true;
      }
    }),
  );

  return changed;
};

export const maybeCacheSessionHeadSeed = (
  cache: SessionHeadBootstrapCache,
  evt: WorkspaceActiveSnapshotEvent,
): boolean => {
  if (evt.type !== "session_head_seed") return false;
  return cache.upsert((evt as { head?: SessionHeadSnapshot }).head);
};
