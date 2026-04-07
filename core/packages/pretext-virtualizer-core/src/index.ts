export type PretextVirtualizerLayoutRevision = string | number;
export type PretextVirtualizerWidthBucket = `w${number}`;
export type PretextVirtualizerAnchorRestoreMode = "offset" | "ratio";

export type PretextVirtualizerPlannedLayout = {
  height: number;
};

export type PretextVirtualizerLogicalAnchor =
  | { kind: "bottom" }
  | {
      kind: "item";
      id: string;
      index: number;
      offsetPx: number;
      offsetRatio: number;
    };

export type PretextVirtualizerVisibleItem<Item> = {
  id: string;
  index: number;
  item: Item;
  layoutRevision: PretextVirtualizerLayoutRevision;
  top: number;
  height: number;
  widthBucket: PretextVirtualizerWidthBucket;
};

export type PretextVirtualizerSnapshot<Item> = {
  scrollTop: number;
  viewportHeight: number;
  viewportWidth: number;
  totalHeight: number;
  widthBucket: PretextVirtualizerWidthBucket;
  anchor: PretextVirtualizerLogicalAnchor;
  visibleItems: readonly PretextVirtualizerVisibleItem<Item>[];
};

export type PretextVirtualizerDiagnosticEvent<Item> = {
  type: string;
  snapshot: PretextVirtualizerSnapshot<Item>;
  detail?: Record<string, unknown> | null;
};

export type PretextVirtualizerCoreOptions<Item> = {
  initialItems?: readonly Item[];
  getPlannedLayout: (
    item: Item,
    viewport: {
      width: number;
      widthBucket: PretextVirtualizerWidthBucket;
    },
  ) => PretextVirtualizerPlannedLayout;
  getId: (item: Item) => string;
  getLayoutRevision: (item: Item) => PretextVirtualizerLayoutRevision;
  overscanPx?: number;
  bottomThresholdPx?: number;
  widthBucketSize?: number;
  viewportHeight?: number;
  viewportWidth?: number;
  onDiagnosticEvent?: (event: PretextVirtualizerDiagnosticEvent<Item>) => void;
};

type PretextVirtualizerCore<Item> = {
  getSnapshot: () => PretextVirtualizerSnapshot<Item>;
  getAnchor: () => PretextVirtualizerLogicalAnchor;
  syncViewport: (viewport: {
    height: number;
    width: number;
    scrollTop: number;
  }) => PretextVirtualizerSnapshot<Item>;
  replaceItems: (
    items: readonly Item[],
    anchorOverride?: PretextVirtualizerLogicalAnchor | null,
  ) => PretextVirtualizerSnapshot<Item>;
  appendItems: (
    items: readonly Item[],
    anchorOverride?: PretextVirtualizerLogicalAnchor | null,
  ) => PretextVirtualizerSnapshot<Item>;
  prependItems: (
    items: readonly Item[],
    anchorOverride?: PretextVirtualizerLogicalAnchor | null,
  ) => PretextVirtualizerSnapshot<Item>;
  syncItems: (
    items: readonly Item[],
    anchorOverride?: PretextVirtualizerLogicalAnchor | null,
  ) => PretextVirtualizerSnapshot<Item>;
  restoreAnchor: (
    anchor: PretextVirtualizerLogicalAnchor,
    mode?: PretextVirtualizerAnchorRestoreMode,
  ) => PretextVirtualizerSnapshot<Item>;
  getOffsetForIndex: (index: number) => number;
  getHeightForIndex: (index: number) => number;
};

const DEFAULT_OVERSCAN_PX = 320;
const DEFAULT_BOTTOM_THRESHOLD_PX = 16;
const DEFAULT_WIDTH_BUCKET_SIZE = 64;
const MIN_VISIBLE_ANCHOR_PX = 4;
const MIN_MEANINGFUL_VISIBLE_ANCHOR_PX = 24;

const normalizeHeight = (value: number): number =>
  Number.isFinite(value) && value > 0 ? Math.max(1, Math.round(value * 16) / 16) : 1;

const normalizeSize = (value: number): number =>
  Number.isFinite(value) && value > 0 ? value : 0;

const clamp = (value: number, min: number, max: number): number =>
  Math.max(min, Math.min(max, value));

export const createWidthBucket = (
  viewportWidth: number,
  widthBucketSize = DEFAULT_WIDTH_BUCKET_SIZE,
): PretextVirtualizerWidthBucket => {
  const normalizedWidth = normalizeSize(viewportWidth);
  const normalizedBucketSize = Math.max(1, Math.round(widthBucketSize));
  return `w${Math.floor(normalizedWidth / normalizedBucketSize)}`;
};

type PretextVirtualizerComputedLayout<Item> = {
  widthBucket: PretextVirtualizerWidthBucket;
  heights: Array<{
    id: string;
    item: Item;
    layoutRevision: PretextVirtualizerLayoutRevision;
    height: number;
  }>;
  offsets: number[];
  totalHeight: number;
};

