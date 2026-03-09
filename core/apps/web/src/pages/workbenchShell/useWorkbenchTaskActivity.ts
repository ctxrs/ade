import { useCallback, useEffect, useMemo } from "react";
import { idToString } from "../../api/client";
import type { SessionCacheEntry, SessionSupervisor, SessionSupervisorSnapshot } from "../../state/sessionSupervisor";
import type { WorkspaceActiveSnapshotEventSource, WorkspaceActiveSnapshotItem, WorkspaceActiveSnapshotState } from "../../state/workspaceActiveSnapshotStore";
import { WORKBENCH_TASK_IDLE_EVENT, type WorkbenchTaskIdleDetail } from "../../utils/updaterEvents";
import { pickPreferredSessionId } from "../../utils/workbenchSelection";
import type { WorkbenchStore } from "../../workbench/store";
import { lastAssistantMessageMs, parseMs } from "../WorkbenchPage.utils";
import type { OptimisticTaskSummary } from "../WorkbenchPage.types";

export type WorkbenchTaskLiveInfo = {
  workingByTask: Set<string>;
  errorByTask: Set<string>;
  lastAssistantMsByTask: Record<string, number>;
};

type TaskActivityArgs = {
  activeTaskId: string | null;
  activeSessionIdFromTab: string | null;
  activeTaskSummary: WorkspaceActiveSnapshotItem | OptimisticTaskSummary | null;
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
  workspaceSnapshot: WorkspaceActiveSnapshotState;
  sessionSnap: SessionSupervisorSnapshot;
  optimisticTasks: OptimisticTaskSummary[];
  optimisticTasksById: Record<string, OptimisticTaskSummary>;
  supervisor: Pick<SessionSupervisor, "setActiveTaskSessionIds" | "setWarmSessionIds">;
  workbenchStore: Pick<WorkbenchStore, "getActiveTab" | "setActiveSessionForActiveTask">;
  workspaceSnapshotStore: Pick<WorkspaceActiveSnapshotEventSource, "setForegroundTaskId">;
  markTaskRead: (taskId: string) => Promise<void>;
};

type SessionTaskProviderSample = { providerId: string; updatedAt: number };

export const deriveActiveTaskSessionIds = (
  activeTaskSummary: WorkspaceActiveSnapshotItem | OptimisticTaskSummary | null,
): {
  sessionSummaries: WorkspaceActiveSnapshotItem["sessions"];
  sessions: WorkspaceActiveSnapshotItem["sessions"][number]["session"][];
  sessionIds: string[];
  primarySessionId: string;
  activeTaskSessionIds: string[];
} => {
  const sessionSummaries = activeTaskSummary?.sessions ?? [];
  const sessions = sessionSummaries.map((summary) => summary.session);
  const sessionIds = sessionSummaries.map((summary) => idToString(summary.session.id)).filter(Boolean);
  const primarySessionId = idToString(activeTaskSummary?.task.primary_session_id ?? "");
  const activeTaskSessionIds = primarySessionId ? [primarySessionId] : sessionIds;
  return {
    sessionSummaries,
    sessions,
    sessionIds,
    primarySessionId,
    activeTaskSessionIds,
  };
};

export const deriveWarmSessionIds = ({
  activeTaskSessionIds,
  tasksById,
  activeIds,
}: {
  activeTaskSessionIds: string[];
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
  activeIds: string[];
}): string[] => {
  const activeSet = new Set(activeTaskSessionIds);
  const candidates: { id: string; updatedAt: number; running: boolean }[] = [];
  for (const taskId of activeIds) {
    const task = tasksById[taskId];
    if (!task) continue;
    for (const sessionSummary of task.sessions) {
      const sessionId = idToString(sessionSummary.session.id);
      if (!sessionId || activeSet.has(sessionId)) continue;
      const updatedAt = parseMs(sessionSummary.last_message_at) ?? parseMs(sessionSummary.session.updated_at) ?? 0;
      const running = sessionSummary.session.status === "active" || sessionSummary.session.status === "running";
      candidates.push({ id: sessionId, updatedAt, running });
    }
  }
  candidates.sort((left, right) => {
    if (left.running !== right.running) return left.running ? -1 : 1;
    return right.updatedAt - left.updatedAt;
  });
  return candidates.map((candidate) => candidate.id).slice(0, 20);
};

