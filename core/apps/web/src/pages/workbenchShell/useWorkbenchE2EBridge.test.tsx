import { render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useWorkbenchE2EBridge } from "./useWorkbenchE2EBridge";

vi.mock("../../utils/desktop", () => ({
  desktopGetViewGeometry: vi.fn(async () => ({
    scaleFactor: 2,
    devicePixelRatio: 2,
    webviewPosition: { x: 0, y: 0 },
    webviewSize: { width: 1728, height: 994 },
    windowInnerPosition: { x: 0, y: 66 },
    windowOuterPosition: { x: 0, y: 66 },
    windowInnerSize: { width: 1728, height: 994 },
    windowOuterSize: { width: 1728, height: 994 },
    screenWidth: 1728,
    screenHeight: 1117,
    innerWidth: 1728,
    innerHeight: 962,
  })),
}));

type E2EWindow = Window & {
  __ctxE2E?: {
    focusNewTask?: () => boolean;
    clearDraftHarness?: () => boolean;
    focusTask?: (taskId: string, sessionId?: string | null) => boolean;
    toggleDiffPane?: () => boolean;
    toggleArtifactsPane?: () => boolean;
    measureTargets?: (selectors: Record<string, string>) => Promise<unknown>;
    measureHarnessOption?: (label: string) => Promise<unknown>;
    measureDiffFile?: (targetPath: string) => Promise<unknown>;
    measureMarkdownParity?: (samples: readonly { name: string; markdown: string }[], width: number) => Promise<unknown>;
    installMarkdownScrollProbe?: (markdown: string, width?: number) => Promise<boolean>;
    removeMarkdownScrollProbe?: () => boolean;
  };
};

function TestBridge(props: {
  focusNewTask: () => void;
  clearDraftHarness: () => void;
  focusTask: (taskId: string, sessionId?: string | null) => boolean;
  toggleDiffPane: () => void;
  toggleArtifactsPane: () => void;
}) {
  useWorkbenchE2EBridge(props);
  return null;
}

describe("useWorkbenchE2EBridge", () => {
  afterEach(() => {
    sessionStorage.clear();
    delete (window as E2EWindow).__ctxE2E;
  });

  it("registers diff and artifacts toggles when ctxE2E mode is enabled", () => {
    sessionStorage.setItem("ctxE2E", "1");
    const focusNewTask = vi.fn();
    const clearDraftHarness = vi.fn();
    const focusTask = vi.fn(() => true);
    const toggleDiffPane = vi.fn();
    const toggleArtifactsPane = vi.fn();
    const e2eWindow = window as E2EWindow;

    const view = render(
      <TestBridge
        focusNewTask={focusNewTask}
        clearDraftHarness={clearDraftHarness}
        focusTask={focusTask}
        toggleDiffPane={toggleDiffPane}
        toggleArtifactsPane={toggleArtifactsPane}
      />,
    );

    expect(typeof e2eWindow.__ctxE2E?.focusNewTask).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.clearDraftHarness).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.focusTask).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.toggleDiffPane).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.toggleArtifactsPane).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.measureTargets).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.measureHarnessOption).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.measureDiffFile).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.measureMarkdownParity).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.installMarkdownScrollProbe).toBe("function");
    expect(typeof e2eWindow.__ctxE2E?.removeMarkdownScrollProbe).toBe("function");
    expect(e2eWindow.__ctxE2E?.focusNewTask?.()).toBe(true);
    expect(e2eWindow.__ctxE2E?.clearDraftHarness?.()).toBe(true);
    expect(e2eWindow.__ctxE2E?.focusTask?.("task-1", "session-1")).toBe(true);
    expect(e2eWindow.__ctxE2E?.toggleDiffPane?.()).toBe(true);
    expect(e2eWindow.__ctxE2E?.toggleArtifactsPane?.()).toBe(true);
    expect(focusNewTask).toHaveBeenCalledTimes(1);
    expect(clearDraftHarness).toHaveBeenCalledTimes(1);
    expect(focusTask).toHaveBeenCalledWith("task-1", "session-1");
    expect(toggleDiffPane).toHaveBeenCalledTimes(1);
    expect(toggleArtifactsPane).toHaveBeenCalledTimes(1);

    view.unmount();
    expect(e2eWindow.__ctxE2E?.focusNewTask).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.clearDraftHarness).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.focusTask).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.toggleDiffPane).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.toggleArtifactsPane).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.measureTargets).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.measureHarnessOption).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.measureDiffFile).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.measureMarkdownParity).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.installMarkdownScrollProbe).toBeUndefined();
    expect(e2eWindow.__ctxE2E?.removeMarkdownScrollProbe).toBeUndefined();
  });
});
