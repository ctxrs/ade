import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from "react";
import { idToString, type SessionHeadSnapshot } from "../../api/client";
import type { WorkspaceActiveSnapshotEvent } from "@ctx/types";
import { SessionHeadBootstrapCache } from "../../state/sessionHeadBootstrapCache";
import {
  useSessionLifecycleCoordinator,
  type SessionSupervisor,
  type SessionSupervisorSnapshot,
} from "../../state/sessionSupervisor";
import type {
  WorkspaceActiveSnapshotEventSource,
  WorkspaceActiveSnapshotItem,
  WorkspaceActiveSnapshotState,
} from "../../state/workspaceActiveSnapshotStore";
import { WORKBENCH_TASK_IDLE_EVENT, type WorkbenchTaskIdleDetail } from "../../utils/updaterEvents";
import { hasSessionActiveTurn } from "../../utils/sessionActivity";
import type { WorkbenchStore } from "../../workbench/store";
import {
  noteNavThreadActivityMismatch,
  noteSwitchStaleVisible,
} from "../../state/foregroundFreshnessTelemetry";
import type { OptimisticTaskSummary } from "./WorkbenchPage.types";
import {
  collectSessionHeadsForSupervisor,
  primeAuthoritativeSessionHeads,
  maybeCacheSessionHeadSeed,
  primePersistedSessionHeads,
} from "./sessionHeadPrefetch";
import {
  canRenderWorkbenchActiveSession,
  deriveActiveTaskSessionIds,
  deriveProviderIdsByTask,
  deriveTaskLiveInfo,
  deriveWarmSessionIds,
  isWorkbenchTaskUnread,
  resolveWorkbenchActiveSessionId,
} from "./workbenchTaskActivity";

type TaskBridgeArgs = {
  activeTaskId: string | null;
  activeSessionIdFromTab: string | null;
  activeTaskSummary: WorkspaceActiveSnapshotItem | OptimisticTaskSummary | null;
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
  workspaceSnapshot: WorkspaceActiveSnapshotState;
  sessionSnap: SessionSupervisorSnapshot;
  optimisticTasks: OptimisticTaskSummary[];
  optimisticTasksById: Record<string, OptimisticTaskSummary>;
  supervisor: Pick<
    SessionSupervisor,
    | "setActiveTaskSessionIds"
    | "setWarmSessionIds"
    | "setSubscribedSessionIdsSink"
    | "setWorkspaceSnapshotState"
    | "setWorkspaceSessionHeads"
    | "upsertWorkspaceSessionHead"
    | "handleWorkspaceEvent"
  >;
  workbenchStore: Pick<WorkbenchStore, "getActiveTab" | "setActiveSessionForActiveTask">;
  workspaceSnapshotStore: Pick<
    WorkspaceActiveSnapshotEventSource,
    | "subscribe"
    | "subscribeEvents"
    | "getSnapshot"
    | "getSessionHeadSnapshot"
    | "setForegroundSessionId"
    | "setSubscribedSessions"
  > & { getSessionHeadsSnapshot?: () => Record<string, SessionHeadSnapshot> };
  markTaskRead: (taskId: string) => Promise<void>;
};

const readWorkspaceSessionHeads = (
  snapshot: WorkspaceActiveSnapshotState,
  store: TaskBridgeArgs["workspaceSnapshotStore"],
  bootstrapHeads: SessionHeadBootstrapCache,
  sessionIds?: readonly string[],
): Record<string, SessionHeadSnapshot> => {
  return collectSessionHeadsForSupervisor(snapshot, store, bootstrapHeads, sessionIds);
};

