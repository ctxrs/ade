import { useEffect } from "react";

type WorkbenchE2EWindow = Window & {
  __ctxE2E?: {
    focusTask?: (taskId: string, sessionId?: string | null) => boolean;
    toggleDiffPane?: () => boolean;
  };
};

type WorkbenchE2EBridgeOptions = {
  focusTask: (taskId: string, sessionId?: string | null) => boolean;
  toggleDiffPane: () => void;
};

export function useWorkbenchE2EBridge({ focusTask, toggleDiffPane }: WorkbenchE2EBridgeOptions) {
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const enabled = window.sessionStorage.getItem("ctxE2E") === "1" || params.get("ctxE2E") === "1";
    if (!enabled) return;

    const win = window as WorkbenchE2EWindow;
    win.__ctxE2E ??= {};
    win.__ctxE2E.focusTask = (taskId: string, sessionId?: string | null) => focusTask(taskId, sessionId);
    win.__ctxE2E.toggleDiffPane = () => {
      toggleDiffPane();
      return true;
    };

    return () => {
      if (!win.__ctxE2E) return;
      delete win.__ctxE2E.focusTask;
      delete win.__ctxE2E.toggleDiffPane;
    };
  }, [focusTask, toggleDiffPane]);
}
