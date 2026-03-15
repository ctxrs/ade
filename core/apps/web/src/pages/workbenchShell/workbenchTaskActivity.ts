import { idToString } from "../../api/client";
import type { SessionSupervisorSnapshot } from "../../state/sessionSupervisor";
import type { WorkspaceActiveSnapshotItem } from "../../state/workspaceActiveSnapshotStore";
import { hasSessionActiveTurn, isSessionWorkingActivity } from "../../utils/sessionActivity";
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

const readPrimarySessionFallbackId = (
  summary: WorkspaceActiveSnapshotItem | OptimisticTaskSummary | null | undefined,
): string => {
  if (!summary || typeof summary !== "object") return "";
  const record = summary as Record<string, unknown>;
  const primarySession = record.primary_session;
  if (!primarySession || typeof primarySession !== "object") return "";
  const primaryRecord = primarySession as Record<string, unknown>;
  if (primaryRecord.session && typeof primaryRecord.session === "object") {
    const sessionRecord = primaryRecord.session as Record<string, unknown>;
    return typeof sessionRecord.id === "string" ? idToString(sessionRecord.id) : "";
  }
  return typeof primaryRecord.id === "string" ? idToString(primaryRecord.id) : "";
};

const resolvePrimarySessionId = (
  summary: WorkspaceActiveSnapshotItem | OptimisticTaskSummary | null | undefined,
): string =>
  idToString(summary?.task.primary_session_id ?? "") ||
  readPrimarySessionFallbackId(summary) ||
  idToString(summary?.primarySessionId ?? "") ||
  idToString(summary?.primarySessionHead?.session?.id ?? "");

export const isPrimarySessionRunning = ({
  primarySessionSummary,
}: {
  primarySessionSummary?: WorkspaceActiveSnapshotItem["sessions"][number];
}): boolean => {
  return isSessionWorkingActivity(primarySessionSummary?.activity);
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
  const primarySessionId = resolvePrimarySessionId(activeTaskSummary);
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
      const running = hasSessionActiveTurn(sessionSummary.activity);
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
  const entryBySessionId = new Map<string, SessionSupervisorSnapshot["sessions"][string]>();

  for (const entry of Object.values(sessions)) {
    const sessionId = entry.session ? idToString(entry.session.id) : "";
    if (sessionId) entryBySessionId.set(sessionId, entry);
  }

  for (const summary of Object.values(tasksForLiveInfo)) {
    const taskId = summary.id;
    const primarySessionId = resolvePrimarySessionId(summary);
    const primarySessionSummary = primarySessionId
      ? summary.sessions.find((sessionSummary) => idToString(sessionSummary.session.id) === primarySessionId)
      : undefined;
    const primaryEntry = primarySessionId ? entryBySessionId.get(primarySessionId) : undefined;
    if (!primarySessionSummary) continue;

    if (isPrimarySessionRunning({ primarySessionSummary })) {
      workingByTask.add(taskId);
    }

    const status = primaryEntry?.session?.status ?? primarySessionSummary.session.status;
    if (status === "failed" || status === "cancelled") {
      errorByTask.add(taskId);
    }

    const liveMs = primaryEntry ? lastAssistantMessageMs(primaryEntry.messages) : null;
    const summaryMs = parseMs(primarySessionSummary.last_message_at);
    const assistantMs = liveMs ?? summaryMs;
    if (assistantMs !== null) {
      lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, assistantMs);
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
