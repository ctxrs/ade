import type {
  SessionHead,
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import { idToString } from "../../api/client";
import { getSessionHead } from "../../api/clientSessions";
import { SessionHeadBootstrapCache } from "../../state/sessionHeadBootstrapCache";
import { HEAD_LIMIT } from "../../state/sessionSupervisor/config";
import { loadSessionHeadV1 } from "../../state/uiStateStore";
import {
  isSessionHeadCompatibleWithSummary,
  shouldReplaceSessionHead,
} from "../../state/workspaceActiveSnapshot/summaryHelpers";
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

const collectTargetSessionIds = (
  snapshot: WorkspaceActiveSnapshotState,
  sessionIds?: readonly string[],
): string[] => {
  return Array.from(new Set((sessionIds ?? collectWorkspaceSessionHeadIds(snapshot)).filter(Boolean)));
};

const findSessionSummary = (
  snapshot: WorkspaceActiveSnapshotState,
  sessionId: string,
): SessionSnapshotSummary | null => {
  const normalizedSessionId = idToString(sessionId);
  if (!normalizedSessionId) return null;
  for (const taskId of snapshot.activeIds) {
    const item = snapshot.tasksById[taskId];
    const summary =
      item?.sessions.find((candidate) => idToString(candidate.session?.id ?? "") === normalizedSessionId) ??
      null;
    if (summary) return summary;
  }
  return null;
};

const buildPrefetchVersionKey = (
  summary: SessionSnapshotSummary | null,
  sessionId: string,
): string => {
  const lastEventSeq =
    typeof summary?.last_event_seq === "number" && Number.isFinite(summary.last_event_seq)
      ? summary.last_event_seq
      : "none";
  const projectionRev =
    typeof summary?.projection_rev === "number" && Number.isFinite(summary.projection_rev)
      ? summary.projection_rev
      : "none";
  const stateRev =
    typeof summary?.state_rev === "number" && Number.isFinite(summary.state_rev)
      ? summary.state_rev
      : "none";
  return `${sessionId}:${lastEventSeq}:${projectionRev}:${stateRev}`;
};

export const collectSessionHeadsForSupervisor = (
  snapshot: WorkspaceActiveSnapshotState,
  store: SessionHeadStoreReader,
  bootstrapCache: SessionHeadBootstrapCache,
  sessionIds?: readonly string[],
): Record<string, SessionHeadSnapshot> => {
  const out: Record<string, SessionHeadSnapshot> = {};
  const targetSessionIds = collectTargetSessionIds(snapshot, sessionIds);
  const batchHeads = store.getSessionHeadsSnapshot?.() ?? {};
  const bootstrapHeads = bootstrapCache.snapshot();

  for (const sessionId of targetSessionIds) {
    const batchHead = batchHeads[sessionId];
    if (batchHead) {
      out[sessionId] = batchHead;
    }
    const head = store.getSessionHeadSnapshot(sessionId);
    if (head && shouldReplaceSessionHead(out[sessionId], head)) {
      out[sessionId] = head;
    }
  }

  for (const [sessionId, head] of Object.entries(bootstrapHeads)) {
    if (targetSessionIds.length > 0 && !targetSessionIds.includes(sessionId)) continue;
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
  sessionIds?: readonly string[],
): Promise<boolean> => {
  const batchHeads = store.getSessionHeadsSnapshot?.() ?? {};
  let changed = false;
  const targetSessionIds = collectTargetSessionIds(snapshot, sessionIds);

  await Promise.all(
    targetSessionIds.map(async (sessionId) => {
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

export const primeAuthoritativeSessionHeads = async (
  snapshot: WorkspaceActiveSnapshotState,
  store: SessionHeadStoreReader,
  bootstrapCache: SessionHeadBootstrapCache,
  sessionIds?: readonly string[],
  opts?: {
    onHead?: (sessionId: string, head: SessionHeadSnapshot) => void;
  },
): Promise<boolean> => {
  const batchHeads = store.getSessionHeadsSnapshot?.() ?? {};
  let changed = false;
  const targetSessionIds = collectTargetSessionIds(snapshot, sessionIds);

  await Promise.all(
    targetSessionIds.map(async (sessionId) => {
      const summary = findSessionSummary(snapshot, sessionId);
      const directHead = batchHeads[sessionId] ?? store.getSessionHeadSnapshot(sessionId);
      if (isSessionHeadCompatibleWithSummary(summary, directHead)) {
        return;
      }
      const bootstrapHead = bootstrapCache.get(sessionId);
      if (isSessionHeadCompatibleWithSummary(summary, bootstrapHead)) {
        return;
      }
      const versionKey = buildPrefetchVersionKey(summary, sessionId);
      if (!bootstrapCache.beginAuthoritativePrefetch(sessionId, versionKey)) {
        return;
      }
      let fetchSucceeded = false;
      try {
        const head = await getSessionHead(sessionId, HEAD_LIMIT, true).catch(() => null);
        if (!head) return;
        fetchSucceeded = isSessionHeadCompatibleWithSummary(summary, head);
        const didChange = bootstrapCache.upsert(head);
        if (didChange) {
          changed = true;
          opts?.onHead?.(sessionId, head);
        }
      } finally {
        bootstrapCache.finishAuthoritativePrefetch(sessionId, versionKey, fetchSucceeded);
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
