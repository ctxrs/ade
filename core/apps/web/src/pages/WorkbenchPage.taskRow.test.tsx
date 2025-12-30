import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TaskRow } from "./WorkbenchPage";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";

function mockRaf() {
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb: FrameRequestCallback) => {
    return window.setTimeout(() => cb(performance.now()), 0) as unknown as number;
  });
}

describe("TaskRow rename draft", () => {
  beforeEach(() => {
    mockRaf();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("persists a draft across remounts during rename", () => {
    const drafts = new Map<string, string>();
    const getRenameDraft = (taskId: string, fallback: string) => drafts.get(taskId) ?? fallback;
    const setRenameDraft = (taskId: string, nextValue: string) => {
      drafts.set(taskId, nextValue);
    };

    const baseProps = {
      taskId: "task-1",
      title: "Initial title",
      archived: false,
      selected: false,
      hovered: false,
      isRenaming: true,
      working: false,
      dotKind: null,
      ageIso: new Date().toISOString(),
      providerCount: 0,
      harnesses: [] as Array<(typeof HARNESS_CATALOG)[number]>,
      getRenameDraft,
      setRenameDraft,
      onFocusTask: vi.fn(),
      onOpenMenu: vi.fn(),
      onToggleArchive: vi.fn().mockResolvedValue(undefined),
      onHoverEnter: vi.fn(),
      onHoverLeave: vi.fn(),
      onCancelRename: vi.fn(),
      onCommitRename: vi.fn(),
    };

    const { rerender } = render(<TaskRow {...baseProps} key="a" />);
    const input = screen.getByLabelText("Rename task") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "Draft title" } });
    expect(input.value).toBe("Draft title");

    rerender(<TaskRow {...baseProps} key="b" title="Server update" />);
    const remountedInput = screen.getByLabelText("Rename task") as HTMLInputElement;
    expect(remountedInput.value).toBe("Draft title");
  });
});
