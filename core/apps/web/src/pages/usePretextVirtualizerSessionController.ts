import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import type {
  PretextVirtualizerItemLocation,
  PretextVirtualizerListMethods,
  PretextVirtualizerScrollLocation,
} from "@pretext-virtualizer/interface";
import { PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION } from "../state/pretextVirtualizerViewportState";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";
import { computeHistoryPrefetchThresholdPx } from "./sessionMessageListControllerUtils";
import { useSessionMessageListDiagnostics } from "./useSessionMessageListDiagnostics";

type Params = {
  sessionId: string;
  isActive: boolean;
  loaded: boolean;
  listItems: WorkbenchListItem[];
  canLoadOlder: boolean;
  loadOlder: () => Promise<void>;
  showDebug: boolean;
  onAtBottomChange?: (atBottom: boolean) => void;
  onInitialContentRendered?: () => void;
};

type Result = {
  methodsRef: MutableRefObject<PretextVirtualizerListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  initialData: WorkbenchListItem[];
  context: WorkbenchMessageListContext;
  initialLocation: PretextVirtualizerItemLocation;
  onScroll: (location: PretextVirtualizerScrollLocation) => void;
  onRenderedDataChange: (range: readonly WorkbenchListItem[]) => void;
};

export function usePretextVirtualizerSessionController(params: Params): Result {
  const {
    sessionId,
    isActive,
    loaded,
    listItems,
    canLoadOlder,
    loadOlder,
    showDebug,
    onAtBottomChange,
    onInitialContentRendered,
  } = params;

  const methodsRef = useRef<PretextVirtualizerListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>(null);
  const lastSessionIdRef = useRef(sessionId);
  const lastIsActiveRef = useRef(isActive);
  const lastAtBottomRef = useRef<boolean | null>(null);
  const lastListOffsetRef = useRef<number | null>(null);
  const pendingHistoryRef = useRef(false);
  const continueHistoryAtTopRef = useRef(false);
  const initialContentRenderedSessionIdRef = useRef<string | null>(null);
  const renderedAnchorIdRef = useRef<string | null>(null);
  const renderedTopIdRef = useRef<string | null>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);

  const context = useMemo(() => ({ loaded, loadingOlder }), [loaded, loadingOlder]);
  const { recordDebugSnapshot } = useSessionMessageListDiagnostics({
    sessionId,
    isActive,
    loaded,
    listItemsLength: listItems.length,
    showDebug,
    methodsRef,
    lastAtBottomRef,
    renderedAnchorIdRef,
    renderedTopIdRef,
  });

  const requestOlderHistory = useCallback(
    (reason: "scroll" | "top-pinned-continue", detail: Record<string, number | boolean | null>) => {
      if (!canLoadOlder || pendingHistoryRef.current) {
        continueHistoryAtTopRef.current = false;
        return false;
      }

      continueHistoryAtTopRef.current = true;
      pendingHistoryRef.current = true;
      setLoadingOlder(true);
      recordDebugSnapshot(
        reason === "scroll" ? "history:request" : "history:continue",
        detail,
      );
      loadOlder()
        .catch(() => {})
        .finally(() => {
          pendingHistoryRef.current = false;
          setLoadingOlder(false);
        });
      return true;
    },
    [canLoadOlder, loadOlder, recordDebugSnapshot],
  );

  useLayoutEffect(() => {
    if (lastSessionIdRef.current === sessionId) return;
    lastSessionIdRef.current = sessionId;
    pendingHistoryRef.current = false;
    continueHistoryAtTopRef.current = false;
    lastListOffsetRef.current = null;
    setLoadingOlder(false);
    lastAtBottomRef.current = true;
    renderedAnchorIdRef.current = null;
    renderedTopIdRef.current = null;
    lastIsActiveRef.current = isActive;
    initialContentRenderedSessionIdRef.current = null;
    onAtBottomChange?.(true);
    recordDebugSnapshot("session:changed", { reason: "openAtBottom" });
  }, [isActive, listItems, onAtBottomChange, recordDebugSnapshot, sessionId]);

  useLayoutEffect(() => {
    const wasActive = lastIsActiveRef.current;
    const becameActive = isActive && !wasActive;
    lastIsActiveRef.current = isActive;
    if (!becameActive) return;
    methodsRef.current?.scrollToBottom("auto");
    lastAtBottomRef.current = true;
    onAtBottomChange?.(true);
    recordDebugSnapshot("session:activated", { reason: "openAtBottom", atBottom: true });
  }, [isActive, onAtBottomChange, recordDebugSnapshot]);

  useLayoutEffect(() => {
    if (!isActive || !loaded) return;
    if (!continueHistoryAtTopRef.current) return;
    if (!canLoadOlder || pendingHistoryRef.current) {
      if (!canLoadOlder) {
        continueHistoryAtTopRef.current = false;
      }
      return;
    }

    const scroller = methodsRef.current?.scrollerElement?.() ?? null;
    if (!scroller) return;

    const stillTopPinned = scroller.scrollTop <= 1;
    continueHistoryAtTopRef.current = false;
    if (!stillTopPinned) return;

    requestOlderHistory("top-pinned-continue", {
      scrollTop: Math.round(scroller.scrollTop * 100) / 100,
      scrollHeight: Math.round(scroller.scrollHeight * 100) / 100,
      clientHeight: Math.round(scroller.clientHeight * 100) / 100,
      canLoadOlder,
    });
  }, [canLoadOlder, isActive, listItems, loaded, requestOlderHistory]);

  const onScroll = useCallback(
    (location: PretextVirtualizerScrollLocation) => {
      if (!isActive) return;

      const atBottom = location.bottomOffset <= 16;
      if (lastAtBottomRef.current !== atBottom) {
        lastAtBottomRef.current = atBottom;
        onAtBottomChange?.(atBottom);
      }

      if (!canLoadOlder || atBottom || pendingHistoryRef.current) {
        if (!canLoadOlder || atBottom) {
          continueHistoryAtTopRef.current = false;
        }
        lastListOffsetRef.current = location.listOffset;
        return;
      }

      const previousListOffset = lastListOffsetRef.current;
      lastListOffsetRef.current = location.listOffset;
      const scrollingUp = previousListOffset == null ? false : location.listOffset > previousListOffset;
      const nearTop = location.listOffset > -computeHistoryPrefetchThresholdPx(location.visibleListHeight);
      if (!nearTop || !scrollingUp) {
        return;
      }

      requestOlderHistory("scroll", {
        listOffset: location.listOffset,
        visibleListHeight: location.visibleListHeight,
        bottomOffset: location.bottomOffset,
        nearTop,
        scrollingUp,
      });
    },
    [canLoadOlder, isActive, onAtBottomChange, requestOlderHistory],
  );

  const onRenderedDataChange = useCallback((range: readonly WorkbenchListItem[]) => {
    const topId = range[0]?.id ?? null;
    const middleIndex = range.length > 0 ? Math.floor(range.length / 2) : 0;
    renderedTopIdRef.current = topId;
    renderedAnchorIdRef.current = range[middleIndex]?.id ?? topId;
    if (
      range.length > 0 &&
      loaded &&
      initialContentRenderedSessionIdRef.current !== sessionId
    ) {
      initialContentRenderedSessionIdRef.current = sessionId;
      onInitialContentRendered?.();
    }
  }, [loaded, onInitialContentRendered, sessionId]);

  return {
    methodsRef,
    initialData: listItems,
    context,
    initialLocation: PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION,
    onScroll,
    onRenderedDataChange,
  };
}
