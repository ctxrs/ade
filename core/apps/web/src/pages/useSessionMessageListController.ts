import { useCallback, useLayoutEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import type {
  AutoscrollToBottom,
  ItemLocation,
  ListScrollLocation,
  VirtuosoMessageListMethods,
} from "@virtuoso.dev/message-list";
import { useRafCoalesced } from "../components/hooks/useRafCoalesced";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";

function debugStableKey(item: WorkbenchListItem): string {
  // Best-effort "identity" key independent of `item.id` to detect id churn.
  // This is DEV-only diagnostics; collisions are possible but still useful.
  const kind = (item as any)?.kind ?? "unknown";
  switch (kind) {
    case "turn_header":
      return `turn_header:${(item as any)?.header?.id ?? ""}`;
    case "tool":
      return `tool:${(item as any)?.tool_call_id ?? ""}`;
    case "ask_user_question":
      return `askq:${(item as any)?.tool_call_id ?? ""}`;
    case "turn_status":
      return `turn_status:${(item as any)?.turn_id ?? ""}`;
    case "thought":
      return `thought:${(item as any)?.turn_id ?? ""}:${(item as any)?.created_at ?? ""}`;
    case "assistant":
      // Prefer turn_id + created_at, but also include the item id prefix if it encodes a domain id (e.g. assistant-msg-<messageId>).
      // This is intentionally "best effort"; we also log direct missing/added ids during reconcile.
      return `assistant:${(item as any)?.turn_id ?? ""}:${(item as any)?.created_at ?? ""}:${String(
        (item as any)?.is_complete ?? "",
      )}:${String((item as any)?.id ?? "").slice(0, 40)}`;
    case "message":
      return `message:${(item as any)?.role ?? ""}:${(item as any)?.created_at ?? ""}`;
    case "spacer":
      return `spacer:${(item as any)?.created_at ?? ""}`;
    default:
      return `${kind}:${(item as any)?.created_at ?? ""}`;
  }
}

function debugItemSummary(item: WorkbenchListItem): Record<string, any> {
  const anyItem: any = item as any;
  const kind = anyItem?.kind ?? "unknown";
  const base: Record<string, any> = {
    id: String(anyItem?.id ?? ""),
    kind,
    created_at: anyItem?.created_at ?? anyItem?.header?.created_at ?? null,
  };

  if (kind === "turn_header") {
    base.turn_id = anyItem?.header?.id ?? null;
    return base;
  }
  if (typeof anyItem?.turn_id === "string") base.turn_id = anyItem.turn_id;
  if (typeof anyItem?.tool_call_id === "string") base.tool_call_id = anyItem.tool_call_id;
  if (typeof anyItem?.event_id === "string") base.event_id = anyItem.event_id;
  if (typeof anyItem?.status === "string") base.status = anyItem.status;
  if (typeof anyItem?.role === "string") base.role = anyItem.role;
  if (typeof anyItem?.is_complete === "boolean") base.is_complete = anyItem.is_complete;
  if (typeof anyItem?.content === "string") base.content_len = anyItem.content.length;
  return base;
}

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
  initialLocation: ItemLocation;
  onScroll: (location: ListScrollLocation) => void;
  onRenderedDataChange: (range: WorkbenchListItem[]) => void;
};

const INITIAL_LOCATION_BOTTOM: ItemLocation = { index: "LAST", align: "end" };

