import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MutableRefObject,
  type ReactNode,
} from "react";
import {
  createPretextVirtualizerCore,
  type PretextVirtualizerDiagnosticEvent,
  type PretextVirtualizerLogicalAnchor,
  type PretextVirtualizerSnapshot,
} from "@pretext-virtualizer/core";
import type {
  PretextVirtualizerItemAlign,
  PretextVirtualizerItemLocation,
  PretextVirtualizerListMethods,
  PretextVirtualizerScrollLocation,
  PretextVirtualizerShortSizeAlign,
} from "@pretext-virtualizer/interface";
import { PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION } from "../state/pretextVirtualizerViewportState";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";
import { recordSessionMessageListRowSizeMismatch } from "./sessionMessageListDebug";
import {
  type WorkbenchMessageListUiState,
} from "./sessionMessageListItemIdentity";
import type { WorkbenchThreadProjectionOp } from "./sessionThreadProjection";
import {
  getOrCreateSessionPretextRuntime,
  markSessionPretextRuntimeVisible,
  noteSessionPretextRuntimeSnapshot,
  SESSION_PRETEXT_BOTTOM_THRESHOLD_PX,
} from "./sessionThread/pretextSessionRuntimeCache";
import {
  resolveFollowBottomAfterScroll,
  shouldRestoreBottomOnViewportResize,
} from "./sessionThread/pretextFollowBottom";
import { noteSessionTranscriptWarmViewport } from "./sessionThread/sessionTranscriptWarmState";

type SessionThreadPretextVirtualizerListProps = {
  style: CSSProperties;
  sessionId: string;
  isActive: boolean;
  listItems: WorkbenchListItem[];
  threadProjectionOp: WorkbenchThreadProjectionOp;
  initialLocation?: PretextVirtualizerItemLocation | null;
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  itemKey: (item: WorkbenchListItem) => string;
  context: WorkbenchMessageListContext;
  onScroll?: (location: PretextVirtualizerScrollLocation) => void;
  onRenderedDataChange?: (range: readonly WorkbenchListItem[]) => void;
  onAtBottomChange?: (atBottom: boolean) => void;
  onDiagnosticEvent?: (event: PretextVirtualizerDiagnosticEvent<WorkbenchListItem>) => void;
  methodsRef?: MutableRefObject<PretextVirtualizerListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  shortSizeAlign?: PretextVirtualizerShortSizeAlign;
};

const BOTTOM_THRESHOLD_PX = SESSION_PRETEXT_BOTTOM_THRESHOLD_PX;
const JUMP_TO_LATEST_THRESHOLD_PX = 200;
const DEBUG_ROW_SIZE_DELTA_PX = 1;

function AuditedPretextRow({
  id,
  itemKind,
  itemKey,
  plannedHeight,
  children,
}: {
  id: string;
  itemKind: WorkbenchListItem["kind"];
  itemKey: string;
  plannedHeight: number;
  children: ReactNode;
}) {
  const rowRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let debugEnabled = false;
    try {
      debugEnabled = new URLSearchParams(window.location.search).get("debug") === "1";
    } catch {
      debugEnabled = false;
    }
    if (!debugEnabled) return;

    const rowEl = rowRef.current;
    if (!rowEl) return;
    const shellEl = rowEl.closest("[data-pretext-virtualizer-row-shell='1']") as HTMLElement | null;
    if (!shellEl) return;

    let lastSignature = "";
    const emitMismatch = (reason: string) => {
      const actualHeight = rowEl.getBoundingClientRect().height;
      const shellHeight = shellEl.getBoundingClientRect().height;
      const plannedVsActualDeltaPx = actualHeight - plannedHeight;
      const plannedVsShellDeltaPx = shellHeight - plannedHeight;
      const shellVsActualDeltaPx = actualHeight - shellHeight;
      if (Math.abs(plannedVsActualDeltaPx) <= DEBUG_ROW_SIZE_DELTA_PX) return;
      const signature = `${reason}:${plannedHeight}:${Math.round(actualHeight)}:${Math.round(shellHeight)}`;
      if (signature === lastSignature) return;
      lastSignature = signature;
      recordSessionMessageListRowSizeMismatch({
        id,
        itemKind,
        itemKey,
        reason,
        dataIndex: null,
        knownSize: plannedHeight,
        actualHeight,
        parentHeight: shellHeight,
        knownVsActualDeltaPx: plannedVsActualDeltaPx,
        knownVsParentDeltaPx: plannedVsShellDeltaPx,
        parentVsActualDeltaPx: shellVsActualDeltaPx,
      });
      // eslint-disable-next-line no-console
      console.log("[PretextVirtualizer][row-size-mismatch]", {
        id,
        itemKind,
        itemKey,
        reason,
        plannedHeight,
        actualHeight,
        shellHeight,
        plannedVsActualDeltaPx,
        plannedVsShellDeltaPx,
        shellVsActualDeltaPx,
      });
    };

    emitMismatch("mount");
    const observer = new ResizeObserver(() => emitMismatch("resize"));
    observer.observe(rowEl);
    observer.observe(shellEl);
    const rafId = requestAnimationFrame(() => emitMismatch("raf"));
    return () => {
      cancelAnimationFrame(rafId);
      observer.disconnect();
    };
  }, [id, itemKey, itemKind, plannedHeight]);

  return (
    <div ref={rowRef} role="listitem" data-thread-item-id={id}>
      {children}
    </div>
  );
}

