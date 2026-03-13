import { useCallback, useLayoutEffect, useRef, type MutableRefObject } from "react";
import type { VirtuosoMessageListMethods } from "@virtuoso.dev/message-list";
import { recordSessionMessageListDebugSnapshot } from "./sessionMessageListDebug";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";

type ScrollState = {
  stickToBottom: boolean;
  anchorItemId: string | null;
  anchorOffset: number | null;
  scrollTop: number | null;
};

type Params = {
  sessionId: string;
  isActive: boolean;
  loaded: boolean;
  preserveScrollOnFocus: boolean;
  scrollState: ScrollState | null | undefined;
  listItemsRef: MutableRefObject<WorkbenchListItem[]>;
  methodsRef: MutableRefObject<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  showDebug: boolean;
};

export function useSessionScrollRestoreOnActivate({
  sessionId,
  isActive,
  loaded,
  preserveScrollOnFocus,
  scrollState,
  listItemsRef,
  methodsRef,
  showDebug,
}: Params) {
  const restoreTokenRef = useRef(0);
  const wasActiveRef = useRef(false);

  const recordRestoreDebug = useCallback(
    (cause: string, detail?: Record<string, unknown> | null) => {
      if (!showDebug) return;
      recordSessionMessageListDebugSnapshot({
        sessionId,
        cause,
        scroller: methodsRef.current?.scrollerElement?.() ?? null,
        isActive,
        loaded,
        listCount: listItemsRef.current.length,
        stickToBottom: scrollState?.stickToBottom ?? null,
        renderedAnchorId: scrollState?.anchorItemId ?? null,
        renderedTopId: scrollState?.anchorItemId ?? null,
        detail: detail ?? null,
      });
      // eslint-disable-next-line no-console
      console.log(`[MessageList][restore] ${JSON.stringify({ sessionId, cause, detail: detail ?? null })}`);
    },
    [isActive, listItemsRef, loaded, methodsRef, scrollState, sessionId, showDebug],
  );

  useLayoutEffect(() => {
    if (!isActive) {
      wasActiveRef.current = false;
      restoreTokenRef.current += 1;
      return;
    }
    if (preserveScrollOnFocus) return;
    if (wasActiveRef.current) return;
    wasActiveRef.current = true;
    const token = (restoreTokenRef.current += 1);
    if (!scrollState || scrollState.stickToBottom) return;
    recordRestoreDebug("restore:activate", {
      persistedScrollTop: scrollState.scrollTop ?? null,
      persistedAnchorItemId: scrollState.anchorItemId ?? null,
      persistedAnchorOffset: scrollState.anchorOffset ?? null,
    });

    const applyScroll = (attempts: number) => {
      if (restoreTokenRef.current !== token) return;
      const methods = methodsRef.current;
      if (!methods) {
        if (attempts === 0 || attempts === 30 || attempts === 90) {
          recordRestoreDebug("restore:await-methods", { attempts });
        }
        if (attempts < 120) requestAnimationFrame(() => applyScroll(attempts + 1));
        return;
      }
      const scroller = methods.scrollerElement?.() ?? null;
      const anchorId = scrollState.anchorItemId;
      if (anchorId) {
        const index = listItemsRef.current.findIndex((item) => item.id === anchorId);
        if (index >= 0) {
          let anchorOffset = scrollState.anchorOffset;
          if (scroller) {
            const maxOffset = scroller.clientHeight;
            if (anchorOffset != null) {
              anchorOffset = Math.min(maxOffset, Math.max(0, anchorOffset));
            }
          }
          recordRestoreDebug("restore:anchor", {
            attempts,
            anchorId,
            anchorOffset: anchorOffset ?? null,
            index,
            targetScrollTop: scrollState.scrollTop ?? null,
          });
          methods.scrollToItem(
            anchorOffset != null
              ? { index, align: "start", behavior: "instant", offset: anchorOffset }
              : { index, align: "start", behavior: "instant" },
          );
          return;
        }
      }
      if (preserveScrollOnFocus) {
        if (attempts === 0 || attempts === 30 || attempts === 90) {
          recordRestoreDebug("restore:await-anchor", {
            attempts,
            anchorId: scrollState.anchorItemId ?? null,
            listCount: listItemsRef.current.length,
          });
        }
        if (attempts < 120) requestAnimationFrame(() => applyScroll(attempts + 1));
        return;
      }
      if (scroller && scrollState.scrollTop != null) {
        const delta = Math.abs(scroller.scrollTop - scrollState.scrollTop);
        if (delta <= 2) {
          recordRestoreDebug("restore:already-at-target", {
            attempts,
            mode: "scrollTop",
            delta,
            actualScrollTop: scroller.scrollTop,
            targetScrollTop: scrollState.scrollTop,
          });
          return;
        }
      }
      if (scroller && scrollState.scrollTop != null) {
        scroller.scrollTop = scrollState.scrollTop;
        recordRestoreDebug("restore:scrollTop", {
          attempts,
          targetScrollTop: scrollState.scrollTop,
          actualScrollTop: scroller.scrollTop,
        });
        return;
      }
      if (attempts === 0 || attempts === 30 || attempts === 90) {
        recordRestoreDebug("restore:retry-no-target", { attempts });
      }
      if (attempts < 120) requestAnimationFrame(() => applyScroll(attempts + 1));
    };

    applyScroll(0);
  }, [isActive, listItemsRef, methodsRef, preserveScrollOnFocus, recordRestoreDebug, scrollState]);
}
