import { describe, expect, it } from "vitest";
import type { PersistedWorkbenchWindowV1 } from "../workbench/types";
import {
  clampSplitRatio,
  focusLeaf,
  resizeSplitRatio,
  splitFocusedLeaf,
} from "./workbenchStoreLayout";

function makeWindow(): PersistedWorkbenchWindowV1 {
  return {
    v: 1,
    layout: {
      kind: "leaf",
      id: "leaf-1",
      tabs: [{ id: "tab-1", kind: "new_task" }],
      activeTabId: "tab-1",
    },
    focusedLeafId: "leaf-1",
  };
}

describe("workbench split layout helpers", () => {
  it("splits the focused leaf into a split with a new task leaf", () => {
    const next = splitFocusedLeaf(makeWindow(), "horizontal", {
      splitId: "split-1",
      leafId: "leaf-2",
      tabId: "tab-2",
      ratio: 0.65,
    });

    expect(next.focusedLeafId).toBe("leaf-2");
    expect(next.layout).toEqual({
      kind: "split",
      id: "split-1",
      direction: "horizontal",
      ratio: 0.65,
      first: {
        kind: "leaf",
        id: "leaf-1",
        tabs: [{ id: "tab-1", kind: "new_task" }],
        activeTabId: "tab-1",
      },
      second: {
        kind: "leaf",
        id: "leaf-2",
        tabs: [{ id: "tab-2", kind: "new_task" }],
        activeTabId: "tab-2",
      },
    });
  });

  it("focuses an existing leaf and falls back to the first leaf for stale ids", () => {
    const split = splitFocusedLeaf(makeWindow(), "vertical", {
      splitId: "split-1",
      leafId: "leaf-2",
      tabId: "tab-2",
    });

    expect(focusLeaf(split, "leaf-1").focusedLeafId).toBe("leaf-1");
    expect(focusLeaf(split, "missing").focusedLeafId).toBe("leaf-1");
  });

  it("clamps split ratios when creating and resizing splits", () => {
    expect(clampSplitRatio(-1)).toBe(0.1);
    expect(clampSplitRatio(2)).toBe(0.9);
    expect(clampSplitRatio(Number.NaN)).toBe(0.5);

    const split = splitFocusedLeaf(makeWindow(), "vertical", {
      splitId: "split-1",
      leafId: "leaf-2",
      tabId: "tab-2",
      ratio: 2,
    });
    expect(split.layout.kind).toBe("split");
    if (split.layout.kind !== "split") return;
    expect(split.layout.ratio).toBe(0.9);

    const resized = resizeSplitRatio(split.layout, "split-1", -1);
    expect(resized.kind).toBe("split");
    if (resized.kind !== "split") return;
    expect(resized.ratio).toBe(0.1);
    expect(resizeSplitRatio(resized, "missing", 0.5)).toBe(resized);
  });
});
