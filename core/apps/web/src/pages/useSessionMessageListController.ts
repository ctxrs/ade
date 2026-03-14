import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import type {
  AutoscrollToBottom,
  ItemLocation,
  ListScrollLocation,
  VirtuosoMessageListMethods,
} from "@virtuoso.dev/message-list";
import { useRafCoalesced } from "../components/hooks/useRafCoalesced";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";
import { debugItemSummary, debugStableKey, findFirstRenderedItemContractViolation } from "./sessionMessageListDataDebug";
import { logSessionMessageListReconcileDebug } from "./sessionMessageListReconcileDebug";
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
};

type Result = {
  methodsRef: MutableRefObject<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  context: WorkbenchMessageListContext;
  initialData: WorkbenchListItem[];
  initialLocation: ItemLocation;
  onScroll: (location: ListScrollLocation) => void;
  onRenderedDataChange: (range: WorkbenchListItem[]) => void;
};

const INITIAL_LOCATION_BOTTOM: ItemLocation = { index: "LAST", align: "end" };

export function useSessionMessageListController(params: Params): Result {
  const {
    sessionId,
    isActive,
    loaded,
    listItems,
    canLoadOlder,
    loadOlder,
    showDebug,
    onAtBottomChange,
  } = params;

  const methodsRef = useRef<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>(null);
  const lastSessionIdRef = useRef(sessionId);
  const lastIsActiveRef = useRef(isActive);
  const contractViolationLoggedRef = useRef<{ sessionId: string; violationKey: string } | null>(null);

  const lastScrollLocationRef = useRef<ListScrollLocation | null>(null);
  const stickToBottomRef = useRef(true);
  const lastAtBottomRef = useRef<boolean | null>(null);
  const lastListOffsetRef = useRef<number | null>(null);

  // Best-effort anchoring based on rendered data (no DOM reads).
  // NOTE: `onRenderedDataChange` can include overscan. Anchoring to `range[0]` can anchor an offscreen
  // row and cause visible jumps, especially with large `increaseViewportBy`. Prefer a mid-range anchor.
  const renderedAnchorIdRef = useRef<string | null>(null);
  const renderedTopIdRef = useRef<string | null>(null);
  const firstListItemIdRef = useRef<string | null>(null);

  const pendingHistoryRef = useRef(false);
  const historyExpectedRef = useRef(false);
  const historyRequestedAtTopRef = useRef(false);
  const historyRequestedAnchorIdRef = useRef<string | null>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const suppressIdDiffLogsRef = useRef<{ sessionId: string; remainingTicks: number } | null>(null);
  const lastScrollDebugAtRef = useRef(0);

  // Coalesce for steady-state updates, but never let it affect session transitions.
  const listItemsCoalesced = useRafCoalesced(listItems);

  const context = useMemo(() => ({ loaded, loadingOlder }), [loaded, loadingOlder]);
  const recordDebugSnapshot = useSessionMessageListDiagnostics({
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
  const logMessageListDebug = useCallback(
    (label: string, detail: Record<string, unknown>) => {
      if (!showDebug) return;
      // eslint-disable-next-line no-console
      console.log(`[MessageList][${label}] ${JSON.stringify({ sessionId, ...detail })}`);
    },
    [sessionId, showDebug],
  );
  const snapToBottom = useCallback(
    (methods: VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext>) => {
      requestAnimationFrame(() => {
        methods.scrollToItem({ index: "LAST", align: "end", behavior: "instant" });
        const scroller = methods.scrollerElement?.() ?? null;
        if (scroller) {
          scroller.scrollTop = scroller.scrollHeight;
        }
      });
    },
    [],
  );

  const appendBehavior = useMemo<AutoscrollToBottom<WorkbenchListItem, WorkbenchMessageListContext>>(
    () => (params) => (params.atBottom ? "auto" : false),
    [],
  );
  const initialLocation: ItemLocation = INITIAL_LOCATION_BOTTOM;
  const initialData = useMemo<WorkbenchListItem[]>(() => listItems, [listItems]);

  // Keep an up-to-date reference without introducing additional hook ordering churn under HMR.
  firstListItemIdRef.current = listItemsCoalesced?.[0]?.id ?? null;

  const onScroll = useCallback(
    (location: ListScrollLocation) => {
      lastScrollLocationRef.current = location;
      if (!isActive) return;

      const scroller = methodsRef.current?.scrollerElement?.() ?? null;
      const atBottomFromLocation = location.bottomOffset <= 16;
      // Prefer the live DOM scroller metrics when available; bottomOffset can be optimistic mid-transition.
      const atBottom =
        scroller
          ? scroller.scrollHeight - (scroller.scrollTop + scroller.clientHeight) <= 16
          : atBottomFromLocation;
      stickToBottomRef.current = atBottom;
      if (isActive && onAtBottomChange && lastAtBottomRef.current !== atBottom) {
        lastAtBottomRef.current = atBottom;
        onAtBottomChange(atBottom);
      }

      // History pagination trigger: use the library-provided scroll location only.
      // Prefetch when approaching top to avoid a hard stop + later prepend “resume”.
      const atTop = location.listOffset === 0;
      const prevOffset = lastListOffsetRef.current;
      lastListOffsetRef.current = location.listOffset;
      // Scrolling up means listOffset moves toward 0 (increases, since it's negative when scrolled down).
      const scrollingUp = prevOffset == null ? false : location.listOffset > prevOffset;
      const prefetchThreshold = -Math.max(250, location.visibleListHeight); // ~1 viewport, min 250px
      const nearTop = location.listOffset > prefetchThreshold;

      if (import.meta.env.DEV && showDebug) {
        const now = Date.now();
        const shouldRecordScroll = now - lastScrollDebugAtRef.current >= 120 || nearTop || atBottom;
        if (shouldRecordScroll) {
          lastScrollDebugAtRef.current = now;
          recordDebugSnapshot("scroll", {
            listOffset: location.listOffset,
            visibleListHeight: location.visibleListHeight,
            bottomOffset: location.bottomOffset,
            atBottom,
          });
        }
      }

      if (import.meta.env.DEV && showDebug && nearTop) {
        // eslint-disable-next-line no-console
        console.debug("[MessageList][history:gate]", {
          sessionId,
          loaded,
          canLoadOlder,
          stickToBottom: stickToBottomRef.current,
          pendingHistory: pendingHistoryRef.current,
          loadingOlder,
          atTop,
          nearTop,
          scrollingUp,
          prefetchThreshold,
          firstRenderedId: renderedTopIdRef.current,
          firstListId: firstListItemIdRef.current,
          listOffset: location.listOffset,
          visibleListHeight: location.visibleListHeight,
          renderedTopId: renderedTopIdRef.current,
          renderedAnchorId: renderedAnchorIdRef.current,
        });
      }

      if (!canLoadOlder) return;
      if (stickToBottomRef.current) return;
      if (pendingHistoryRef.current || loadingOlder) return;
      if (!nearTop) return;
      if (!scrollingUp) return;

      pendingHistoryRef.current = true;
      historyExpectedRef.current = true;
      historyRequestedAtTopRef.current = atTop;
      historyRequestedAnchorIdRef.current = renderedAnchorIdRef.current;
      setLoadingOlder(true);
      if (import.meta.env.DEV && showDebug) {
        // eslint-disable-next-line no-console
        console.debug("[MessageList][history:request]", {
          sessionId,
          atTop,
          nearTop,
          scrollingUp,
          anchorId: renderedAnchorIdRef.current,
          firstRenderedId: renderedTopIdRef.current,
          firstListId: firstListItemIdRef.current,
          listOffset: location.listOffset,
          visibleListHeight: location.visibleListHeight,
        });
      }
      loadOlder()
        .catch(() => {})
        .finally(() => {
          pendingHistoryRef.current = false;
          setLoadingOlder(false);
        });
    },
    [
      canLoadOlder,
      loaded,
      loadOlder,
      loadingOlder,
      onAtBottomChange,
      sessionId,
      showDebug,
      isActive,
    ],
  );

  const onRenderedDataChange = useCallback((range: WorkbenchListItem[]) => {
    const topId = range?.[0]?.id ?? null;
    const middleIndex = range.length > 0 ? Math.floor(range.length / 2) : 0;
    const anchorId = range?.[middleIndex]?.id ?? topId;
    renderedTopIdRef.current = topId;
    renderedAnchorIdRef.current = anchorId;
  }, []);

  useLayoutEffect(() => {
    const becameActive = isActive && !lastIsActiveRef.current;
    lastIsActiveRef.current = isActive;
    if (!becameActive) return;

    const methods = methodsRef.current;
    if (!methods) return;
    stickToBottomRef.current = true;
    lastAtBottomRef.current = true;
    methods.cancelSmoothScroll();
    snapToBottom(methods);
    onAtBottomChange?.(true);
    recordDebugSnapshot("session:activated", { reason: "focusBottom" });
  }, [isActive, onAtBottomChange, recordDebugSnapshot, snapToBottom]);

  useLayoutEffect(() => {
    if (!isActive) return;
    const methods = methodsRef.current;
    if (!methods) return;

    // Use raw data for session boundaries; coalescing can lag a frame across session switches.
    const nextRaw = listItems;
    let next = listItemsCoalesced;
    const current = methods.data.get();
    const sessionChanged = lastSessionIdRef.current !== sessionId;

    // Suppress noisy "ids missing" diagnostics during session transitions / initial hydration.
    // The list is expected to change dramatically in these windows and the logs are not actionable.
    if (!sessionChanged) {
      const suppress = suppressIdDiffLogsRef.current;
      if (suppress && suppress.sessionId === sessionId && suppress.remainingTicks > 0) {
        suppress.remainingTicks -= 1;
        if (suppress.remainingTicks <= 0) suppressIdDiffLogsRef.current = null;
      }
    }

    if (import.meta.env.DEV && showDebug) {
      // Validate the derived list item contract. This makes missing identity fields obvious
      // before we start chasing down reconcile/replace artifacts in MessageList.
      const violation = findFirstRenderedItemContractViolation(nextRaw);
      if (violation) {
        const violationKey = `${violation.kind}:${violation.reason}:${violation.id}`;
        const prev = contractViolationLoggedRef.current;
        if (!prev || prev.sessionId !== sessionId || prev.violationKey !== violationKey) {
          contractViolationLoggedRef.current = { sessionId, violationKey };
          // eslint-disable-next-line no-console
          console.error("[MessageList][contract-violation]", {
            sessionId,
            ...violation,
          });
        }
      }

      const seen = new Set<string>();
      const dupes: string[] = [];
      for (const it of next) {
        const itemId = String(it?.id ?? "");
        if (!itemId) continue;
        if (seen.has(itemId)) dupes.push(itemId);
        else seen.add(itemId);
      }
      if (dupes.length > 0) {
        // eslint-disable-next-line no-console
        console.error("[MessageList] duplicate WorkbenchListItem.id values detected", {
          count: dupes.length,
          sample: dupes.slice(0, 10),
        });
      }

      // Detect id churn by comparing stable identity keys between current/next.
      const currentByStable = new Map<string, string>();
      const stableKeyCollisions: Array<{ stableKey: string; ids: string[] }> = [];
      for (const it of current) {
        const stableKey = debugStableKey(it);
        const id = String(it.id ?? "");
        if (!stableKey || !id) continue;
        const prev = currentByStable.get(stableKey);
        if (prev && prev !== id) {
          stableKeyCollisions.push({ stableKey, ids: [prev, id] });
        } else {
          currentByStable.set(stableKey, id);
        }
      }
      const nextByStable = new Map<string, string>();
      const stableIdChanges: Array<{ stableKey: string; from: string; to: string }> = [];
      for (const it of next) {
        const stableKey = debugStableKey(it);
        const id = String(it.id ?? "");
        if (!stableKey || !id) continue;
        const prev = nextByStable.get(stableKey);
        if (prev && prev !== id) {
          stableKeyCollisions.push({ stableKey, ids: [prev, id] });
          continue;
        }
        nextByStable.set(stableKey, id);
        const from = currentByStable.get(stableKey);
        if (from && from !== id) {
          stableIdChanges.push({ stableKey, from, to: id });
        }
      }
      if (stableIdChanges.length > 0) {
        const currentById = new Map(current.map((it) => [it.id, it] as const));
        const nextById = new Map(next.map((it) => [it.id, it] as const));
        const sample = stableIdChanges.slice(0, 10).map((c) => ({
          ...c,
          fromItem: debugItemSummary(currentById.get(c.from) ?? { id: c.from }),
          toItem: debugItemSummary(nextById.get(c.to) ?? { id: c.to }),
        }));
        // eslint-disable-next-line no-console
        console.warn("[MessageList] possible unstable WorkbenchListItem.id detected (stableKey id changed)", {
          sessionId,
          count: stableIdChanges.length,
          sample,
        });
      }
      if (stableKeyCollisions.length > 0) {
        // eslint-disable-next-line no-console
        console.warn("[MessageList] stableKey collisions detected (diagnostic key too weak or duplicate items)", {
          sessionId,
          count: stableKeyCollisions.length,
          sample: stableKeyCollisions.slice(0, 5),
        });
      }
    }

    if (sessionChanged) {
      lastSessionIdRef.current = sessionId;
      pendingHistoryRef.current = false;
      historyExpectedRef.current = false;
      historyRequestedAtTopRef.current = false;
      historyRequestedAnchorIdRef.current = null;
      setLoadingOlder(false);
      lastScrollLocationRef.current = null;
      lastListOffsetRef.current = null;
      stickToBottomRef.current = true;
      lastAtBottomRef.current = true;
      renderedAnchorIdRef.current = null;
      renderedTopIdRef.current = null;
      firstListItemIdRef.current = null;
      methods.cancelSmoothScroll();
      suppressIdDiffLogsRef.current = { sessionId, remainingTicks: 3 };
      methods.data.replace(nextRaw, { initialLocation, purgeItemSizes: true });
      snapToBottom(methods);
      recordDebugSnapshot("data:replace", {
        reason: "sessionChanged",
        nextLen: nextRaw.length,
        currentLen: current.length,
      });
      logMessageListDebug("data:replace", {
        reason: "sessionChanged",
        nextLen: nextRaw.length,
        currentLen: current.length,
      });
      return;
    }

    const nextLen = next.length;
    const currentLen = current.length;

    // Initial population: never treat empty->non-empty as prepend/append.
    // Use `replace(..., initialLocation: LAST)` so opening a session lands at bottom deterministically.
    if (currentLen === 0) {
      if (nextRaw.length === 0) return;
      historyExpectedRef.current = false;
      methods.cancelSmoothScroll();
      suppressIdDiffLogsRef.current = { sessionId, remainingTicks: 2 };
      methods.data.replace(nextRaw, { initialLocation, purgeItemSizes: true });
      snapToBottom(methods);
      recordDebugSnapshot("data:replace", {
        reason: "initialPopulation",
        nextLen: nextRaw.length,
        currentLen,
      });
      logMessageListDebug("data:replace", {
        reason: "initialPopulation",
        nextLen: nextRaw.length,
        currentLen,
      });
      return;
    }

    if (nextLen === 0) {
      historyExpectedRef.current = false;
      methods.data.deleteRange(0, currentLen);
      recordDebugSnapshot("data:deleteRange", {
        offset: 0,
        count: currentLen,
      });
      if (import.meta.env.DEV && showDebug) {
        // eslint-disable-next-line no-console
        console.debug("[MessageList][data:deleteRange]", { sessionId, offset: 0, count: currentLen });
      }
      return;
    }

    // If we just requested history and the next update is not a pure prepend (e.g. streaming appended too),
    // apply it as an extension update instead of falling back to `replace()`.
    if (historyExpectedRef.current && currentLen > 0 && nextLen >= currentLen) {
      const wasAtTop = historyRequestedAtTopRef.current;
      const requestedAnchorId = historyRequestedAnchorIdRef.current;
      const firstId = current[0]?.id ?? null;
      const lastId = current[currentLen - 1]?.id ?? null;
      const firstIndex = firstId ? next.findIndex((it) => it.id === firstId) : -1;
      const lastIndex = lastId ? next.findIndex((it) => it.id === lastId) : -1;
      if (firstIndex >= 0 && lastIndex >= firstIndex) {
        const currentIdSet = new Set(current.map((it) => it.id));
        const prefix = next.slice(0, firstIndex).filter((it) => !currentIdSet.has(it.id));
        const suffix = next.slice(lastIndex + 1).filter((it) => !currentIdSet.has(it.id));
        const nextById = new Map(next.map((it) => [it.id, it] as const));

        if (import.meta.env.DEV && showDebug) {
          const nextIdSet = new Set(next.map((it) => it.id));
          const missingFromNext: string[] = [];
          for (const it of current) if (!nextIdSet.has(it.id)) missingFromNext.push(it.id);
          if (missingFromNext.length > 0) {
            const currentById = new Map(current.map((it) => [it.id, it] as const));
            // eslint-disable-next-line no-console
            console.warn("[MessageList][history:extend][ids:missing-from-next]", {
              sessionId,
              count: missingFromNext.length,
              sample: missingFromNext.slice(0, 12).map((id) =>
                debugItemSummary(currentById.get(id) ?? { id }),
              ),
            });
          }
        }

        // Avoid batching `prepend()` with other ops; let MessageList manage scroll stabilization.
        if (prefix.length > 0) methods.data.prepend(prefix);
        if (suffix.length > 0) methods.data.append(suffix, appendBehavior);

        // For non-bottom, keep a rendered item anchored as size estimates settle.
        if (!stickToBottomRef.current) {
          const anchorId = requestedAnchorId ?? renderedAnchorIdRef.current;
          const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;
          if (anchorIndex >= 0) methods.data.mapWithAnchor((item) => nextById.get(item.id) ?? item, anchorIndex);
          else methods.data.map((item) => nextById.get(item.id) ?? item);
        } else {
          methods.data.map((item) => nextById.get(item.id) ?? item, "auto");
        }

        // If the user actually hit the top, force the pre-history first item back to the top.
        // This uses the library's own scroll API (no DOM reads/offset math).
        if (wasAtTop && !stickToBottomRef.current && firstIndex >= 0) {
          // `prepend()` schedules internal rAF scroll stabilization; schedule our pin after it (pre-paint).
          requestAnimationFrame(() => methods.scrollToItem({ index: firstIndex, align: "start", behavior: "instant" }));
        }
        historyExpectedRef.current = false;
        historyRequestedAtTopRef.current = false;
        historyRequestedAnchorIdRef.current = null;
        recordDebugSnapshot("history:extend", {
          prefixLen: prefix.length,
          suffixLen: suffix.length,
          firstIndex,
          lastIndex,
          nextLen,
          currentLen,
          requestedAnchorId,
          wasAtTop,
        });
        if (import.meta.env.DEV && showDebug) {
          // eslint-disable-next-line no-console
          console.debug("[MessageList][history:extend]", {
            sessionId,
            prefixLen: prefix.length,
            suffixLen: suffix.length,
            firstIndex,
            lastIndex,
            nextLen,
            currentLen,
          });
        }
        return;
      }
    }

    // Pure prepend: next ends with current.
    if (nextLen > currentLen) {
      let isPurePrepend = true;
      for (let i = 0; i < currentLen; i += 1) {
        if (next[nextLen - currentLen + i]?.id !== current[i]?.id) {
          isPurePrepend = false;
          break;
        }
      }
      if (isPurePrepend) {
        const wasAtTop = historyRequestedAtTopRef.current;
        const requestedAnchorId = historyRequestedAnchorIdRef.current;
        const prefix = next.slice(0, nextLen - currentLen);
        const nextById = new Map(next.map((it) => [it.id, it] as const));
        const anchorId = renderedAnchorIdRef.current;
        const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;

        // Avoid batching `prepend()` with other ops; rely on MessageList prepend stabilization.
        if (prefix.length > 0) methods.data.prepend(prefix);

        if (!stickToBottomRef.current) {
          const reqAnchorId = requestedAnchorId ?? renderedAnchorIdRef.current;
          const reqAnchorIndex = reqAnchorId ? next.findIndex((it) => it.id === reqAnchorId) : -1;
          if (reqAnchorIndex >= 0) methods.data.mapWithAnchor((item) => nextById.get(item.id) ?? item, reqAnchorIndex);
          else methods.data.map((item) => nextById.get(item.id) ?? item);
        } else {
          methods.data.map((item) => nextById.get(item.id) ?? item, "auto");
        }

        // If we were at the very top, pin the previous first item back to the top.
        if (wasAtTop && !stickToBottomRef.current) {
          const targetIndex = prefix.length;
          requestAnimationFrame(() => methods.scrollToItem({ index: targetIndex, align: "start", behavior: "instant" }));
        }
        recordDebugSnapshot("data:prepend", {
          prefixLen: prefix.length,
          nextLen,
          currentLen,
          anchorId,
          anchorIndex,
          requestedAnchorId,
          wasAtTop,
        });
        if (import.meta.env.DEV && showDebug) {
          // eslint-disable-next-line no-console
          console.debug("[MessageList][data:prepend]", {
            sessionId,
            prefixLen: prefix.length,
            nextLen,
            currentLen,
            anchorId,
            anchorIndex,
          });
        }
        if (historyExpectedRef.current) {
          historyExpectedRef.current = false;
          historyRequestedAtTopRef.current = false;
          historyRequestedAnchorIdRef.current = null;
          if (import.meta.env.DEV && showDebug) {
            // eslint-disable-next-line no-console
            console.debug("[MessageList][history:applied]", { sessionId, prefixLen: prefix.length });
          }
        }
        return;
      }
    }

    // Pure append: next starts with current.
    if (nextLen > currentLen) {
      let isPureAppend = true;
      for (let i = 0; i < currentLen; i += 1) {
        if (next[i]?.id !== current[i]?.id) {
          isPureAppend = false;
          break;
        }
      }
      if (isPureAppend) {
        const suffix = next.slice(currentLen);
        if (suffix.length > 0) methods.data.append(suffix, appendBehavior);
        if (!stickToBottomRef.current) {
          const nextById = new Map(next.map((it) => [it.id, it] as const));
          const anchorId = renderedAnchorIdRef.current;
          const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;
          if (anchorIndex >= 0) {
            methods.data.mapWithAnchor((item) => nextById.get(item.id) ?? item, anchorIndex);
          } else {
            methods.data.map((item) => nextById.get(item.id) ?? item);
          }
        } else {
          methods.data.map(
            (item) => item,
            stickToBottomRef.current ? ("auto" as const) : undefined,
          );
        }
        recordDebugSnapshot("data:append", {
          suffixLen: suffix.length,
          nextLen,
          currentLen,
        });
        logMessageListDebug("data:append", {
          suffixLen: suffix.length,
          nextLen,
          currentLen,
          stickToBottom: stickToBottomRef.current,
          anchorId: renderedAnchorIdRef.current,
        });
        return;
      }
    }

    // Same IDs/order: update in place (streaming/tool status, expands, etc).
    if (nextLen === currentLen) {
      let same = true;
      for (let i = 0; i < currentLen; i += 1) {
        if (next[i]?.id !== current[i]?.id) {
          same = false;
          break;
        }
      }
      if (same) {
        const nextById = new Map(next.map((it) => [it.id, it] as const));
        const anchorId = renderedAnchorIdRef.current;
        const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;
        if (import.meta.env.DEV && showDebug) {
          // Count by reference to detect “content changes” even when IDs/order are stable.
          let changedByRef = 0;
          const sampleChangedIds: string[] = [];
          for (let i = 0; i < currentLen; i += 1) {
            if (current[i] !== next[i]) {
              changedByRef += 1;
              if (sampleChangedIds.length < 8) sampleChangedIds.push(String(next[i]?.id ?? current[i]?.id ?? ""));
            }
          }
          const mapMode =
            !stickToBottomRef.current && anchorIndex >= 0
              ? "mapWithAnchor"
              : stickToBottomRef.current
                ? "map:auto"
                : "map";
          // eslint-disable-next-line no-console
          console.debug("[MessageList][data:map]", {
            sessionId,
            nextLen,
            currentLen,
            stickToBottom: stickToBottomRef.current,
            anchorId,
            anchorIndex,
            mapMode,
            changedByRef,
            sampleChangedIds,
            renderedTopId: renderedTopIdRef.current,
          });
        }
        if (!stickToBottomRef.current && anchorIndex >= 0) {
          methods.data.mapWithAnchor((item) => nextById.get(item.id) ?? item, anchorIndex);
        } else {
          methods.data.map(
            (item) => nextById.get(item.id) ?? item,
            stickToBottomRef.current ? ("auto" as const) : undefined,
          );
        }
        recordDebugSnapshot("data:map", {
          nextLen,
          currentLen,
          anchorId,
          anchorIndex,
          stickToBottom: stickToBottomRef.current,
        });
        logMessageListDebug("data:map", {
          nextLen,
          currentLen,
          anchorId,
          anchorIndex,
          stickToBottom: stickToBottomRef.current,
        });
        return;
      }
    }

    // Structural reconcile (no replace): transform `current` into `next` using only MessageList data methods.
    // This covers mixed updates (middle inserts/deletes/reorders) which can happen during history/hydration.
    const currentIds = current.map((it) => it.id);
    const nextIds = next.map((it) => it.id);

    let prefixLen = 0;
    while (prefixLen < currentLen && prefixLen < nextLen && currentIds[prefixLen] === nextIds[prefixLen]) {
      prefixLen += 1;
    }
    let suffixLen = 0;
    while (
      suffixLen < currentLen - prefixLen &&
      suffixLen < nextLen - prefixLen &&
      currentIds[currentLen - 1 - suffixLen] === nextIds[nextLen - 1 - suffixLen]
    ) {
      suffixLen += 1;
    }

    const deleteCount = currentLen - prefixLen - suffixLen;
    const insertData = next.slice(prefixLen, nextLen - suffixLen);
    const nextById = new Map(next.map((it) => [it.id, it] as const));
    const anchorId = renderedAnchorIdRef.current;
    const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;

    const suppress = suppressIdDiffLogsRef.current;
    const suppressIdDiffLogs = Boolean(suppress && suppress.sessionId === sessionId && suppress.remainingTicks > 0);
    const historyExpected = historyExpectedRef.current;
    if (historyExpected) {
      historyExpectedRef.current = false;
    }
    logSessionMessageListReconcileDebug({
      devEnabled: import.meta.env.DEV,
      showDebug,
      sessionId,
      current,
      next,
      currentIds,
      nextIds,
      currentLen,
      nextLen,
      prefixLen,
      suffixLen,
      deleteCount,
      insertDataLength: insertData.length,
      anchorId,
      anchorIndex,
      historyExpected,
      stickToBottom: stickToBottomRef.current,
      suppressIdDiffLogs,
    });

    methods.data.batch(
      () => {
        if (deleteCount > 0) methods.data.deleteRange(prefixLen, deleteCount);
        if (insertData.length > 0) methods.data.insert(insertData, prefixLen, appendBehavior);
        if (!stickToBottomRef.current && anchorIndex >= 0) {
          methods.data.mapWithAnchor((item) => nextById.get(item.id) ?? item, anchorIndex);
        } else {
          methods.data.map(
            (item) => nextById.get(item.id) ?? item,
            stickToBottomRef.current ? ("auto" as const) : undefined,
          );
        }
      },
      appendBehavior,
    );
    recordDebugSnapshot("data:reconcile", {
      nextLen,
      currentLen,
      prefixLen,
      suffixLen,
      deleteCount,
      insertLen: insertData.length,
      anchorId,
      anchorIndex,
      stickToBottom: stickToBottomRef.current,
    });
  }, [
    appendBehavior,
    initialLocation,
    isActive,
    listItems,
    listItemsCoalesced,
    recordDebugSnapshot,
    sessionId,
    showDebug,
    snapToBottom,
  ]);

  return {
    methodsRef,
    context,
    initialData,
    initialLocation,
    onScroll,
    onRenderedDataChange,
  };
}