export function useSessionMessageListController(params: Params): Result {
  const { sessionId, isActive, loaded, listItems, canLoadOlder, loadOlder, showDebug, onAtBottomChange } = params;

  const methodsRef = useRef<VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>(null);
  const lastSessionIdRef = useRef(sessionId);

  const didSeeScrollRef = useRef(false);
  const lastScrollLocationRef = useRef<ListScrollLocation | null>(null);
  const stickToBottomRef = useRef(true);
  const lastAtBottomRef = useRef<boolean | null>(null);

  // Best-effort anchoring based on rendered data (no DOM reads).
  const renderedAnchorIdRef = useRef<string | null>(null);

  const pendingHistoryRef = useRef(false);
  const historyExpectedRef = useRef(false);
  const [loadingOlder, setLoadingOlder] = useState(false);

  const listItemsCoalesced = useRafCoalesced(listItems);

  const context = useMemo(() => ({ loaded, loadingOlder }), [loaded, loadingOlder]);

  const onScroll = useCallback(
    (location: ListScrollLocation) => {
      lastScrollLocationRef.current = location;
      didSeeScrollRef.current = true;

      const atBottom = Boolean(location.isAtBottom);
      stickToBottomRef.current = atBottom;
      if (onAtBottomChange && lastAtBottomRef.current !== atBottom) {
        lastAtBottomRef.current = atBottom;
        onAtBottomChange(atBottom);
      }
    },
    [onAtBottomChange],
  );

  const onRenderedDataChange = useCallback(
    (range: WorkbenchListItem[]) => {
      renderedAnchorIdRef.current = range?.[0]?.id ?? null;
      if (!isActive) return;
      if (!loaded) return;
      if (!canLoadOlder) return;
      if (!didSeeScrollRef.current) return;
      if (stickToBottomRef.current) return;
      if (pendingHistoryRef.current || loadingOlder) return;

      const loc = lastScrollLocationRef.current;
      // `listOffset` is 0 at top and negative when scrolled down.
      const nearTop = loc?.listOffset != null ? loc.listOffset > -250 : false;
      if (!nearTop) return;

      pendingHistoryRef.current = true;
      historyExpectedRef.current = true;
      setLoadingOlder(true);
      if (import.meta.env.DEV && showDebug) {
        // eslint-disable-next-line no-console
        console.debug("[MessageList][history:request]", { sessionId, listOffset: loc?.listOffset ?? null });
      }
      loadOlder()
        .catch(() => {})
        .finally(() => {
          pendingHistoryRef.current = false;
          setLoadingOlder(false);
        });
    },
    [canLoadOlder, isActive, loaded, loadOlder, loadingOlder, sessionId, showDebug],
  );

  useLayoutEffect(() => {
    if (!isActive) return;
    const methods = methodsRef.current;
    if (!methods) return;

    const next = listItemsCoalesced;
    const current = methods.data.get();
    const sessionChanged = lastSessionIdRef.current !== sessionId;

    if (import.meta.env.DEV && showDebug) {
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
        const id = String((it as any)?.id ?? "");
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
        const id = String((it as any)?.id ?? "");
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
          fromItem: debugItemSummary(currentById.get(c.from) ?? ({ id: c.from } as any)),
          toItem: debugItemSummary(nextById.get(c.to) ?? ({ id: c.to } as any)),
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

    const appendBehavior: AutoscrollToBottom<WorkbenchListItem, WorkbenchMessageListContext> = (params) =>
      params.atBottom ? "auto" : false;

    if (sessionChanged) {
      lastSessionIdRef.current = sessionId;
      pendingHistoryRef.current = false;
      historyExpectedRef.current = false;
      setLoadingOlder(false);
      methods.data.replace(next, { initialLocation: INITIAL_LOCATION_BOTTOM, purgeItemSizes: true });
      if (import.meta.env.DEV && showDebug) {
        // eslint-disable-next-line no-console
        console.debug("[MessageList][data:replace]", { sessionId, nextLen: next.length, reason: "sessionChanged" });
      }
      return;
    }

    const nextLen = next.length;
    const currentLen = current.length;

    // Initial population: never treat empty->non-empty as prepend/append.
    // Use `replace(..., initialLocation: LAST)` so opening a session lands at bottom deterministically.
    if (currentLen === 0) {
      if (nextLen === 0) return;
      historyExpectedRef.current = false;
      methods.data.replace(next, { initialLocation: INITIAL_LOCATION_BOTTOM, purgeItemSizes: true });
      if (import.meta.env.DEV && showDebug) {
        // eslint-disable-next-line no-console
        console.debug("[MessageList][data:replace]", { sessionId, nextLen, currentLen, reason: "initialPopulation" });
      }
      return;
    }

    if (nextLen === 0) {
      historyExpectedRef.current = false;
      methods.data.deleteRange(0, currentLen);
      if (import.meta.env.DEV && showDebug) {
        // eslint-disable-next-line no-console
        console.debug("[MessageList][data:deleteRange]", { sessionId, offset: 0, count: currentLen });
      }
      return;
    }

    // If we just requested history and the next update is not a pure prepend (e.g. streaming appended too),
    // apply it as an extension update instead of falling back to `replace()`.
    if (historyExpectedRef.current && currentLen > 0 && nextLen >= currentLen) {
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
                debugItemSummary(currentById.get(id) ?? ({ id } as any)),
              ),
            });
          }
        }

        methods.data.batch(() => {
          if (prefix.length > 0) methods.data.prepend(prefix);
          if (suffix.length > 0) methods.data.append(suffix, appendBehavior);
          const anchorId = renderedAnchorIdRef.current;
          const anchorIndex = anchorId ? next.findIndex((it) => it.id === anchorId) : -1;
          if (!stickToBottomRef.current && anchorIndex >= 0) {
            methods.data.mapWithAnchor((item) => nextById.get(item.id) ?? item, anchorIndex);
          } else {
            methods.data.map(
              (item) => nextById.get(item.id) ?? item,
              stickToBottomRef.current ? ("auto" as const) : undefined,
            );
          }
        });
        historyExpectedRef.current = false;
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
        const prefix = next.slice(0, nextLen - currentLen);
        if (prefix.length > 0) methods.data.prepend(prefix);
        if (import.meta.env.DEV && showDebug) {
          // eslint-disable-next-line no-console
          console.debug("[MessageList][data:prepend]", { sessionId, prefixLen: prefix.length, nextLen, currentLen });
        }
        if (historyExpectedRef.current) {
          historyExpectedRef.current = false;
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
        if (import.meta.env.DEV && showDebug) {
          // eslint-disable-next-line no-console
          console.debug("[MessageList][data:append]", { sessionId, suffixLen: suffix.length, nextLen, currentLen });
        }
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
        if (!stickToBottomRef.current && anchorIndex >= 0) {
          methods.data.mapWithAnchor((item) => nextById.get(item.id) ?? item, anchorIndex);
        } else {
          methods.data.map(
            (item) => nextById.get(item.id) ?? item,
            stickToBottomRef.current ? ("auto" as const) : undefined,
          );
        }
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

    if (import.meta.env.DEV && showDebug) {
      const nextIdSet = new Set(nextIds);
      const currentIdSet = new Set(currentIds);
      const missingFromNext: string[] = [];
      for (const id of currentIds) if (!nextIdSet.has(id)) missingFromNext.push(id);
      const addedInNext: string[] = [];
      for (const id of nextIds) if (!currentIdSet.has(id)) addedInNext.push(id);

      if (missingFromNext.length > 0) {
        const currentById = new Map(current.map((it) => [it.id, it] as const));
        // eslint-disable-next-line no-console
        console.warn("[MessageList][ids:missing-from-next]", {
          sessionId,
          count: missingFromNext.length,
          sample: missingFromNext.slice(0, 12).map((id) => debugItemSummary(currentById.get(id) ?? ({ id } as any))),
        });
      }
      if (addedInNext.length > 0) {
        const nextByIdLocal = new Map(next.map((it) => [it.id, it] as const));
        // eslint-disable-next-line no-console
        console.debug("[MessageList][ids:added-in-next]", {
          sessionId,
          count: addedInNext.length,
          sample: addedInNext.slice(0, 8).map((id) => debugItemSummary(nextByIdLocal.get(id) ?? ({ id } as any))),
        });
      }

      if (missingFromNext.length === 0 && addedInNext.length === 0) {
        // eslint-disable-next-line no-console
        console.warn("[MessageList][ids:reorder-only]", { sessionId, currentLen, nextLen, prefixLen, suffixLen });
      }
    }

    if (historyExpectedRef.current) {
      historyExpectedRef.current = false;
      if (import.meta.env.DEV && showDebug) {
        // eslint-disable-next-line no-console
        console.warn("[MessageList][history:mixed-update]", {
          sessionId,
          nextLen,
          currentLen,
          prefixLen,
          suffixLen,
          deleteCount,
          insertLen: insertData.length,
          anchorId,
          anchorIndex,
        });
      }
    } else if (import.meta.env.DEV && showDebug) {
      // eslint-disable-next-line no-console
      console.debug("[MessageList][data:reconcile]", {
        sessionId,
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
    }

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
  }, [isActive, listItemsCoalesced, sessionId, showDebug]);

  return {
    methodsRef,
    context,
    initialLocation: INITIAL_LOCATION_BOTTOM,
    onScroll,
    onRenderedDataChange,
  };
}
