import { useCallback, useEffect, useRef, type MutableRefObject } from "react";
import type { VirtuosoMessageListMethods } from "@virtuoso.dev/message-list";
import { recordSessionMessageListDebugSnapshot } from "./sessionMessageListDebug";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";

type Params = {
  sessionId: string;
  isActive: boolean;
  loaded: boolean;
  listItemsLength: number;
  showDebug: boolean;
  methodsRef: MutableRefObject<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>;
  lastAtBottomRef: MutableRefObject<boolean | null>;
  renderedAnchorIdRef: MutableRefObject<string | null>;
  renderedTopIdRef: MutableRefObject<string | null>;
};

export function useSessionMessageListDiagnostics({
  sessionId,
  isActive,
  loaded,
  listItemsLength,
  showDebug,
  methodsRef,
  lastAtBottomRef,
  renderedAnchorIdRef,
  renderedTopIdRef,
}: Params) {
  const debugLoggedSessionRef = useRef<string | null>(null);

  const recordDebugSnapshot = useCallback(
    (cause: string, detail?: Record<string, unknown> | null) => {
      if (!showDebug) return;
      recordSessionMessageListDebugSnapshot({
        sessionId,
        cause,
        scroller: methodsRef.current?.scrollerElement?.() ?? null,
        isActive,
        loaded,
        listCount: listItemsLength,
        stickToBottom: lastAtBottomRef.current,
        renderedAnchorId: renderedAnchorIdRef.current,
        renderedTopId: renderedTopIdRef.current,
        detail: detail ?? null,
      });
    },
    [
      isActive,
      lastAtBottomRef,
      listItemsLength,
      loaded,
      methodsRef,
      renderedAnchorIdRef,
      renderedTopIdRef,
      sessionId,
      showDebug,
    ],
  );

  useEffect(() => {
    if (!showDebug) return;
    if (debugLoggedSessionRef.current === sessionId) return;
    debugLoggedSessionRef.current = sessionId;
    // eslint-disable-next-line no-console
    console.debug("[MessageList][debug]", { sessionId, isActive, loaded });
  }, [isActive, loaded, sessionId, showDebug]);

  useEffect(() => {
    recordDebugSnapshot("session:init");
  }, [recordDebugSnapshot, sessionId]);

  useEffect(() => {
    if (!showDebug || !isActive) return;
    let cancelled = false;
    let rafId: number | null = null;
    let cleanup: (() => void) | null = null;

    const attach = () => {
      if (cancelled) return;
      const scroller = methodsRef.current?.scrollerElement?.() ?? null;
      if (!scroller) {
        rafId = requestAnimationFrame(attach);
        return;
      }
      const sessionView = scroller.closest(".wb-session-view") as HTMLElement | null;
      const sessionBottom = sessionView?.querySelector(".wb-session-bottom") as HTMLElement | null;
      const composer = sessionView?.querySelector(".wb-active-composer") as HTMLElement | null;
      const queuePanel = sessionView?.querySelector(".queue-panel") as HTMLElement | null;
      const observer = new ResizeObserver((entries) => {
        for (const entry of entries) {
          const target = entry.target;
          const targetLabel =
            target === scroller
              ? "scroller"
              : target === sessionBottom
                ? "session-bottom"
                : target === composer
                  ? "composer"
                  : target === queuePanel
                    ? "queue-panel"
                    : "unknown";
          recordDebugSnapshot(`resize:${targetLabel}`, {
            width: Math.round(entry.contentRect.width * 100) / 100,
            height: Math.round(entry.contentRect.height * 100) / 100,
          });
        }
      });
      observer.observe(scroller);
      if (sessionBottom) observer.observe(sessionBottom);
      if (composer) observer.observe(composer);
      if (queuePanel) observer.observe(queuePanel);
      recordDebugSnapshot("debug:attach");
      cleanup = () => observer.disconnect();
    };

    attach();
    return () => {
      cancelled = true;
      if (rafId != null) cancelAnimationFrame(rafId);
      recordDebugSnapshot("debug:detach");
      cleanup?.();
    };
  }, [isActive, methodsRef, recordDebugSnapshot, sessionId, showDebug]);

  return recordDebugSnapshot;
}
