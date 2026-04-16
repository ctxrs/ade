import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  type PretextVirtualizerLogicalAnchor,
  type PretextVirtualizerSnapshot,
} from "@pretext-virtualizer/core";
import type {
  PretextVirtualizerItemLocation,
} from "@pretext-virtualizer/interface";
import { PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION } from "../state/pretextVirtualizerViewportState";
import {
  addPretextPerfBucket,
  hashPretextPerfValue,
  incrementPretextPerfCounter,
} from "../utils/pretextPerfDiagnostics";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";
import {
  getWorkbenchMessageListLayoutRevision,
  type WorkbenchMessageListUiState,
} from "./sessionMessageListItemIdentity";
import type { WorkbenchThreadProjectionOp } from "./sessionThreadProjection";
import {
  buildSessionPretextRuntimeLayoutKey,
  getOrCreateSessionPretextRuntime,
  noteSessionPretextRuntimeSnapshot,
  readSessionPretextRuntimePreparedState,
  SESSION_PRETEXT_BOTTOM_THRESHOLD_PX,
} from "./sessionThread/pretextSessionRuntimeCache";
import {
  computeBottomOffsetPx,
  resolveFollowBottomAfterScroll,
  shouldFollowBottomOnItemsUpdate,
  shouldRestoreBottomOnViewportResize,
} from "./sessionThread/pretextFollowBottom";
import { createSessionThreadPretextVirtualizerMethods } from "./sessionThread/pretextVirtualizerMethods";
import {
  approximateIndexForLocation,
  AuditedPretextRow,
  createVisibleItemAnchor,
  haveSameItemIds,
  haveSameLayoutInputs,
  isLocalizedProjectionOp,
  resolveInteractionItemId,
  resolveLocalizedAnchorOverride,
  resolveScrollTopForLocation,
  syncSnapshotForProjectionOp,
} from "./sessionThread/pretextVirtualizerListInternals";
import {
  clearSessionThreadDomMeasurementCaches,
  consumeSessionThreadDomMeasurementFallbackItemIds,
} from "./sessionThread/sessionThreadDomMeasurement";
import {
  noteSessionTranscriptWarmViewport,
} from "./sessionThread/sessionTranscriptWarmState";
import { usePretextTranscriptScrollbar } from "./sessionThread/usePretextTranscriptScrollbar";
import type { SessionThreadPretextVirtualizerListProps } from "./SessionThreadMessageList.pretextVirtualizer.types";
import {
  createInitialSessionThreadPretextSnapshot,
  isBottomOpenLocation,
  isSnapshotReadyForDisplay,
} from "./sessionThread/pretextVirtualizerDisplayState";
import {
  commitSessionThreadRuntimeSnapshot,
  getRenderedItemsFromSnapshot,
} from "./sessionThread/pretextVirtualizerRuntimeState";

