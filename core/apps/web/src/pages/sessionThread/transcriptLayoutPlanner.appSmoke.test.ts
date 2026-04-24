import { describe, expect, it } from "vitest";
import type { WorkbenchListItem } from "../sessionView";
import { getPretextVirtualizerRowLayout } from "./pretextVirtualizerRowLayout";
import {
  getTranscriptRowPlannedLayout,
  planTranscriptRowLayout,
} from "./transcriptLayoutPlanner";

describe("transcriptLayoutPlanner app smoke", () => {
  const item: WorkbenchListItem = {
    kind: "assistant",
    id: "assistant-1",
    turn_id: "turn-1",
    content: "Planner seam verification with `inline code` and plain text.",
    thought: "",
    is_complete: true,
    created_at: "2026-04-19T00:00:00Z",
  };

  it("keeps the app shim wired to the extracted planner and row-layout package", () => {
    expect(getTranscriptRowPlannedLayout(item, 640, {})).toEqual(
      getPretextVirtualizerRowLayout(item, 640, {}),
    );

    const plan = planTranscriptRowLayout(item, 640, {});
    expect(plan.itemId).toBe(item.id);
    expect(plan.rowKind).toBe(item.kind);
    expect(plan.totalHeight).toBe(plan.plannedLayout.height);
  });
});