function resolveScrollTopForLocation(
  snapshot: PretextVirtualizerSnapshot<WorkbenchListItem>,
  location: PretextVirtualizerItemLocation,
  core: ReturnType<typeof createPretextVirtualizerCore<WorkbenchListItem>>,
  itemCount: number,
): number {
  if (snapshot.visibleItems.length === 0 && location.index === "LAST") {
    return Math.max(0, snapshot.totalHeight - snapshot.viewportHeight);
  }
  const totalHeight = snapshot.totalHeight;
  const viewportHeight = snapshot.viewportHeight;
  const maxScrollTop = Math.max(0, totalHeight - viewportHeight);
  const rawIndex = location.index === "LAST" ? Math.max(0, itemCount - 1) : location.index;
  const targetIndex = Math.max(0, Math.min(rawIndex, Math.max(0, itemCount - 1)));
  const top = core.getOffsetForIndex(targetIndex);
  const height = core.getHeightForIndex(targetIndex);
  const align: PretextVirtualizerItemAlign = location.align ?? "start";
  if (align === "end") {
    return Math.max(0, Math.min(maxScrollTop, top + height - viewportHeight));
  }
  if (align === "center") {
    return Math.max(0, Math.min(maxScrollTop, top + height / 2 - viewportHeight / 2));
  }
  return Math.max(0, Math.min(maxScrollTop, top));
}

function approximateIndexForLocation(
  items: readonly WorkbenchListItem[],
  location: PretextVirtualizerItemLocation,
): number {
  if (items.length === 0) return 0;
  if (location.index === "LAST") return items.length - 1;
  return Math.max(0, Math.min(location.index, items.length - 1));
}

