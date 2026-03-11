import { useEffect, useMemo, useRef, useState } from "react";
import type { WorkspaceActiveSnapshotItem } from "../../state/workspaceActiveSnapshotStore";
import type { OptimisticTaskSummary } from "../WorkbenchPage.types";

type UseWorkbenchOptimisticTasksArgs = {
  activeTaskId: string | null;
  activeTaskIdFromTab: string | null;
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
};

export function useWorkbenchOptimisticTasks({
  activeTaskId,
  activeTaskIdFromTab,
  tasksById,
}: UseWorkbenchOptimisticTasksArgs) {
  const [optimisticTasks, setOptimisticTasks] = useState<OptimisticTaskSummary[]>([]);
  // Keep a synchronous fallback for first paint when an optimistic task tab races state commit.
  const optimisticStartingTaskRef = useRef<OptimisticTaskSummary | null>(null);

  const optimisticTasksById = useMemo(
    () => Object.fromEntries(optimisticTasks.map((item) => [item.id, item])),
    [optimisticTasks],
  );

  const optimisticSessionIdSet = useMemo(() => {
    const ids = new Set<string>();
    for (const item of optimisticTasks) {
      if (item.localStatus === "failed") continue;
      const server = tasksById[item.id] ?? null;
      const serverHasSession =
        Boolean(server?.task.primary_session_id) || (server?.sessions?.length ?? 0) > 0;
      if (item.localStatus === "synced" && serverHasSession) continue;
      const sessionId = String(item.primarySessionId ?? "");
      if (sessionId) ids.add(sessionId);
    }
    return ids;
  }, [optimisticTasks, tasksById]);

  const optimisticFailureBySessionId = useMemo(() => {
    const out: Record<string, { prompt: string; error: string | null }> = {};
    for (const item of optimisticTasks) {
      if (item.localStatus !== "failed") continue;
      const sessionId = String(item.primarySessionId ?? "");
      if (!sessionId) continue;
      out[sessionId] = { prompt: item.localPrompt, error: item.localError ?? null };
    }
    return out;
  }, [optimisticTasks]);

  const activeTaskSummary = useMemo(() => {
    if (!activeTaskId) return null;
    const optimistic = optimisticTasksById[activeTaskId];
    const server = tasksById[activeTaskId] ?? null;
    const serverHasSession =
      Boolean(server?.task.primary_session_id) || (server?.sessions?.length ?? 0) > 0;
    if (optimistic) {
      if (optimistic.localStatus !== "synced") return optimistic;
      if (!serverHasSession) return optimistic;
    }
    if (server) return server;
    const fallback = optimisticStartingTaskRef.current;
    if (fallback && fallback.id === activeTaskId && fallback.localStatus !== "synced") {
      return fallback;
    }
    return optimistic ?? null;
  }, [activeTaskId, optimisticTasksById, tasksById]);

  useEffect(() => {
    const current = optimisticStartingTaskRef.current;
    if (!current) return;
    if (optimisticTasksById[current.id]) {
      optimisticStartingTaskRef.current = null;
      return;
    }
    if (activeTaskId && activeTaskId !== current.id && activeTaskIdFromTab) {
      optimisticStartingTaskRef.current = null;
    }
  }, [activeTaskId, activeTaskIdFromTab, optimisticTasksById]);

  return {
    optimisticTasks,
    setOptimisticTasks,
    optimisticStartingTaskRef,
    optimisticTasksById,
    optimisticSessionIdSet,
    optimisticFailureBySessionId,
    activeTaskSummary,
  };
}
