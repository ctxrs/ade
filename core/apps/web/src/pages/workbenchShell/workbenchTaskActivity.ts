import { idToString } from "../../api/client";
import type { SessionCacheEntry, SessionSupervisorSnapshot } from "../../state/sessionSupervisor";
import type { WorkspaceActiveSnapshotItem } from "../../state/workspaceActiveSnapshotStore";
import { pickPreferredSessionId } from "../../utils/workbenchSelection";
import { lastAssistantMessageMs, parseMs } from "../WorkbenchPage.utils";
import type { OptimisticTaskSummary } from "../WorkbenchPage.types";

export type WorkbenchTaskLiveInfo = {
  workingByTask: Set<string>;
  errorByTask: Set<string>;
  lastAssistantMsByTask: Record<string, number>;
};

export type WorkbenchTaskStatusKind = "error" | "working" | "unread" | "idle";

type SessionTaskProviderSample = { providerId: string; updatedAt: number };

const hasRunningTurn = (
  turns: SessionCacheEntry["turns"] | NonNullable<WorkspaceActiveSnapshotItem["primarySessionHead"]>["turns"] | null | undefined,
) => Array.isArray(turns) && turns.some((turn) => turn.status === "running");

const hasRunningActivity = (
  activity:
    | WorkspaceActiveSnapshotItem["sessions"][number]["activity"]
    | NonNullable<WorkspaceActiveSnapshotItem["primarySessionHead"]>["activity"]
    | null
    | undefined,
) => activity?.is_working === true && activity.last_turn_status === "running";

export const isPrimarySessionRunning = ({
  primarySessionSummary,
  primarySessionHead,
  primaryEntry,
}: {
  primarySessionSummary?: WorkspaceActiveSnapshotItem["sessions"][number];
  primarySessionHead?: WorkspaceActiveSnapshotItem["primarySessionHead"] | null;
  primaryEntry?: SessionCacheEntry;
}): boolean => {
  if (hasRunningTurn(primaryEntry?.turns)) return true;
  if (hasRunningTurn(primarySessionHead?.turns)) return true;
  if (hasRunningActivity(primarySessionHead?.activity)) return true;
  return hasRunningActivity(primarySessionSummary?.activity);
};

export const deriveWorkbenchTaskStatusKind = ({
  hasError,
  working,
  unread,
  localStatus,
}: {
  hasError: boolean;
  working: boolean;
  unread: boolean;
  localStatus: "starting" | "synced" | "failed" | null;
}): WorkbenchTaskStatusKind => {
  if (localStatus === "failed" || hasError) return "error";
  if (working) return "working";
  if (unread) return "unread";
  return "idle";
};

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
    const primarySessionHead =
      primarySessionId && idToString(summary.primarySessionHead?.session?.id) === primarySessionId
        ? summary.primarySessionHead
        : null;

    if (primarySessionSummary || primaryEntry?.session || primarySessionHead?.session) {
      if (isPrimarySessionRunning({ primarySessionSummary, primarySessionHead, primaryEntry })) {
        workingByTask.add(taskId);
      }

      const status =
        primaryEntry?.session?.status ?? primarySessionSummary?.session.status ?? primarySessionHead?.session.status;
      if (status === "failed" || status === "cancelled") {
        errorByTask.add(taskId);
      }

      const liveMs = primaryEntry ? lastAssistantMessageMs(primaryEntry.messages) : null;
      const headMs = primarySessionHead ? lastAssistantMessageMs(primarySessionHead.messages) : null;
      const summaryMs = parseMs(primarySessionSummary?.last_message_at ?? null);
      const lastAssistantMs =
        liveMs !== null
          ? Math.max(liveMs, headMs ?? 0, summaryMs ?? 0)
          : headMs !== null && summaryMs !== null
            ? Math.max(headMs, summaryMs)
            : headMs ?? summaryMs;
      if (lastAssistantMs !== null) {
        lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, lastAssistantMs);
      }
      continue;
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
  if (sessions.length === 0) return activeSessionIdFromTab ?? null;
  return pickPreferredSessionId(sessions, activeSessionIdFromTab);
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
