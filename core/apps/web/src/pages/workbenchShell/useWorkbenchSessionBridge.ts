import { useCallback, useEffect, useMemo } from "react";
import { idToString, type SessionHeadSnapshot } from "../../api/client";
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
import type { WorkbenchStore } from "../../workbench/store";
import type { OptimisticTaskSummary } from "../WorkbenchPage.types";
import {
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
    | "handleWorkspaceEvent"
  >;
  workbenchStore: Pick<WorkbenchStore, "getActiveTab" | "setActiveSessionForActiveTask">;
  workspaceSnapshotStore: Pick<
    WorkspaceActiveSnapshotEventSource,
    | "subscribe"
    | "subscribeEvents"
    | "getSnapshot"
    | "getSessionHeadSnapshot"
    | "setForegroundTaskId"
    | "setSubscribedSessionIds"
  > & { getSessionHeadsSnapshot?: () => Record<string, SessionHeadSnapshot> };
  markTaskRead: (taskId: string) => Promise<void>;
};

const collectWorkspaceSessionHeadIds = (snapshot: WorkspaceActiveSnapshotState): string[] => {
  const ids = new Set<string>();
  for (const taskId of snapshot.activeIds) {
    const item = snapshot.tasksById[taskId];
    if (!item) continue;
    const primaryId =
      item.primarySessionId ||
      idToString(item.task.primary_session_id ?? "") ||
      idToString(item.primarySessionHead?.session?.id ?? "");
    if (primaryId) ids.add(primaryId);
    for (const summary of item.sessions ?? []) {
      const sessionId = idToString(summary.session.id);
      if (sessionId) ids.add(sessionId);
    }
  }
  return Array.from(ids);
};

const readWorkspaceSessionHeads = (
  snapshot: WorkspaceActiveSnapshotState,
  store: TaskBridgeArgs["workspaceSnapshotStore"],
): Record<string, SessionHeadSnapshot> => {
  const batchHeads = store.getSessionHeadsSnapshot?.();
  if (batchHeads) return batchHeads;
  const heads: Record<string, SessionHeadSnapshot> = {};
  for (const sessionId of collectWorkspaceSessionHeadIds(snapshot)) {
    const head = store.getSessionHeadSnapshot(sessionId);
    if (head) {
      heads[sessionId] = head;
    }
  }
  return heads;
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
  const { sessionSummaries, sessions, sessionIds, primarySessionId, activeTaskSessionIds } = useMemo(
    () => deriveActiveTaskSessionIds(activeTaskSummary),
    [activeTaskSummary],
  );

  const warmSessionIds = useMemo(
    () =>
      deriveWarmSessionIds({
        activeTaskSessionIds,
        tasksById,
        activeIds: workspaceSnapshot.activeIds,
      }),
    [activeTaskSessionIds, tasksById, workspaceSnapshot.activeIds],
  );

  useEffect(() => {
    supervisor.setActiveTaskSessionIds(activeTaskSessionIds);
  }, [activeTaskSessionIds, supervisor]);

  useEffect(() => {
    supervisor.setWarmSessionIds(warmSessionIds);
  }, [supervisor, warmSessionIds]);

  useEffect(() => {
    workspaceSnapshotStore.setForegroundTaskId?.(activeTaskId ?? null);
  }, [activeTaskId, workspaceSnapshotStore]);

  useEffect(() => {
    supervisor.setSubscribedSessionIdsSink((sessionIdsForSubscription) => {
      workspaceSnapshotStore.setSubscribedSessionIds?.(sessionIdsForSubscription);
    });
    const syncWorkspace = () => {
      const snapshot = workspaceSnapshotStore.getSnapshot();
      supervisor.setWorkspaceSessionHeads(readWorkspaceSessionHeads(snapshot, workspaceSnapshotStore));
      supervisor.setWorkspaceSnapshotState(snapshot);
      lifecycleCoordinator.setWorkspaceSnapshotState(snapshot);
    };
    syncWorkspace();
    const unsubState = workspaceSnapshotStore.subscribe(syncWorkspace);
    const unsubEvents = workspaceSnapshotStore.subscribeEvents((evt) => supervisor.handleWorkspaceEvent(evt));
    return () => {
      unsubEvents();
      unsubState();
      supervisor.setSubscribedSessionIdsSink(null);
      supervisor.setWorkspaceSessionHeads({});
      supervisor.setWorkspaceSnapshotState(null);
      lifecycleCoordinator.setWorkspaceSnapshotState(null);
    };
  }, [lifecycleCoordinator, supervisor, workspaceSnapshotStore]);

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
    const nextSessionId = primarySessionId || resolveWorkbenchActiveSessionId({
      activeSessionIdFromTab: previousSessionId,
      primarySessionId,
      sessions,
    });
    if (activeTab?.kind === "task" && activeTab.ref.taskId === activeTaskId && nextSessionId !== previousSessionId) {
      workbenchStore.setActiveSessionForActiveTask(nextSessionId, { source: "system" });
    }
  }, [
    activeTaskId,
    activeTaskSummary,
    optimisticTasksById,
    primarySessionId,
    sessions,
    sessionSnap.sessions,
    taskArchived,
    workbenchStore,
    workspaceSnapshot.fetchState.active,
    workspaceSnapshot.initialized,
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

  const providerIdsByTaskFromSessions = useMemo(
    () => deriveProviderIdsByTask(sessionSnap.sessions),
    [sessionSnap.sessions],
  );

  const activeSessionId = useMemo(
    () =>
      resolveWorkbenchActiveSessionId({
        activeSessionIdFromTab,
        primarySessionId,
        sessions,
      }),
    [activeSessionIdFromTab, primarySessionId, sessions],
  );

  return {
    sessionSummaries,
    sessions,
    sessionIds,
    activeSessionId,
    primarySessionId,
    activeTaskSessionIds,
    taskLiveInfo,
    providerIdsByTaskFromSessions,
    isTaskUnread,
  };
}