const buildTasksForLiveInfo = (
  tasksById: Record<string, WorkspaceActiveSnapshotItem>,
  optimisticTasks: OptimisticTaskSummary[],
): Record<string, WorkspaceActiveSnapshotItem | OptimisticTaskSummary> => {
  const merged: Record<string, WorkspaceActiveSnapshotItem | OptimisticTaskSummary> = { ...tasksById };
  for (const item of optimisticTasks) {
    if (item.localStatus === "failed" || !merged[item.id]) {
      merged[item.id] = item;
    }
  }
  return merged;
};

const deriveTaskLiveInfoFromSources = (
  tasksForLiveInfo: Record<string, WorkspaceActiveSnapshotItem | OptimisticTaskSummary>,
  sessions: SessionSupervisorSnapshot["sessions"],
): WorkbenchTaskLiveInfo => {
  const workingByTask = new Set<string>();
  const errorByTask = new Set<string>();
  const lastAssistantMsByTask: Record<string, number> = {};
  const entryBySessionId = new Map<string, SessionCacheEntry>();

  for (const entry of Object.values(sessions)) {
    const sessionId = entry.session ? idToString(entry.session.id) : "";
    if (sessionId) entryBySessionId.set(sessionId, entry);
  }

  for (const summary of Object.values(tasksForLiveInfo)) {
    const taskId = summary.id;
    const primarySessionId = summary.task.primary_session_id
      ? idToString(summary.task.primary_session_id)
      : "";
    const primarySessionSummary = primarySessionId
      ? summary.sessions.find((sessionSummary) => idToString(sessionSummary.session.id) === primarySessionId)
      : undefined;
    const primaryEntry = primarySessionId ? entryBySessionId.get(primarySessionId) : undefined;

    if (primarySessionSummary) {
      if (primarySessionSummary.activity?.is_working === true) {
        workingByTask.add(taskId);
      }

      const status = primaryEntry?.session?.status ?? primarySessionSummary.session.status;
      if (status === "failed" || status === "cancelled") {
        errorByTask.add(taskId);
      }

      const liveMs = primaryEntry ? lastAssistantMessageMs(primaryEntry.messages) : null;
      const summaryMs = parseMs(primarySessionSummary.last_message_at ?? null);
      const lastAssistantMs =
        liveMs !== null && summaryMs !== null ? Math.max(liveMs, summaryMs) : liveMs ?? summaryMs;
      if (lastAssistantMs !== null) {
        lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, lastAssistantMs);
      }
      continue;
    }

    if (primaryEntry?.session) {
      const status = primaryEntry.session.status;
      if (status === "failed" || status === "cancelled") {
        errorByTask.add(taskId);
      }
      const lastAssistantMs = lastAssistantMessageMs(primaryEntry.messages);
      if (lastAssistantMs !== null) {
        lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, lastAssistantMs);
      }
    }
  }

  for (const entry of Object.values(sessions)) {
    const session = entry.session;
    const taskId = session ? idToString(session.task_id) : "";
    if (!taskId || tasksForLiveInfo[taskId]) continue;
    if (session?.parent_session_id || session?.relationship === "sub_agent") continue;
    const status = session?.status;
    if (status === "failed" || status === "cancelled") {
      errorByTask.add(taskId);
    }
    const lastAssistantMs = lastAssistantMessageMs(entry.messages);
    if (lastAssistantMs !== null) {
      lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, lastAssistantMs);
    }
  }

  return { workingByTask, errorByTask, lastAssistantMsByTask };
};

export const deriveTaskLiveInfo = ({
  tasksById,
  optimisticTasks,
  sessions,
}: {
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
  optimisticTasks: OptimisticTaskSummary[];
  sessions: SessionSupervisorSnapshot["sessions"];
}): WorkbenchTaskLiveInfo =>
  deriveTaskLiveInfoFromSources(buildTasksForLiveInfo(tasksById, optimisticTasks), sessions);

