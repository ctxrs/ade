import {
  forwardRef,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type HTMLAttributes,
  type MutableRefObject,
  type PointerEvent,
} from "react";
import { ChevronDown, CornerUpRight, Pencil, Trash2 } from "lucide-react";
import type { StateSnapshot, VirtuosoHandle } from "react-virtuoso";
import {
  deleteMessage,
  Message,
  type MessageAttachment,
  postMessage,
  Session,
  SessionEvent,
  SubagentInvocation,
  setSessionModel,
  authenticateSession,
  type ProviderOptions,
  getSettings,
  idToString,
  interruptSession,
  submitAskUserQuestion,
  uploadBlob,
} from "../api/client";
import { useOpenSession, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { loadSessionViewPrefsV1, saveSessionViewPrefsV1, type SessionViewVerbosity } from "../state/uiStateStore";
import { AskUserQuestionCard } from "../components/AskUserQuestionCard";
import { type SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import { useSettingsSnapshot, useSettingsStore } from "../state/settingsStore";
import {
  WorkbenchComposer as UnifiedWorkbenchComposer,
  type ContextWindowInfo,
  type WorkbenchModeId,
} from "../components/WorkbenchComposer";
import { imageFilesToInlineAttachments } from "../utils/messageAttachments";
import { registerDropScope } from "../utils/dragDropScopes";
import { useRelativeNowMs } from "../utils/useRelativeNowMs";
import { useStatsigGate } from "../utils/statsig";
import { useDictationController } from "../utils/useDictationController";
import { usePinnedScrollManager } from "./usePinnedScrollManager";
import { useWorkbenchStore } from "../workbench/store";
import { buildModelsFromProviderOptions } from "../components/workbenchComposer/WorkbenchComposer.utils";
import {
  AssistantEntry,
  ThreadItemView,
  WorkbenchThreadStack,
  WorkbenchThoughtRow,
  WorkbenchToolGroupRow,
  WorkbenchToolRow,
  WorkbenchTurnHeaderView,
  WorkbenchTurnStatusRow,
} from "./SessionPage.thread";
import type {
  AskUserQuestionAnswerState,
  ScrollbarDragState,
  ThreadItem,
  WorkbenchListItem,
} from "./SessionPage.types";
import {
  appendSegment,
  attachmentDisplayName,
  formatElapsedMs,
  formatSubagentChildMeta,
  humanToolStatus,
  markdownToPlainText,
  subagentChildLabel,
} from "./SessionPage.helpers";
import {
  buildPendingTurns,
  buildWorkbenchThreadViewModelFromTurns,
  collectAskUserQuestionAnswers,
  deriveAuthUi,
  deriveMessagesKey,
  deriveProviderGuardNotice,
  deriveSessionError,
  deriveTurnsKey,
  filterQueuedMessagesForPanel,
  filterTurnsForQueuedMessages,
  filterThreadItemsForVerbosity,
  mergeMessagesForView,
  mergeQueuedMessagesForPanel,
  normalizeContextWindowMetrics,
} from "./SessionPage.workbenchViewModel";

type PendingMessageEntry = {
  clientId: string;
  message: Message;
};

// Edge case: the workspace stream can deliver the real message before the
// POST response updates the optimistic entry. We drop client-id pending
// messages when a matching real message arrives within this time window,
// allowing small client/server clock skew.
const PENDING_MATCH_WINDOW_MS = 15_000;
const PENDING_MATCH_EARLY_SKEW_MS = 2_000;

const isClientMessageId = (value: unknown): boolean => {
  const id = typeof value === "string" ? value : "";
  return Boolean(id && id.startsWith("client-"));
};

const createClientMessageId = (): string => {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `client-${crypto.randomUUID()}`;
  }
  return `client-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
};

const normalizeAttachmentKey = (value: MessageAttachment): string => {
  const key = String((value as any)?.blob_id ?? (value as any)?.name ?? (value as any)?.kind ?? "").trim();
  return key;
};

const readAnchorMetaFromScroller = (
  scroller: HTMLDivElement | null,
): { id: string; offset: number } | null => {
  if (!scroller) return null;
  const scrollerRect = scroller.getBoundingClientRect();
  const items = Array.from(scroller.querySelectorAll('[role="listitem"]'));
  for (const listItem of items) {
    const rect = listItem.getBoundingClientRect();
    if (rect.bottom <= scrollerRect.top + 4) continue;
    const anchorEl = listItem.querySelector("[data-thread-item-id]") as HTMLElement | null;
    const anchorId = anchorEl?.getAttribute("data-thread-item-id");
    if (anchorId) {
      return { id: anchorId, offset: rect.top - scrollerRect.top };
    }
  }
  return null;
};

const buildAttachmentSignature = (attachments?: MessageAttachment[]): string => {
  if (!Array.isArray(attachments) || attachments.length === 0) return "";
  const keys = attachments.map(normalizeAttachmentKey).filter(Boolean).sort();
  return keys.join("|");
};

const buildMessageSignature = (message: Message): string => {
  if (!message || message.role !== "user") return "";
  const content = String(message.content ?? "");
  const attachments = buildAttachmentSignature(message.attachments);
  return `${content}::${attachments}`;
};

const parseMessageTimestamp = (value?: string | null): number | null => {
  const ts = Date.parse(value ?? "");
  return Number.isFinite(ts) ? ts : null;
};

const buildSignatureTimestampIndex = (messages: Message[]): Map<string, number[]> => {
  const index = new Map<string, number[]>();
  for (const message of messages) {
    if (!message || message.role !== "user") continue;
    const signature = buildMessageSignature(message);
    if (!signature) continue;
    const ts = parseMessageTimestamp(message.created_at);
    if (ts == null) continue;
    const list = index.get(signature);
    if (list) {
      list.push(ts);
    } else {
      index.set(signature, [ts]);
    }
  }
  return index;
};

const shouldDropPendingMessage = (
  pending: Message,
  realIds: Set<string>,
  realBySignature: Map<string, number[]>,
): boolean => {
  const pid = idToString(pending.id);
  if (pid && realIds.has(pid)) return true;
  if (!isClientMessageId(pid)) return false;
  if (pending.role !== "user") return false;
  const signature = buildMessageSignature(pending);
  if (!signature) return false;
  const pendingTs = parseMessageTimestamp(pending.created_at);
  if (pendingTs == null) return false;
  const candidates = realBySignature.get(signature);
  if (!candidates || candidates.length === 0) return false;
  for (const realTs of candidates) {
    if (realTs + PENDING_MATCH_EARLY_SKEW_MS < pendingTs) continue;
    if (realTs - pendingTs > PENDING_MATCH_WINDOW_MS) continue;
    return true;
  }
  return false;
};

function useRafCoalesced<T>(value: T): T {
  const latestRef = useRef(value);
  const [coalesced, setCoalesced] = useState(value);
  const rafRef = useRef<number | null>(null);
  latestRef.current = value;

  useEffect(() => {
    if (Object.is(value, coalesced)) return;
    if (rafRef.current != null) return;
    const schedule =
      typeof window !== "undefined" && typeof window.requestAnimationFrame === "function"
        ? window.requestAnimationFrame.bind(window)
        : (cb: FrameRequestCallback) => setTimeout(() => cb(Date.now()), 16);
    const cancel =
      typeof window !== "undefined" && typeof window.cancelAnimationFrame === "function"
        ? window.cancelAnimationFrame.bind(window)
        : clearTimeout;
    rafRef.current = schedule(() => {
      rafRef.current = null;
      setCoalesced(latestRef.current);
    }) as unknown as number;
    return () => {
      if (rafRef.current != null) {
        cancel(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [value, coalesced]);

  return coalesced;
}

function isSameContextWindow(a: ContextWindowInfo | null, b: ContextWindowInfo | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.windowTokens === b.windowTokens &&
    a.usedTokens === b.usedTokens &&
    a.remainingTokens === b.remainingTokens &&
    a.remainingFraction === b.remainingFraction
  );
}

export function SessionView({
  sessionId,
  isActive = true,
  autoOpenSession = true,
  draft,
  onDraftChange,
  onDraftPersistNow,
  onModeChange,
  preserveScrollOnFocus = false,
  scrollState,
  onScrollStateChange,
}: {
  sessionId: string;
  isActive?: boolean;
  draft?: { text: string; modeId: WorkbenchModeId } | null;
  onDraftChange?: ((text: string) => void) | null;
  onDraftPersistNow?: (() => void | Promise<void>) | null;
  onModeChange?: ((modeId: WorkbenchModeId) => void) | null;
  preserveScrollOnFocus?: boolean;
  scrollState?: {
    stickToBottom: boolean;
    anchorItemId: string | null;
    anchorOffset: number | null;
    scrollTop: number | null;
    virtuosoState?: unknown | null;
  } | null;
  onScrollStateChange?: ((
    next: {
      stickToBottom: boolean;
      anchorItemId: string | null;
      anchorOffset: number | null;
      scrollTop: number | null;
      virtuosoState?: unknown | null;
    },
  ) => void) | null;
  autoOpenSession?: boolean;
}) {
  const id = sessionId;
  const supervisor = useSessionSupervisor();
  const workbenchStore = useWorkbenchStore();
  const initialVirtuosoIndex = 100000;
  const showDebug = useMemo(() => {
    try {
      return new URLSearchParams(window.location.search).get("debug") === "1";
    } catch {
      return false;
    }
  }, [id]);
  const perfEnabled = useMemo(() => {
    try {
      return new URLSearchParams(window.location.search).get("perf") === "1";
    } catch {
      return false;
    }
  }, [id]);
  const perfStartRef = useRef<number>(0);
  const bottomThresholdPx = 16;
  const userIntentWindowMs = 1000;
  const [verbosity, setVerbosity] = useState<SessionViewVerbosity>("default");
  const [inputInternal, setInputInternal] = useState("");
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const [dropActive, setDropActive] = useState(false);
  const [workbenchModeInternal, setWorkbenchModeInternal] = useState<WorkbenchModeId>("default");
  const [sendBusy, setSendBusy] = useState(false);
  const sendBusyRef = useRef(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [queueActionBusyId, setQueueActionBusyId] = useState<string | null>(null);
  const [pendingMessages, setPendingMessages] = useState<PendingMessageEntry[]>([]);
  const [pendingQueueMessages, setPendingQueueMessages] = useState<PendingMessageEntry[]>([]);
  const [optimisticQueueRemovalIds, setOptimisticQueueRemovalIds] = useState<string[]>([]);
  const [fileOpenError, setFileOpenError] = useState<string | null>(null);
  const [modifierDown, setModifierDown] = useState(false);
  const [atBottom, setAtBottom] = useState(true);
  const [stickToBottom, setStickToBottom] = useState(true);
  const stickToBottomRef = useRef(true);
  const [authMethodId, setAuthMethodId] = useState<string>("");
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [optimisticAskAnswers, setOptimisticAskAnswers] = useState<Record<string, AskUserQuestionAnswerState>>({});
  const [expandedTurnHeaders, setExpandedTurnHeaders] = useState<Record<string, boolean>>({});
  const [expandedTurnDetailsById, setExpandedTurnDetailsById] = useState<Record<string, boolean>>({});
  const [expandedToolById, setExpandedToolById] = useState<Record<string, boolean>>({});
  const [lastContextWindow, setLastContextWindow] = useState<ContextWindowInfo | null>(null);
  const virtuosoRef = useRef<VirtuosoHandle | null>(null);

  useEffect(() => {
    setPendingMessages([]);
    setPendingQueueMessages([]);
    setOptimisticQueueRemovalIds([]);
    setDraftAttachments([]);
    setSendError(null);
    setFileOpenError(null);
    setDropActive(false);
    setOptimisticAskAnswers({});
    setExpandedTurnHeaders({});
    setExpandedTurnDetailsById({});
    setExpandedToolById({});
    setLastContextWindow(null);
    setAuthMethodId("");
    setAuthBusy(false);
    setAuthError(null);
    setProviderGuardActionError(null);
    setProviderGuardActionBusy(false);
    setAtBottom(true);
    setStickToBottom(true);
    stickToBottomRef.current = true;
    lastScrollPersistedRef.current = null;
    liveScrollTopRef.current = null;
    restorePendingRef.current = true;
    restoringScrollRef.current = false;
    setRestoreInProgress(false);
    if (restoreCooldownRef.current) {
      window.clearTimeout(restoreCooldownRef.current);
      restoreCooldownRef.current = null;
    }
    if (dropHideTimerRef.current) {
      window.clearTimeout(dropHideTimerRef.current);
      dropHideTimerRef.current = null;
    }
  }, [id]);
  const didInitialScrollRef = useRef(false);
  const lastScrollPersistedRef = useRef<{
    stickToBottom: boolean;
    anchorItemId: string | null;
    anchorOffset: number | null;
    scrollTop: number | null;
    virtuosoState?: unknown | null;
  } | null>(null);
  const liveScrollTopRef = useRef<number | null>(null);
  const scrollSyncRafRef = useRef<number | null>(null);
  const autoScrollRef = useRef(0);
  const autoScrollRafRef = useRef<number | null>(null);
  const autoScrollAttemptRef = useRef(0);
  const userScrollIntentRef = useRef(0);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const [scrollerNode, setScrollerNode] = useState<HTMLDivElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const bottomSentinelRef = useRef<HTMLDivElement | null>(null);
  const [bottomSentinelNode, setBottomSentinelNode] = useState<HTMLDivElement | null>(null);
  const sentinelMeasuredRef = useRef(false);
  const sentinelVisibleRef = useRef(true);
  const [scrollbarNeeded, setScrollbarNeeded] = useState(false);
  const [scrollbarActive, setScrollbarActive] = useState(false);
  const [scrollbarDragging, setScrollbarDragging] = useState(false);
  const scrollbarActiveRef = useRef(false);
  const scrollbarNeededRef = useRef(false);
  const scrollbarDraggingRef = useRef(false);
  const scrollbarHideTimerRef = useRef<number | null>(null);
  const scrollbarRafRef = useRef<number | null>(null);
  const scrollbarTrackRef = useRef<HTMLDivElement | null>(null);
  const scrollbarThumbRef = useRef<HTMLDivElement | null>(null);
  const scrollbarThumbHeightRef = useRef(0);
  const scrollbarLastScrollTopRef = useRef<number | null>(null);
  const scrollbarDragRef = useRef<ScrollbarDragState | null>(null);
  const latestAnchorIdRef = useRef<string | null>(null);
  const restoringScrollRef = useRef(false);
  const dropHideTimerRef = useRef<number | null>(null);
  const restoreCooldownRef = useRef<number | null>(null);
  const restorePendingRef = useRef(true);
  const pendingScrollToBottomRef = useRef(false);
  const settingsStore = useSettingsStore();
  const settingsSnapshot = useSettingsSnapshot();
  const [providerGuardActionError, setProviderGuardActionError] = useState<string | null>(null);
  const [providerGuardActionBusy, setProviderGuardActionBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    loadSessionViewPrefsV1()
      .then((prefs) => {
        if (!cancelled && prefs?.verbosity) setVerbosity(prefs.verbosity);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);


  useEffect(() => {
    const update = (event: KeyboardEvent) => {
      setModifierDown(event.metaKey || event.ctrlKey);
    };
    const handleBlur = () => setModifierDown(false);
    window.addEventListener("keydown", update);
    window.addEventListener("keyup", update);
    window.addEventListener("blur", handleBlur);
    return () => {
      window.removeEventListener("keydown", update);
      window.removeEventListener("keyup", update);
      window.removeEventListener("blur", handleBlur);
    };
  }, []);

  const setVerbosityPref = useCallback((next: SessionViewVerbosity) => {
    setVerbosity(next);
    saveSessionViewPrefsV1(next).catch(() => {});
  }, []);

  const input = draft?.text ?? inputInternal;
  const hasDraftContent = input.trim().length > 0 || draftAttachments.length > 0;
  const setInput = useCallback(
    (next: string) => {
      if (draft) {
        onDraftChange?.(next);
        return;
      }
      setInputInternal(next);
    },
    [draft, onDraftChange],
  );
  const workbenchMode = draft?.modeId ?? workbenchModeInternal;
  const setWorkbenchMode = useCallback(
    (next: WorkbenchModeId) => {
      if (draft) {
        onModeChange?.(next);
        return;
      }
      setWorkbenchModeInternal(next);
    },
    [draft, onModeChange],
  );
  const queueActionBusy = queueActionBusyId !== null;

  const persistScroll = useCallback(
    (next: {
      stickToBottom: boolean;
      anchorItemId: string | null;
      anchorOffset: number | null;
      scrollTop: number | null;
      virtuosoState?: unknown | null;
    }) => {
      if (!onScrollStateChange) return;
      const prev = lastScrollPersistedRef.current;
      if (
        prev &&
        prev.stickToBottom === next.stickToBottom &&
        prev.anchorItemId === next.anchorItemId &&
        prev.anchorOffset === next.anchorOffset &&
        prev.scrollTop === next.scrollTop &&
        prev.virtuosoState === (next.virtuosoState ?? null)
      ) {
        return;
      }
      lastScrollPersistedRef.current = { ...next, virtuosoState: next.virtuosoState ?? null };
      onScrollStateChange({ ...next, virtuosoState: next.virtuosoState ?? null });
    },
    [onScrollStateChange],
  );

  const setScrollbarActiveState = useCallback((next: boolean) => {
    if (scrollbarActiveRef.current === next) return;
    scrollbarActiveRef.current = next;
    setScrollbarActive(next);
  }, []);

  const setScrollbarNeededState = useCallback((next: boolean) => {
    if (scrollbarNeededRef.current === next) return;
    scrollbarNeededRef.current = next;
    setScrollbarNeeded(next);
  }, []);

  const updateScrollbar = useCallback(() => {
    const scroller = scrollerRef.current;
    if (!scroller) return;
    const { scrollHeight, clientHeight, scrollTop } = scroller;
    const needsScrollbar = scrollHeight > clientHeight + 1;
    setScrollbarNeededState(needsScrollbar);
    if (!needsScrollbar) return;
    const track = scrollbarTrackRef.current;
    const thumb = scrollbarThumbRef.current;
    if (!track || !thumb) return;
    const trackHeight = track.clientHeight;
    if (trackHeight <= 0 || clientHeight <= 0) return;
    const thumbHeight = Math.max((clientHeight / scrollHeight) * trackHeight, 24);
    scrollbarThumbHeightRef.current = thumbHeight;
    const maxThumbTop = Math.max(trackHeight - thumbHeight, 0);
    const maxScrollTop = Math.max(scrollHeight - clientHeight, 1);
    const thumbTop = Math.min(maxThumbTop, Math.max(0, (scrollTop / maxScrollTop) * maxThumbTop));
    thumb.style.height = `${thumbHeight}px`;
    thumb.style.transform = `translateY(${thumbTop}px)`;
  }, [setScrollbarNeededState]);

  const scheduleScrollbarUpdate = useCallback(() => {
    if (scrollbarRafRef.current != null) return;
    scrollbarRafRef.current = window.requestAnimationFrame(() => {
      scrollbarRafRef.current = null;
      updateScrollbar();
    });
  }, [updateScrollbar]);

  const showScrollbarTemporarily = useCallback(() => {
    const scroller = scrollerRef.current;
    if (scroller) {
      setScrollbarNeededState(scroller.scrollHeight > scroller.clientHeight + 1);
    }
    setScrollbarActiveState(true);
    if (scrollbarHideTimerRef.current) window.clearTimeout(scrollbarHideTimerRef.current);
    scrollbarHideTimerRef.current = window.setTimeout(() => {
      setScrollbarActiveState(false);
    }, 900);
    updateScrollbar();
  }, [setScrollbarActiveState, setScrollbarNeededState, updateScrollbar]);

  const markUserScrollIntent = useCallback(() => {
    userScrollIntentRef.current = Date.now();
  }, []);

  const handleScrollbarTrackPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      if (event.target === scrollbarThumbRef.current) return;
      markUserScrollIntent();
      const scroller = scrollerRef.current;
      const track = scrollbarTrackRef.current;
      if (!scroller || !track) return;
      event.preventDefault();
      const rect = track.getBoundingClientRect();
      const ratio = Math.min(1, Math.max(0, (event.clientY - rect.top) / rect.height));
      const maxScrollTop = Math.max(scroller.scrollHeight - scroller.clientHeight, 0);
      scroller.scrollTop = ratio * maxScrollTop;
      scheduleScrollbarUpdate();
      showScrollbarTemporarily();
    },
    [markUserScrollIntent, scheduleScrollbarUpdate, showScrollbarTemporarily],
  );

  const handleScrollbarThumbPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      markUserScrollIntent();
      const scroller = scrollerRef.current;
      const track = scrollbarTrackRef.current;
      if (!scroller || !track) return;
      event.preventDefault();
      event.stopPropagation();
      updateScrollbar();
      const trackHeight = track.clientHeight;
      const thumbHeight = scrollbarThumbHeightRef.current;
      const maxScrollTop = scroller.scrollHeight - scroller.clientHeight;
      if (maxScrollTop <= 0 || trackHeight <= thumbHeight) return;
      if (scrollbarHideTimerRef.current) window.clearTimeout(scrollbarHideTimerRef.current);
      setScrollbarActiveState(true);
      setScrollbarDragging(true);
      scrollbarDraggingRef.current = true;
      scrollbarDragRef.current = {
        pointerId: event.pointerId,
        startY: event.clientY,
        startScrollTop: scroller.scrollTop,
        trackHeight,
        thumbHeight,
        scrollHeight: scroller.scrollHeight,
        clientHeight: scroller.clientHeight,
      };
      scrollbarThumbRef.current?.setPointerCapture(event.pointerId);
    },
    [markUserScrollIntent, setScrollbarActiveState, updateScrollbar],
  );

  const handleScrollbarThumbPointerMove = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const drag = scrollbarDragRef.current;
      const scroller = scrollerRef.current;
      if (!drag || !scroller || drag.pointerId !== event.pointerId) return;
      markUserScrollIntent();
      const maxScrollTop = Math.max(drag.scrollHeight - drag.clientHeight, 0);
      const maxThumbTop = Math.max(drag.trackHeight - drag.thumbHeight, 1);
      const delta = event.clientY - drag.startY;
      const nextScrollTop = drag.startScrollTop + (delta / maxThumbTop) * maxScrollTop;
      scroller.scrollTop = Math.min(maxScrollTop, Math.max(0, nextScrollTop));
      scheduleScrollbarUpdate();
    },
    [markUserScrollIntent, scheduleScrollbarUpdate],
  );

  const handleScrollbarThumbPointerUp = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const drag = scrollbarDragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      scrollbarDragRef.current = null;
      scrollbarThumbRef.current?.releasePointerCapture(event.pointerId);
      scrollbarDraggingRef.current = false;
      setScrollbarDragging(false);
      showScrollbarTemporarily();
    },
    [showScrollbarTemporarily],
  );

  const handleScrollbarMouseLeave = useCallback(() => {
    if (scrollbarDraggingRef.current) return;
    if (scrollbarHideTimerRef.current) window.clearTimeout(scrollbarHideTimerRef.current);
    setScrollbarActiveState(false);
  }, [setScrollbarActiveState]);

  useEffect(() => {
    return () => {
      if (scrollbarHideTimerRef.current) window.clearTimeout(scrollbarHideTimerRef.current);
      if (scrollbarRafRef.current != null) window.cancelAnimationFrame(scrollbarRafRef.current);
    };
  }, []);


  const handleFileOpenError = useCallback((message: string | null) => {
    setFileOpenError(message);
  }, []);

  useOpenSession(autoOpenSession ? id ?? "" : "", { watchDiff: true });
  const refreshAll = useCallback(async () => {
    if (!id) return;
    await supervisor.refreshQueue(id);
    supervisor.refreshSession(id, { watchDiff: true });
  }, [id, supervisor]);

  useLayoutEffect(() => {
    return () => {
      if (!onScrollStateChange) return;
      const el = scrollerRef.current;
      if (!el) {
        const last = lastScrollPersistedRef.current;
        if (last) {
          onScrollStateChange({
            stickToBottom: last.stickToBottom,
            anchorItemId: last.anchorItemId,
            anchorOffset: last.anchorOffset,
            scrollTop: last.scrollTop,
            virtuosoState: last.virtuosoState ?? undefined,
          });
        }
        return;
      }
      const observedTop = el.scrollTop;
      const recordedTop = liveScrollTopRef.current;
      const rawTop =
        recordedTop != null && Math.abs(observedTop - recordedTop) > 4
          ? recordedTop
          : observedTop;
      const maxScrollTop = Math.max(0, el.scrollHeight - el.clientHeight);
      const scrollTop = Math.min(Math.max(rawTop, 0), maxScrollTop);
      const stickToBottom = stickToBottomRef.current;
      const anchorMeta = readAnchorMetaFromScroller(el);
      const anchorItemId = stickToBottom ? null : anchorMeta?.id ?? latestAnchorIdRef.current;
      const anchorOffset = stickToBottom ? null : anchorMeta?.offset ?? latestAnchorOffsetRef.current;
      onScrollStateChange({
        stickToBottom,
        anchorItemId,
        anchorOffset,
        scrollTop: stickToBottom ? null : scrollTop,
        virtuosoState: lastScrollPersistedRef.current?.virtuosoState ?? undefined,
      });
    };
  }, [bottomThresholdPx, onScrollStateChange]);

  useLayoutEffect(() => {
    restorePendingRef.current = true;
    didInitialScrollRef.current = false;
    lastScrollPersistedRef.current = null;
    latestAnchorIdRef.current = null;
    liveScrollTopRef.current = null;
    restoringScrollRef.current = false;
    autoScrollRef.current = 0;
    autoScrollAttemptRef.current = 0;
    userScrollIntentRef.current = 0;
    sentinelMeasuredRef.current = false;
    sentinelVisibleRef.current = true;
    if (autoScrollRafRef.current != null) {
      window.cancelAnimationFrame(autoScrollRafRef.current);
      autoScrollRafRef.current = null;
    }
    if (scrollSyncRafRef.current != null) {
      window.cancelAnimationFrame(scrollSyncRafRef.current);
      scrollSyncRafRef.current = null;
    }
    setAtBottom(true);
    setStickToBottom(true);
    setExpandedTurnHeaders({});
    setExpandedToolById({});
    setSendError(null);
    setFileOpenError(null);
    setAuthMethodId("");
    setAuthError(null);
    setOptimisticAskAnswers({});
  }, [id]);

  useEffect(() => {
    return () => {
      if (scrollSyncRafRef.current != null) {
        window.cancelAnimationFrame(scrollSyncRafRef.current);
        scrollSyncRafRef.current = null;
      }
      if (restoreCooldownRef.current) {
        window.clearTimeout(restoreCooldownRef.current);
        restoreCooldownRef.current = null;
      }
      if (autoScrollRafRef.current != null) {
        window.cancelAnimationFrame(autoScrollRafRef.current);
        autoScrollRafRef.current = null;
      }
    };
  }, []);

  const {
    dictationRecording,
    dictationError,
    dictationDebugText,
    startDictation,
    stopDictation,
  } = useDictationController({
    text: input,
    setText: setInput,
    appendSegment,
  });

  const entry = useSessionEntry(id ?? "");
  const session: Session | null = entry?.session ?? null;
  const openChildSession = useCallback(
    (childSessionId: string) => {
      if (!session) return;
      const taskId = idToString(session.task_id);
      if (!taskId) return;
      workbenchStore.focusTask(taskId, childSessionId || null);
    },
    [session, workbenchStore],
  );

  const worktreeId = session ? idToString(session.worktree_id) : null;
  const turns = entry?.turns ?? [];
  const turnToolsByTurnId = entry?.turnToolsByTurnId ?? {};
  const turnToolsLoading = entry?.turnToolsLoading ?? [];
  const toolSummariesReady = entry?.toolSummariesReady ?? false;
  const hasMoreTurns = entry?.hasMoreTurns ?? false;
  const events: SessionEvent[] = entry?.events ?? [];
  const messages: Message[] = entry?.messages ?? [];
  const queue: Message[] = entry?.queue ?? [];
  const subagentInvocations: SubagentInvocation[] = entry?.subagentInvocations ?? [];
  const subagentInvocationsLoading = entry?.subagentInvocationsLoading ?? false;
  const eventsKey = `${entry?.lastEventSeq ?? 0}:${events.length}`;
  const turnsKey = deriveTurnsKey(turns);
  const messagesKey = deriveMessagesKey(messages);
  const optimisticQueueRemovalSet = useMemo(
    () => new Set(optimisticQueueRemovalIds),
    [optimisticQueueRemovalIds],
  );
  const markQueueOptimisticallyRemoved = useCallback((messageId: string) => {
    if (!messageId) return;
    setOptimisticQueueRemovalIds((prev) => (prev.includes(messageId) ? prev : [...prev, messageId]));
  }, []);
  const rollbackOptimisticQueueRemoval = useCallback((messageId: string) => {
    if (!messageId) return;
    setOptimisticQueueRemovalIds((prev) => prev.filter((id) => id !== messageId));
  }, []);
  const shouldKeepQueueRemovalOnError = (error: unknown) => {
    const msg = String((error as any)?.message ?? "");
    return msg.startsWith("400") || msg.startsWith("404");
  };
  const mergedQueueForPanel = useMemo(
    () => mergeQueuedMessagesForPanel(queue, pendingQueueMessages),
    [queue, pendingQueueMessages],
  );
  const queueForPanel = useMemo(
    () => {
      const filtered = filterQueuedMessagesForPanel(mergedQueueForPanel, turns);
      if (optimisticQueueRemovalIds.length === 0) return filtered;
      return filtered.filter((message) => {
        const mid = idToString(message.id);
        return !mid || !optimisticQueueRemovalSet.has(mid);
      });
    },
    [mergedQueueForPanel, turnsKey, optimisticQueueRemovalIds.length, optimisticQueueRemovalSet],
  );
  useEffect(() => {
    if (optimisticQueueRemovalIds.length === 0) return;
    const liveIds = new Set(
      mergedQueueForPanel.map((message) => idToString(message.id)).filter((id): id is string => !!id),
    );
    setOptimisticQueueRemovalIds((prev) => {
      const next = prev.filter((id) => liveIds.has(id));
      return next.length === prev.length ? prev : next;
    });
  }, [mergedQueueForPanel, optimisticQueueRemovalIds.length]);
  const showQueuePanel = queueForPanel.length > 0;
  const queuedMessageIdsForThread = useMemo(() => {
    const ids = new Set<string>();
    for (const message of queueForPanel) {
      const mid = idToString(message.id);
      if (mid) ids.add(mid);
    }
    if (optimisticQueueRemovalIds.length > 0) {
      for (const mid of optimisticQueueRemovalIds) {
        ids.add(mid);
      }
    }
    return ids;
  }, [queueForPanel, optimisticQueueRemovalIds]);
  const turnStatusByUserMessageId = useMemo(() => {
    const map = new Map<string, string>();
    for (const turn of turns) {
      const mid = turn.user_message_id ? idToString(turn.user_message_id) : "";
      if (!mid) continue;
      map.set(mid, String(turn.status));
    }
    return map;
  }, [turnsKey]);
  const queuedMessageIdsToShow = useMemo(() => {
    const ids = new Set<string>();
    for (const message of messages) {
      if (message.delivery !== "queued") continue;
      const mid = idToString(message.id);
      if (!mid) continue;
      const status = turnStatusByUserMessageId.get(mid);
      if (status && status !== "queued") {
        ids.add(mid);
      }
    }
    return ids;
  }, [messagesKey, turnStatusByUserMessageId]);
  useEffect(() => {
    if (pendingMessages.length === 0) return;
    const realIds = new Set(messages.map((m) => idToString(m.id)));
    const realBySignature = buildSignatureTimestampIndex(messages);
    setPendingMessages((prev) => {
      if (prev.length === 0) return prev;
      const next = prev.filter((entry) => {
        return !shouldDropPendingMessage(entry.message, realIds, realBySignature);
      });
      return next.length === prev.length ? prev : next;
    });
  }, [messagesKey, pendingMessages.length, messages]);
  useEffect(() => {
    if (pendingQueueMessages.length === 0) return;
    const realIds = new Set(queue.map((m) => idToString(m.id)));
    const realBySignature = buildSignatureTimestampIndex(queue);
    setPendingQueueMessages((prev) => {
      if (prev.length === 0) return prev;
      const next = prev.filter((entry) => {
        return !shouldDropPendingMessage(entry.message, realIds, realBySignature);
      });
      return next.length === prev.length ? prev : next;
    });
  }, [queue, pendingQueueMessages.length]);

  const displayMessages = useMemo(
    () => mergeMessagesForView(messages, pendingMessages, queuedMessageIdsToShow),
    [messagesKey, pendingMessages, queuedMessageIdsToShow],
  );
  const displayMessagesKey = deriveMessagesKey(displayMessages);
  const pendingTurns = useMemo(
    () => buildPendingTurns(turns, displayMessages),
    [turnsKey, displayMessagesKey],
  );
  const displayTurns = useMemo(
    () => (pendingTurns.length > 0 ? [...turns, ...pendingTurns] : turns),
    [turnsKey, pendingTurns],
  );
  const displayTurnsKey = deriveTurnsKey(displayTurns);
  const coalescedEvents = useRafCoalesced(events);
  const coalescedEventsKey = useRafCoalesced(eventsKey);
  const coalescedDisplayMessages = useRafCoalesced(displayMessages);
  const coalescedDisplayMessagesKey = useRafCoalesced(displayMessagesKey);
  const coalescedDisplayTurns = useRafCoalesced(displayTurns);
  const coalescedDisplayTurnsKey = useRafCoalesced(displayTurnsKey);
  const displayTurnsForThread = useMemo(
    () => filterTurnsForQueuedMessages(coalescedDisplayTurns, queuedMessageIdsForThread),
    [coalescedDisplayTurnsKey, queuedMessageIdsForThread],
  );
  const displayTurnsForThreadKey = useMemo(
    () => deriveTurnsKey(displayTurnsForThread),
    [displayTurnsForThread],
  );
  const computedContextWindow = useMemo<ContextWindowInfo | null>(() => {
    for (let i = turns.length - 1; i >= 0; i -= 1) {
      const metrics = turns[i]?.metrics_json;
      if (!metrics) continue;
      const normalized = normalizeContextWindowMetrics(metrics);
      if (normalized) return normalized;
    }
    return null;
  }, [turnsKey]);
  useEffect(() => {
    if (!computedContextWindow) return;
    setLastContextWindow((prev) =>
      isSameContextWindow(prev, computedContextWindow) ? prev : computedContextWindow,
    );
  }, [computedContextWindow]);
  const contextWindow = computedContextWindow ?? lastContextWindow;
  const hasActiveTurn = useMemo(
    () => turns.some((turn) => turn.status === "running" || turn.status === "queued"),
    [turnsKey],
  );
  const queuedMessagesEnabled = useStatsigGate("queued_messages_enabled", false);
  const sessionError = useMemo(
    () => deriveSessionError(turns, events),
    [turnsKey, eventsKey],
  );
  const providerGuardNotice = useMemo(
    () => deriveProviderGuardNotice(events),
    [eventsKey],
  );
  const providerGuardCountdownTarget = providerGuardNotice?.killAtMs ?? null;
  const needsNowMs =
    hasActiveTurn || (providerGuardCountdownTarget != null && providerGuardCountdownTarget > Date.now());
  const nowMs = useRelativeNowMs(1000, needsNowMs);
  const displayNowMs = nowMs;

  const providerGuardNoticeKey = providerGuardNotice
    ? `${providerGuardNotice.kind}:${providerGuardNotice.stage}:${providerGuardNotice.pid ?? ""}:${providerGuardNotice.killAtMs ?? ""}`
    : "";

  useEffect(() => {
    setProviderGuardActionError(null);
  }, [providerGuardNoticeKey]);

  const activeAskToolCallId = useMemo(() => {
    const answered = new Set<string>();
    for (const ev of events) {
      if (ev.event_type !== "notice") continue;
      if (ev.payload_json?.kind !== "ask_user_question_answered") continue;
      const toolCallId = String(ev.payload_json?.tool_call_id ?? "").trim();
      if (toolCallId) answered.add(toolCallId);
    }
    for (const toolCallId of Object.keys(optimisticAskAnswers)) {
      if (toolCallId) answered.add(toolCallId);
    }

    for (let i = events.length - 1; i >= 0; i--) {
      const ev = events[i];
      if (ev.event_type !== "notice") continue;
      if (ev.payload_json?.kind !== "ask_user_question") continue;
      const toolCallId = String(ev.payload_json?.tool_call_id ?? "").trim();
      if (!toolCallId || answered.has(toolCallId)) continue;
      return toolCallId;
    }
    return null;
  }, [eventsKey, optimisticAskAnswers]);

  const askUserQuestionAnswers = useMemo(
    () => collectAskUserQuestionAnswers(events, optimisticAskAnswers),
    [eventsKey, optimisticAskAnswers],
  );

  const applyProviderGuardSettings = useCallback(
    async (opts: {
      enabled?: boolean;
      mode?: "auto" | "custom";
      memoryHighMb?: number | null;
      memoryMaxMb?: number | null;
    }) => {
      setProviderGuardActionError(null);
      setProviderGuardActionBusy(true);
      try {
        const current = settingsSnapshot.settings ?? (await getSettings());
        const guard = current.provider_guard ?? { enabled: true, mode: "auto" };
        const nextGuard = {
          enabled: opts.enabled ?? guard.enabled ?? true,
          mode: opts.mode ?? guard.mode ?? "auto",
          memory_high_mb: opts.memoryHighMb ?? guard.memory_high_mb ?? null,
          memory_max_mb: opts.memoryMaxMb ?? guard.memory_max_mb ?? null,
          interval_ms: guard.interval_ms ?? null,
          grace_period_ms: guard.grace_period_ms ?? null,
        };
        await settingsStore.update({ provider_guard: nextGuard });
      } catch (e: any) {
        setProviderGuardActionError(e?.message ?? String(e));
      } finally {
        setProviderGuardActionBusy(false);
      }
    },
    [settingsSnapshot.settings, settingsStore],
  );

  const raiseProviderGuardLimit = useCallback(async () => {
    const totalMb = providerGuardNotice?.systemTotalMb;
    if (!totalMb || !Number.isFinite(totalMb)) {
      setProviderGuardActionError("System memory total is unavailable.");
      return;
    }
    const maxMb = Math.max(1024, Math.floor(totalMb * 0.9));
    let highMb = Math.floor(totalMb * 0.85);
    if (highMb > maxMb) highMb = maxMb;
    await applyProviderGuardSettings({
      enabled: true,
      mode: "custom",
      memoryHighMb: highMb,
      memoryMaxMb: maxMb,
    });
  }, [applyProviderGuardSettings, providerGuardNotice?.systemTotalMb]);

  const disableProviderGuard = useCallback(async () => {
    await applyProviderGuardSettings({ enabled: false });
  }, [applyProviderGuardSettings]);

  useEffect(() => {
    if (!perfEnabled) return;
    perfStartRef.current = performance.now();
  }, [id, perfEnabled]);

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
  }, [perfEnabled, entry?.loading, entry?.events.length, entry?.diff]);

  const workbenchThreadView = useMemo(() => {
    if (displayTurnsForThread.length === 0) {
      return { groups: [], debugEvents: [] };
    }
    return buildWorkbenchThreadViewModelFromTurns(
      displayTurnsForThread,
      coalescedDisplayMessages,
      toolSummariesReady ? turnToolsByTurnId : {},
      coalescedEvents,
      askUserQuestionAnswers,
    );
    // messages are canonical for turn headers; include in memo key
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    displayTurnsForThreadKey,
    coalescedDisplayMessagesKey,
    toolSummariesReady ? turnToolsByTurnId : null,
    coalescedEventsKey,
    displayTurnsForThread.length,
    askUserQuestionAnswers,
  ]);

  const debugEvents = workbenchThreadView.debugEvents;
  const wbGroups = useMemo(
    () =>
      workbenchThreadView.groups.map((group) => ({
        ...group,
        items: filterThreadItemsForVerbosity(group.items, verbosity),
      })),
    [workbenchThreadView.groups, verbosity],
  );
  const wbListItems = useMemo<WorkbenchListItem[]>(() => {
    const out: WorkbenchListItem[] = [];
    for (const g of wbGroups) {
      if (g.header) {
        out.push({ kind: "turn_header", id: `turn-header-${g.header.id}`, header: g.header });
      }
      out.push(...g.items);
    }
    return out;
  }, [wbGroups]);

  const [firstItemIndex, setFirstItemIndex] = useState(initialVirtuosoIndex);
  const prevSessionForIndexRef = useRef(id);
  const prevItemsRef = useRef<WorkbenchListItem[]>(wbListItems);
  const pendingPrependRef = useRef(false);
  const pendingPrependAnchorRef = useRef<string | null>(null);
  const pendingPrependOffsetRef = useRef<number | null>(null);
  const latestAnchorOffsetRef = useRef<number | null>(null);
  const lastScrollHeightRef = useRef<number | null>(null);
  const updateAnchorFromScroller = useCallback(
    (scroller: HTMLDivElement | null) => {
      const meta = readAnchorMetaFromScroller(scroller);
      if (meta?.id) {
        latestAnchorIdRef.current = meta.id;
        latestAnchorOffsetRef.current = meta.offset;
        return meta.id;
      }
      return null;
    },
    [],
  );
  const adjustAnchorOffset = useCallback((anchorId: string, offset: number) => {
    requestAnimationFrame(() => {
      const scroller = scrollerRef.current;
      if (!scroller) return;
      const anchorEl = scroller.querySelector(
        `[data-thread-item-id=\"${anchorId}\"]`,
      ) as HTMLElement | null;
      const listItem = anchorEl?.closest('[role="listitem"]') as HTMLElement | null;
      if (!listItem) return;
      const rect = listItem.getBoundingClientRect();
      const scrollerRect = scroller.getBoundingClientRect();
      const currentOffset = rect.top - scrollerRect.top;
      const delta = currentOffset - offset;
      if (Math.abs(delta) > 1) {
        const handle = virtuosoRef.current;
        if (handle) {
          handle.scrollBy({ top: delta });
        } else {
          scroller.scrollTop += delta;
        }
      }
    });
  }, []);

  useLayoutEffect(() => {
    if (prevSessionForIndexRef.current !== id) {
      prevSessionForIndexRef.current = id;
      setFirstItemIndex(initialVirtuosoIndex);
      prevItemsRef.current = wbListItems;
      return;
    }
    if (preserveScrollOnFocus && !isActive) {
      prevItemsRef.current = wbListItems;
      return;
    }
    if (wbListItems.length === 0) {
      setFirstItemIndex(initialVirtuosoIndex);
      prevItemsRef.current = wbListItems;
      return;
    }
    const prevItems = prevItemsRef.current;
    if (prevItems.length === 0) {
      setFirstItemIndex(initialVirtuosoIndex);
      prevItemsRef.current = wbListItems;
      return;
    }
    const idsUnchanged =
      prevItems.length === wbListItems.length &&
      prevItems.every((item, index) => item.id === wbListItems[index]?.id);
    if (idsUnchanged) {
      prevItemsRef.current = wbListItems;
      return;
    }
    const pendingAnchor = pendingPrependRef.current ? pendingPrependAnchorRef.current : null;
    const listIncreased = wbListItems.length > prevItems.length;
    const shouldAnchor =
      pendingPrependRef.current || stickToBottomRef.current === false || scrollState?.stickToBottom === false;
    const anchorId = shouldAnchor
      ? pendingAnchor ?? latestAnchorIdRef.current ?? scrollState?.anchorItemId ?? null
      : null;
    let didShift = false;
    if (anchorId) {
      const prevIndex = prevItems.findIndex((item) => item.id === anchorId);
      const nextIndex = wbListItems.findIndex((item) => item.id === anchorId);
      if (prevIndex >= 0 && nextIndex >= 0) {
        const indexShift = nextIndex - prevIndex;
        if (indexShift !== 0) {
          setFirstItemIndex((prev) => prev - indexShift);
          didShift = true;
        }
        const offset = pendingPrependOffsetRef.current ?? latestAnchorOffsetRef.current;
        if (offset != null) {
          adjustAnchorOffset(anchorId, offset);
        }
      }
    }
    if (
      !didShift &&
      listIncreased &&
      (pendingPrependRef.current || stickToBottomRef.current === false || scrollState?.stickToBottom === false)
    ) {
      const prevFirstId = prevItems[0]?.id;
      if (prevFirstId) {
        const startIndex = wbListItems.findIndex((item) => item.id === prevFirstId);
        if (startIndex > 0) {
          setFirstItemIndex((prev) => prev - startIndex);
        }
      }
    }
    pendingPrependRef.current = false;
    pendingPrependAnchorRef.current = null;
    pendingPrependOffsetRef.current = null;
    prevItemsRef.current = wbListItems;
  }, [
    adjustAnchorOffset,
    firstItemIndex,
    id,
    initialVirtuosoIndex,
    isActive,
    preserveScrollOnFocus,
    scrollState?.anchorItemId,
    scrollState?.stickToBottom,
    wbListItems,
  ]);

  useEffect(() => {
    scheduleScrollbarUpdate();
  }, [scheduleScrollbarUpdate, wbListItems.length]);

  useLayoutEffect(() => {
    updateScrollbar();
  }, [updateScrollbar, wbListItems.length]);


  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  stickToBottomRef.current = stickToBottom;
  const wasActiveRef = useRef(isActive);
  const prevActiveRef = useRef(isActive);
  const lastSessionIdRef = useRef(id);
  const virtuosoPersistTimerRef = useRef<number | null>(null);
  const scrollSyncKey = useMemo(
    () => `${coalescedEventsKey}:${coalescedDisplayMessagesKey}:${coalescedDisplayTurnsKey}`,
    [coalescedEventsKey, coalescedDisplayMessagesKey, coalescedDisplayTurnsKey],
  );
  const [restoreInProgress, setRestoreInProgress] = useState(false);
  const { initialTopMostItemIndex, markAutoScroll, scheduleAutoScroll } = usePinnedScrollManager({
      isActive,
      preserveScrollOnFocus,
      bottomThresholdPx,
      userIntentWindowMs,
      itemsLength: wbListItems.length,
      firstItemIndex,
      scrollStateStickToBottom: scrollState?.stickToBottom,
      syncKey: scrollSyncKey,
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
    });

  useEffect(() => {
    const scroller = scrollerRef.current;
    if (!scroller) return;
    const observer = new ResizeObserver(() => {
      scheduleScrollbarUpdate();
      if (!stickToBottomRef.current) return;
      scheduleAutoScroll();
    });
    observer.observe(scroller);
    return () => observer.disconnect();
  }, [scheduleAutoScroll, scheduleScrollbarUpdate]);
  useLayoutEffect(() => {
    const sessionChanged = lastSessionIdRef.current !== id;
    if (sessionChanged) {
      lastSessionIdRef.current = id;
      wasActiveRef.current = false;
      restorePendingRef.current = true;
      didInitialScrollRef.current = false;
      restoringScrollRef.current = false;
    }
    if (isActive && (!wasActiveRef.current || sessionChanged)) {
      const shouldRestore = !preserveScrollOnFocus || sessionChanged || !didInitialScrollRef.current;
      restorePendingRef.current = shouldRestore;
      if (shouldRestore) {
        didInitialScrollRef.current = false;
      } else {
        restoringScrollRef.current = false;
        setRestoreInProgress(false);
      }
      const hasScrolledAway = scrollState?.stickToBottom === false;
      const nextStickToBottom = !hasScrolledAway;
      stickToBottomRef.current = nextStickToBottom;
      setStickToBottom(nextStickToBottom);
      if (nextStickToBottom) {
        liveScrollTopRef.current = null;
      }
    }
    wasActiveRef.current = isActive;
  }, [id, isActive, preserveScrollOnFocus, scrollState?.stickToBottom, scrollState?.virtuosoState]);

  const persistCurrentScroll = useCallback(() => {
    if (!onScrollStateChange) return;
    const el = scrollerRef.current;
    if (!el) return;
    const observedTop = el.scrollTop;
    const recordedTop = liveScrollTopRef.current;
    const rawTop =
      recordedTop != null && Math.abs(observedTop - recordedTop) > 4 ? recordedTop : observedTop;
    const maxScrollTop = Math.max(0, el.scrollHeight - el.clientHeight);
    const scrollTop = Math.min(Math.max(rawTop, 0), maxScrollTop);
    const stickToBottom = stickToBottomRef.current;
    const next = {
      stickToBottom,
      anchorItemId: stickToBottom ? null : latestAnchorIdRef.current,
      anchorOffset: stickToBottom ? null : latestAnchorOffsetRef.current,
      scrollTop: stickToBottom ? null : scrollTop,
    };
    const handle = virtuosoRef.current;
    if (handle) {
      handle.getState((state) => {
        persistScroll({ ...next, virtuosoState: state });
      });
      return;
    }
    persistScroll(next);
  }, [onScrollStateChange, persistScroll]);

  useEffect(() => {
    if (!preserveScrollOnFocus) {
      prevActiveRef.current = isActive;
      return;
    }
    if (prevActiveRef.current && !isActive) {
      persistCurrentScroll();
    }
    prevActiveRef.current = isActive;
  }, [isActive, persistCurrentScroll, preserveScrollOnFocus]);

  useEffect(() => {
    if (!isActive) setRestoreInProgress(false);
  }, [isActive]);

  useLayoutEffect(() => {
    if (!isActive) return;
    const items = wbListItems;
    if (items.length === 0) return;
    if (scrollState?.virtuosoState && !preserveScrollOnFocus && scrollState?.stickToBottom === false) {
      restorePendingRef.current = false;
      return;
    }
    if (!restorePendingRef.current) return;
    const state = scrollState ?? {
      stickToBottom: true,
      anchorItemId: null,
      anchorOffset: null,
      scrollTop: null,
    };
    setAtBottom(state.stickToBottom);
    const restoreAnchorId = !state.stickToBottom ? (state.anchorItemId ?? null) : null;
    const restoreAnchorOffset = !state.stickToBottom ? (state.anchorOffset ?? null) : null;
    const restoreScrollTop = !state.stickToBottom ? (state.scrollTop ?? null) : null;
    const restoreAnchorIndex =
      restoreAnchorId ? items.findIndex((it) => it?.id === restoreAnchorId) : -1;
    const hasAnchor = restoreAnchorId != null && restoreAnchorIndex >= 0;
    const shouldUseAnchor = hasAnchor && restoreAnchorOffset != null;
    if (state.stickToBottom && initialTopMostItemIndex != null) {
      restorePendingRef.current = false;
      didInitialScrollRef.current = true;
      restoringScrollRef.current = false;
      setRestoreInProgress(false);
      return;
    }

    restoringScrollRef.current = true;
    setRestoreInProgress(true);
    const finalizeRestore = () => {
      restorePendingRef.current = false;
      didInitialScrollRef.current = true;
      if (restoreCooldownRef.current) window.clearTimeout(restoreCooldownRef.current);
      restoreCooldownRef.current = window.setTimeout(() => {
        restoreCooldownRef.current = null;
        restoringScrollRef.current = false;
        setRestoreInProgress(false);
      }, 200);
    };

    const attemptRestore = () => {
      const el = scrollerRef.current;
      const handle = virtuosoRef.current;
      if (!el && !handle) {
        restoringScrollRef.current = false;
        setRestoreInProgress(false);
        return;
      }
      if (shouldUseAnchor) {
        const lastIndex = firstItemIndex + items.length - 1;
        latestAnchorIdRef.current = restoreAnchorId ?? null;
        markAutoScroll();
        handle?.scrollToIndex({
          index: Math.min(firstItemIndex + restoreAnchorIndex, lastIndex),
          align: "start",
        });
        if (restoreAnchorId && restoreAnchorOffset != null) {
          adjustAnchorOffset(restoreAnchorId, restoreAnchorOffset);
        }
      } else if (restoreScrollTop !== null) {
        const target = Math.max(0, restoreScrollTop);
        markAutoScroll();
        if (handle) {
          handle.scrollTo({ top: target });
        } else if (el) {
          el.scrollTop = target;
        }
      } else {
        const lastIndex = firstItemIndex + items.length - 1;
        markAutoScroll();
        handle?.scrollToIndex({ index: lastIndex, align: "end" });
      }
      finalizeRestore();
    };

    requestAnimationFrame(() => {
      attemptRestore();
    });
  }, [
    adjustAnchorOffset,
    id,
    isActive,
    preserveScrollOnFocus,
    scrollState?.anchorItemId,
    scrollState?.anchorOffset,
    scrollState?.stickToBottom,
    scrollState?.scrollTop,
    scrollState?.virtuosoState,
    initialTopMostItemIndex,
    firstItemIndex,
    wbListItems.length,
  ]);

  const authUi = useMemo(() => deriveAuthUi(events), [eventsKey]);
  const providerGuardMemoryLimitMb =
    providerGuardNotice?.stage === "high" ? providerGuardNotice?.limitHighMb : providerGuardNotice?.limitMaxMb;
  const providerGuardCountdownMs =
    providerGuardNotice?.killAtMs != null ? providerGuardNotice.killAtMs - displayNowMs : null;
  const providerGuardCountdownText =
    providerGuardNotice?.kind === "provider_guard_warning" &&
    providerGuardNotice?.stage === "max" &&
    providerGuardNotice?.killAtMs != null
      ? providerGuardCountdownMs != null && providerGuardCountdownMs > 0
        ? `Kill in ${formatElapsedMs(providerGuardCountdownMs)} unless memory drops.`
        : "Kill imminent unless memory drops."
      : null;
  const providerGuardHeading =
    providerGuardNotice?.kind === "provider_guard_kill"
      ? "Provider guard kill"
      : providerGuardNotice?.stage === "max"
        ? "Provider memory limit"
        : "Provider memory warning";
  const providerGuardMessage =
    providerGuardNotice?.message ??
    (providerGuardNotice?.kind === "provider_guard_kill"
      ? "Provider process killed after exceeding memory limits."
      : "Provider memory is above the guard threshold.");
  const providerGuardLimitLabel =
    providerGuardNotice?.stage === "high"
      ? "high limit"
      : providerGuardNotice?.stage === "max"
        ? "max limit"
        : "limit";
  const providerGuardProviderLabel = providerGuardNotice?.provider ?? session?.provider_id ?? undefined;
  const providerGuardPidLabel =
    providerGuardNotice?.pid != null ? `PID ${Math.round(providerGuardNotice.pid)}` : null;
  const canRaiseProviderGuard =
    providerGuardNotice?.systemTotalMb != null && Number.isFinite(providerGuardNotice.systemTotalMb);

  useEffect(() => {
    if (authMethodId) return;
    if (authUi.methods.length > 0) {
      setAuthMethodId(authUi.methods[0].id);
    }
  }, [authUi.methods, authMethodId]);

  const modelOptions = useMemo(() => {
    const models = entry?.acpModels;
    if (models) {
      const parsed = buildModelsFromProviderOptions({ models } as ProviderOptions);
      if (parsed.length > 0) return parsed;
    }
    const fallbackId = String(session?.model_id ?? "").trim();
    return fallbackId ? [{ id: fallbackId, name: fallbackId }] : [];
  }, [entry?.acpModels, session?.model_id]);
  const currentModelId = useMemo(() => {
    const fromMeta = String(entry?.acpCurrentModelId ?? "").trim();
    if (fromMeta) return fromMeta;
    return String(session?.model_id ?? "").trim();
  }, [entry?.acpCurrentModelId, session?.model_id]);

  const restoreStateFrom =
    scrollState?.stickToBottom !== false || !restorePendingRef.current
      ? undefined
      : (scrollState?.virtuosoState ?? undefined) as StateSnapshot | undefined;

  const jumpToLatestWorkbench = useCallback(() => {
    if (wbListItems.length > 0) {
      stickToBottomRef.current = true;
      setStickToBottom(true);
      setAtBottom(true);
      latestAnchorIdRef.current = null;
      liveScrollTopRef.current = null;
      persistScroll({ stickToBottom: true, anchorItemId: null, anchorOffset: null, scrollTop: null });
      scheduleAutoScroll();
    }
  }, [persistScroll, scheduleAutoScroll, wbListItems.length]);

  useEffect(() => {
    if (!pendingScrollToBottomRef.current) return;
    if (!stickToBottom) {
      pendingScrollToBottomRef.current = false;
      return;
    }
    if (restoreInProgress) {
      restoringScrollRef.current = false;
      setRestoreInProgress(false);
      if (restoreCooldownRef.current) {
        window.clearTimeout(restoreCooldownRef.current);
        restoreCooldownRef.current = null;
      }
    }
    if (preserveScrollOnFocus && !isActive) return;
    if (wbListItems.length === 0) return;
    pendingScrollToBottomRef.current = false;
    requestAnimationFrame(() => {
      jumpToLatestWorkbench();
    });
  }, [stickToBottom, restoreInProgress, preserveScrollOnFocus, isActive, wbListItems.length, jumpToLatestWorkbench]);
  const handleWorkbenchRangeChanged = useCallback(
    (range: { startIndex: number }) => {
      if (restoringScrollRef.current) return;
    if (updateAnchorFromScroller(scrollerRef.current)) return;
    const dataIndex = range.startIndex - firstItemIndex;
    const item = wbListItems[dataIndex];
    if (!item) return;
    const nextId = item.id ?? null;
    latestAnchorIdRef.current = nextId;
    const scroller = scrollerRef.current;
    if (scroller && nextId) {
      const anchorEl = scroller.querySelector(
        `[data-thread-item-id=\"${nextId}\"]`,
      ) as HTMLElement | null;
      const listItem = anchorEl?.closest('[role="listitem"]') as HTMLElement | null;
      if (listItem) {
        const rect = listItem.getBoundingClientRect();
        const scrollerRect = scroller.getBoundingClientRect();
        latestAnchorOffsetRef.current = rect.top - scrollerRect.top;
        return;
      }
    }
    latestAnchorOffsetRef.current = null;
  },
  [firstItemIndex, updateAnchorFromScroller, wbListItems],
);

  const handleStartReached = useCallback(() => {
    if (!hasMoreTurns) return;
    if (pendingPrependRef.current) return;
    pendingPrependRef.current = true;
    if (stickToBottomRef.current) {
      stickToBottomRef.current = false;
      setStickToBottom(false);
      setAtBottom(false);
    }
    const scroller = scrollerRef.current;
    const anchorMeta = scroller ? readAnchorMetaFromScroller(scroller) : null;
    pendingPrependAnchorRef.current = anchorMeta?.id ?? latestAnchorIdRef.current;
    pendingPrependOffsetRef.current = anchorMeta?.offset ?? latestAnchorOffsetRef.current ?? null;
    supervisor.loadMoreTurns(id);
  }, [hasMoreTurns, id, supervisor]);

  const getQueuedAttachments = (message: Message): MessageAttachment[] => {
    return Array.isArray(message.attachments) ? message.attachments : [];
  };

  const formatQueuedPreview = (message: Message, attachments: MessageAttachment[]): string => {
    const base = markdownToPlainText(message.content ?? "");
    const compact = base.replace(/\s+/g, " ").trim();
    if (compact) return compact;
    if (attachments.length > 0) return "Message with attachments";
    return "Queued message";
  };

  const formatQueuedAttachmentMeta = (attachments: MessageAttachment[]) => {
    if (attachments.length === 0) return null;
    const names = attachments.map((a) => attachmentDisplayName(a.name));
    const label = attachments.length === 1 ? "1 attachment" : `${attachments.length} attachments`;
    const preview = names.slice(0, 2).join(", ");
    const overflow = names.length > 2 ? ` +${names.length - 2}` : "";
    return {
      label,
      detail: preview ? `${preview}${overflow}` : null,
      title: names.join(", "),
    };
  };

  const setSendBusySafe = (next: boolean) => {
    sendBusyRef.current = next;
    setSendBusy(next);
  };

  const getNextOptimisticOrderSeq = () => {
    let maxSeq = 0;
    const consider = (message: Message) => {
      const seq = Number(message.order_seq ?? Number.NaN);
      if (Number.isFinite(seq) && seq > maxSeq) {
        maxSeq = seq;
      }
    };
    for (const message of messages) {
      consider(message);
    }
    for (const entry of pendingMessages) {
      consider(entry.message);
    }
    for (const entry of pendingQueueMessages) {
      consider(entry.message);
    }
    return Math.max(maxSeq + 1, 1);
  };

  const sendNow = async () => {
    if (!id) return;
    if (sendBusyRef.current) return;
    if (hasActiveTurn && !queuedMessagesEnabled) {
      // Temporarily disabled: queued messages are gated off while a turn is running.
      return;
    }
    setSendBusySafe(true);
    let text = "";
    try {
      text = (dictationRecording ? await stopDictation({ awaitFinal: true }) : input).trim();
    } catch (e: any) {
      setSendError(e?.message ? String(e.message) : String(e));
      setSendBusySafe(false);
      return;
    }
    if (!text) {
      setSendBusySafe(false);
      return;
    }
    const attachmentsToSend = draftAttachments;
    const shouldQueue = hasActiveTurn && queuedMessagesEnabled;
    const optimisticId = createClientMessageId();
    const optimisticMessage: Message = {
      id: optimisticId,
      session_id: id,
      task_id: session?.task_id ?? "",
      turn_id: null,
      turn_sequence: null,
      order_seq: getNextOptimisticOrderSeq(),
      role: "user",
      content: text,
      attachments: attachmentsToSend,
      delivery: shouldQueue ? "queued" : "immediate",
      created_at: new Date().toISOString(),
    };
    setSendError(null);
    if (shouldQueue) {
      setPendingQueueMessages((prev) => [...prev, { clientId: optimisticId, message: optimisticMessage }]);
    } else {
      setPendingMessages((prev) => [...prev, { clientId: optimisticId, message: optimisticMessage }]);
    }
    stickToBottomRef.current = true;
    setStickToBottom(true);
    liveScrollTopRef.current = null;
    pendingScrollToBottomRef.current = true;
    setInput("");
    setDraftAttachments([]);
    try {
      const posted = await postMessage(id, text, shouldQueue ? "queued" : undefined, attachmentsToSend);
      if (shouldQueue) {
        setPendingQueueMessages((prev) =>
          prev.map((entry) => (entry.clientId === optimisticId ? { ...entry, message: posted } : entry)),
        );
      } else {
        setPendingMessages((prev) =>
          prev.map((entry) => (entry.clientId === optimisticId ? { ...entry, message: posted } : entry)),
        );
      }
      try {
        await onDraftPersistNow?.();
      } catch {
        // best-effort
      }
    } catch (e: any) {
      if (shouldQueue) {
        setPendingQueueMessages((prev) => prev.filter((entry) => entry.clientId !== optimisticId));
      } else {
        setPendingMessages((prev) => prev.filter((entry) => entry.clientId !== optimisticId));
      }
      setInput(text);
      setDraftAttachments(attachmentsToSend);
      setSendError(e?.message ? String(e.message) : String(e));
    } finally {
      setSendBusySafe(false);
    }
  };

  const onRemoveQueued = async (messageId: string) => {
    if (!id) return;
    if (!messageId) return;
    if (queueActionBusy) return;
    markQueueOptimisticallyRemoved(messageId);
    setQueueActionBusyId(messageId);
    setSendError(null);
    try {
      await deleteMessage(messageId);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== messageId),
      );
    } catch (e: any) {
      if (!shouldKeepQueueRemovalOnError(e)) {
        rollbackOptimisticQueueRemoval(messageId);
      }
      setSendError(e?.message ? String(e.message) : String(e));
    } finally {
      setQueueActionBusyId(null);
    }
  };

  const onEditQueued = async (message: Message) => {
    if (!id) return;
    if (queueActionBusy) return;
    const mid = idToString(message.id);
    if (!mid) return;
    const attachments = getQueuedAttachments(message);
    setInput(message.content ?? "");
    setDraftAttachments(attachments);
    markQueueOptimisticallyRemoved(mid);
    setQueueActionBusyId(mid);
    setSendError(null);
    try {
      await deleteMessage(mid);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== mid),
      );
    } catch (e: any) {
      if (!shouldKeepQueueRemovalOnError(e)) {
        rollbackOptimisticQueueRemoval(mid);
      }
      setSendError(e?.message ? String(e.message) : String(e));
    } finally {
      setQueueActionBusyId(null);
    }
  };

  const onSendQueuedNow = async (message: Message) => {
    if (!id) return;
    if (queueActionBusy || sendBusyRef.current) return;
    const mid = idToString(message.id);
    if (!mid) return;
    const attachments = getQueuedAttachments(message);
    const content = message.content ?? "";
    markQueueOptimisticallyRemoved(mid);
    setQueueActionBusyId(mid);
    setSendError(null);
    try {
      await interruptSession(id);
    } catch (e: any) {
      rollbackOptimisticQueueRemoval(mid);
      setSendError(e?.message ? String(e.message) : String(e));
      setQueueActionBusyId(null);
      return;
    }
    try {
      await deleteMessage(mid);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== mid),
      );
    } catch (e: any) {
      if (!shouldKeepQueueRemovalOnError(e)) {
        rollbackOptimisticQueueRemoval(mid);
      }
      setSendError(e?.message ? String(e.message) : String(e));
      setQueueActionBusyId(null);
      return;
    }
    setSendBusySafe(true);

    const optimisticId = createClientMessageId();
    const optimisticMessage: Message = {
      id: optimisticId,
      session_id: id,
      task_id: session?.task_id ?? "",
      turn_id: null,
      turn_sequence: null,
      role: "user",
      content,
      attachments,
      delivery: "immediate",
      created_at: new Date().toISOString(),
    };
    setPendingMessages((prev) => [...prev, { clientId: optimisticId, message: optimisticMessage }]);
    stickToBottomRef.current = true;
    setStickToBottom(true);
    liveScrollTopRef.current = null;
    pendingScrollToBottomRef.current = true;

    try {
      const posted = await postMessage(id, content, "immediate", attachments);
      setPendingMessages((prev) =>
        prev.map((entry) => (entry.clientId === optimisticId ? { ...entry, message: posted } : entry)),
      );
    } catch (e: any) {
      setPendingMessages((prev) => prev.filter((entry) => entry.clientId !== optimisticId));
      setSendError(e?.message ? String(e.message) : String(e));
    } finally {
      setSendBusySafe(false);
      setQueueActionBusyId(null);
    }
  };

  const fallbackSlashCommands = useMemo<SlashCommandDescriptor[]>(() => {
    const provider = session?.provider_id;
    if (provider === "codex") {
      return [
        { name: "review", description: "Review my current changes and find issues" },
        { name: "review-branch", description: "Review a branch" },
        { name: "review-commit", description: "Review a commit" },
        { name: "init", description: "Create an AGENTS.md file" },
        { name: "compact", description: "Summarize conversation to save context" },
        { name: "logout", description: "Log out" },
      ];
    }
    if (provider === "claude") {
      return [
        { name: "login", description: "Log in" },
        { name: "logout", description: "Log out" },
        { name: "compact", description: "Summarize conversation to save context" },
        { name: "help", description: "Show help" },
      ];
    }
    if (provider === "codex-crp") {
      return [
        { name: "review", description: "Review my current changes and find issues" },
        { name: "undo", description: "Undo the last turn" },
        { name: "compact", description: "Summarize conversation to save context" },
      ];
    }
    return [{ name: "compact", description: "Summarize conversation to save context" }];
  }, [session?.provider_id]);

  const slashCommands = fallbackSlashCommands;

  const virtuosoStyle = useMemo(() => ({ flex: 1, minHeight: 0 } as const), []);
  const workbenchViewportBy = useMemo(() => ({ top: 1000, bottom: 1000 }), []);
  const followOutput = useCallback(
    (_isAtBottom: boolean) => {
      if (!isActive) return false;
      if (scrollState?.stickToBottom === false) return false;
      return stickToBottomRef.current ? true : false;
    },
    [isActive, scrollState?.stickToBottom],
  );

  const wrapperClass = "wb-session-view";
  const leftClass = "wb-session-left";

  const dropScopeRef = useRef<HTMLDivElement | null>(null);

  const onDropFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      const next = await imageFilesToInlineAttachments(files);
      if (next.length === 0) return;
      setDraftAttachments((prev) => [...prev, ...next]);
    },
    [],
  );

  const showDropOverlay = useCallback(() => {
    setDropActive(true);
    if (dropHideTimerRef.current) window.clearTimeout(dropHideTimerRef.current);
    dropHideTimerRef.current = window.setTimeout(() => setDropActive(false), 140);
  }, []);

  const hideDropOverlay = useCallback(() => {
    if (dropHideTimerRef.current) window.clearTimeout(dropHideTimerRef.current);
    dropHideTimerRef.current = null;
    setDropActive(false);
  }, []);

  const extractFilesFromTransfer = useCallback(
    (dt: DataTransfer | null): File[] => {
      if (!dt) return [];
      const out: File[] = [];
      const files = dt.files ? Array.from(dt.files) : [];
      out.push(...files);
      const items = dt.items;
      if (out.length === 0 && items && items.length > 0) {
        for (const item of Array.from(items as any) as DataTransferItem[]) {
          if (item.kind !== "file") continue;
          const f = item.getAsFile?.();
          if (f) out.push(f);
        }
      }
      return out;
    },
    [],
  );

  const extractFirstUrlFromTransfer = useCallback((dt: DataTransfer | null): string | null => {
    if (!dt) return null;
    const uriRaw = (dt.getData?.("text/uri-list") ?? "").trim();
    if (uriRaw) {
      for (const line of uriRaw.split("\n")) {
        const v = line.trim();
        if (!v || v.startsWith("#")) continue;
        return v;
      }
    }
    const html = (dt.getData?.("text/html") ?? "").trim();
    if (html) {
      const m = html.match(/<img[^>]*\ssrc=("([^"]+)"|'([^']+)'|([^\s>]+))/i);
      const src = (m?.[2] ?? m?.[3] ?? m?.[4] ?? "").trim();
      if (src) return src;
    }
    const text = (dt.getData?.("text/plain") ?? "").trim();
    if (text && /^(https?:|data:image\/|blob:)/i.test(text)) return text;
    return null;
  }, []);

  const urlToImageFile = useCallback(async (url: string): Promise<File | null> => {
    try {
      const res = await fetch(url);
      if (!res.ok) return null;
      const blob = await res.blob();
      const type = blob.type || "";
      if (!type.startsWith("image/")) return null;
      const baseName = (() => {
        try {
          const u = new URL(url, window.location.href);
          const last = u.pathname.split("/").filter(Boolean).pop() || "image";
          return last.replace(/[?#].*$/, "") || "image";
        } catch {
          return "image";
        }
      })();
      const ext = type.split("/")[1] || "";
      const name = ext && !baseName.toLowerCase().endsWith(`.${ext.toLowerCase()}`) ? `${baseName}.${ext}` : baseName;
      return new File([blob], name, { type });
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    const el = dropScopeRef.current;
    if (!el) return;
    return registerDropScope({
      element: el,
      onDragOver: () => showDropOverlay(),
      onDrop: (dt) => {
        hideDropOverlay();
        void (async () => {
          const files = extractFilesFromTransfer(dt);
          if (files.length > 0) {
            await onDropFiles(files);
            return;
          }
          const url = extractFirstUrlFromTransfer(dt);
          if (!url) return;
          const asFile = await urlToImageFile(url);
          if (!asFile) return;
          await onDropFiles([asFile]);
        })();
      },
    });
  }, [extractFilesFromTransfer, extractFirstUrlFromTransfer, hideDropOverlay, onDropFiles, showDropOverlay, urlToImageFile]);

  const renderThreadItem = useCallback((item: ThreadItem) => {
    if (item.kind === "spacer") {
      return <div style={{ height: 1 }} />;
    }
    if (item.kind === "thought") {
      return <WorkbenchThoughtRow item={item} />;
    }
    if (item.kind === "turn_status") {
      return <WorkbenchTurnStatusRow item={item} nowMs={displayNowMs} />;
    }
    if (item.kind === "assistant") {
      if (!item.is_complete && item.content.trim().length === 0) {
        return null;
      }
      return (
        <AssistantEntry
          content={item.content}
          worktreeId={worktreeId}
          onFileOpenError={handleFileOpenError}
          modifierDown={modifierDown}
        />
      );
    }
    if (item.kind === "tool_group") {
      const expanded = expandedTurnDetailsById[item.turn_id] ?? false;
      const toolsLoading = turnToolsLoading.includes(item.turn_id);
      return (
        <WorkbenchToolGroupRow
          item={item}
          verbosity={verbosity}
          expanded={expanded}
          toolsLoading={toolsLoading}
          onToggle={() =>
            setExpandedTurnDetailsById((prev) => ({ ...prev, [item.turn_id]: !expanded }))
          }
          onRequestTools={() => supervisor.loadTurnTools(id, item.turn_id)}
          onToggleTool={(toolId) =>
            setExpandedToolById((prev) => ({ ...prev, [toolId]: !prev[toolId] }))
          }
          expandedToolById={expandedToolById}
        />
      );
    }
    if (item.kind === "tool") {
      const toolExpanded = expandedToolById[item.id] ?? false;
      return (
        <WorkbenchToolRow
          item={item}
          verbosity={verbosity}
          expanded={toolExpanded}
          onToggle={() => setExpandedToolById((prev) => ({ ...prev, [item.id]: !toolExpanded }))}
        />
      );
    }
    if (item.kind === "ask_user_question") {
      const isActive = item.tool_call_id === activeAskToolCallId;
      return (
        <AskUserQuestionCard
          input={item.input}
          answers={item.answers}
          outcome={item.outcome}
          readOnly={item.answered}
          active={isActive}
          onCancel={
            item.answered
              ? undefined
              : async () => {
                if (!id) throw new Error("Missing session id.");
                await submitAskUserQuestion(id, item.tool_call_id, "cancelled", {});
                setOptimisticAskAnswers((prev) => ({
                  ...prev,
                  [item.tool_call_id]: { outcome: "cancelled", answers: {} },
                }));
              }
          }
          onSubmit={
            item.answered
              ? undefined
              : async (answers) => {
                if (!id) throw new Error("Missing session id.");
                await submitAskUserQuestion(id, item.tool_call_id, "submitted", answers);
                setOptimisticAskAnswers((prev) => ({
                  ...prev,
                  [item.tool_call_id]: { outcome: "submitted", answers },
                }));
              }
          }
        />
      );
    }
    return (
      <ThreadItemView
        item={item}
        worktreeId={worktreeId}
        onFileOpenError={handleFileOpenError}
        modifierDown={modifierDown}
      />
    );
  }, [
    activeAskToolCallId,
    expandedToolById,
    expandedTurnDetailsById,
    handleFileOpenError,
    modifierDown,
    id,
    displayNowMs,
    supervisor,
    turnToolsLoading,
    worktreeId,
  ]);

  const workbenchItemContent = useCallback(
    (_: number, item: WorkbenchListItem) => {
      if (!item) return <div style={{ height: 1 }} />;
      const itemId = item.id;
      if (item.kind === "turn_header") {
        const header = (item as Extract<WorkbenchListItem, { kind: "turn_header" }>).header;
        const plainText = header.plain_text ?? markdownToPlainText(header.content ?? "");
        const isLong = plainText.split("\n").length > 4 || plainText.length > 280;
        const expanded = expandedTurnHeaders[header.id] ?? !isLong;
        return (
          <div data-thread-item-id={itemId} style={{ display: "contents" }}>
            <WorkbenchTurnHeaderView
              header={header}
              plainText={plainText}
              expanded={expanded}
              onToggle={() => setExpandedTurnHeaders((prev) => ({ ...prev, [header.id]: !expanded }))}
            />
          </div>
        );
      }
      const content = renderThreadItem(item as ThreadItem);
      return (
        <div className="wb-thread-indent" data-thread-item-id={itemId}>
          {content}
        </div>
      );
    },
    [expandedTurnHeaders, renderThreadItem],
  );

  const workbenchComponents = useMemo(
    () => ({
    Scroller: forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div
          {...props}
          ref={(node) => {
            scrollerRef.current = node;
            setScrollerNode((prev) => (prev === node ? prev : node));
            if (node) {
              scrollbarLastScrollTopRef.current = node.scrollTop;
              lastScrollHeightRef.current = node.scrollHeight;
              scheduleScrollbarUpdate();
            } else {
              scrollbarLastScrollTopRef.current = null;
              lastScrollHeightRef.current = null;
            }
            if (typeof ref === "function") ref(node);
            else if (ref) (ref as MutableRefObject<HTMLDivElement | null>).current = node;
          }}
          className={`wb-thread-scroller ${props.className ?? ""}`}
          style={{ ...props.style, overflowX: "hidden" }}
          onWheel={(event) => {
            props.onWheel?.(event);
            markUserScrollIntent();
          }}
          onPointerDown={(event) => {
            props.onPointerDown?.(event);
            markUserScrollIntent();
          }}
          onTouchStart={(event) => {
            props.onTouchStart?.(event);
            markUserScrollIntent();
          }}
          onScroll={(event) => {
            props.onScroll?.(event);
            if (!isActiveRef.current) return;
            const trusted =
              typeof (event as any).isTrusted === "boolean"
                ? (event as any).isTrusted
                : typeof (event as any).nativeEvent?.isTrusted === "boolean"
                  ? (event as any).nativeEvent.isTrusted
                  : true;
            const scroller = scrollerRef.current;
            const nextScrollTop = scroller?.scrollTop ?? 0;
            const prevScrollTop = scrollbarLastScrollTopRef.current ?? nextScrollTop;
            const didScroll = Math.abs(nextScrollTop - prevScrollTop) > 0.5;
            const now = Date.now();
            const hasUserIntent = now - userScrollIntentRef.current < userIntentWindowMs;
            const userIntent = hasUserIntent || scrollbarDraggingRef.current || trusted;
            scrollbarLastScrollTopRef.current = nextScrollTop;
            if (didScroll && (trusted || userIntent)) showScrollbarTemporarily();
            scheduleScrollbarUpdate();
            if (restoringScrollRef.current && !userIntent) return;
            if (restoringScrollRef.current && userIntent) {
              restoringScrollRef.current = false;
              if (restoreCooldownRef.current) {
                window.clearTimeout(restoreCooldownRef.current);
                restoreCooldownRef.current = null;
              }
              setRestoreInProgress(false);
            }
            const userScroll = didScroll && userIntent;
            if (userScroll && stickToBottomRef.current && scroller) {
              const remaining = scroller.scrollHeight - (scroller.scrollTop + scroller.clientHeight);
              const nearBottom = remaining <= bottomThresholdPx;
              if (!nearBottom) {
                stickToBottomRef.current = false;
                setStickToBottom(false);
              }
            }
            if (scrollerRef.current) {
              liveScrollTopRef.current = scrollerRef.current.scrollTop;
              if (pendingPrependRef.current && scrollerRef.current.scrollTop <= 1) {
                const anchorMeta = readAnchorMetaFromScroller(scrollerRef.current);
                if (anchorMeta?.id) {
                  pendingPrependAnchorRef.current = anchorMeta.id;
                  pendingPrependOffsetRef.current = anchorMeta.offset;
                } else if (!pendingPrependAnchorRef.current) {
                  pendingPrependAnchorRef.current = latestAnchorIdRef.current;
                  pendingPrependOffsetRef.current = latestAnchorOffsetRef.current ?? null;
                }
              }
            }
            if (!didInitialScrollRef.current) didInitialScrollRef.current = true;
            if (scrollSyncRafRef.current != null) return;
            scrollSyncRafRef.current = window.requestAnimationFrame(() => {
              scrollSyncRafRef.current = null;
              const el = scrollerRef.current;
              if (!el) return;
              const scrollTop = el.scrollTop;
              lastScrollHeightRef.current = el.scrollHeight;
              const remaining = el.scrollHeight - (scrollTop + el.clientHeight);
              const nearBottom = remaining <= bottomThresholdPx;
              let nextStickToBottom = stickToBottomRef.current;
              if (userScroll) {
                nextStickToBottom = nearBottom;
              }
              if (nextStickToBottom !== stickToBottomRef.current) {
                stickToBottomRef.current = nextStickToBottom;
                setStickToBottom(nextStickToBottom);
              }
              if (!nextStickToBottom) {
                updateAnchorFromScroller(el);
              }
              const persistStick = nextStickToBottom;
              if (!userScroll && persistStick) return;
              const next = {
                stickToBottom: persistStick,
                anchorItemId: persistStick ? null : latestAnchorIdRef.current,
                anchorOffset: persistStick ? null : latestAnchorOffsetRef.current,
                scrollTop: persistStick ? null : scrollTop,
              };
              if (virtuosoPersistTimerRef.current) {
                window.clearTimeout(virtuosoPersistTimerRef.current);
                virtuosoPersistTimerRef.current = null;
              }
              const handle = virtuosoRef.current;
              if (handle) {
                virtuosoPersistTimerRef.current = window.setTimeout(() => {
                  virtuosoPersistTimerRef.current = null;
                  handle.getState((state) => {
                    persistScroll({ ...next, virtuosoState: state });
                  });
                }, 120);
              } else {
                persistScroll(next);
              }
            });
          }}
        />
      )),
      List: forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div
          {...props}
          ref={(node) => {
            listRef.current = node;
            if (typeof ref === "function") ref(node);
            else if (ref) (ref as MutableRefObject<HTMLDivElement | null>).current = node;
          }}
          role="list"
          className={`wb-thread-list ${props.className ?? ""}`}
        />
      )),
      Footer: forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div
          {...props}
          ref={(node) => {
            bottomSentinelRef.current = node;
            setBottomSentinelNode((prev) => (prev === node ? prev : node));
            if (typeof ref === "function") ref(node);
            else if (ref) (ref as MutableRefObject<HTMLDivElement | null>).current = node;
          }}
          aria-hidden="true"
          data-bottom-sentinel="true"
          style={{ height: 1, minHeight: 1, ...props.style }}
        />
      )),
      Item: forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div {...props} ref={ref} role="listitem" />
      )),
    }),
    [
      bottomThresholdPx,
      persistScroll,
      scheduleScrollbarUpdate,
      showScrollbarTemporarily,
      updateAnchorFromScroller,
      userIntentWindowMs,
    ],
  );

  return (
    <div
      className={`${wrapperClass} ctx-drop-scope`}
      ref={dropScopeRef}
      data-testid="session-view"
      data-session-id={id}
      data-thread-count={wbListItems.length}
    >
      {dropActive && (
        <div className="ctx-drop-overlay" aria-hidden="true">
          <div className="ctx-drop-overlay-text">Drop image to attach</div>
        </div>
      )}
      <div className={leftClass}>
        {entry?.error && (
          <div className="banner">
            <span className="error">{entry.error}</span>
          </div>
        )}
        {sessionError && (
          <div className="banner" role="alert">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <strong>Error</strong>
              {sessionError.provider ? <span className="muted">{sessionError.provider}</span> : null}
            </div>
            <div className="error" style={{ whiteSpace: "pre-wrap" }}>
              {sessionError.message}
            </div>
          </div>
        )}
        {providerGuardNotice && (
          <div className="banner" role="alert">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <strong>{providerGuardHeading}</strong>
              {providerGuardProviderLabel ? <span className="muted">{providerGuardProviderLabel}</span> : null}
            </div>
            <div
              className={providerGuardNotice.kind === "provider_guard_kill" ? "error" : "muted"}
              style={{ whiteSpace: "pre-wrap" }}
            >
              {providerGuardMessage}
            </div>
            <div className="row" style={{ flexWrap: "wrap", gap: 12 }}>
              {providerGuardNotice.memoryMb != null ? (
                <span className="muted">
                  Memory {formatMemoryMb(providerGuardNotice.memoryMb)}
                  {providerGuardMemoryLimitMb != null
                    ? ` / ${formatMemoryMb(providerGuardMemoryLimitMb)} (${providerGuardLimitLabel})`
                    : ""}
                </span>
              ) : null}
              {providerGuardNotice.systemUsedMb != null && providerGuardNotice.systemTotalMb != null ? (
                <span className="muted">
                  System {formatMemoryMb(providerGuardNotice.systemUsedMb)} / {formatMemoryMb(providerGuardNotice.systemTotalMb)}
                </span>
              ) : null}
              {providerGuardPidLabel ? <span className="muted">{providerGuardPidLabel}</span> : null}
            </div>
            {providerGuardCountdownText && <div className="muted">{providerGuardCountdownText}</div>}
            <div className="row" style={{ flexWrap: "wrap", gap: 8 }}>
              <button
                type="button"
                disabled={providerGuardActionBusy || !canRaiseProviderGuard}
                onClick={raiseProviderGuardLimit}
                title={!canRaiseProviderGuard ? "System memory total is unavailable." : undefined}
              >
                Raise limit to 90%
              </button>
              <button
                type="button"
                disabled={providerGuardActionBusy}
                onClick={disableProviderGuard}
              >
                Disable guard
              </button>
              {providerGuardActionError && <span className="error">{providerGuardActionError}</span>}
            </div>
          </div>
        )}
        {showDebug && (
          <div className="wb-muted" style={{ fontFamily: "var(--mono)" }}>
            debug: events={events.length} messages={messages.length} userMessages={messages.filter((m) => m.role === "user").length} items={wbListItems.length}
          </div>
        )}
        {(authUi.status === "required" || authUi.status === "failed") && (
          <div className="banner">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <strong>Authentication required</strong>
              <span className="muted">{authUi.provider ?? session?.provider_id}</span>
            </div>
            <div className="muted">
              {authUi.message ??
                "This provider requires authentication before it can run."}
            </div>
            {authUi.methods.length > 0 ? (
              <div className="row" style={{ flexWrap: "wrap" }}>
                {authUi.methods.length > 1 && (
                  <label>
                    Method
                    <select
                      value={authMethodId}
                      onChange={(e) => setAuthMethodId(e.target.value)}
                    >
                      {authUi.methods.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.name}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
                <button
                  type="button"
                  disabled={authBusy || !id || !authMethodId}
                  onClick={async () => {
                    if (!id) return;
                    setAuthBusy(true);
                    setAuthError(null);
                    try {
                      await authenticateSession(id, authMethodId);
                      await refreshAll();
                    } catch (e: any) {
                      setAuthError(e?.message ?? String(e));
                    } finally {
                      setAuthBusy(false);
                    }
                  }}
                >
                  {authBusy ? "Authenticating..." : "Authenticate"}
                </button>
              </div>
            ) : (
              <div className="muted">
                No authentication methods were advertised by the provider.
              </div>
            )}
            {(authError || authUi.status === "failed") && (
              <div className="muted">
                {authError ??
                  "Authentication attempt failed. Check provider logs and try again."}
              </div>
            )}
          </div>
        )}
        {(subagentInvocationsLoading || subagentInvocations.length > 0) && (
          <div className="subagent-invocations card">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <strong>Subagent invocations</strong>
              {subagentInvocationsLoading ? (
                <span className="muted">Loading...</span>
              ) : (
                <span className="muted">{subagentInvocations.length}</span>
              )}
            </div>
            {subagentInvocations.length === 0 ? (
              <div className="muted">Waiting for subagent data...</div>
            ) : (
              <div className="subagent-invocation-list">
                {subagentInvocations.map((invocation) => {
                  const children = invocation.children ?? [];
                  const countLabel = `${children.length}/${invocation.requested_count}`;
                  return (
                    <div key={invocation.id} className="subagent-invocation-row">
                      <div className="row" style={{ justifyContent: "space-between", alignItems: "baseline" }}>
                        <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
                          <span className="badge">{humanToolStatus(invocation.status)}</span>
                          <span className="muted">Subagents {countLabel}</span>
                        </div>
                      </div>
                      {children.length > 0 ? (
                        <ul className="sublist subagent-invocation-children">
                          {children.map((child) => {
                            const childId = idToString(child.child_session_id);
                            const label = subagentChildLabel(child);
                            const meta = formatSubagentChildMeta(child);
                            return (
                              <li key={`${invocation.id}:${childId || child.position}`} className="row subagent-child-row">
                                <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
                                  <span className="badge">{humanToolStatus(child.status)}</span>
                                  <button
                                    type="button"
                                    className="subagent-child-link"
                                    onClick={() => childId && openChildSession(childId)}
                                    disabled={!childId}
                                  >
                                    {label}
                                  </button>
                                </div>
                                <span className="muted">{meta}</span>
                              </li>
                            );
                          })}
                        </ul>
                      ) : (
                        <div className="muted">No child sessions yet.</div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {showDebug && debugEvents.length > 0 && (
          <DebugPanel events={debugEvents} />
        )}

        <WorkbenchThreadStack
          virtuosoStyle={virtuosoStyle}
          data={wbListItems}
          firstItemIndex={firstItemIndex}
          virtuosoRef={virtuosoRef}
          initialTopMostItemIndex={initialTopMostItemIndex}
          restoreStateFrom={restoreStateFrom}
          increaseViewportBy={workbenchViewportBy}
          onStartReached={handleStartReached}
          onRangeChanged={handleWorkbenchRangeChanged}
          components={workbenchComponents}
          itemContent={workbenchItemContent}
          showJumpToLatest={!atBottom}
          onJumpToLatest={jumpToLatestWorkbench}
          followOutput={followOutput}
          onAtBottomStateChange={setAtBottom}
          scrollbarActive={scrollbarActive}
          scrollbarDragging={scrollbarDragging}
          scrollbarNeeded={scrollbarNeeded}
          scrollbarTrackRef={scrollbarTrackRef}
          scrollbarThumbRef={scrollbarThumbRef}
          onScrollbarMouseLeave={handleScrollbarMouseLeave}
          onScrollbarTrackPointerDown={handleScrollbarTrackPointerDown}
          onScrollbarThumbPointerDown={handleScrollbarThumbPointerDown}
          onScrollbarThumbPointerMove={handleScrollbarThumbPointerMove}
          onScrollbarThumbPointerUp={handleScrollbarThumbPointerUp}
          scheduleScrollbarUpdate={scheduleScrollbarUpdate}
        />
        <div className="wb-session-bottom">
          {showQueuePanel && (
            <div className="queue-panel card" aria-label="Queued messages">
              <div className="queue-header">
                <ChevronDown size={14} aria-hidden="true" />
                <span className="queue-header-title">{queueForPanel.length} Queued</span>
              </div>
              <ul className="queue-list" role="list">
                {queueForPanel.map((m, index) => {
                  const messageId = idToString(m.id);
                  const rowKey = messageId || `queued-${index}`;
                  const attachments = getQueuedAttachments(m);
                  const preview = formatQueuedPreview(m, attachments);
                  const attachmentMeta = formatQueuedAttachmentMeta(attachments);
                  const isPending = !!messageId && messageId.startsWith("client-");
                  const canInteract = !!messageId && !isPending;
                  const canSendNow = index === 0 && canInteract;
                  return (
                    <li key={rowKey} className="queue-item">
                      <span className="queue-item-dot" aria-hidden="true" />
                      <div className="queue-item-body">
                        <div className="queue-item-content" title={preview}>
                          {preview}
                        </div>
                        {attachmentMeta && (
                          <div className="queue-item-meta" title={attachmentMeta.title}>
                            <span>{attachmentMeta.label}</span>
                            {attachmentMeta.detail && (
                              <span className="queue-item-meta-detail">{attachmentMeta.detail}</span>
                            )}
                          </div>
                        )}
                      </div>
                      <div className="queue-item-actions">
                        {canSendNow && (
                          <button
                            type="button"
                            className="queue-action"
                            disabled={queueActionBusy || sendBusy}
                            onClick={() => onSendQueuedNow(m)}
                            aria-label="Send now"
                            title="Send now"
                          >
                            <CornerUpRight size={14} aria-hidden="true" />
                          </button>
                        )}
                        <button
                          type="button"
                          className="queue-action"
                          disabled={queueActionBusy || !canInteract}
                          onClick={() => onEditQueued(m)}
                          aria-label="Edit queued message"
                          title="Edit"
                        >
                          <Pencil size={14} aria-hidden="true" />
                        </button>
                        <button
                          type="button"
                          className="queue-action"
                          disabled={queueActionBusy || !canInteract}
                          onClick={() => onRemoveQueued(messageId)}
                          aria-label="Cancel queued message"
                          title="Cancel"
                        >
                          <Trash2 size={14} aria-hidden="true" />
                        </button>
                      </div>
                    </li>
                  );
                })}
              </ul>
            </div>
          )}

          <UnifiedWorkbenchComposer
            variant="activeSession"
            value={input}
            setValue={setInput}
            placeholder="@ for context, / for commands"
            inputDisabled={dictationRecording}
            sessionIdForAutocomplete={id ?? null}
            slashCommands={slashCommands}
            attachments={draftAttachments}
            setAttachments={setDraftAttachments}
            onSend={sendNow}
            sendDisabled={sendBusy || !hasDraftContent}
            sendDisabledReason={sendBusy ? "Sending..." : !hasDraftContent ? "Enter a message." : null}
            onInterrupt={id ? () => interruptSession(id) : null}
            isWorking={hasActiveTurn}
            verbosity={verbosity}
            onSetVerbosity={setVerbosityPref}
            modeId={workbenchMode}
            setModeId={setWorkbenchMode}
            contextWindow={contextWindow}
            recording={dictationRecording}
            onToggleRecording={() => {
              if (dictationRecording) stopDictation().catch(() => { });
              else startDictation().catch(() => { });
            }}
            harnessLabel={
              HARNESS_CATALOG.find((h) => h.id === (session?.provider_id ?? ""))?.label ??
              (session?.provider_id ?? "Provider")
            }
            harnessLogoSrc={HARNESS_CATALOG.find((h) => h.id === (session?.provider_id ?? ""))?.logoSrc}
            harnessLogoInvert={HARNESS_CATALOG.find((h) => h.id === (session?.provider_id ?? ""))?.invertInDark}
            harnessLogoInvertInLight={
              HARNESS_CATALOG.find((h) => h.id === (session?.provider_id ?? ""))?.invertInLight
            }
            availableModels={modelOptions}
            currentModelId={currentModelId}
            onSetModelId={async (next) => {
              if (!id) return;
              const updated = await setSessionModel(id, next);
              supervisor.setSession(updated);
            }}
          />
          {sendError && <div className="wb-banner">{sendError}</div>}
          {fileOpenError && <div className="wb-banner">{fileOpenError}</div>}
          {dictationDebugText && <div className="wb-banner">{dictationDebugText}</div>}
          {dictationError && <div className="wb-banner">{dictationError}</div>}
        </div>

        <div className="sr-only" aria-live="polite">
          {session && (atBottom ? "Agent output updating." : "New agent activity.")}
        </div>
      </div>

    </div>
  );
}

function DebugPanel({ events }: { events: SessionEvent[] }) {
  const [open, setOpen] = useState(false);
  const kinds = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const e of events) {
      counts[e.event_type] = (counts[e.event_type] ?? 0) + 1;
    }
    return counts;
  }, [events.length]);

  return (
    <div className="debug card">
      <button type="button" className="debug-header" onClick={() => setOpen((v) => !v)}>
        <strong>Debug</strong>
        <span className="muted">
          {Object.entries(kinds)
            .map(([k, v]) => `${k}:${v}`)
            .join(" · ")}
        </span>
        <span className="thinking-chev">{open ? "▴" : "▾"}</span>
      </button>
      {open && (
        <div className="debug-body">
          {events.map((e) => {
            const key = idToString(e.id) || `${e.created_at}-${e.event_type}`;
            return (
              <details key={key} className="debug-event">
                <summary>
                  <span className="muted">{new Date(e.created_at).toLocaleTimeString()}</span>{" "}
                  <strong>{e.event_type}</strong>
                </summary>
                <pre className="json">{JSON.stringify(e.payload_json, null, 2)}</pre>
              </details>
            );
          })}
        </div>
      )}
    </div>
  );
}
