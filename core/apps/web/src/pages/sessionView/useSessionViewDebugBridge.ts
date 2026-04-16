import { useEffect, useRef } from "react";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { SessionThreadProjection } from "../../state/sessionThreadProjection/selectors";
import type { SessionCacheEntry } from "../../state/sessionSupervisor/entryState";

type Params = {
  sessionId: string;
  entry: SessionCacheEntry | null | undefined;
  listItems: WorkbenchListItem[];
  threadProjection: SessionThreadProjection;
  perfEnabled: boolean;
};

export function useSessionViewDebugBridge({
  sessionId,
  entry,
  listItems,
  threadProjection,
  perfEnabled,
}: Params): void {
  const perfStartRef = useRef<number>(0);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const bridgeTarget = window as Window & {
      __ctxE2E?: {
        getVisibleSessionEntryDebug?: () => {
          sessionId: string | null;
          lastEventSeq: number | null;
          messageContents: string[];
        };
      };
    };
    if (!bridgeTarget.__ctxE2E) return;
    bridgeTarget.__ctxE2E.getVisibleSessionEntryDebug = () => ({
      sessionId: sessionId ?? null,
      lastEventSeq: typeof entry?.lastEventSeq === "number" ? entry.lastEventSeq : null,
      messageContents: Array.isArray(entry?.messages) ? entry.messages.map((message) => message.content) : [],
    });
    return () => {
      if (!bridgeTarget.__ctxE2E) return;
      delete bridgeTarget.__ctxE2E.getVisibleSessionEntryDebug;
    };
  }, [entry?.lastEventSeq, entry?.messages, sessionId]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const bridgeTarget = window as Window & {
      __ctxE2E?: {
        getVisibleSessionThreadDebug?: () => {
          sessionId: string | null;
          projectionRev: number;
          turnsStamp: string;
          messagesStamp: string;
          listItemIds: string[];
          assistantContents: string[];
        };
      };
    };
    if (!bridgeTarget.__ctxE2E) return;
    bridgeTarget.__ctxE2E.getVisibleSessionThreadDebug = () => ({
      sessionId: sessionId ?? null,
      projectionRev: threadProjection.projectionRev,
      turnsStamp: threadProjection.turnsStamp,
      messagesStamp: threadProjection.messagesStamp,
      listItemIds: listItems.map((item) => item.id),
      assistantContents: listItems
        .filter((item): item is Extract<WorkbenchListItem, { kind: "assistant" }> => item.kind === "assistant")
        .map((item) => item.content),
    });
    return () => {
      if (!bridgeTarget.__ctxE2E) return;
      delete bridgeTarget.__ctxE2E.getVisibleSessionThreadDebug;
    };
  }, [
    listItems,
    sessionId,
    threadProjection.messagesStamp,
    threadProjection.projectionRev,
    threadProjection.turnsStamp,
  ]);

  useEffect(() => {
    if (!perfEnabled) return;
    perfStartRef.current = performance.now();
  }, [perfEnabled, sessionId]);

  useEffect(() => {
    if (!perfEnabled) return;
    if (!perfStartRef.current) return;
    if (!entry) return;
    if (entry.loading) return;
    // eslint-disable-next-line no-console
    console.log(
      `[perf] session_ready_ms=${(performance.now() - perfStartRef.current).toFixed(1)} events=${entry.events.length} diff_bytes=${(entry.diff ?? "").length}`,
    );
    perfStartRef.current = 0;
  }, [entry, perfEnabled]);
}
