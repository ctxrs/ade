type SessionMessageListDebugDetail = Record<string, unknown> | null;

export type SessionMessageListDebugEntry = {
  seq: number;
  atMs: number;
  sessionId: string;
  cause: string;
  isActive: boolean;
  loaded: boolean;
  listCount: number;
  stickToBottom: boolean | null;
  renderedAnchorId: string | null;
  renderedTopId: string | null;
  scrollTop: number | null;
  clientHeight: number | null;
  scrollHeight: number | null;
  maxScrollTop: number | null;
  distanceFromMaxScrollPx: number | null;
  blankTailPx: number | null;
  lastItemId: string | null;
  lastItemTopPx: number | null;
  lastItemBottomPx: number | null;
  lastItemOffscreenAbove: boolean | null;
  renderedItemCount: number | null;
  sessionViewHeight: number | null;
  sessionBottomHeight: number | null;
  composerHeight: number | null;
  queueHeight: number | null;
  impossibleTail: boolean;
  detail: SessionMessageListDebugDetail;
};

type SessionMessageListDebugStore = {
  seq: number;
  entries: SessionMessageListDebugEntry[];
};

type RecordSessionMessageListDebugSnapshotParams = {
  sessionId: string;
  cause: string;
  scroller: HTMLElement | null;
  isActive: boolean;
  loaded: boolean;
  listCount: number;
  stickToBottom: boolean | null;
  renderedAnchorId: string | null;
  renderedTopId: string | null;
  detail?: SessionMessageListDebugDetail;
};

declare global {
  interface Window {
    __wbSessionMessageListDebug?: SessionMessageListDebugStore;
  }
}

const MAX_SESSION_MESSAGE_LIST_DEBUG_ENTRIES = 400;
const IMPOSSIBLE_TAIL_THRESHOLD_PX = 96;

function roundPx(value: number | null): number | null {
  if (value == null || !Number.isFinite(value)) return null;
  return Math.round(value * 100) / 100;
}

function getStore(): SessionMessageListDebugStore {
  const existing = window.__wbSessionMessageListDebug;
  if (existing) return existing;
  const created: SessionMessageListDebugStore = { seq: 0, entries: [] };
  window.__wbSessionMessageListDebug = created;
  return created;
}

function lastRenderedItem(scroller: HTMLElement): HTMLElement | null {
  const rendered = scroller.querySelectorAll<HTMLElement>("[role=\"listitem\"]");
  return rendered.length > 0 ? rendered.item(rendered.length - 1) : null;
}

export function recordSessionMessageListDebugSnapshot({
  sessionId,
  cause,
  scroller,
  isActive,
  loaded,
  listCount,
  stickToBottom,
  renderedAnchorId,
  renderedTopId,
  detail = null,
}: RecordSessionMessageListDebugSnapshotParams): void {
  if (!import.meta.env.DEV) return;
  if (typeof window === "undefined") return;

  const scrollerRect = scroller?.getBoundingClientRect() ?? null;
  const sessionView = scroller?.closest(".wb-session-view") as HTMLElement | null;
  const sessionBottom = sessionView?.querySelector(".wb-session-bottom") as HTMLElement | null;
  const composer = sessionView?.querySelector(".wb-active-composer") as HTMLElement | null;
  const queuePanel = sessionView?.querySelector(".queue-panel") as HTMLElement | null;
  const lastItem = scroller ? lastRenderedItem(scroller) : null;
  const lastItemRect = lastItem?.getBoundingClientRect() ?? null;

  const scrollTop = scroller ? roundPx(scroller.scrollTop) : null;
  const clientHeight = scroller ? roundPx(scroller.clientHeight) : null;
  const scrollHeight = scroller ? roundPx(scroller.scrollHeight) : null;
  const maxScrollTop =
    scroller && clientHeight != null && scrollHeight != null ? roundPx(Math.max(0, scrollHeight - clientHeight)) : null;
  const distanceFromMaxScrollPx =
    scrollTop != null && maxScrollTop != null ? roundPx(Math.max(0, maxScrollTop - scrollTop)) : null;
  const blankTailPx =
    scrollerRect && lastItemRect ? roundPx(Math.max(0, scrollerRect.bottom - lastItemRect.bottom)) : null;
  const lastItemTopPx = scrollerRect && lastItemRect ? roundPx(lastItemRect.top - scrollerRect.top) : null;
  const lastItemBottomPx = scrollerRect && lastItemRect ? roundPx(lastItemRect.bottom - scrollerRect.top) : null;
  const lastItemOffscreenAbove =
    scrollerRect && lastItemRect ? lastItemRect.bottom < scrollerRect.top - 1 : null;
  const impossibleTail =
    Boolean(distanceFromMaxScrollPx != null && distanceFromMaxScrollPx <= 2) &&
    Boolean((blankTailPx != null && blankTailPx > IMPOSSIBLE_TAIL_THRESHOLD_PX) || lastItemOffscreenAbove);

  const entry: SessionMessageListDebugEntry = {
    seq: 0,
    atMs: Date.now(),
    sessionId,
    cause,
    isActive,
    loaded,
    listCount,
    stickToBottom,
    renderedAnchorId,
    renderedTopId,
    scrollTop,
    clientHeight,
    scrollHeight,
    maxScrollTop,
    distanceFromMaxScrollPx,
    blankTailPx,
    lastItemId: lastItem?.getAttribute("data-thread-item-id") ?? null,
    lastItemTopPx,
    lastItemBottomPx,
    lastItemOffscreenAbove,
    renderedItemCount: scroller ? scroller.querySelectorAll("[role=\"listitem\"]").length : null,
    sessionViewHeight: sessionView ? roundPx(sessionView.getBoundingClientRect().height) : null,
    sessionBottomHeight: sessionBottom ? roundPx(sessionBottom.getBoundingClientRect().height) : null,
    composerHeight: composer ? roundPx(composer.getBoundingClientRect().height) : null,
    queueHeight: queuePanel ? roundPx(queuePanel.getBoundingClientRect().height) : null,
    impossibleTail,
    detail,
  };

  try {
    const store = getStore();
    entry.seq = store.seq + 1;
    store.seq = entry.seq;
    store.entries.push(entry);
    if (store.entries.length > MAX_SESSION_MESSAGE_LIST_DEBUG_ENTRIES) {
      store.entries.splice(0, store.entries.length - MAX_SESSION_MESSAGE_LIST_DEBUG_ENTRIES);
    }
  } catch {
    // Debug-only helper; never break the workbench for diagnostics.
  }
}