export const deriveProviderIdsByTask = (
  sessions: SessionSupervisorSnapshot["sessions"],
): Record<string, string[]> => {
  const providerSamplesByTask: Record<string, SessionTaskProviderSample[]> = {};
  for (const entry of Object.values(sessions)) {
    const session = entry.session;
    const taskId = session ? idToString(session.task_id) : "";
    const providerId = String(session?.provider_id ?? "").trim();
    if (!taskId || !providerId) continue;
    (providerSamplesByTask[taskId] ??= []).push({ providerId, updatedAt: entry.updatedAtMs ?? 0 });
  }

  const byTask: Record<string, string[]> = {};
  for (const [taskId, samples] of Object.entries(providerSamplesByTask)) {
    const seen = new Set<string>();
    byTask[taskId] = samples
      .slice()
      .sort((left, right) => (right.updatedAt ?? 0) - (left.updatedAt ?? 0))
      .map((sample) => sample.providerId)
      .filter((providerId) => {
        if (seen.has(providerId)) return false;
        seen.add(providerId);
        return true;
      });
  }
  return byTask;
};

export const deriveProviderIdsByTaskFromSessions = deriveProviderIdsByTask;

export const resolveWorkbenchActiveSessionId = ({
  activeSessionIdFromTab,
  primarySessionId,
  sessions,
}: {
  activeSessionIdFromTab: string | null;
  primarySessionId: string;
  sessions: WorkspaceActiveSnapshotItem["sessions"][number]["session"][];
}): string | null => {
  if (primarySessionId) return primarySessionId;
  if (activeSessionIdFromTab) return activeSessionIdFromTab;
  return pickPreferredSessionId(sessions, null);
};

export const isWorkbenchTaskUnread = ({
  taskId,
  tasksById,
  taskLiveInfo,
}: {
  taskId: string;
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
  taskLiveInfo: WorkbenchTaskLiveInfo;
}): boolean => {
  const summary = tasksById[taskId];
  const task = summary?.task;
  if (!task) return false;
  const serverLastAssistantMs = parseMs(task.last_assistant_message_at ?? null);
  const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[taskId] ?? null;
  const lastAssistantMs =
    liveLastAssistantMs !== null && serverLastAssistantMs !== null
      ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
      : liveLastAssistantMs ?? serverLastAssistantMs;
  if (lastAssistantMs === null) return false;
  const seenMs = parseMs(task.assistant_seen_at ?? null);
  return seenMs === null || lastAssistantMs > seenMs;
};

export function useWorkbenchTaskActivity({
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
}: TaskActivityArgs) {
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
    workspaceSnapshotStore.setForegroundTaskId?.(activeTaskId ?? null);
  }, [activeTaskId, workspaceSnapshotStore]);

  useEffect(() => {
    supervisor.setWarmSessionIds(warmSessionIds);
  }, [supervisor, warmSessionIds]);

  const taskLiveInfo = useMemo(
    () =>
      deriveTaskLiveInfo({
        tasksById,
        optimisticTasks,
        sessions: sessionSnap.sessions,
      }),
    [optimisticTasks, sessionSnap.sessions, tasksById],
  );

  useEffect(() => {
    if (!activeTaskId) return;
    const snapshotReady = workspaceSnapshot.initialized && workspaceSnapshot.fetchState.active === "idle";
    if (!activeTaskSummary) {
      if (!snapshotReady) return;
      workbenchStore.setActiveSessionForActiveTask(null, { source: "system" });
      return;
    }
    if (sessions.length === 0 && !primarySessionId) {
      if (!snapshotReady) return;
      workbenchStore.setActiveSessionForActiveTask(null, { source: "system" });
      return;
    }
    const activeTab = workbenchStore.getActiveTab();
    const previousSessionId =
      activeTab?.kind === "task" && activeTab.ref.taskId === activeTaskId ? (activeTab.ref.sessionId ?? null) : null;
    const nextSessionId = primarySessionId || pickPreferredSessionId(sessions, previousSessionId ?? null);
    if (activeTab?.kind === "task" && activeTab.ref.taskId === activeTaskId && nextSessionId !== previousSessionId) {
      workbenchStore.setActiveSessionForActiveTask(nextSessionId, { source: "system" });
    }
  }, [
    activeTaskId,
    activeTaskSummary,
    primarySessionId,
    sessions,
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
