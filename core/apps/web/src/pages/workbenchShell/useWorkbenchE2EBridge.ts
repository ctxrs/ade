import { useEffect } from "react";
import {
  measureWorkbenchDiffFile,
  measureWorkbenchHarnessOption,
  measureWorkbenchTargets,
  type WorkbenchE2EMeasuredTargetResult,
  type WorkbenchE2EMeasureTargetsResult,
} from "./workbenchE2EMeasurements";
import {
  installWorkbenchMarkdownScrollProbe,
  measureWorkbenchMarkdownParity,
  removeWorkbenchMarkdownScrollProbe,
  type WorkbenchMarkdownParityMeasurement,
  type WorkbenchMarkdownParitySample,
} from "./workbenchE2EMarkdown";

type WorkbenchE2EWindow = Window & {
  __ctxE2E?: {
    focusNewTask?: () => boolean;
    clearDraftHarness?: () => boolean;
    focusTask?: (taskId: string, sessionId?: string | null) => boolean;
    toggleDiffPane?: () => boolean;
    toggleArtifactsPane?: () => boolean;
    measureTargets?: (selectors: Record<string, string>) => Promise<WorkbenchE2EMeasureTargetsResult>;
    measureHarnessOption?: (label: string) => Promise<WorkbenchE2EMeasuredTargetResult | null>;
    measureDiffFile?: (targetPath: string) => Promise<WorkbenchE2EMeasuredTargetResult | null>;
    measureMarkdownParity?: (
      samples: readonly WorkbenchMarkdownParitySample[],
      width: number,
    ) => Promise<WorkbenchMarkdownParityMeasurement[]>;
    installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
    removeMarkdownScrollProbe?: () => boolean;
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
    win.__ctxE2E.measureTargets = (selectors: Record<string, string>) => measureWorkbenchTargets(selectors);
    win.__ctxE2E.measureHarnessOption = (label: string) => measureWorkbenchHarnessOption(label);
    win.__ctxE2E.measureDiffFile = (targetPath: string) => measureWorkbenchDiffFile(targetPath);
    win.__ctxE2E.measureMarkdownParity = (
      samples: readonly WorkbenchMarkdownParitySample[],
      width: number,
    ) => measureWorkbenchMarkdownParity(samples, width);
    win.__ctxE2E.installMarkdownScrollProbe = (markdown: string, width?: number) =>
      installWorkbenchMarkdownScrollProbe(markdown, width);
    win.__ctxE2E.removeMarkdownScrollProbe = () => {
      removeWorkbenchMarkdownScrollProbe();
      return true;
    };

    return () => {
      if (!win.__ctxE2E) return;
      delete win.__ctxE2E.focusNewTask;
      delete win.__ctxE2E.clearDraftHarness;
      delete win.__ctxE2E.focusTask;
      delete win.__ctxE2E.toggleDiffPane;
      delete win.__ctxE2E.toggleArtifactsPane;
      delete win.__ctxE2E.measureTargets;
      delete win.__ctxE2E.measureHarnessOption;
      delete win.__ctxE2E.measureDiffFile;
      delete win.__ctxE2E.measureMarkdownParity;
      delete win.__ctxE2E.installMarkdownScrollProbe;
      delete win.__ctxE2E.removeMarkdownScrollProbe;
    };
  }, [clearDraftHarness, focusNewTask, focusTask, toggleArtifactsPane, toggleDiffPane]);
}
