import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, type MutableRefObject } from "react";
import type { IndexLocationWithAlign, VirtuosoHandle } from "react-virtuoso";

export type PinnedScrollPersistState = {
  stickToBottom: boolean;
  anchorItemId: string | null;
  anchorOffset: number | null;
  scrollTop: number | null;
};

export type UsePinnedScrollManagerArgs = {
  isActive: boolean;
  preserveScrollOnFocus: boolean;
  bottomThresholdPx: number;
  userIntentWindowMs: number;
  itemsLength: number;
  firstItemIndex: number;
  scrollStateStickToBottom: boolean | null | undefined;
  syncKey: string;
  restoreInProgress: boolean;

  stickToBottomRef: MutableRefObject<boolean>;
  setStickToBottom: (next: boolean) => void;
  setAtBottom: (next: boolean) => void;

  latestAnchorIdRef: MutableRefObject<string | null>;
  latestAnchorOffsetRef: MutableRefObject<number | null>;
  liveScrollTopRef: MutableRefObject<number | null>;
  userScrollIntentRef: MutableRefObject<number>;
  scrollbarDraggingRef: MutableRefObject<boolean>;

  persistScroll: (next: PinnedScrollPersistState) => void;

  virtuosoRef: MutableRefObject<VirtuosoHandle | null>;
  scrollerRef: MutableRefObject<HTMLDivElement | null>;
  listRef: MutableRefObject<HTMLDivElement | null>;
  scrollerNode: HTMLDivElement | null;
  bottomSentinelNode: HTMLDivElement | null;
  sentinelMeasuredRef: MutableRefObject<boolean>;
  sentinelVisibleRef: MutableRefObject<boolean>;

  autoScrollRef: MutableRefObject<number>;
  autoScrollRafRef: MutableRefObject<number | null>;
  autoScrollAttemptRef: MutableRefObject<number>;
};

export type UsePinnedScrollManagerReturn = {
  initialTopMostItemIndex: IndexLocationWithAlign | number | undefined;
  markAutoScroll: () => void;
  scrollToBottomNow: () => void;
  scheduleAutoScroll: () => void;
};

