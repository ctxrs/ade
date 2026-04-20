import {
  type PretextVirtualizerLogicalAnchor,
  type PretextVirtualizerSnapshot,
  type createPretextVirtualizerCore,
} from "@pretext-virtualizer/core";
import type { WorkbenchListItem } from "../SessionPage.types";
import type { WorkbenchThreadProjectionOp } from "../sessionThreadProjection";
import { computeBottomOffsetPx, shouldFollowBottomOnItemsUpdate } from "./pretextFollowBottom";
import {
  haveSameLayoutInputs,
  isLocalizedProjectionOp,
  resolveHistoryPrependAnchorOverride,
  resolveLocalizedAnchorOverride,
  syncSnapshotForProjectionOp,
} from "./pretextVirtualizerProjectionHelpers";

export type SessionThreadVisibleProjectionPreparedState = {
  snapshot: PretextVirtualizerSnapshot<WorkbenchListItem>;
  listItems: readonly WorkbenchListItem[];
};

export type SessionThreadVisibleProjectionUpdatePlan =
  | {
      kind: "noop";
      projectionOpKey: string | null;
      uiStateChanged: boolean;
      itemsChanged: boolean;
      projectionChanged: boolean;
    }
  | {
      kind: "apply";
      projectionOpKey: string | null;
      uiStateChanged: boolean;
      itemsChanged: boolean;
      projectionChanged: boolean;
      changeKind: "full-relayout" | "localized-patch";
      reason: string;
      currentSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>;
      nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>;
      anchorOverride: PretextVirtualizerLogicalAnchor;
      shouldFollowBottom: boolean;
    };

function buildProjectionOpKey(projectionOp: WorkbenchThreadProjectionOp): string | null {
  if (projectionOp.kind === "noop") {
    return null;
  }
  return [
    projectionOp.projectionRevision,
    projectionOp.kind,
    projectionOp.changedItemIds.join(","),
    projectionOp.remeasureItemIds.join(","),
  ].join("|");
}

export function buildVisibleProjectionUpdatePlan(params: {
  core: ReturnType<typeof createPretextVirtualizerCore<WorkbenchListItem>>;
  preparedState: SessionThreadVisibleProjectionPreparedState;
  listItems: readonly WorkbenchListItem[];
  threadProjectionOp: WorkbenchThreadProjectionOp;
  runtimeUiStateLayoutRevision: string;
  lastAppliedUiStateLayoutRevision: string | null;
  lastAppliedProjectionOpKey: string | null;
  viewport: {
    width: number;
    height: number;
    scrollTop: number;
  };
  followBottom: boolean;
  atBottom: boolean;
  activeChangedItemId: string | null;
  getLayoutRevision: (item: WorkbenchListItem) => string | number;
  bottomThresholdPx: number;
}): SessionThreadVisibleProjectionUpdatePlan {
  const uiStateChanged =
    params.lastAppliedUiStateLayoutRevision !== params.runtimeUiStateLayoutRevision;
  const itemsChanged = !haveSameLayoutInputs(
    params.preparedState.listItems,
    params.listItems,
    params.getLayoutRevision,
  );
  const projectionOpKey = buildProjectionOpKey(params.threadProjectionOp);
  const projectionChanged =
    projectionOpKey != null && params.lastAppliedProjectionOpKey !== projectionOpKey;

  if (!itemsChanged && !projectionChanged && !uiStateChanged) {
    return {
      kind: "noop",
      projectionOpKey,
      uiStateChanged,
      itemsChanged,
      projectionChanged,
    };
  }

  const changeKind =
    uiStateChanged ||
    !projectionChanged ||
    !isLocalizedProjectionOp(params.threadProjectionOp.kind)
      ? "full-relayout"
      : "localized-patch";
  const reason = projectionChanged
    ? `visible:${params.threadProjectionOp.kind}`
    : uiStateChanged
      ? "visible:ui-state"
      : "visible:items";

  const currentSnapshot = params.core.syncViewport({
    height: params.viewport.height,
    width: params.viewport.width,
    scrollTop: params.viewport.scrollTop,
  });
  const bottomOffsetPx = computeBottomOffsetPx({
    totalHeight: currentSnapshot.totalHeight,
    scrollTop: currentSnapshot.scrollTop,
    viewportHeight: currentSnapshot.viewportHeight,
  });
  const shouldFollowBottom = shouldFollowBottomOnItemsUpdate(
    {
      followBottom: params.followBottom,
      atBottom: params.atBottom,
    },
    bottomOffsetPx,
    params.bottomThresholdPx,
  );
  const defaultAnchorOverride: PretextVirtualizerLogicalAnchor = shouldFollowBottom
    ? { kind: "bottom" }
    : params.core.getAnchor("detached");
  const anchorOverride = shouldFollowBottom
    ? defaultAnchorOverride
    : params.threadProjectionOp.kind === "prepend_history"
      ? resolveHistoryPrependAnchorOverride(currentSnapshot, defaultAnchorOverride)
      : resolveLocalizedAnchorOverride(
          currentSnapshot,
          params.threadProjectionOp,
          params.activeChangedItemId,
          defaultAnchorOverride,
        );
  const nextSnapshot = syncSnapshotForProjectionOp({
    core: params.core,
    items: params.listItems,
    projectionOp: params.threadProjectionOp,
    previousItems: params.preparedState.listItems,
    anchorOverride,
  });

  return {
    kind: "apply",
    projectionOpKey,
    uiStateChanged,
    itemsChanged,
    projectionChanged,
    changeKind,
    reason,
    currentSnapshot,
    nextSnapshot,
    anchorOverride,
    shouldFollowBottom,
  };
}