export const SessionThreadPretextVirtualizerList = memo(function SessionThreadPretextVirtualizerList({
  style,
  sessionId,
  isActive: _isActive,
  listItems,
  threadProjectionOp,
  initialLocation = PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION,
  itemContent,
  itemKey,
  context,
  onScroll,
  onRenderedDataChange,
  onAtBottomChange,
  onDiagnosticEvent,
  methodsRef,
  shortSizeAlign = "top",
}: SessionThreadPretextVirtualizerListProps) {
  const listItemsRef = useRef(listItems);
  const onScrollRef = useRef(onScroll);
  const onRenderedDataChangeRef = useRef(onRenderedDataChange);
  const onAtBottomChangeRef = useRef(onAtBottomChange);
  const onDiagnosticEventRef = useRef(onDiagnosticEvent);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const followBottomRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  const lastSyncedItemCountRef = useRef(listItems.length);
  const lastAtBottomRef = useRef<boolean | null>(null);
  const snapshotRef = useRef<PretextVirtualizerSnapshot<WorkbenchListItem> | null>(null);
  const pendingProgrammaticTopRef = useRef<number | null>(null);
  const pendingProgrammaticBehaviorRef = useRef<ScrollBehavior>("auto");
  const pendingRestoreRef = useRef(false);
  const [showJumpToLatest, setShowJumpToLatest] = useState(false);

  listItemsRef.current = listItems;
  onScrollRef.current = onScroll;
  onRenderedDataChangeRef.current = onRenderedDataChange;
  onAtBottomChangeRef.current = onAtBottomChange;
  onDiagnosticEventRef.current = onDiagnosticEvent;

  const runtimeUiState = useMemo<WorkbenchMessageListUiState>(
    () => ({
      expandedTurnHeaders: { ...(context.expandedTurnHeaders ?? {}) },
      expandedTurnDetailsById: { ...(context.expandedTurnDetailsById ?? {}) },
      expandedToolById: { ...(context.expandedToolById ?? {}) },
      expandedMessageById: { ...(context.expandedMessageById ?? {}) },
      turnToolsLoading: [...(context.turnToolsLoading ?? [])],
      verbosity: context.verbosity,
    }),
    [
      context.expandedMessageById,
      context.expandedToolById,
      context.expandedTurnDetailsById,
      context.expandedTurnHeaders,
      context.turnToolsLoading,
      context.verbosity,
    ],
  );

  const runtime = useMemo(
    () =>
      getOrCreateSessionPretextRuntime(sessionId, {
        uiState: runtimeUiState,
        onDiagnosticEvent: (event) => {
          onDiagnosticEventRef.current?.(event);
        },
      }),
    [runtimeUiState, sessionId],
  );
  const core = runtime.core;

  const [snapshot, setSnapshot] = useState<PretextVirtualizerSnapshot<WorkbenchListItem>>(() => runtime.lastSnapshot);
  snapshotRef.current = snapshot;

  const emitRenderedData = useCallback((nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>) => {
    onRenderedDataChangeRef.current?.(nextSnapshot.visibleItems.map((item) => item.item));
  }, []);

  const commitRuntimeSnapshot = useCallback(
    (
      nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>,
      nextItems: readonly WorkbenchListItem[] = listItemsRef.current,
    ) => {
      noteSessionPretextRuntimeSnapshot(runtime, nextSnapshot, nextItems);
      noteSessionTranscriptWarmViewport({
        width: nextSnapshot.viewportWidth,
        height: nextSnapshot.viewportHeight,
      });
    },
    [runtime],
  );

  const emitScrollState = useCallback((scroller: HTMLDivElement, nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>) => {
    const bottomOffsetPx = Math.max(0, scroller.scrollHeight - (scroller.scrollTop + scroller.clientHeight));
    const atBottom = bottomOffsetPx <= BOTTOM_THRESHOLD_PX;
    if (lastAtBottomRef.current !== atBottom) {
      lastAtBottomRef.current = atBottom;
      onAtBottomChangeRef.current?.(atBottom);
    }
    setShowJumpToLatest(bottomOffsetPx > JUMP_TO_LATEST_THRESHOLD_PX);
    onScrollRef.current?.({
      listOffset: -scroller.scrollTop,
      visibleListHeight: scroller.clientHeight,
      bottomOffset: bottomOffsetPx,
    });
    emitRenderedData(nextSnapshot);
  }, [emitRenderedData]);

  const applySnapshotToDom = useCallback(
    (
      nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>,
      options?: { behavior?: ScrollBehavior; followBottom?: boolean },
    ) => {
      const scroller = containerRef.current;
      if (!scroller) {
        setSnapshot(nextSnapshot);
        return;
      }
      const targetTop = Math.max(0, Math.min(nextSnapshot.scrollTop, Math.max(0, nextSnapshot.totalHeight - nextSnapshot.viewportHeight)));
      pendingProgrammaticTopRef.current = targetTop;
      pendingProgrammaticBehaviorRef.current = options?.behavior ?? "auto";
      if (options?.behavior && typeof scroller.scrollTo === "function") {
        scroller.scrollTo({ top: targetTop, behavior: options.behavior });
      } else {
        scroller.scrollTop = targetTop;
      }
      if (options?.followBottom != null) {
        followBottomRef.current = options.followBottom;
      }
      lastScrollTopRef.current = targetTop;
      commitRuntimeSnapshot(nextSnapshot);
      setSnapshot(nextSnapshot);
      emitScrollState(scroller, nextSnapshot);
      if (Math.abs(scroller.scrollTop - targetTop) <= 1) {
        pendingProgrammaticTopRef.current = null;
      }
    },
    [commitRuntimeSnapshot, emitScrollState],
  );

  const syncFromDom = useCallback(
    (scrollTopOverride?: number): PretextVirtualizerSnapshot<WorkbenchListItem> => {
      const scroller = containerRef.current;
      if (!scroller) {
      const nextSnapshot = core.getSnapshot();
      commitRuntimeSnapshot(nextSnapshot);
      setSnapshot(nextSnapshot);
      return nextSnapshot;
    }
      const nextSnapshot = core.syncViewport({
        height: scroller.clientHeight,
        width: scroller.clientWidth,
        scrollTop: scrollTopOverride ?? scroller.scrollTop,
      });
      lastScrollTopRef.current = scroller.scrollTop;
      commitRuntimeSnapshot(nextSnapshot);
      setSnapshot(nextSnapshot);
      emitScrollState(scroller, nextSnapshot);
      return nextSnapshot;
    },
    [commitRuntimeSnapshot, core, emitScrollState],
  );

  const restoreBottom = useCallback(
    (behavior: ScrollBehavior = "auto") => {
      const nextSnapshot = core.restoreAnchor({ kind: "bottom" });
      applySnapshotToDom(nextSnapshot, { behavior, followBottom: true });
    },
    [applySnapshotToDom, core],
  );

  const scrollToOffset = useCallback(
    (scrollTop: number, behavior: ScrollBehavior = "auto") => {
      const scroller = containerRef.current;
      if (!scroller) return;
      const nextTop = Math.max(0, scrollTop);
      pendingProgrammaticTopRef.current = Math.max(0, scrollTop);
      pendingProgrammaticBehaviorRef.current = behavior;
      if (typeof scroller.scrollTo === "function") {
        scroller.scrollTo({ top: nextTop, behavior });
      } else {
        scroller.scrollTop = nextTop;
      }
      if (behavior === "auto") {
        syncFromDom(nextTop);
        if (pendingProgrammaticTopRef.current != null && Math.abs(scroller.scrollTop - pendingProgrammaticTopRef.current) <= 1) {
          pendingProgrammaticTopRef.current = null;
        }
      }
    },
    [syncFromDom],
  );

  const scrollToItem = useCallback(
    (location: PretextVirtualizerItemLocation) => {
      const nextSnapshot = core.getSnapshot();
      const targetIndex = approximateIndexForLocation(listItemsRef.current, location);
      const resolvedLocation =
        location.index === "LAST" ? location : { ...location, index: targetIndex };
      const targetTop = resolveScrollTopForLocation(
        nextSnapshot,
        resolvedLocation,
        core,
        listItemsRef.current.length,
      );
      const followBottom = location.index === "LAST" && (location.align ?? "start") === "end";
      scrollToOffset(targetTop, location.behavior ?? "auto");
      if (followBottom) {
        followBottomRef.current = true;
      }
    },
    [core, scrollToOffset],
  );

  const syncItemsFromProjectionOp = useCallback(
    (
      items: readonly WorkbenchListItem[],
      projectionOp: WorkbenchThreadProjectionOp,
      anchorOverride?: PretextVirtualizerLogicalAnchor | null,
    ) => {
      const changedCount = projectionOp.changedItemIds.length;
      const previousCount = lastSyncedItemCountRef.current;
      let nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>;
      switch (projectionOp.kind) {
        case "replace_session":
          nextSnapshot = core.replaceItems(items, anchorOverride);
          break;
        case "append_stream":
          if (changedCount > 0 && items.length === previousCount + changedCount) {
            nextSnapshot = core.appendItems(items.slice(items.length - changedCount), anchorOverride);
            break;
          }
          nextSnapshot = core.syncItems(items, anchorOverride);
          break;
        case "prepend_history":
          if (changedCount > 0 && items.length === previousCount + changedCount) {
            nextSnapshot = core.prependItems(items.slice(0, changedCount), anchorOverride);
            break;
          }
          nextSnapshot = core.syncItems(items, anchorOverride);
          break;
        default:
          nextSnapshot = core.syncItems(items, anchorOverride);
          break;
      }
      lastSyncedItemCountRef.current = items.length;
      return nextSnapshot;
    },
    [core],
  );

  useLayoutEffect(() => {
    const scroller = containerRef.current;
    const pendingProgrammaticTop = pendingProgrammaticTopRef.current;
    if (!scroller || pendingProgrammaticTop == null) return;
    if (pendingProgrammaticBehaviorRef.current === "smooth") return;
    const maxScrollTop = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    const clampedTop = Math.max(0, Math.min(pendingProgrammaticTop, maxScrollTop));
    if (Math.abs(scroller.scrollTop - clampedTop) > 1) {
      scroller.scrollTop = clampedTop;
    }
    lastScrollTopRef.current = scroller.scrollTop;
    if (Math.abs(scroller.scrollTop - clampedTop) <= 1) {
      pendingProgrammaticTopRef.current = null;
      pendingProgrammaticBehaviorRef.current = "auto";
    }
  }, [snapshot.scrollTop, snapshot.totalHeight, snapshot.viewportHeight]);

  useLayoutEffect(() => {
    followBottomRef.current =
      initialLocation?.index === "LAST" && (initialLocation.align ?? "start") === "end";
    pendingProgrammaticTopRef.current = null;
    pendingProgrammaticBehaviorRef.current = "auto";
    pendingRestoreRef.current = false;
    lastAtBottomRef.current = null;
    lastSyncedItemCountRef.current = listItemsRef.current.length;
    setShowJumpToLatest(false);
    const scroller = containerRef.current;
    if (!scroller) {
      const nextSnapshot = core.getSnapshot();
      commitRuntimeSnapshot(nextSnapshot);
      setSnapshot(nextSnapshot);
      return;
    }
    const previousSnapshot = runtime.lastSnapshot;
    const revisitWithWarmRuntime = runtime.hasVisibleMount;
    const baseSnapshot = core.syncViewport({
      height: scroller.clientHeight,
      width: scroller.clientWidth,
      scrollTop: revisitWithWarmRuntime ? previousSnapshot.scrollTop : scroller.scrollTop,
    });
    commitRuntimeSnapshot(baseSnapshot);
    if (revisitWithWarmRuntime) {
      const followBottom = previousSnapshot.anchor.kind === "bottom";
      followBottomRef.current = followBottom;
      const widthChanged =
        previousSnapshot.viewportWidth !== baseSnapshot.viewportWidth ||
        previousSnapshot.widthBucket !== baseSnapshot.widthBucket;
      const restoredSnapshot = widthChanged
        ? core.restoreAnchor(previousSnapshot.anchor, previousSnapshot.anchor.kind === "item" ? "ratio" : "offset")
        : baseSnapshot;
      applySnapshotToDom(restoredSnapshot, { behavior: "auto", followBottom });
      markSessionPretextRuntimeVisible(runtime, true);
      return;
    }
    if (followBottomRef.current) {
      applySnapshotToDom(core.restoreAnchor({ kind: "bottom" }), { behavior: "auto", followBottom: true });
    } else {
      const targetIndex = approximateIndexForLocation(listItemsRef.current, initialLocation ?? PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION);
      const targetTop = resolveScrollTopForLocation(
        baseSnapshot,
        { ...(initialLocation ?? PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION), index: targetIndex },
        core,
        listItemsRef.current.length,
      );
      scrollToOffset(targetTop, initialLocation?.behavior ?? "auto");
    }
    markSessionPretextRuntimeVisible(runtime, true);
  }, [applySnapshotToDom, commitRuntimeSnapshot, core, initialLocation, runtime, scrollToOffset, sessionId]);

  useLayoutEffect(() => {
    const scroller = containerRef.current;
    if (!scroller) return;
    const canReuseWarmLayout =
      runtime.lastItems === listItems &&
      threadProjectionOp.kind === "noop";
    if (canReuseWarmLayout) {
      lastSyncedItemCountRef.current = listItems.length;
      const nextSnapshot = core.syncViewport({
        height: scroller.clientHeight,
        width: scroller.clientWidth,
        scrollTop: runtime.lastSnapshot.scrollTop,
      });
      commitRuntimeSnapshot(nextSnapshot);
      setSnapshot(nextSnapshot);
      emitScrollState(scroller, nextSnapshot);
      return;
    }
    pendingRestoreRef.current = true;
    core.syncViewport({
      height: scroller.clientHeight,
      width: scroller.clientWidth,
      scrollTop: scroller.scrollTop,
    });
    const bottomOffsetPx = Math.max(0, scroller.scrollHeight - (scroller.scrollTop + scroller.clientHeight));
    const shouldFollowBottom = followBottomRef.current && bottomOffsetPx <= BOTTOM_THRESHOLD_PX;
    followBottomRef.current = shouldFollowBottom;
    const anchorOverride: PretextVirtualizerLogicalAnchor = shouldFollowBottom ? { kind: "bottom" } : core.getAnchor();
    const nextSnapshot = syncItemsFromProjectionOp(listItems, threadProjectionOp, anchorOverride);
    applySnapshotToDom(nextSnapshot, {
      behavior: "auto",
      followBottom: shouldFollowBottom,
    });
    pendingRestoreRef.current = false;
  }, [applySnapshotToDom, commitRuntimeSnapshot, core, emitScrollState, listItems, runtime.lastItems, runtime.lastSnapshot.scrollTop, syncItemsFromProjectionOp, threadProjectionOp]);

  useEffect(() => {
    const scroller = containerRef.current;
    if (!scroller) return;
    let lastWidth = scroller.clientWidth;
    let lastHeight = scroller.clientHeight;
    const observer = new ResizeObserver(() => {
      const nextWidth = scroller.clientWidth;
      const nextHeight = scroller.clientHeight;
      const sizeChanged = nextWidth !== lastWidth || nextHeight !== lastHeight;
      lastWidth = nextWidth;
      lastHeight = nextHeight;
      const previousSnapshot = snapshotRef.current;
      void previousSnapshot;
      const shouldRestoreBottom = shouldRestoreBottomOnViewportResize(sizeChanged, followBottomRef.current);
      if (sizeChanged) {
        core.syncViewport({
          height: nextHeight,
          width: nextWidth,
          scrollTop: scroller.scrollTop,
        });
      }
      if (shouldRestoreBottom) {
        followBottomRef.current = true;
        applySnapshotToDom(core.restoreAnchor({ kind: "bottom" }), { behavior: "auto", followBottom: true });
        return;
      }
      syncFromDom();
    });
    observer.observe(scroller);
    return () => observer.disconnect();
  }, [applySnapshotToDom, core, syncFromDom]);

  const handleScroll = useCallback(() => {
    const scroller = containerRef.current;
    if (!scroller) return;
    const currentScrollTop = scroller.scrollTop;
    const previousScrollTop = lastScrollTopRef.current;
    const pendingProgrammaticTop = pendingProgrammaticTopRef.current;
    let programmaticScroll = false;
    if (pendingProgrammaticTop != null) {
      const deltaToPendingTop = Math.abs(currentScrollTop - pendingProgrammaticTop);
      if (deltaToPendingTop <= 1) {
        pendingProgrammaticTopRef.current = null;
        pendingProgrammaticBehaviorRef.current = "auto";
        programmaticScroll = true;
      } else if (pendingProgrammaticBehaviorRef.current === "smooth") {
        programmaticScroll = true;
      } else {
        pendingProgrammaticTopRef.current = null;
        pendingProgrammaticBehaviorRef.current = "auto";
      }
    }
    lastScrollTopRef.current = currentScrollTop;
    const nextSnapshot = core.syncViewport({
      height: scroller.clientHeight,
      width: scroller.clientWidth,
      scrollTop: currentScrollTop,
    });
    const bottomOffsetPx = Math.max(0, scroller.scrollHeight - (currentScrollTop + scroller.clientHeight));
    followBottomRef.current = resolveFollowBottomAfterScroll({
      followBottom: followBottomRef.current,
      previousScrollTop,
      currentScrollTop,
      bottomOffsetPx,
      thresholdPx: BOTTOM_THRESHOLD_PX,
      programmaticScroll,
    });
    commitRuntimeSnapshot(nextSnapshot);
    setSnapshot(nextSnapshot);
    emitScrollState(scroller, nextSnapshot);
  }, [commitRuntimeSnapshot, core, emitScrollState]);

  const handleWheel = useCallback((event: React.WheelEvent<HTMLDivElement>) => {
    if (event.deltaY < 0) {
      followBottomRef.current = false;
    }
  }, []);

  const pretextVirtualizerMethods = useMemo<PretextVirtualizerListMethods<WorkbenchListItem, WorkbenchMessageListContext>>(
    () => ({
      cancelSmoothScroll: () => {
        const scroller = containerRef.current;
        if (!scroller) return;
        scroller.scrollTo({ top: scroller.scrollTop, behavior: "auto" });
        pendingProgrammaticTopRef.current = null;
        pendingProgrammaticBehaviorRef.current = "auto";
      },
      scrollerElement: () => containerRef.current,
      restoreAnchor: (anchor) => {
        pendingRestoreRef.current = true;
        const nextSnapshot = core.restoreAnchor(anchor);
        applySnapshotToDom(nextSnapshot, {
          behavior: "auto",
          followBottom: anchor.kind === "bottom",
        });
        pendingRestoreRef.current = false;
      },
      scrollToBottom: (behavior = "auto") => {
        restoreBottom(behavior);
      },
      scrollToOffset: (scrollTop, behavior = "auto") => {
        scrollToOffset(scrollTop, behavior);
      },
      scrollToItem: (location) => {
        scrollToItem(location);
      },
    }),
    [applySnapshotToDom, core, restoreBottom, scrollToItem, scrollToOffset],
  );

  useLayoutEffect(() => {
    if (!methodsRef) return;
    methodsRef.current = pretextVirtualizerMethods;
    return () => {
      if (methodsRef.current === pretextVirtualizerMethods) {
        methodsRef.current = null;
      }
    };
  }, [methodsRef, pretextVirtualizerMethods]);

  const bottomOffsetPx = Math.max(0, snapshot.totalHeight - (snapshot.scrollTop + snapshot.viewportHeight));
  const shortThreadOffsetPx =
    shortSizeAlign === "bottom" && snapshot.totalHeight < snapshot.viewportHeight
      ? snapshot.viewportHeight - snapshot.totalHeight
      : 0;
  const innerHeight = Math.max(snapshot.totalHeight + shortThreadOffsetPx, snapshot.viewportHeight);
  const renderedItems = snapshot.visibleItems;

  return (
    <div
      className="wb-pretext-transcript-shell wb-thread-stack wb-thread-scroller--message-list"
      style={{ position: "relative", minWidth: 0 }}
    >
      <div
        ref={containerRef}
        style={{
          ...style,
          position: "absolute",
          inset: 0,
          width: "auto",
          minWidth: 0,
        }}
        className="wb-thread-scroller"
        role="list"
        onScroll={handleScroll}
        onWheel={handleWheel}
        data-pretext-virtualizer-list="1"
        data-pretext-virtualizer-snapshot-scroll-top={String(Math.round(snapshot.scrollTop))}
        data-pretext-virtualizer-snapshot-first-index={String(snapshot.visibleItems[0]?.index ?? -1)}
        data-pretext-virtualizer-snapshot-last-index={String(snapshot.visibleItems.at(-1)?.index ?? -1)}
        data-pretext-virtualizer-rendered-first-index={String(renderedItems[0]?.index ?? -1)}
        data-pretext-virtualizer-rendered-last-index={String(renderedItems.at(-1)?.index ?? -1)}
        data-pretext-virtualizer-programmatic-pending={pendingProgrammaticTopRef.current != null ? "1" : "0"}
        data-pretext-virtualizer-pending-restore={pendingRestoreRef.current ? "1" : "0"}
      >
        <div
          className="wb-thread-list"
          data-pretext-virtualizer-content="1"
          style={{ position: "relative", height: `${innerHeight}px` }}
        >
          {renderedItems.map((visibleItem) => (
            <div
              key={itemKey(visibleItem.item)}
              data-pretext-virtualizer-row-shell="1"
              style={{
                position: "absolute",
                top: `${visibleItem.top + shortThreadOffsetPx}px`,
                left: 0,
                right: 0,
                width: "100%",
              }}
            >
              <div
                className="wb-pretext-virtualizer-row"
                data-pretext-virtualizer-row="1"
                data-pretext-virtualizer-item-id={visibleItem.id}
                data-pretext-virtualizer-planned-height={String(visibleItem.height)}
              >
                <AuditedPretextRow
                  id={visibleItem.id}
                  itemKind={visibleItem.item.kind}
                  itemKey={itemKey(visibleItem.item)}
                  plannedHeight={visibleItem.height}
                >
                  {itemContent(visibleItem.index, visibleItem.item)}
                </AuditedPretextRow>
              </div>
            </div>
          ))}
        </div>
      </div>
      {showJumpToLatest && bottomOffsetPx > JUMP_TO_LATEST_THRESHOLD_PX ? (
        <div
          style={{
            position: "absolute",
            left: "50%",
            bottom: 12,
            transform: "translateX(-50%)",
            pointerEvents: "none",
          }}
        >
          <button
            type="button"
            className="new-activity-overlay"
            aria-label="Jump to latest"
            title="Jump to latest"
            onClick={() => restoreBottom("auto")}
            style={{ pointerEvents: "auto" }}
          >
            ↓
          </button>
        </div>
      ) : null}
    </div>
  );
});