const BOTTOM_THRESHOLD_PX = SESSION_PRETEXT_BOTTOM_THRESHOLD_PX;
const JUMP_TO_LATEST_THRESHOLD_PX = 200;

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
  const runtimeUiStateLayoutKeyRef = useRef<string | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const followBottomRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  const lastSyncedItemCountRef = useRef(listItems.length);
  const lastAtBottomRef = useRef<boolean | null>(null);
  const snapshotRef = useRef<PretextVirtualizerSnapshot<WorkbenchListItem> | null>(null);
  const lastAppliedUiStateLayoutRevisionRef = useRef<string | null>(null);
  const lastAppliedProjectionOpRef = useRef<string | null>(null);
  const lastInteractedItemIdRef = useRef<string | null>(null);
  const pendingProgrammaticTopRef = useRef<number | null>(null);
  const pendingProgrammaticBehaviorRef = useRef<ScrollBehavior>("auto");
  const pendingRestoreRef = useRef(false);
  const pendingMismatchRemeasureIdsRef = useRef<Set<string>>(new Set());
  const mismatchRemeasureFrameRef = useRef<number | null>(null);
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
  const runtimeUiStateLayoutRevision = useMemo(
    () => getWorkbenchMessageListLayoutRevision(runtimeUiState),
    [runtimeUiState],
  );
  const runtimeUiStateLayoutKey = useMemo(
    () => buildSessionPretextRuntimeLayoutKey({ uiState: runtimeUiState }),
    [runtimeUiState],
  );
  runtimeUiStateLayoutKeyRef.current = runtimeUiStateLayoutKey;
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
  if (lastAppliedUiStateLayoutRevisionRef.current == null) {
    lastAppliedUiStateLayoutRevisionRef.current = runtimeUiStateLayoutRevision;
  }

  const [snapshot, setSnapshot] = useState<PretextVirtualizerSnapshot<WorkbenchListItem>>(() =>
    createInitialSessionThreadPretextSnapshot({
      runtime,
      sessionId,
      listItems,
      uiState: runtimeUiState,
      layoutKey: runtimeUiStateLayoutKey,
    }),
  );
  const requireBottomAlignmentForDisplay = isBottomOpenLocation(initialLocation);
  const [surfaceReady, setSurfaceReady] = useState(() =>
    isSnapshotReadyForDisplay(
      snapshot,
      listItems.length,
      requireBottomAlignmentForDisplay,
      BOTTOM_THRESHOLD_PX,
    ),
  );
  snapshotRef.current = snapshot;
  const {
    scrollbarActive,
    scrollbarDragging,
    scrollbarNeeded,
    scrollbarThumbRef,
    scrollbarTrackRef,
    handleScrollbarMouseLeave,
    handleScrollbarThumbPointerDown,
    handleScrollbarThumbPointerMove,
    handleScrollbarThumbPointerUp,
    handleScrollbarTrackPointerDown,
    scheduleScrollbarUpdate,
    showScrollbarTemporarily,
  } = usePretextTranscriptScrollbar({
    containerRef,
    followBottomRef,
  });

  const emitRenderedData = useCallback((nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>) => {
    onRenderedDataChangeRef.current?.(getRenderedItemsFromSnapshot(nextSnapshot, listItemsRef.current));
  }, []);

  const commitRuntimeSnapshot = useCallback(
    (nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>, nextItems: readonly WorkbenchListItem[]) => {
      commitSessionThreadRuntimeSnapshot(runtime, nextSnapshot, nextItems);
    },
    [runtime],
  );

  const emitScrollState = useCallback((nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>) => {
    const bottomOffsetPx = computeBottomOffsetPx({
      totalHeight: nextSnapshot.totalHeight,
      scrollTop: nextSnapshot.scrollTop,
      viewportHeight: nextSnapshot.viewportHeight,
    });
    const atBottom = bottomOffsetPx <= BOTTOM_THRESHOLD_PX;
    if (lastAtBottomRef.current !== atBottom) {
      lastAtBottomRef.current = atBottom;
      onAtBottomChangeRef.current?.(atBottom);
    }
    setShowJumpToLatest(bottomOffsetPx > JUMP_TO_LATEST_THRESHOLD_PX);
    onScrollRef.current?.({
      listOffset: -nextSnapshot.scrollTop,
      visibleListHeight: nextSnapshot.viewportHeight,
      bottomOffset: bottomOffsetPx,
    });
    emitRenderedData(nextSnapshot);
    scheduleScrollbarUpdate();
  }, [emitRenderedData, scheduleScrollbarUpdate]);

  const applySnapshotToDom = useCallback(
    (
      nextSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>,
      options?: {
        behavior?: ScrollBehavior;
        followBottom?: boolean;
        nextItems?: readonly WorkbenchListItem[];
      },
    ) => {
      const nextItems = options?.nextItems ?? readSessionPretextRuntimePreparedState(runtime).listItems;
      const scroller = containerRef.current;
      if (!scroller) {
        setSurfaceReady(
          isSnapshotReadyForDisplay(
            nextSnapshot,
            nextItems.length,
            options?.followBottom ?? followBottomRef.current,
            BOTTOM_THRESHOLD_PX,
          ),
        );
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
      commitRuntimeSnapshot(nextSnapshot, nextItems);
      setSurfaceReady(
        isSnapshotReadyForDisplay(
          nextSnapshot,
          nextItems.length,
          options?.followBottom ?? followBottomRef.current,
          BOTTOM_THRESHOLD_PX,
        ),
      );
      setSnapshot(nextSnapshot);
      emitScrollState(nextSnapshot);
      if (Math.abs(scroller.scrollTop - targetTop) <= 1) {
        pendingProgrammaticTopRef.current = null;
      }
    },
    [commitRuntimeSnapshot, emitScrollState, runtime],
  );

  const flushPendingMismatchRemeasure = useCallback(() => {
    mismatchRemeasureFrameRef.current = null;
    const scroller = containerRef.current;
    if (!scroller || scroller.clientWidth <= 0 || scroller.clientHeight <= 0) {
      return;
    }
    const pendingIds = [...pendingMismatchRemeasureIdsRef.current];
    pendingMismatchRemeasureIdsRef.current.clear();
    if (pendingIds.length === 0) {
      return;
    }
    clearSessionThreadDomMeasurementCaches();
    const currentSnapshot = core.syncViewport({
      height: scroller.clientHeight,
      width: scroller.clientWidth,
      scrollTop: scroller.scrollTop,
    });
    const anchorOverride: PretextVirtualizerLogicalAnchor = followBottomRef.current
      ? { kind: "bottom" }
      : currentSnapshot.visibleItems.length > 0
        ? createVisibleItemAnchor(currentSnapshot.visibleItems[0], currentSnapshot.scrollTop)
        : core.getAnchor("detached");
    const nextSnapshot = core.patchItems(
      listItemsRef.current,
      pendingIds,
      pendingIds,
      anchorOverride,
    );
    applySnapshotToDom(nextSnapshot, {
      behavior: "auto",
      followBottom: followBottomRef.current,
      nextItems: listItemsRef.current,
    });
  }, [applySnapshotToDom, core]);

  const noteHeightMismatch = useCallback((itemId: string) => {
    if (!itemId) return;
    pendingMismatchRemeasureIdsRef.current.add(itemId);
    if (mismatchRemeasureFrameRef.current != null) {
      return;
    }
    mismatchRemeasureFrameRef.current = requestAnimationFrame(() => {
      flushPendingMismatchRemeasure();
    });
  }, [flushPendingMismatchRemeasure]);

  const syncFromDom = useCallback(
    (scrollTopOverride?: number): PretextVirtualizerSnapshot<WorkbenchListItem> => {
      const scroller = containerRef.current;
      if (!scroller) {
        const nextSnapshot = core.getSnapshot();
        commitRuntimeSnapshot(nextSnapshot, listItemsRef.current);
        setSurfaceReady(
          isSnapshotReadyForDisplay(
            nextSnapshot,
            listItemsRef.current.length,
            followBottomRef.current,
            BOTTOM_THRESHOLD_PX,
          ),
        );
        setSnapshot(nextSnapshot);
        return nextSnapshot;
      }
      const nextSnapshot = core.syncViewport({
        height: scroller.clientHeight,
        width: scroller.clientWidth,
        scrollTop: scrollTopOverride ?? scroller.scrollTop,
      });
      lastScrollTopRef.current = scroller.scrollTop;
      commitRuntimeSnapshot(nextSnapshot, listItemsRef.current);
      setSurfaceReady(
        isSnapshotReadyForDisplay(
          nextSnapshot,
          listItemsRef.current.length,
          followBottomRef.current,
          BOTTOM_THRESHOLD_PX,
        ),
      );
      setSnapshot(nextSnapshot);
      emitScrollState(nextSnapshot);
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

  useLayoutEffect(() => {
    const scroller = containerRef.current;
    const pendingProgrammaticTop = pendingProgrammaticTopRef.current;
    if (!scroller || pendingProgrammaticTop == null) return;
    if (pendingProgrammaticBehaviorRef.current === "smooth") return;
    const maxScrollTop = Math.max(0, snapshot.totalHeight - snapshot.viewportHeight);
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
    setSurfaceReady(false);
    pendingProgrammaticTopRef.current = null;
    pendingProgrammaticBehaviorRef.current = "auto";
    pendingRestoreRef.current = false;
    lastAtBottomRef.current = null;
    lastSyncedItemCountRef.current = listItemsRef.current.length;
    lastAppliedUiStateLayoutRevisionRef.current = runtimeUiStateLayoutRevision;
    lastAppliedProjectionOpRef.current = null;
    setShowJumpToLatest(false);
    const currentItems = listItemsRef.current;
    const scroller = containerRef.current;
    const currentItems = listItemsRef.current;
    if (!scroller) {
      const nextSnapshot = core.getSnapshot();
      commitRuntimeSnapshot(nextSnapshot, currentItems);
      setSnapshot(nextSnapshot);
      return;
    }
    const preparedState = readSessionPretextRuntimePreparedState(runtime);
    const preparedLayoutMismatch =
      preparedState.layoutKey == null || preparedState.layoutKey !== runtimeUiStateLayoutKeyRef.current;
    const preparedWidthMismatch =
      scroller.clientWidth > 0 && preparedState.snapshot.viewportWidth !== scroller.clientWidth;
    let baseSnapshot = core.syncViewport({
      height: scroller.clientHeight,
      width: scroller.clientWidth,
      scrollTop: scroller.scrollTop,
    });
    lastSyncedItemCountRef.current = preparedState.listItems.length;
    if (
      preparedLayoutMismatch ||
      preparedWidthMismatch ||
      !haveSameLayoutInputs(preparedState.listItems, currentItems, runtime.callbacks.getLayoutRevision)
    ) {
      incrementPretextPerfCounter("pretext_full_relayout_calls");
      incrementPretextPerfCounter("pretext_full_relayout_item_count", currentItems.length);
      addPretextPerfBucket(
        "pretext_full_relayout_reason",
        preparedLayoutMismatch
          ? "visible:mount-layout-key-mismatch"
          : preparedWidthMismatch
            ? "visible:mount-width-mismatch"
            : "visible:mount-sync-items",
      );
      const initialAnchor = followBottomRef.current
        ? ({ kind: "bottom" } satisfies PretextVirtualizerLogicalAnchor)
        : null;
      baseSnapshot = haveSameItemIds(preparedState.listItems, currentItems)
        ? core.patchItems(
            currentItems,
            currentItems.map((item) => item.id),
            currentItems.map((item) => item.id),
            initialAnchor,
          )
        : core.syncItems(currentItems, initialAnchor);
      lastSyncedItemCountRef.current = currentItems.length;
    }
    commitRuntimeSnapshot(baseSnapshot, currentItems);
    if (followBottomRef.current) {
      applySnapshotToDom(core.restoreAnchor({ kind: "bottom" }), {
        behavior: "auto",
        followBottom: true,
        nextItems: currentItems,
      });
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
  }, [
    applySnapshotToDom,
    commitRuntimeSnapshot,
    core,
    initialLocation,
    runtime,
    scrollToOffset,
    sessionId,
  ]);

  useLayoutEffect(() => {
    const scroller = containerRef.current;
    if (!scroller) return;
    const uiStateChanged =
      lastAppliedUiStateLayoutRevisionRef.current !== runtimeUiStateLayoutRevision;
    const preparedState = readSessionPretextRuntimePreparedState(runtime);
    const itemsChanged = !haveSameLayoutInputs(
      preparedState.listItems,
      listItems,
      runtime.callbacks.getLayoutRevision,
    );
    const projectionOpKey =
      threadProjectionOp.kind === "noop"
        ? null
        : [
            threadProjectionOp.projectionRevision,
            threadProjectionOp.kind,
            threadProjectionOp.changedItemIds.join(","),
            threadProjectionOp.remeasureItemIds.join(","),
          ].join("|");
    const projectionChanged =
      projectionOpKey != null && lastAppliedProjectionOpRef.current !== projectionOpKey;
    if (!itemsChanged && !projectionChanged && !uiStateChanged) {
      return;
    }
    const shouldCountFullRelayout =
      uiStateChanged ||
      !projectionChanged ||
      !isLocalizedProjectionOp(threadProjectionOp.kind);
    if (shouldCountFullRelayout) {
      incrementPretextPerfCounter("pretext_full_relayout_calls");
      incrementPretextPerfCounter("pretext_full_relayout_item_count", listItems.length);
      addPretextPerfBucket(
        "pretext_full_relayout_reason",
        projectionChanged ? `visible:${threadProjectionOp.kind}` : uiStateChanged ? "visible:ui-state" : "visible:items",
      );
    } else {
      incrementPretextPerfCounter("pretext_localized_patch_calls");
      incrementPretextPerfCounter("pretext_localized_patch_item_count", threadProjectionOp.remeasureItemIds.length);
      addPretextPerfBucket("pretext_localized_patch_kind", threadProjectionOp.kind);
    }
    if (uiStateChanged) {
      addPretextPerfBucket(
        "pretext_ui_state_revision",
        hashPretextPerfValue(runtimeUiStateLayoutRevision),
      );
    }
    pendingRestoreRef.current = true;
    const currentSnapshot = core.syncViewport({
      height: scroller.clientHeight,
      width: scroller.clientWidth,
      scrollTop: scroller.scrollTop,
    });
    const bottomOffsetPx = computeBottomOffsetPx({
      totalHeight: currentSnapshot.totalHeight,
      scrollTop: currentSnapshot.scrollTop,
      viewportHeight: currentSnapshot.viewportHeight,
    });
    const shouldFollowBottom = shouldFollowBottomOnItemsUpdate(
      {
        followBottom: followBottomRef.current,
        atBottom: lastAtBottomRef.current === true,
      },
      bottomOffsetPx,
      BOTTOM_THRESHOLD_PX,
    );
    followBottomRef.current = shouldFollowBottom;
    const activeChangedItemId = (() => {
      const interactedItemId = lastInteractedItemIdRef.current;
      if (interactedItemId && threadProjectionOp.changedItemIds.includes(interactedItemId)) {
        return interactedItemId;
      }
      if (typeof document === "undefined") return null;
      const activeElement = document.activeElement;
      if (!(activeElement instanceof HTMLElement)) return null;
      const owner = activeElement.closest<HTMLElement>("[data-thread-item-id]");
      const ownerId = owner?.dataset.threadItemId ?? null;
      if (!ownerId) return null;
      return threadProjectionOp.changedItemIds.includes(ownerId) ? ownerId : null;
    })();
    const defaultAnchorOverride: PretextVirtualizerLogicalAnchor = shouldFollowBottom
      ? { kind: "bottom" }
      : core.getAnchor("detached");
    const anchorOverride = shouldFollowBottom
      ? defaultAnchorOverride
      : resolveLocalizedAnchorOverride(
          currentSnapshot,
          threadProjectionOp,
          activeChangedItemId,
          defaultAnchorOverride,
        );
    incrementPretextPerfCounter("pretext_visible_sync_items_calls");
    addPretextPerfBucket("pretext_visible_sync_items_kind", threadProjectionOp.kind);
    const nextSnapshot = syncSnapshotForProjectionOp({
      core,
      items: listItems,
      projectionOp: threadProjectionOp,
      previousCount: lastSyncedItemCountRef.current,
      anchorOverride,
    });
    lastSyncedItemCountRef.current = listItems.length;
    applySnapshotToDom(nextSnapshot, {
      behavior: "auto",
      followBottom: shouldFollowBottom,
      nextItems: listItems,
    });
    lastInteractedItemIdRef.current = null;
    pendingRestoreRef.current = false;
    lastAppliedUiStateLayoutRevisionRef.current = runtimeUiStateLayoutRevision;
    lastAppliedProjectionOpRef.current = projectionOpKey;
  }, [
    applySnapshotToDom,
    core,
    listItems,
    runtime,
    runtimeUiStateLayoutRevision,
    threadProjectionOp,
  ]);

  useEffect(() => {
    const scroller = containerRef.current;
    if (!scroller) return;
    let lastWidth = scroller.clientWidth;
    let lastHeight = scroller.clientHeight;
    let resizeFrameId: number | null = null;
    const processResize = () => {
      resizeFrameId = null;
      const nextWidth = scroller.clientWidth;
      const nextHeight = scroller.clientHeight;
      const sizeChanged = nextWidth !== lastWidth || nextHeight !== lastHeight;
      if (!sizeChanged) return;
      lastWidth = nextWidth;
      lastHeight = nextHeight;
      scheduleScrollbarUpdate();
      const previousSnapshot = snapshotRef.current;
      if (!previousSnapshot) return;
      const shouldRestoreBottom = shouldRestoreBottomOnViewportResize(
        sizeChanged,
        {
          followBottom: followBottomRef.current,
          atBottom: lastAtBottomRef.current === true,
        },
      );
      if (sizeChanged) {
        if (nextWidth !== previousSnapshot.viewportWidth) {
          incrementPretextPerfCounter("pretext_full_relayout_calls");
          incrementPretextPerfCounter("pretext_full_relayout_item_count", listItemsRef.current.length);
          addPretextPerfBucket("pretext_full_relayout_reason", "visible:resize-width");
        }
        core.syncViewport({
          height: nextHeight,
          width: nextWidth,
          scrollTop: scroller.scrollTop,
        });
        if (nextWidth !== previousSnapshot.viewportWidth) {
          core.syncItems(
            listItemsRef.current,
            shouldRestoreBottom
              ? { kind: "bottom" }
              : previousSnapshot.anchor.kind === "item"
                ? previousSnapshot.anchor
                : null,
          );
        }
        if (!shouldRestoreBottom && previousSnapshot.anchor.kind === "item") {
          followBottomRef.current = false;
          applySnapshotToDom(
            core.restoreAnchor(previousSnapshot.anchor, "ratio"),
            { behavior: "auto", followBottom: false },
          );
          return;
        }
      }
      if (shouldRestoreBottom) {
        followBottomRef.current = true;
        applySnapshotToDom(core.restoreAnchor({ kind: "bottom" }), { behavior: "auto", followBottom: true });
        return;
      }
      syncFromDom();
    };
    const observer = new ResizeObserver(() => {
      if (resizeFrameId != null) {
        cancelAnimationFrame(resizeFrameId);
      }
      resizeFrameId = requestAnimationFrame(processResize);
    });
    observer.observe(scroller);
    return () => {
      observer.disconnect();
      if (resizeFrameId != null) {
        cancelAnimationFrame(resizeFrameId);
      }
    };
  }, [applySnapshotToDom, core, scheduleScrollbarUpdate, syncFromDom]);

  useLayoutEffect(() => {
    const scroller = containerRef.current;
    if (!scroller || scroller.clientWidth <= 0 || scroller.clientHeight <= 0) {
      return;
    }
    const pendingFallbackItemIds = consumeSessionThreadDomMeasurementFallbackItemIds(
      listItems.map((item) => item.id),
    );
    if (pendingFallbackItemIds.length === 0) {
      return;
    }
    const currentSnapshot = core.syncViewport({
      height: scroller.clientHeight,
      width: scroller.clientWidth,
      scrollTop: scroller.scrollTop,
    });
    const anchorOverride: PretextVirtualizerLogicalAnchor = followBottomRef.current
      ? { kind: "bottom" }
      : currentSnapshot.visibleItems.length > 0
        ? createVisibleItemAnchor(currentSnapshot.visibleItems[0], currentSnapshot.scrollTop)
        : core.getAnchor("detached");
    const nextSnapshot = core.patchItems(
      listItems,
      pendingFallbackItemIds,
      pendingFallbackItemIds,
      anchorOverride,
    );
    applySnapshotToDom(nextSnapshot, {
      behavior: "auto",
      followBottom: followBottomRef.current,
      nextItems: listItems,
    });
  }, [applySnapshotToDom, core, listItems]);

  useEffect(() => {
    return () => {
      if (mismatchRemeasureFrameRef.current != null) {
        cancelAnimationFrame(mismatchRemeasureFrameRef.current);
      }
      pendingMismatchRemeasureIdsRef.current.clear();
    };
  }, []);

  const handleScroll = useCallback(() => {
    const scroller = containerRef.current;
    if (!scroller) return;
    incrementPretextPerfCounter("pretext_visible_scroll_events");
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
    if (Math.abs(currentScrollTop - previousScrollTop) > 0.5 && !programmaticScroll) {
      showScrollbarTemporarily();
    }
    lastScrollTopRef.current = currentScrollTop;
    const nextSnapshot = core.syncViewport({
      height: scroller.clientHeight,
      width: scroller.clientWidth,
      scrollTop: currentScrollTop,
    });
    const bottomOffsetPx = computeBottomOffsetPx({
      totalHeight: nextSnapshot.totalHeight,
      scrollTop: nextSnapshot.scrollTop,
      viewportHeight: nextSnapshot.viewportHeight,
    });
    followBottomRef.current = resolveFollowBottomAfterScroll({
      followBottom: followBottomRef.current,
      previousScrollTop,
      currentScrollTop,
      bottomOffsetPx,
      thresholdPx: BOTTOM_THRESHOLD_PX,
      programmaticScroll,
    });
    commitRuntimeSnapshot(nextSnapshot, listItemsRef.current);
    setSnapshot(nextSnapshot);
    emitScrollState(nextSnapshot);
  }, [commitRuntimeSnapshot, core, emitScrollState, showScrollbarTemporarily]);

  const handleWheel = useCallback((event: React.WheelEvent<HTMLDivElement>) => {
    if (event.deltaY < 0) {
      followBottomRef.current = false;
    }
    showScrollbarTemporarily();
  }, [showScrollbarTemporarily]);

  const handleClickCapture = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    lastInteractedItemIdRef.current = resolveInteractionItemId(event.target);
  }, []);

  const handleKeyDownCapture = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    lastInteractedItemIdRef.current = resolveInteractionItemId(event.target);
  }, []);

  const pretextVirtualizerMethods = useMemo(
    () =>
      createSessionThreadPretextVirtualizerMethods({
        applySnapshotToDom,
        containerRef,
        pendingProgrammaticBehaviorRef,
        pendingProgrammaticTopRef,
        restoreAnchorSnapshot: (anchor) => {
          pendingRestoreRef.current = true;
          const nextSnapshot = core.restoreAnchor(anchor);
          pendingRestoreRef.current = false;
          return nextSnapshot;
        },
        restoreBottom,
        scrollToItemFn: scrollToItem,
        scrollToOffsetFn: scrollToOffset,
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

  useLayoutEffect(() => {
    scheduleScrollbarUpdate();
  }, [scheduleScrollbarUpdate, snapshot.scrollTop, snapshot.totalHeight, snapshot.viewportHeight]);

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
      onMouseLeave={handleScrollbarMouseLeave}
    >
      <div
        ref={containerRef}
        style={{
          ...style,
          position: "absolute",
          inset: 0,
          width: "auto",
          minWidth: 0,
          visibility: surfaceReady ? "visible" : "hidden",
        }}
        className="wb-thread-scroller"
        role="list"
        onClickCapture={handleClickCapture}
        onKeyDownCapture={handleKeyDownCapture}
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
          {renderedItems.map((visibleItem) => {
            const latestItem = listItemsRef.current[visibleItem.index];
            const currentItem = latestItem?.id === visibleItem.id ? latestItem : visibleItem.item;
            return (
            <div
              key={itemKey(currentItem)}
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
                data-pretext-virtualizer-item-id={currentItem.id}
                data-pretext-virtualizer-planned-height={String(visibleItem.height)}
              >
                <AuditedPretextRow
                  id={visibleItem.id}
                  itemKind={currentItem.kind}
                  itemKey={itemKey(currentItem)}
                  plannedHeight={visibleItem.height}
                  onHeightMismatch={noteHeightMismatch}
                >
                  {itemContent(visibleItem.index, currentItem)}
                </AuditedPretextRow>
              </div>
            </div>
          );})}
        </div>
      </div>
      <div
        className={`wb-scrollbar${scrollbarActive ? " is-active" : ""}${scrollbarDragging ? " is-dragging" : ""}${scrollbarNeeded ? "" : " is-hidden"}`}
        aria-hidden="true"
      >
        <div
          className="wb-scrollbar-track"
          ref={(node) => {
            scrollbarTrackRef.current = node;
            if (node) {
              scheduleScrollbarUpdate();
            }
          }}
          onPointerDown={handleScrollbarTrackPointerDown}
        >
          <div
            className="wb-scrollbar-thumb"
            ref={(node) => {
              scrollbarThumbRef.current = node;
              if (node) {
                scheduleScrollbarUpdate();
              }
            }}
            onPointerDown={handleScrollbarThumbPointerDown}
            onPointerMove={handleScrollbarThumbPointerMove}
            onPointerUp={handleScrollbarThumbPointerUp}
            onPointerCancel={handleScrollbarThumbPointerUp}
          />
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
