import { render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useWorkbenchE2EBridge } from "./useWorkbenchE2EBridge";

type E2EWindow = Window & {
  __ctxE2E?: {
    focusNewTask?: () => boolean;
    clearDraftHarness?: () => boolean;
    focusTask?: (taskId: string, sessionId?: string | null) => boolean;
    toggleDiffPane?: () => boolean;
    toggleArtifactsPane?: () => boolean;
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
  });
});