export function useWorkbenchSessionBridge({
  activeTaskId,
  activeSessionIdFromTab,
  activeTaskSummary,
  tasksById,
  workspaceSnapshot,
  sessionSnap,
  optimisticTasks,
  optimisticTasksById,
  supervisor,
  workbenchStore,
  workspaceSnapshotStore,
  markTaskRead,
}: TaskBridgeArgs) {
  const lifecycleCoordinator = useSessionLifecycleCoordinator();
  const { sessionSummaries, sessions, sessionIds, primarySessionId } = useMemo(
    () => deriveActiveTaskSessionIds(activeTaskSummary, activeSessionIdFromTab),
    [activeSessionIdFromTab, activeTaskSummary],
  );

  const sessionHeadBootstrapCache = useMemo(() => new SessionHeadBootstrapCache(), []);
  const sessionHeadPrefetchCancelledRef = useRef(false);
  const prefetchSessionIdsRef = useRef<Set<string>>(new Set());
  const activeTaskHeadSessionIds = useMemo(
    () =>
      Array.from(new Set([
        ...sessionIds,
        ...sessions.map((session) => idToString(session.id)).filter(Boolean),
        ...(activeSessionIdFromTab ? [activeSessionIdFromTab] : []),
      ])),
    [activeSessionIdFromTab, sessionIds, sessions],
  );
  const primeAuthoritativeHeadsForSessions = useCallback(
    async (sessionIdsToPrime: readonly string[]) => {
      if (sessionIdsToPrime.length === 0 || sessionHeadPrefetchCancelledRef.current) return;
      await primeAuthoritativeSessionHeads(
        workspaceSnapshotStore.getSnapshot(),
        workspaceSnapshotStore,
        sessionHeadBootstrapCache,
        sessionIdsToPrime,
        {
          onHead: (sessionId, head) => {
            if (sessionHeadPrefetchCancelledRef.current) return;
            supervisor.upsertWorkspaceSessionHead(sessionId, head);
          },
        },
      );
    },
    [sessionHeadBootstrapCache, supervisor, workspaceSnapshotStore],
  );

  useEffect(() => {
    supervisor.setSubscribedSessionIdsSink((sessionIdsForSubscription) => {
      workspaceSnapshotStore.setSubscribedSessions?.(sessionIdsForSubscription);
    });
    const syncWorkspace = () => {
      const snapshot = workspaceSnapshotStore.getSnapshot();
      supervisor.setWorkspaceSessionHeads(
        readWorkspaceSessionHeads(snapshot, workspaceSnapshotStore, sessionHeadBootstrapCache),
      );
      supervisor.setWorkspaceSnapshotState(snapshot);
      lifecycleCoordinator.setWorkspaceSnapshotState(snapshot);
    };
    const handleWorkspaceEvent = (evt: WorkspaceActiveSnapshotEvent) => {
      const didCacheSeed = maybeCacheSessionHeadSeed(sessionHeadBootstrapCache, evt);
      const sessionId =
        evt.type === "session_head_delta"
          ? idToString(evt.delta.session_id)
          : evt.type === "session_head_seed"
            ? idToString(evt.head.session.id)
            : evt.type === "session_summary_delta"
              ? idToString(evt.delta.session_id)
              : evt.type === "session_summary"
                ? idToString(evt.summary.session.id)
            : "";
      if (sessionId) {
        const head = workspaceSnapshotStore.getSessionHeadSnapshot(sessionId);
        if (head) {
          supervisor.upsertWorkspaceSessionHead(sessionId, head);
        } else if (
          (evt.type === "session_summary_delta" || evt.type === "session_summary") &&
          prefetchSessionIdsRef.current.has(sessionId)
        ) {
          void primeAuthoritativeHeadsForSessions([sessionId]);
        }
      } else if (didCacheSeed) {
        const snapshot = workspaceSnapshotStore.getSnapshot();
        supervisor.setWorkspaceSessionHeads(
          readWorkspaceSessionHeads(snapshot, workspaceSnapshotStore, sessionHeadBootstrapCache),
        );
      }
      supervisor.handleWorkspaceEvent(evt);
    };
    syncWorkspace();
    const unsubState = workspaceSnapshotStore.subscribe(syncWorkspace);
    const unsubEvents = workspaceSnapshotStore.subscribeEvents(handleWorkspaceEvent);
    return () => {
      unsubEvents();
      unsubState();
      supervisor.setSubscribedSessionIdsSink(null);
      sessionHeadBootstrapCache.clear();
      supervisor.setWorkspaceSessionHeads({});
      supervisor.setWorkspaceSnapshotState(null);
      lifecycleCoordinator.setWorkspaceSnapshotState(null);
    };
  }, [
    lifecycleCoordinator,
    primeAuthoritativeHeadsForSessions,
    supervisor,
    sessionHeadBootstrapCache,
    workspaceSnapshotStore,
  ]);

  const taskLiveInfo = useMemo(
    () =>
      deriveTaskLiveInfo({
        tasksById,
        optimisticTasks,
        sessions: sessionSnap.sessions,
      }),
    [optimisticTasks, sessionSnap.sessions, tasksById],
  );
  const taskArchived = Boolean(activeTaskSummary?.task.archived_at);

  useEffect(() => {
    if (!activeTaskId) return;
    const snapshotReady = workspaceSnapshot.initialized && workspaceSnapshot.fetchState.active === "idle";
    const activeTab = workbenchStore.getActiveTab();
    const previousSessionId =
      activeTab?.kind === "task" && activeTab.ref.taskId === activeTaskId ? (activeTab.ref.sessionId ?? null) : null;
    const previousSessionEntry = previousSessionId ? sessionSnap.sessions[previousSessionId] ?? null : null;
    const optimisticActiveTask = optimisticTasksById[activeTaskId];
    const optimisticPrimarySessionId = optimisticActiveTask ? (optimisticActiveTask.primarySessionId ?? null) : null;
    if (!activeTaskSummary) {
      if (!snapshotReady) return;
      workbenchStore.setActiveSessionForActiveTask(null, { source: "system" });
      return;
    }
    if (sessions.length === 0 && !primarySessionId) {
      if (!snapshotReady) return;
      if (!taskArchived) {
        if (previousSessionId && optimisticPrimarySessionId && previousSessionId === optimisticPrimarySessionId) {
          return;
        }
        workbenchStore.setActiveSessionForActiveTask(null, { source: "system" });
        return;
      }
      if (previousSessionId && !previousSessionEntry) return;
      if (previousSessionEntry && previousSessionEntry.loadState !== "fatal") return;
      workbenchStore.setActiveSessionForActiveTask(null, { source: "system" });
      return;
    }
    const nextSessionId = resolveWorkbenchActiveSessionId({
      activeSessionIdFromTab: previousSessionId,
      primarySessionId,
      sessions,
    });
    if (!canRenderWorkbenchActiveSession(nextSessionId ? sessionSnap.sessions[nextSessionId] ?? null : null)) {
      return;
    }
    if (activeTab?.kind === "task" && activeTab.ref.taskId === activeTaskId && nextSessionId !== previousSessionId) {
      if (nextSessionId) {
        const snapshot = workspaceSnapshotStore.getSnapshot();
        const nextHead = readWorkspaceSessionHeads(
          snapshot,
          workspaceSnapshotStore,
          sessionHeadBootstrapCache,
          [nextSessionId],
        )[nextSessionId];
        if (nextHead) {
          supervisor.upsertWorkspaceSessionHead(nextSessionId, nextHead);
        }
      }
      if (previousSessionId && sessionSnap.sessions[previousSessionId]) {
        noteSwitchStaleVisible(activeTaskId, previousSessionId, nextSessionId ?? "");
      }
      workbenchStore.setActiveSessionForActiveTask(nextSessionId, { source: "system" });
    }
  }, [
    activeTaskId,
    activeTaskSummary,
    optimisticTasksById,
    primarySessionId,
    sessions,
    sessionSnap.sessions,
    sessionHeadBootstrapCache,
    supervisor,
    taskArchived,
    workbenchStore,
    workspaceSnapshot.fetchState.active,
    workspaceSnapshot.initialized,
    workspaceSnapshotStore,
  ]);

  useEffect(() => {
    const detail: WorkbenchTaskIdleDetail = {
      allTasksIdle: taskLiveInfo.workingByTask.size === 0,
    };
    window.dispatchEvent(
      new CustomEvent<WorkbenchTaskIdleDetail>(WORKBENCH_TASK_IDLE_EVENT, {
        detail,
      }),
    );
  }, [taskLiveInfo.workingByTask.size]);

  const isTaskUnread = useCallback(
    (taskId: string) => isWorkbenchTaskUnread({ taskId, tasksById, taskLiveInfo }),
    [taskLiveInfo, tasksById],
  );

  useEffect(() => {
    if (!activeTaskId) return;
    const task = tasksById[activeTaskId]?.task;
    if (!task) return;
    if (optimisticTasksById[activeTaskId]) return;
    if (taskLiveInfo.workingByTask.has(activeTaskId)) return;
    if (!isTaskUnread(activeTaskId)) return;
    void markTaskRead(activeTaskId);
  }, [activeTaskId, isTaskUnread, markTaskRead, optimisticTasksById, taskLiveInfo.workingByTask, tasksById]);

  const activeSessionId = useMemo(
    () =>
      resolveWorkbenchActiveSessionId({
        activeSessionIdFromTab,
        primarySessionId,
        sessions,
      }),
    [activeSessionIdFromTab, primarySessionId, sessions],
  );

  useEffect(() => {
    if (!activeTaskId || !activeSessionId) return;
    const activeEntry = sessionSnap.sessions[activeSessionId];
    if (!activeEntry) return;
    const navWorking = taskLiveInfo.workingByTask.has(activeTaskId);
    const latestTurnStatus = activeEntry.turns.at(-1)?.status ?? null;
    const threadWorking = hasSessionActiveTurn(activeEntry.activity, latestTurnStatus);
    if (navWorking === threadWorking) return;
    noteNavThreadActivityMismatch(activeTaskId, activeSessionId, navWorking, threadWorking);
  }, [activeSessionId, activeTaskId, sessionSnap.sessions, taskLiveInfo.workingByTask]);

  const providerIdsByTaskFromSessions = useMemo(
    () => deriveProviderIdsByTask(sessionSnap.sessions),
    [sessionSnap.sessions],
  );

  const foregroundSessionIds = useMemo(
    () => (activeSessionId ? [activeSessionId] : primarySessionId ? [primarySessionId] : []),
    [activeSessionId, primarySessionId],
  );

  const warmSessionIds = useMemo(
    () =>
      deriveWarmSessionIds({
        activeTaskSessionIds: foregroundSessionIds,
        tasksById,
        activeIds: workspaceSnapshot.activeIds,
      }),
    [foregroundSessionIds, tasksById, workspaceSnapshot.activeIds],
  );
  const prefetchSessionIds = useMemo(
    () => Array.from(new Set([...activeTaskHeadSessionIds, ...warmSessionIds])),
    [activeTaskHeadSessionIds, warmSessionIds],
  );

  useEffect(() => {
    prefetchSessionIdsRef.current = new Set(prefetchSessionIds);
  }, [prefetchSessionIds]);

  useEffect(() => {
    sessionHeadPrefetchCancelledRef.current = false;
    const prefetchHeads = async () => {
      if (!workspaceSnapshot.initialized) return;
      const snapshot = workspaceSnapshotStore.getSnapshot();
      const persistedChanged = await primePersistedSessionHeads(
        snapshot,
        workspaceSnapshotStore,
        sessionHeadBootstrapCache,
        prefetchSessionIds,
      );
      if (persistedChanged && !sessionHeadPrefetchCancelledRef.current) {
        const nextSnapshot = workspaceSnapshotStore.getSnapshot();
        supervisor.setWorkspaceSessionHeads(
          readWorkspaceSessionHeads(nextSnapshot, workspaceSnapshotStore, sessionHeadBootstrapCache),
        );
      }
      await primeAuthoritativeHeadsForSessions(prefetchSessionIds);
    };
    void prefetchHeads();
    return () => {
      sessionHeadPrefetchCancelledRef.current = true;
    };
  }, [
    prefetchSessionIds,
    primeAuthoritativeHeadsForSessions,
    sessionHeadBootstrapCache,
    supervisor,
    workspaceSnapshot.activeIds,
    workspaceSnapshot.initialized,
    workspaceSnapshot.tasksById,
    workspaceSnapshotStore,
  ]);

  useLayoutEffect(() => {
    if (prefetchSessionIds.length > 0) {
      const snapshot = workspaceSnapshotStore.getSnapshot();
      supervisor.setWorkspaceSessionHeads(
        readWorkspaceSessionHeads(
          snapshot,
          workspaceSnapshotStore,
          sessionHeadBootstrapCache,
          prefetchSessionIds,
        ),
      );
    }
    supervisor.setActiveTaskSessionIds(foregroundSessionIds);
  }, [
    foregroundSessionIds,
    prefetchSessionIds,
    sessionHeadBootstrapCache,
    supervisor,
    workspaceSnapshotStore,
  ]);

  useEffect(() => {
    supervisor.setWarmSessionIds(warmSessionIds);
  }, [supervisor, warmSessionIds]);

  useEffect(() => {
    workspaceSnapshotStore.setForegroundSessionId?.(activeSessionId ?? primarySessionId ?? null);
  }, [activeSessionId, primarySessionId, workspaceSnapshotStore]);

  return {
    sessionSummaries,
    sessions,
    sessionIds,
    activeSessionId,
    primarySessionId,
    activeTaskSessionIds: foregroundSessionIds,
    taskLiveInfo,
    providerIdsByTaskFromSessions,
    isTaskUnread,
  };
}
