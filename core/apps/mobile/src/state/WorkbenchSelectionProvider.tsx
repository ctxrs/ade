import AsyncStorage from "@react-native-async-storage/async-storage";
import React, { createContext, useContext, useEffect, useMemo, useState } from "react";

import { useWorkspaceSelection } from "./WorkspaceSelectionProvider";

type WorkbenchSelectionState = {
  taskId: string | null;
  sessionId: string | null;
  setSelection: (next: { taskId?: string | null; sessionId?: string | null }) => void;
  clearSelection: () => void;
};

const WorkbenchSelectionContext = createContext<WorkbenchSelectionState | undefined>(undefined);

const storageKey = (workspaceId: string) => `contextMobileWorkbenchSelection.v1.${workspaceId}`;

export const WorkbenchSelectionProvider: React.FC<React.PropsWithChildren> = ({ children }) => {
  const { workspaceId } = useWorkspaceSelection();
  const [taskId, setTaskId] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);

  useEffect(() => {
    setTaskId(null);
    setSessionId(null);
    if (!workspaceId) return;
    let cancelled = false;
    AsyncStorage.getItem(storageKey(workspaceId))
      .then((raw) => {
        if (cancelled || !raw) return;
        const parsed = JSON.parse(raw) as { taskId?: string | null; sessionId?: string | null };
        if (parsed?.taskId) setTaskId(String(parsed.taskId));
        if (parsed?.sessionId) setSessionId(String(parsed.sessionId));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  const value = useMemo<WorkbenchSelectionState>(() => {
    const persist = (next: { taskId: string | null; sessionId: string | null }) => {
      if (!workspaceId) return;
      AsyncStorage.setItem(storageKey(workspaceId), JSON.stringify(next)).catch(() => {});
    };

    return {
      taskId,
      sessionId,
      setSelection: (next) => {
        const nextTaskId = next.taskId !== undefined ? next.taskId : taskId;
        const taskChanged = next.taskId !== undefined && next.taskId !== taskId;
        const nextSessionId = taskChanged ? next.sessionId ?? null : next.sessionId !== undefined ? next.sessionId : sessionId;
        setTaskId(nextTaskId ?? null);
        setSessionId(nextSessionId ?? null);
        persist({ taskId: nextTaskId ?? null, sessionId: nextSessionId ?? null });
      },
      clearSelection: () => {
        setTaskId(null);
        setSessionId(null);
        if (!workspaceId) return;
        AsyncStorage.removeItem(storageKey(workspaceId)).catch(() => {});
      },
    };
  }, [taskId, sessionId, workspaceId]);

  return <WorkbenchSelectionContext.Provider value={value}>{children}</WorkbenchSelectionContext.Provider>;
};

export function useWorkbenchSelection(): WorkbenchSelectionState {
  const ctx = useContext(WorkbenchSelectionContext);
  if (!ctx) throw new Error("useWorkbenchSelection must be used within WorkbenchSelectionProvider");
  return ctx;
}
