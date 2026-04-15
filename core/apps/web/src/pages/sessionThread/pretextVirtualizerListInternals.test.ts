import type { PretextVirtualizerLogicalAnchor, PretextVirtualizerSnapshot } from "@pretext-virtualizer/core";
import { describe, expect, it } from "vitest";
import type { WorkbenchListItem } from "../SessionPage.types";
import type { WorkbenchThreadProjectionOp } from "../sessionThreadProjection";
import { resolveLocalizedAnchorOverride } from "./pretextVirtualizerListInternals";

function makeMessage(id: string): Extract<WorkbenchListItem, { kind: "message" }> {
  return {
    kind: "message",
    id,
    role: "user",
    content: id,
    attachments: [],
    created_at: "2026-04-15T00:00:00.000Z",
  };
}

function makeSnapshot(
  visibleItems: PretextVirtualizerSnapshot<WorkbenchListItem>["visibleItems"],
): PretextVirtualizerSnapshot<WorkbenchListItem> {
  return {
    scrollTop: 100,
    viewportHeight: 300,
    viewportWidth: 900,
    totalHeight: 2000,
    widthBucket: "w14",
    anchor: { kind: "bottom" },
    visibleItems,
  };
}

const projectionOp: WorkbenchThreadProjectionOp = {
  kind: "toggle_expansion",
  projectionRevision: 1,
  changedItemIds: ["changed"],
  remeasureItemIds: ["changed"],
};

describe("resolveLocalizedAnchorOverride", () => {
  it("keeps the fallback anchor when the interacted row starts below the viewport top", () => {
    const snapshot = makeSnapshot([
      {
        id: "top",
        index: 0,
        item: makeMessage("top"),
        layoutRevision: "top",
        top: 80,
        height: 60,
        widthBucket: "w14",
      },
      {
        id: "changed",
        index: 1,
        item: makeMessage("changed"),
        layoutRevision: "changed",
        top: 220,
        height: 140,
        widthBucket: "w14",
      },
    ]);
    const fallback: PretextVirtualizerLogicalAnchor = {
      kind: "item",
      id: "top",
      index: 0,
      offsetPx: 20,
      offsetRatio: 20 / 60,
    };

    expect(resolveLocalizedAnchorOverride(snapshot, projectionOp, "changed", fallback)).toEqual(fallback);
  });

  it("anchors to the interacted row when that row already overlaps the viewport top", () => {
    const snapshot = makeSnapshot([
      {
        id: "changed",
        index: 0,
        item: makeMessage("changed"),
        layoutRevision: "changed",
        top: 60,
        height: 140,
        widthBucket: "w14",
      },
    ]);
    const fallback: PretextVirtualizerLogicalAnchor = { kind: "bottom" };

    expect(resolveLocalizedAnchorOverride(snapshot, projectionOp, "changed", fallback)).toEqual({
      kind: "item",
      id: "changed",
      index: 0,
      offsetPx: 40,
      offsetRatio: 40 / 140,
    });
  });
});