export function usePinnedScrollManager(args: UsePinnedScrollManagerArgs): UsePinnedScrollManagerReturn {
  const {
    isActive,
    preserveScrollOnFocus,
    bottomThresholdPx,
    userIntentWindowMs,
    itemsLength,
    firstItemIndex,
    scrollStateStickToBottom,
    syncKey,
    restoreInProgress,
    stickToBottomRef,
    setStickToBottom,
    setAtBottom,
    latestAnchorIdRef,
    latestAnchorOffsetRef,
    liveScrollTopRef,
    userScrollIntentRef,
    scrollbarDraggingRef,
    persistScroll,
    virtuosoRef,
    scrollerRef,
    listRef,
    scrollerNode,
    bottomSentinelNode,
    sentinelMeasuredRef,
    sentinelVisibleRef,
    autoScrollRef,
    autoScrollRafRef,
    autoScrollAttemptRef,
  } = args;

  const markAutoScroll = useCallback(() => {
    autoScrollRef.current = Date.now();
  }, [autoScrollRef]);

  const initialTopMostItemIndex = useMemo<IndexLocationWithAlign | number | undefined>(() => {
    if (scrollStateStickToBottom === false) return undefined;
    if (itemsLength === 0) return undefined;
    return { index: firstItemIndex + itemsLength - 1, align: "end" };
  }, [firstItemIndex, itemsLength, scrollStateStickToBottom]);

  const scrollToBottomNow = useCallback(() => {
    if (itemsLength === 0) return;
    const handle = virtuosoRef.current;
    const el = scrollerRef.current;
    if (handle) {
      markAutoScroll();
      handle.scrollToIndex({ index: firstItemIndex + itemsLength - 1, align: "end" });
      return;
    }
    if (el) {
      markAutoScroll();
      el.scrollTop = el.scrollHeight;
    }
  }, [firstItemIndex, itemsLength, markAutoScroll, scrollerRef, virtuosoRef]);

  const syncAtBottom = useCallback(() => {
    if (sentinelMeasuredRef.current) {
      const visible = sentinelVisibleRef.current;
      setAtBottom(visible);
      return visible;
    }
    const node = scrollerRef.current;
    if (!node) return null;
    const remaining = node.scrollHeight - (node.scrollTop + node.clientHeight);
    const nearBottom = remaining <= bottomThresholdPx;
    setAtBottom(nearBottom);
    return nearBottom;
  }, [bottomThresholdPx, scrollerRef, sentinelMeasuredRef, sentinelVisibleRef, setAtBottom]);

  const scheduleAutoScroll = useCallback(() => {
    if (itemsLength === 0) return;
    if (!stickToBottomRef.current) {
      autoScrollAttemptRef.current = 0;
      return;
    }
    if (restoreInProgress) {
      autoScrollAttemptRef.current = 0;
      return;
    }
    if (preserveScrollOnFocus && !isActive) {
      autoScrollAttemptRef.current = 0;
      return;
    }
    if (autoScrollRafRef.current != null) return;
    autoScrollRafRef.current = window.requestAnimationFrame(() => {
      autoScrollRafRef.current = null;
      if (!stickToBottomRef.current) {
        autoScrollAttemptRef.current = 0;
        return;
      }
      if (restoreInProgress) {
        autoScrollAttemptRef.current = 0;
        return;
      }
      if (preserveScrollOnFocus && !isActive) {
        autoScrollAttemptRef.current = 0;
        return;
      }
      scrollToBottomNow();
      window.requestAnimationFrame(() => {
        const el = scrollerRef.current;
        if (!el) return;
        const remaining = el.scrollHeight - (el.scrollTop + el.clientHeight);
        const nearBottom = remaining <= bottomThresholdPx;
        if (!nearBottom && autoScrollAttemptRef.current < 6) {
          autoScrollAttemptRef.current += 1;
          scheduleAutoScroll();
          return;
        }
        setAtBottom(nearBottom);
        if (nearBottom) autoScrollAttemptRef.current = 0;
      });
    });
  }, [
    bottomThresholdPx,
    isActive,
    itemsLength,
    preserveScrollOnFocus,
    restoreInProgress,
    scrollerRef,
    scrollToBottomNow,
    setAtBottom,
    stickToBottomRef,
  ]);

  useLayoutEffect(() => {
    if (preserveScrollOnFocus && !isActive) return;
    const nearBottom = syncAtBottom();
    if (stickToBottomRef.current && nearBottom === false) {
      scheduleAutoScroll();
    }
    if (stickToBottomRef.current && nearBottom === true) {
      autoScrollAttemptRef.current = 0;
    }
  }, [
    isActive,
    preserveScrollOnFocus,
    scheduleAutoScroll,
    syncAtBottom,
    syncKey,
  ]);

  useEffect(() => {
    if (!isActive) return;
    if (!stickToBottomRef.current) return;
    requestAnimationFrame(() => {
      if (!stickToBottomRef.current) return;
      if (preserveScrollOnFocus && !isActive) return;
      scrollToBottomNow();
    });
  }, [isActive, preserveScrollOnFocus, scrollToBottomNow, stickToBottomRef]);

  useEffect(() => {
    const sentinel = bottomSentinelNode;
    const scroller = scrollerNode;
    if (!sentinel || !scroller) return;
    sentinelMeasuredRef.current = false;
    const observer = new IntersectionObserver(
      (entries) => {
        const entry = entries[entries.length - 1];
        const visible = entry?.isIntersecting ?? false;
        sentinelMeasuredRef.current = true;
        sentinelVisibleRef.current = visible;
        setAtBottom(visible);
        if (visible) {
          autoScrollAttemptRef.current = 0;
          return;
        }
        if (!stickToBottomRef.current) return;
        if (restoreInProgress) return;
        const now = Date.now();
        const hasUserIntent = now - userScrollIntentRef.current < userIntentWindowMs;
        if (hasUserIntent || scrollbarDraggingRef.current) {
          stickToBottomRef.current = false;
          setStickToBottom(false);
          const el = scrollerRef.current;
          const scrollTop = el?.scrollTop ?? liveScrollTopRef.current ?? 0;
          persistScroll({
            stickToBottom: false,
            anchorItemId: latestAnchorIdRef.current,
            anchorOffset: latestAnchorOffsetRef.current,
            scrollTop,
          });
          return;
        }
        if (preserveScrollOnFocus && !isActive) return;
      },
      {
        root: scroller,
        threshold: 0,
        rootMargin: `0px 0px ${bottomThresholdPx}px 0px`,
      },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [
    bottomSentinelNode,
    bottomThresholdPx,
    isActive,
    latestAnchorIdRef,
    latestAnchorOffsetRef,
    liveScrollTopRef,
    persistScroll,
    preserveScrollOnFocus,
    restoreInProgress,
    scheduleAutoScroll,
    scrollerNode,
    scrollerRef,
    scrollbarDraggingRef,
    sentinelMeasuredRef,
    sentinelVisibleRef,
    setAtBottom,
    setStickToBottom,
    stickToBottomRef,
    userIntentWindowMs,
    userScrollIntentRef,
  ]);

  return {
    initialTopMostItemIndex,
    markAutoScroll,
    scrollToBottomNow,
    scheduleAutoScroll,
  };
}