export const createPretextVirtualizerCore = <Item,>({
  initialItems = [],
  getPlannedLayout,
  getId,
  getLayoutRevision,
  overscanPx = DEFAULT_OVERSCAN_PX,
  bottomThresholdPx = DEFAULT_BOTTOM_THRESHOLD_PX,
  widthBucketSize = DEFAULT_WIDTH_BUCKET_SIZE,
  viewportHeight = 0,
  viewportWidth = 0,
  onDiagnosticEvent,
}: PretextVirtualizerCoreOptions<Item>): PretextVirtualizerCore<Item> => {
  const state = {
    items: [...initialItems],
    viewportHeight: normalizeSize(viewportHeight),
    viewportWidth: normalizeSize(viewportWidth),
    scrollTop: 0,
  };
  let layoutCache: PretextVirtualizerComputedLayout<Item> | null = null;

  const invalidateLayout = () => {
    layoutCache = null;
  };

  const computeLayout = (): PretextVirtualizerComputedLayout<Item> => {
    if (layoutCache) return layoutCache;
    const widthBucket = createWidthBucket(state.viewportWidth, widthBucketSize);
    const heights = state.items.map((item) => ({
      id: getId(item),
      item,
      layoutRevision: getLayoutRevision(item),
      height: normalizeHeight(getPlannedLayout(item, { width: state.viewportWidth, widthBucket }).height),
    }));
    const offsets = new Array<number>(heights.length);
    let runningTop = 0;
    for (let index = 0; index < heights.length; index += 1) {
      offsets[index] = runningTop;
      runningTop += heights[index]!.height;
    }
    layoutCache = {
      widthBucket,
      heights,
      offsets,
      totalHeight: runningTop,
    };
    return layoutCache;
  };

  const getMaxScrollTop = (totalHeight: number): number =>
    Math.max(0, totalHeight - normalizeSize(state.viewportHeight));

  const clampScrollTop = (scrollTop: number, totalHeight: number): number =>
    clamp(Number.isFinite(scrollTop) ? scrollTop : 0, 0, getMaxScrollTop(totalHeight));

  const captureAnchor = (
    layout = computeLayout(),
    scrollTop = state.scrollTop,
  ): PretextVirtualizerLogicalAnchor => {
    const normalizedScrollTop = clampScrollTop(scrollTop, layout.totalHeight);
    const bottomOffsetPx = layout.totalHeight - (normalizedScrollTop + state.viewportHeight);
    if (bottomOffsetPx <= bottomThresholdPx) {
      return { kind: "bottom" };
    }
    const viewportBottom = normalizedScrollTop + state.viewportHeight;
    let fallbackAnchor: PretextVirtualizerLogicalAnchor | null = null;
    for (let index = 0; index < layout.heights.length; index += 1) {
      const entry = layout.heights[index]!;
      const top = layout.offsets[index]!;
      const bottom = top + entry.height;
      if (bottom <= normalizedScrollTop) continue;
      const visibleHeight = Math.min(bottom, viewportBottom) - Math.max(top, normalizedScrollTop);
      if (visibleHeight <= MIN_VISIBLE_ANCHOR_PX) continue;
      const offsetPx = clamp(normalizedScrollTop - top, 0, Math.max(0, entry.height - 1));
      const nextAnchor: PretextVirtualizerLogicalAnchor = {
        kind: "item",
        id: entry.id,
        index,
        offsetPx,
        offsetRatio: entry.height > 0 ? offsetPx / entry.height : 0,
      };
      if (visibleHeight >= MIN_MEANINGFUL_VISIBLE_ANCHOR_PX) {
        return nextAnchor;
      }
      fallbackAnchor ??= nextAnchor;
    }
    return fallbackAnchor ?? { kind: "bottom" };
  };

  const createSnapshot = (): PretextVirtualizerSnapshot<Item> => {
    const layout = computeLayout();
    state.scrollTop = clampScrollTop(state.scrollTop, layout.totalHeight);
    const visibleTop = Math.max(0, state.scrollTop - overscanPx);
    const visibleBottom = state.scrollTop + state.viewportHeight + overscanPx;
    const visibleItems: PretextVirtualizerVisibleItem<Item>[] = [];
    for (let index = 0; index < layout.heights.length; index += 1) {
      const entry = layout.heights[index]!;
      const top = layout.offsets[index]!;
      const bottom = top + entry.height;
      if (bottom < visibleTop) continue;
      if (top > visibleBottom) break;
      visibleItems.push({
        id: entry.id,
        index,
        item: entry.item,
        layoutRevision: entry.layoutRevision,
        top,
        height: entry.height,
        widthBucket: layout.widthBucket,
      });
    }
    return {
      scrollTop: state.scrollTop,
      viewportHeight: state.viewportHeight,
      viewportWidth: state.viewportWidth,
      totalHeight: layout.totalHeight,
      widthBucket: layout.widthBucket,
      anchor: captureAnchor(layout, state.scrollTop),
      visibleItems,
    };
  };

  const emitDiagnostic = (
    type: string,
    snapshot: PretextVirtualizerSnapshot<Item>,
    detail?: Record<string, unknown> | null,
  ) => {
    onDiagnosticEvent?.({ type, snapshot, detail });
  };

  const restoreAnchorIntoState = (
    anchor: PretextVirtualizerLogicalAnchor,
    mode: PretextVirtualizerAnchorRestoreMode = "offset",
  ): PretextVirtualizerSnapshot<Item> => {
    const layout = computeLayout();
    if (anchor.kind === "bottom") {
      state.scrollTop = getMaxScrollTop(layout.totalHeight);
      const snapshot = createSnapshot();
      emitDiagnostic("restore:bottom", snapshot, { anchorKind: "bottom" });
      return snapshot;
    }
    const resolvedIndex = layout.heights.findIndex((entry) => entry.id === anchor.id);
    const targetIndex = resolvedIndex >= 0 ? resolvedIndex : clamp(anchor.index, 0, Math.max(0, layout.heights.length - 1));
    const target = layout.heights[targetIndex];
    if (!target) {
      const snapshot = createSnapshot();
      emitDiagnostic("restore:missing", snapshot, { anchorKind: "item", missingId: anchor.id });
      return snapshot;
    }
    const top = layout.offsets[targetIndex] ?? 0;
    const targetOffset =
      mode === "ratio"
        ? clamp(anchor.offsetRatio * target.height, 0, Math.max(0, target.height - 1))
        : clamp(anchor.offsetPx, 0, Math.max(0, target.height - 1));
    state.scrollTop = clampScrollTop(top + targetOffset, layout.totalHeight);
    const snapshot = createSnapshot();
    emitDiagnostic("restore:item", snapshot, {
      anchorKind: "item",
      anchorId: anchor.id,
      targetIndex,
      mode,
    });
    return snapshot;
  };

  const preserveAnchorAcrossItems = (
    nextItems: readonly Item[],
    anchorOverride?: PretextVirtualizerLogicalAnchor | null,
  ): PretextVirtualizerSnapshot<Item> => {
    const retainedAnchor = anchorOverride ?? createSnapshot().anchor;
    state.items = [...nextItems];
    invalidateLayout();
    return restoreAnchorIntoState(retainedAnchor, "offset");
  };

  return {
    getSnapshot: () => createSnapshot(),
    getAnchor: () => createSnapshot().anchor,
    syncViewport: ({ height, width, scrollTop }) => {
      state.viewportHeight = normalizeSize(height);
      const normalizedWidth = normalizeSize(width);
      if (normalizedWidth !== state.viewportWidth) {
        state.viewportWidth = normalizedWidth;
        invalidateLayout();
      } else {
        state.viewportWidth = normalizedWidth;
      }
      const layout = computeLayout();
      state.scrollTop = clampScrollTop(scrollTop, layout.totalHeight);
      const snapshot = createSnapshot();
      emitDiagnostic("viewport:sync", snapshot, {
        viewportHeight: state.viewportHeight,
        viewportWidth: state.viewportWidth,
      });
      return snapshot;
    },
    replaceItems: (items, anchorOverride) => {
      const snapshot = preserveAnchorAcrossItems(items, anchorOverride);
      emitDiagnostic("items:replace", snapshot, {
        itemCount: items.length,
        anchorOverrideKind: anchorOverride?.kind ?? null,
      });
      return snapshot;
    },
    appendItems: (items, anchorOverride) => {
      const snapshot = preserveAnchorAcrossItems([...state.items, ...items], anchorOverride);
      emitDiagnostic("items:append", snapshot, {
        itemCount: state.items.length,
        deltaCount: items.length,
        anchorOverrideKind: anchorOverride?.kind ?? null,
      });
      return snapshot;
    },
    prependItems: (items, anchorOverride) => {
      const snapshot = preserveAnchorAcrossItems([...items, ...state.items], anchorOverride);
      emitDiagnostic("items:prepend", snapshot, {
        itemCount: state.items.length,
        deltaCount: items.length,
        anchorOverrideKind: anchorOverride?.kind ?? null,
      });
      return snapshot;
    },
    syncItems: (items, anchorOverride) => {
      const snapshot = preserveAnchorAcrossItems(items, anchorOverride);
      emitDiagnostic("items:sync", snapshot, {
        itemCount: items.length,
        anchorOverrideKind: anchorOverride?.kind ?? null,
      });
      return snapshot;
    },
    restoreAnchor: (anchor, mode = "offset") => restoreAnchorIntoState(anchor, mode),
    getOffsetForIndex: (index) => {
      const layout = computeLayout();
      const clampedIndex = clamp(index, 0, Math.max(0, layout.offsets.length - 1));
      return layout.offsets[clampedIndex] ?? 0;
    },
    getHeightForIndex: (index) => {
      const layout = computeLayout();
      const clampedIndex = clamp(index, 0, Math.max(0, layout.heights.length - 1));
      return layout.heights[clampedIndex]?.height ?? 0;
    },
  };
};
