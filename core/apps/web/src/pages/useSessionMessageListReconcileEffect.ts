import {
  useLayoutEffect,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from "react";
import type {
  AutoscrollToBottom,
  ItemLocation,
  VirtuosoMessageListMethods,
} from "@virtuoso.dev/message-list";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";
import type { WorkbenchThreadProjectionOp } from "./sessionThreadProjection";
import { debugItemSummary } from "./sessionMessageListDataDebug";
import { runSessionMessageListDevValidation } from "./sessionMessageListDevValidation";
import { logSessionMessageListReconcileDebug } from "./sessionMessageListReconcileDebug";
import {
  applyStableListUpdate,
  applyStructuralStableListUpdate,
} from "./sessionMessageListStableUpdate";
import {
  assertWholeListPurgeAllowed,
  computeHistoryPrependTailReconcilePlan,
  findSharedItemSizeCacheKeyChanges,
  haveSameItemIdSequence,
  isExactContiguousIdWindow,
  shouldReplaceBottomLockedStructuralUpdate,
  trimTrailingAppendsWhileScrolledUp,
} from "./sessionMessageListControllerUtils";

type MessageListMethods = VirtuosoMessageListMethods<
  WorkbenchListItem,
  WorkbenchMessageListContext
>;

type Params = {
  sessionId: string;
  isActive: boolean;
  listItems: WorkbenchListItem[];
  visibleListItems: WorkbenchListItem[];
  loadingOlder: boolean;
  deferTrailingAppends: boolean;
  layoutRevision: string;
  itemSizeCacheKey: (item: WorkbenchListItem) => string | null;
  threadOp?: WorkbenchThreadProjectionOp | null;
  showDebug: boolean;
  initialLocation: ItemLocation;
  appendBehavior: AutoscrollToBottom<WorkbenchListItem, WorkbenchMessageListContext>;
  methodsRef: MutableRefObject<MessageListMethods | null>;
  lastSessionIdRef: MutableRefObject<string>;
  contractViolationLoggedRef: MutableRefObject<{ sessionId: string; violationKey: string } | null>;
  lastScrollLocationRef: MutableRefObject<unknown>;
  lastAtBottomRef: MutableRefObject<boolean | null>;
  lastListOffsetRef: MutableRefObject<number | null>;
  stickToBottomRef: MutableRefObject<boolean>;
  renderedAnchorIdRef: MutableRefObject<string | null>;
  renderedTopIdRef: MutableRefObject<string | null>;
  firstListItemIdRef: MutableRefObject<string | null>;
  pendingHistoryRef: MutableRefObject<boolean>;
  historyExpectedRef: MutableRefObject<boolean>;
  historyRequestedAtTopRef: MutableRefObject<boolean>;
  historyRequestedAnchorIdRef: MutableRefObject<string | null>;
  lastLayoutRevisionRef: MutableRefObject<string>;
  reconcileEpochRef: MutableRefObject<number>;
  suppressIdDiffLogsRef: MutableRefObject<{ sessionId: string; remainingTicks: number } | null>;
  setLoadingOlder: Dispatch<SetStateAction<boolean>>;
  setDeferTrailingAppends: Dispatch<SetStateAction<boolean>>;
  snapToBottom: (methods: MessageListMethods) => void;
  recordDebugSnapshot: (cause: string, detail?: Record<string, unknown> | null) => void;
  startFlashProbe: (cause: string, detail?: Record<string, unknown> | null) => void;
  logMessageListDebug: (label: string, detail: Record<string, unknown>) => void;
};

function applyPrependDrivenHistoryUpdate({
  methods,
  current,
  retainedNext,
  prefix,
  suffix,
  stickToBottom,
  appendBehavior,
}: {
  methods: MessageListMethods;
  current: WorkbenchListItem[];
  retainedNext: WorkbenchListItem[];
  prefix: WorkbenchListItem[];
  suffix: WorkbenchListItem[];
  stickToBottom: boolean;
  appendBehavior: AutoscrollToBottom<WorkbenchListItem, WorkbenchMessageListContext>;
}) {
  applyStableListUpdate({
    methods,
    current,
    next: retainedNext,
    prefix,
    suffix,
    stickToBottom,
    anchorIndex: -1,
    appendBehavior,
    allowAnchorMap: false,
  });
}

export function useSessionMessageListReconcileEffect({
  sessionId,
  isActive,
  listItems,
  visibleListItems,
  loadingOlder,
  deferTrailingAppends,
  layoutRevision,
  itemSizeCacheKey,
  threadOp,
  showDebug,
  initialLocation,
  appendBehavior,
  methodsRef,
  lastSessionIdRef,
  contractViolationLoggedRef,
  lastScrollLocationRef,
  lastAtBottomRef,
  lastListOffsetRef,
  stickToBottomRef,
  renderedAnchorIdRef,
  renderedTopIdRef,
  firstListItemIdRef,
  pendingHistoryRef,
  historyExpectedRef,
  historyRequestedAtTopRef,
  historyRequestedAnchorIdRef,
  lastLayoutRevisionRef,
  reconcileEpochRef,
  suppressIdDiffLogsRef,
  setLoadingOlder,
  setDeferTrailingAppends,
  snapToBottom,
  recordDebugSnapshot,
  startFlashProbe,
  logMessageListDebug,
}: Params) {
  useLayoutEffect(() => {
    if (!isActive) return;
    const methods = methodsRef.current;
    if (!methods) return;
    const reconcileEpoch = ++reconcileEpochRef.current;

    const nextRaw = listItems;
    let next = visibleListItems;
    const current = methods.data.get();
    const sessionChanged = lastSessionIdRef.current !== sessionId;

    if (!sessionChanged) {
      const suppress = suppressIdDiffLogsRef.current;
      if (suppress && suppress.sessionId === sessionId && suppress.remainingTicks > 0) {
        suppress.remainingTicks -= 1;
        if (suppress.remainingTicks <= 0) suppressIdDiffLogsRef.current = null;
      }
    }

    runSessionMessageListDevValidation({
      sessionId,
      showDebug,
      nextRaw,
      current,
      next,
      contractViolationLoggedRef,
    });

    if (sessionChanged) {
      lastSessionIdRef.current = sessionId;
      lastLayoutRevisionRef.current = layoutRevision;
      pendingHistoryRef.current = false;
      historyExpectedRef.current = false;
      historyRequestedAtTopRef.current = false;
      historyRequestedAnchorIdRef.current = null;
      setLoadingOlder(false);
      if (deferTrailingAppends) setDeferTrailingAppends(false);
      lastScrollLocationRef.current = null;
      lastListOffsetRef.current = null;
      stickToBottomRef.current = true;
      lastAtBottomRef.current = true;
      renderedAnchorIdRef.current = null;
      renderedTopIdRef.current = null;
      firstListItemIdRef.current = null;
      methods.cancelSmoothScroll();
      suppressIdDiffLogsRef.current = { sessionId, remainingTicks: 3 };
      methods.data.replace(next);
      snapToBottom(methods);
      recordDebugSnapshot("data:replace", {
        reason: "sessionChanged",
        nextLen: next.length,
        currentLen: current.length,
      });
      logMessageListDebug("data:replace", {
        reason: "sessionChanged",
        nextLen: next.length,
        currentLen: current.length,
      });
      return;
    }

    const nextLen = next.length;
    const currentLen = current.length;
    const currentIds = current.map((it) => it.id);
    const shouldDeferTrailingAppends =
      currentLen > 0 &&
      !stickToBottomRef.current &&
      (deferTrailingAppends ||
        historyExpectedRef.current ||
        pendingHistoryRef.current ||
        loadingOlder);
    if (shouldDeferTrailingAppends) {
      const trimmedNext = trimTrailingAppendsWhileScrolledUp(currentIds, next);
      if (trimmedNext.length !== next.length) {
        next = trimmedNext;
        if (!deferTrailingAppends) setDeferTrailingAppends(true);
        recordDebugSnapshot("data:deferTrailingAppends", {
          nextLen,
          trimmedLen: trimmedNext.length,
        });
        logMessageListDebug("data:deferTrailingAppends", {
          nextLen,
          trimmedLen: trimmedNext.length,
          currentLen,
        });
      }
    }
    const nextIds = next.map((it) => it.id);
    const effectiveNextLen = next.length;
    const layoutRevisionChanged = lastLayoutRevisionRef.current !== layoutRevision;
    const sameIdSequence = haveSameItemIdSequence(current, next);
    const hasLocalizedThreadOp =
      Boolean(threadOp) &&
      threadOp?.kind !== "noop" &&
      threadOp?.kind !== "replace_session" &&
      sameIdSequence;

    if (currentLen === 0) {
      if (next.length === 0) return;
      lastLayoutRevisionRef.current = layoutRevision;
      historyExpectedRef.current = false;
      methods.cancelSmoothScroll();
      suppressIdDiffLogsRef.current = { sessionId, remainingTicks: 2 };
      methods.data.replace(next, { initialLocation, purgeItemSizes: false });
      snapToBottom(methods);
      recordDebugSnapshot("data:replace", {
        reason: "initialPopulation",
        nextLen: next.length,
        currentLen,
      });
      logMessageListDebug("data:replace", {
        reason: "initialPopulation",
        nextLen: next.length,
        currentLen,
      });
      return;
    }

    if (effectiveNextLen === 0) {
      lastLayoutRevisionRef.current = layoutRevision;
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

    if (layoutRevisionChanged) {
      lastLayoutRevisionRef.current = layoutRevision;
      historyExpectedRef.current = false;
      historyRequestedAtTopRef.current = false;
      historyRequestedAnchorIdRef.current = null;
      if (threadOp && threadOp.kind !== "replace_session") {
        recordDebugSnapshot("data:layout-op", {
          reason: threadOp?.kind ?? "unknown",
          nextLen: effectiveNextLen,
          currentLen,
          remeasureCount: threadOp?.remeasureItemIds.length ?? 0,
        });
      } else {
        const atBottom = stickToBottomRef.current;
        const purgeAnchorId = atBottom ? null : renderedTopIdRef.current ?? renderedAnchorIdRef.current;
        const purgeAnchorIndex = purgeAnchorId ? next.findIndex((item) => item.id === purgeAnchorId) : -1;
        const replaceLocation: ItemLocation =
          atBottom
            ? initialLocation
            : purgeAnchorIndex >= 0
              ? { index: purgeAnchorIndex, align: "start" }
              : initialLocation;
        assertWholeListPurgeAllowed({ reason: "layoutRevisionChanged", threadOp });
        methods.cancelSmoothScroll();
        suppressIdDiffLogsRef.current = { sessionId, remainingTicks: 1 };
        startFlashProbe("data:replace", {
          reason: "layoutRevisionChanged",
          layoutRevision,
          nextLen: effectiveNextLen,
          currentLen,
          atBottom,
          purgeAnchorId,
          purgeAnchorIndex,
        });
        methods.data.replace(next, { initialLocation: replaceLocation, purgeItemSizes: true });
        if (atBottom) {
          snapToBottom(methods);
        }
        recordDebugSnapshot("data:replace", {
          reason: "layoutRevisionChanged",
          layoutRevision,
          nextLen: effectiveNextLen,
          currentLen,
          atBottom,
          purgeAnchorId,
          purgeAnchorIndex,
        });
        logMessageListDebug("data:replace", {
          reason: "layoutRevisionChanged",
          layoutRevision,
          nextLen: effectiveNextLen,
          currentLen,
          atBottom,
          purgeAnchorId,
          purgeAnchorIndex,
        });
        return;
      }
    }

    const sizeCacheKeyChanges = sameIdSequence
      ? findSharedItemSizeCacheKeyChanges(current, next, itemSizeCacheKey)
      : { count: 0, sampleIds: [] as string[] };
    if (sizeCacheKeyChanges.count > 0 && !threadOp) {
      const atBottom = stickToBottomRef.current;
      const purgeAnchorId = atBottom ? null : renderedTopIdRef.current ?? renderedAnchorIdRef.current;
      const purgeAnchorIndex = purgeAnchorId ? next.findIndex((item) => item.id === purgeAnchorId) : -1;
      const replaceLocation: ItemLocation =
        atBottom
          ? initialLocation
          : purgeAnchorIndex >= 0
            ? { index: purgeAnchorIndex, align: "start" }
            : initialLocation;
      assertWholeListPurgeAllowed({ reason: "sizeCacheKeyChanged", threadOp });
      methods.cancelSmoothScroll();
      suppressIdDiffLogsRef.current = { sessionId, remainingTicks: 1 };
      startFlashProbe("data:replace", {
        reason: "sizeCacheKeyChanged",
        changedCount: sizeCacheKeyChanges.count,
        changedSampleIds: sizeCacheKeyChanges.sampleIds,
        nextLen: effectiveNextLen,
        currentLen,
        atBottom,
        purgeAnchorId,
        purgeAnchorIndex,
      });
      methods.data.replace(next, { initialLocation: replaceLocation, purgeItemSizes: true });
      if (atBottom) {
        snapToBottom(methods);
      }
      recordDebugSnapshot("data:replace", {
        reason: "sizeCacheKeyChanged",
        changedCount: sizeCacheKeyChanges.count,
        changedSampleIds: sizeCacheKeyChanges.sampleIds,
        nextLen: effectiveNextLen,
        currentLen,
        atBottom,
        purgeAnchorId,
        purgeAnchorIndex,
      });
      logMessageListDebug("data:replace", {
        reason: "sizeCacheKeyChanged",
        changedCount: sizeCacheKeyChanges.count,
        changedSampleIds: sizeCacheKeyChanges.sampleIds,
        nextLen: effectiveNextLen,
        currentLen,
        atBottom,
        purgeAnchorId,
        purgeAnchorIndex,
      });
      return;
    }

    if (historyExpectedRef.current && currentLen > 0 && effectiveNextLen >= currentLen) {
      const wasAtTop = historyRequestedAtTopRef.current;
      const requestedAnchorId = historyRequestedAnchorIdRef.current;
      const firstId = current[0]?.id ?? null;
      const lastId = current[currentLen - 1]?.id ?? null;
      const firstIndex = firstId ? next.findIndex((it) => it.id === firstId) : -1;
      const lastIndex = lastId ? next.findIndex((it) => it.id === lastId) : -1;
      const exactContiguousWindow = isExactContiguousIdWindow(currentIds, nextIds, firstIndex);
      if (firstIndex >= 0 && lastIndex >= firstIndex && exactContiguousWindow) {
        const retainedNext = next.slice(firstIndex, lastIndex + 1);
        let retainedMatchesCurrent = retainedNext.length === currentLen;
        if (retainedMatchesCurrent) {
          for (let index = 0; index < currentLen; index += 1) {
            if (retainedNext[index]?.id !== current[index]?.id) {
              retainedMatchesCurrent = false;
              break;
            }
          }
        }
        if (!retainedMatchesCurrent) {
          historyExpectedRef.current = false;
        } else {
          const currentIdSet = new Set(current.map((it) => it.id));
          const prefix = next.slice(0, firstIndex).filter((it) => !currentIdSet.has(it.id));
          const suffix = next.slice(lastIndex + 1).filter((it) => !currentIdSet.has(it.id));
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
          startFlashProbe("history:extend", {
            currentLen,
            nextLen: effectiveNextLen,
            prefixLen: prefix.length,
            suffixLen: suffix.length,
            firstIndex,
            lastIndex,
            requestedAnchorId,
            wasAtTop,
          });

          applyPrependDrivenHistoryUpdate({
            methods,
            current,
            retainedNext,
            prefix,
            suffix,
            stickToBottom: stickToBottomRef.current,
            appendBehavior,
          });

          historyExpectedRef.current = false;
          historyRequestedAtTopRef.current = false;
          historyRequestedAnchorIdRef.current = null;
          recordDebugSnapshot("history:extend", {
            prefixLen: prefix.length,
            suffixLen: suffix.length,
            firstIndex,
            lastIndex,
            nextLen: effectiveNextLen,
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
              nextLen: effectiveNextLen,
              currentLen,
            });
          }
          return;
        }
      }
      const mixedHistoryPlan = computeHistoryPrependTailReconcilePlan({
        currentIds,
        nextIds,
        startIndex: firstIndex,
        anchorId: renderedAnchorIdRef.current,
      });
      if (mixedHistoryPlan) {
        const nextById = new Map(next.map((it) => [it.id, it] as const));
        const prefix = next.slice(0, mixedHistoryPlan.prefixLen);
        const insertData = next.slice(
          mixedHistoryPlan.insertStart,
          mixedHistoryPlan.insertStart + mixedHistoryPlan.insertCount,
        );

        startFlashProbe("history:prepend-tail-reconcile", {
          currentLen,
          nextLen: effectiveNextLen,
          prefixLen: mixedHistoryPlan.prefixLen,
          overlapLen: mixedHistoryPlan.overlapLen,
          deleteOffset: mixedHistoryPlan.deleteOffset,
          deleteCount: mixedHistoryPlan.deleteCount,
          insertLen: insertData.length,
          suffixLen: mixedHistoryPlan.suffixLen,
          requestedAnchorId,
          wasAtTop,
        });

        methods.data.prepend(prefix);
        requestAnimationFrame(() => {
          if (reconcileEpochRef.current !== reconcileEpoch) return;
          const liveMethods = methodsRef.current;
          if (!liveMethods) return;
          if (mixedHistoryPlan.deleteCount > 0 || insertData.length > 0) {
            liveMethods.data.batch(
              () => {
                if (mixedHistoryPlan.deleteCount > 0) {
                  liveMethods.data.deleteRange(
                    mixedHistoryPlan.deleteOffset,
                    mixedHistoryPlan.deleteCount,
                  );
                }
                if (insertData.length > 0) {
                  liveMethods.data.insert(
                    insertData,
                    mixedHistoryPlan.deleteOffset,
                    appendBehavior,
                  );
                }
                liveMethods.data.map(
                  (item) => nextById.get(item.id) ?? item,
                  stickToBottomRef.current ? ("auto" as const) : undefined,
                );
              },
              appendBehavior,
            );
            return;
          }
          liveMethods.data.map(
            (item) => nextById.get(item.id) ?? item,
            stickToBottomRef.current ? ("auto" as const) : undefined,
          );
        });

        historyExpectedRef.current = false;
        historyRequestedAtTopRef.current = false;
        historyRequestedAnchorIdRef.current = null;
        recordDebugSnapshot("history:prepend-tail-reconcile", {
          prefixLen: mixedHistoryPlan.prefixLen,
          overlapLen: mixedHistoryPlan.overlapLen,
          deleteOffset: mixedHistoryPlan.deleteOffset,
          deleteCount: mixedHistoryPlan.deleteCount,
          insertLen: insertData.length,
          suffixLen: mixedHistoryPlan.suffixLen,
          nextLen: effectiveNextLen,
          currentLen,
          requestedAnchorId,
          wasAtTop,
        });
        if (import.meta.env.DEV && showDebug) {
          // eslint-disable-next-line no-console
          console.debug("[MessageList][history:prepend-tail-reconcile]", {
            sessionId,
            prefixLen: mixedHistoryPlan.prefixLen,
            overlapLen: mixedHistoryPlan.overlapLen,
            deleteOffset: mixedHistoryPlan.deleteOffset,
            deleteCount: mixedHistoryPlan.deleteCount,
            insertLen: insertData.length,
            suffixLen: mixedHistoryPlan.suffixLen,
            nextLen: effectiveNextLen,
            currentLen,
          });
        }
        return;
      }
      if (import.meta.env.DEV && showDebug && firstIndex >= 0 && lastIndex >= firstIndex) {
        // eslint-disable-next-line no-console
        console.debug("[MessageList][history:extend:skipped]", {
          sessionId,
          reason: "nonContiguousWindow",
          firstIndex,
          lastIndex,
          nextLen: effectiveNextLen,
          currentLen,
          requestedAnchorId,
          wasAtTop,
        });
      }
    }

    if (effectiveNextLen > currentLen) {
      let isPurePrepend = true;
      for (let i = 0; i < currentLen; i += 1) {
        if (next[effectiveNextLen - currentLen + i]?.id !== current[i]?.id) {
          isPurePrepend = false;
          break;
        }
      }
      if (isPurePrepend) {
        const wasAtTop = historyRequestedAtTopRef.current;
        const requestedAnchorId = historyRequestedAnchorIdRef.current;
        const prefix = next.slice(0, effectiveNextLen - currentLen);
        const anchorId = renderedAnchorIdRef.current;
        const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;

        startFlashProbe("data:prepend", {
          currentLen,
          nextLen: effectiveNextLen,
          prefixLen: prefix.length,
          anchorId,
          anchorIndex,
          requestedAnchorId,
          wasAtTop,
        });

        applyPrependDrivenHistoryUpdate({
          methods,
          current,
          retainedNext: next.slice(effectiveNextLen - currentLen),
          prefix,
          suffix: [],
          stickToBottom: stickToBottomRef.current,
          appendBehavior,
        });

        recordDebugSnapshot("data:prepend", {
          prefixLen: prefix.length,
          nextLen: effectiveNextLen,
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
            nextLen: effectiveNextLen,
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

    if (effectiveNextLen > currentLen) {
      let isPureAppend = true;
      for (let i = 0; i < currentLen; i += 1) {
        if (next[i]?.id !== current[i]?.id) {
          isPureAppend = false;
          break;
        }
      }
      if (isPureAppend) {
        const suffix = next.slice(currentLen);
        const anchorId = renderedAnchorIdRef.current;
        const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;
        const updateResult = applyStableListUpdate({
          methods,
          current,
          next: next.slice(0, currentLen),
          suffix,
          stickToBottom: stickToBottomRef.current,
          anchorIndex,
          appendBehavior,
        });
        if (stickToBottomRef.current && updateResult.mode === "remeasure") {
          snapToBottom(methods);
        }
        recordDebugSnapshot("data:append", {
          suffixLen: suffix.length,
          nextLen: effectiveNextLen,
          currentLen,
          changedSpans: updateResult.changedSpans,
        });
        logMessageListDebug("data:append", {
          suffixLen: suffix.length,
          nextLen: effectiveNextLen,
          currentLen,
          stickToBottom: stickToBottomRef.current,
          anchorId,
          changedSpans: updateResult.changedSpans,
        });
        return;
      }
    }

    if (effectiveNextLen === currentLen) {
      let same = true;
      for (let i = 0; i < currentLen; i += 1) {
        if (next[i]?.id !== current[i]?.id) {
          same = false;
          break;
        }
      }
      if (same) {
        const anchorId = renderedAnchorIdRef.current;
        const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;
        const updateResult = applyStableListUpdate({
          methods,
          current,
          next,
          stickToBottom: stickToBottomRef.current,
          anchorIndex,
          appendBehavior,
          forceRemeasureItemIds: hasLocalizedThreadOp ? (threadOp?.remeasureItemIds ?? []) : [],
        });
        if (stickToBottomRef.current && updateResult.mode === "remeasure") {
          snapToBottom(methods);
        }
        const updateLabel = updateResult.mode === "remeasure" ? "data:remeasure" : "data:map";
        if (updateResult.mode === "remeasure") {
          startFlashProbe("data:remeasure", {
            nextLen: effectiveNextLen,
            currentLen,
            anchorId,
            anchorIndex,
            stickToBottom: stickToBottomRef.current,
            changedSpans: updateResult.changedSpans,
          });
        }
        if (import.meta.env.DEV && showDebug) {
          let changedByRef = 0;
          const sampleChangedIds: string[] = [];
          for (let i = 0; i < currentLen; i += 1) {
            if (current[i] !== next[i]) {
              changedByRef += 1;
              if (sampleChangedIds.length < 8) {
                sampleChangedIds.push(String(next[i]?.id ?? current[i]?.id ?? ""));
              }
            }
          }
          const mapMode =
            updateResult.mode === "remeasure"
              ? "batch:remeasure"
              : !stickToBottomRef.current && anchorIndex >= 0
                ? "mapWithAnchor"
                : stickToBottomRef.current
                  ? "map:auto"
                  : "map";
          // eslint-disable-next-line no-console
          console.debug(`[MessageList][${updateLabel}]`, {
            sessionId,
            nextLen: effectiveNextLen,
            currentLen,
            stickToBottom: stickToBottomRef.current,
            anchorId,
            anchorIndex,
            mapMode,
            changedByRef,
            sampleChangedIds,
            changedSpans: updateResult.changedSpans,
            renderedTopId: renderedTopIdRef.current,
          });
        }
        recordDebugSnapshot(updateLabel, {
          nextLen: effectiveNextLen,
          currentLen,
          anchorId,
          anchorIndex,
          stickToBottom: stickToBottomRef.current,
          changedSpans: updateResult.changedSpans,
        });
        logMessageListDebug(updateLabel, {
          nextLen: effectiveNextLen,
          currentLen,
          anchorId,
          anchorIndex,
          stickToBottom: stickToBottomRef.current,
          changedSpans: updateResult.changedSpans,
        });
        return;
      }
    }

    let prefixLen = 0;
    while (
      prefixLen < currentLen &&
      prefixLen < effectiveNextLen &&
      currentIds[prefixLen] === nextIds[prefixLen]
    ) {
      prefixLen += 1;
    }
    let suffixLen = 0;
    while (
      suffixLen < currentLen - prefixLen &&
      suffixLen < effectiveNextLen - prefixLen &&
      currentIds[currentLen - 1 - suffixLen] === nextIds[effectiveNextLen - 1 - suffixLen]
    ) {
      suffixLen += 1;
    }

    const deleteCount = currentLen - prefixLen - suffixLen;
    const insertData = next.slice(prefixLen, effectiveNextLen - suffixLen);
    const anchorId = renderedAnchorIdRef.current;
    const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;

    const suppress = suppressIdDiffLogsRef.current;
    const suppressIdDiffLogs = Boolean(
      suppress && suppress.sessionId === sessionId && suppress.remainingTicks > 0,
    );
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
      nextLen: effectiveNextLen,
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

    const replaceBottomLockedStructuralUpdate = shouldReplaceBottomLockedStructuralUpdate({
      stickToBottom: stickToBottomRef.current,
      currentLen,
      nextLen: effectiveNextLen,
      prefixLen,
      suffixLen,
      deleteCount,
      insertCount: insertData.length,
    });
    if (replaceBottomLockedStructuralUpdate) {
      assertWholeListPurgeAllowed({ reason: "bottomLockedStructuralReconcile", threadOp });
      methods.cancelSmoothScroll();
      suppressIdDiffLogsRef.current = { sessionId, remainingTicks: 1 };
      startFlashProbe("data:replace", {
        reason: "bottomLockedStructuralReconcile",
        currentLen,
        nextLen: effectiveNextLen,
        prefixLen,
        suffixLen,
        deleteCount,
        insertLen: insertData.length,
        anchorId,
        anchorIndex,
        historyExpected,
        stickToBottom: stickToBottomRef.current,
      });
      methods.data.replace(next, { initialLocation, purgeItemSizes: true });
      snapToBottom(methods);
      recordDebugSnapshot("data:replace", {
        reason: "bottomLockedStructuralReconcile",
        nextLen: effectiveNextLen,
        currentLen,
        prefixLen,
        suffixLen,
        deleteCount,
        insertLen: insertData.length,
        anchorId,
        anchorIndex,
        historyExpected,
        stickToBottom: stickToBottomRef.current,
      });
      logMessageListDebug("data:replace", {
        reason: "bottomLockedStructuralReconcile",
        nextLen: effectiveNextLen,
        currentLen,
        prefixLen,
        suffixLen,
        deleteCount,
        insertLen: insertData.length,
        anchorId,
        anchorIndex,
        historyExpected,
        stickToBottom: stickToBottomRef.current,
      });
      return;
    }

    startFlashProbe("data:reconcile", {
      currentLen,
      nextLen: effectiveNextLen,
      prefixLen,
      suffixLen,
      deleteCount,
      insertLen: insertData.length,
      anchorId,
      anchorIndex,
      historyExpected,
      stickToBottom: stickToBottomRef.current,
    });

    const updateResult = applyStructuralStableListUpdate({
      methods,
      current,
      next,
      prefixLen,
      suffixLen,
      stickToBottom: stickToBottomRef.current,
      anchorIndex,
      appendBehavior,
      forceRemeasureItemIds: hasLocalizedThreadOp ? (threadOp?.remeasureItemIds ?? []) : [],
    });
    if (stickToBottomRef.current) {
      snapToBottom(methods);
    }
    recordDebugSnapshot("data:reconcile", {
      nextLen: effectiveNextLen,
      currentLen,
      prefixLen,
      suffixLen,
      deleteCount,
      insertLen: insertData.length,
      anchorId,
      anchorIndex,
      stickToBottom: stickToBottomRef.current,
      changedSpans: updateResult.changedSpans,
    });
  }, [
    appendBehavior,
    contractViolationLoggedRef,
    deferTrailingAppends,
    firstListItemIdRef,
    historyExpectedRef,
    historyRequestedAnchorIdRef,
    historyRequestedAtTopRef,
    initialLocation,
    isActive,
    itemSizeCacheKey,
    lastAtBottomRef,
    lastLayoutRevisionRef,
    lastListOffsetRef,
    lastScrollLocationRef,
    layoutRevision,
    listItems,
    loadingOlder,
    logMessageListDebug,
    methodsRef,
    pendingHistoryRef,
    reconcileEpochRef,
    recordDebugSnapshot,
    renderedAnchorIdRef,
    renderedTopIdRef,
    sessionId,
    setDeferTrailingAppends,
    setLoadingOlder,
    showDebug,
    snapToBottom,
    startFlashProbe,
    stickToBottomRef,
    suppressIdDiffLogsRef,
    threadOp,
    visibleListItems,
  ]);
}
