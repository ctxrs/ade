import { useEffect } from "react";

type WorkbenchE2EWindow = Window & {
  __ctxE2E?: {
    focusNewTask?: () => boolean;
    clearDraftHarness?: () => boolean;
    focusTask?: (taskId: string, sessionId?: string | null) => boolean;
    toggleDiffPane?: () => boolean;
    toggleArtifactsPane?: () => boolean;
  };
};

type WorkbenchE2EBridgeOptions = {
  focusNewTask: () => void;
  clearDraftHarness: () => void;
  focusTask: (taskId: string, sessionId?: string | null) => boolean;
  toggleDiffPane: () => void;
  toggleArtifactsPane: () => void;
};

export function useWorkbenchE2EBridge({
  focusNewTask,
  clearDraftHarness,
  focusTask,
  toggleDiffPane,
  toggleArtifactsPane,
}: WorkbenchE2EBridgeOptions) {
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const enabled = window.sessionStorage.getItem("ctxE2E") === "1" || params.get("ctxE2E") === "1";
    if (!enabled) return;

    const win = window as WorkbenchE2EWindow;
    win.__ctxE2E ??= {};
    win.__ctxE2E.focusNewTask = () => {
      focusNewTask();
      return true;
    };
    win.__ctxE2E.clearDraftHarness = () => {
      clearDraftHarness();
      return true;
    };
    win.__ctxE2E.focusTask = (taskId: string, sessionId?: string | null) => focusTask(taskId, sessionId);
    win.__ctxE2E.toggleDiffPane = () => {
      toggleDiffPane();
      return true;
    };
    win.__ctxE2E.toggleArtifactsPane = () => {
      toggleArtifactsPane();
      return true;
    };

    return () => {
      if (!win.__ctxE2E) return;
      delete win.__ctxE2E.focusNewTask;
      delete win.__ctxE2E.clearDraftHarness;
      delete win.__ctxE2E.focusTask;
      delete win.__ctxE2E.toggleDiffPane;
      delete win.__ctxE2E.toggleArtifactsPane;
    };
  }, [clearDraftHarness, focusNewTask, focusTask, toggleArtifactsPane, toggleDiffPane]);
}
