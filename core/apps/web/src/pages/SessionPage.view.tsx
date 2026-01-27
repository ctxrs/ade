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
  DictationSettings,
  resolveDaemonWsBaseUrl,
  Message,
  type MessageAttachment,
  postMessage,
  Session,
  SessionEvent,
  SubagentInvocation,
  setSessionModel,
  authenticateSession,
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
import { startMicPcmStream } from "../utils/micPcmStream";
import { parseWsJson } from "../utils/wsJson";
import { imageFilesToInlineAttachments } from "../utils/messageAttachments";
import { registerDropScope } from "../utils/dragDropScopes";
import { useRelativeNowMs } from "../utils/useRelativeNowMs";
import { usePinnedScrollManager } from "./usePinnedScrollManager";
import { useWorkbenchStore } from "../workbench/store";
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

const createClientMessageId = (): string => {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `client-${crypto.randomUUID()}`;
  }
  return `client-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
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
    scrollTop: number | null;
    virtuosoState?: unknown | null;
  } | null;
  onScrollStateChange?: ((
    next: {
      stickToBottom: boolean;
      anchorItemId: string | null;
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
  const [sendError, setSendError] = useState<string | null>(null);
  const [queueActionBusyId, setQueueActionBusyId] = useState<string | null>(null);
  const [pendingMessages, setPendingMessages] = useState<PendingMessageEntry[]>([]);
  const [pendingQueueMessages, setPendingQueueMessages] = useState<PendingMessageEntry[]>([]);
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
    restoreRetryRef.current = 0;
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
  const restoreRetryRef = useRef(0);
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
      scrollTop: number | null;
      virtuosoState?: unknown | null;
    }) => {
      if (!onScrollStateChange) return;
      const prev = lastScrollPersistedRef.current;
      if (
        prev &&
        prev.stickToBottom === next.stickToBottom &&
        prev.anchorItemId === next.anchorItemId &&
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
    const scroller = scrollerRef.current;
    if (!scroller) return;
    const observer = new ResizeObserver(() => scheduleScrollbarUpdate());
    observer.observe(scroller);
    return () => observer.disconnect();
  }, [scheduleScrollbarUpdate]);

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
    restorePendingRef.current = true;
    restoreRetryRef.current = 0;
    didInitialScrollRef.current = false;
    lastScrollPersistedRef.current = null;
    latestAnchorIdRef.current = null;
    liveScrollTopRef.current = null;
    restoringScrollRef.current = false;
    autoScrollRef.current = 0;
    autoScrollAttemptRef.current = 0;
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

  useEffect(() => {
    return () => {
      if (!onScrollStateChange) return;
      if (stickToBottomRef.current) return;
      const el = scrollerRef.current;
      if (!el) return;
      const scrollTop = el.scrollTop;
      const remaining = el.scrollHeight - (scrollTop + el.clientHeight);
      const nearBottom = remaining <= bottomThresholdPx;
      const stickToBottom = stickToBottomRef.current ? nearBottom : false;
      persistScroll({
        stickToBottom,
        anchorItemId: stickToBottom ? null : latestAnchorIdRef.current,
        scrollTop: stickToBottom ? null : scrollTop,
        virtuosoState: lastScrollPersistedRef.current?.virtuosoState ?? undefined,
      });
    };
  }, [id, onScrollStateChange, persistScroll, bottomThresholdPx]);

  const [dictationSettings, setDictationSettings] = useState<DictationSettings | null>(null);
  const [dictationRecording, setDictationRecording] = useState(false);
  const [dictationError, setDictationError] = useState<string | null>(null);
  const dictationWsRef = useRef<WebSocket | null>(null);
  const dictationMicRef = useRef<{ stop: () => Promise<void> } | null>(null);
  const dictationBaseRef = useRef<string>("");
  const dictationCommittedRef = useRef<string>("");
  const dictationInterimRef = useRef<string>("");
  const dictationDebugEnabled = useMemo(() => {
    try {
      return new URLSearchParams(window.location.search).get("dictation_debug") === "1";
    } catch {
      return false;
    }
  }, []);
  const [dictationDebugText, setDictationDebugText] = useState<string | null>(null);
  const dictationAudioBytesRef = useRef(0);
  const dictationAudioChunksRef = useRef(0);
  const dictationReadyRef = useRef(false);
  const dictationAudioStartedRef = useRef(false);
  const dictationTranscriptMsgsRef = useRef(0);
  const dictationFinalizeWaiterRef = useRef<{ promise: Promise<void>; resolve: () => void } | null>(null);
  const dictationSuppressUpdatesRef = useRef(false);

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
  const mergedQueueForPanel = useMemo(
    () => mergeQueuedMessagesForPanel(queue, pendingQueueMessages),
    [queue, pendingQueueMessages],
  );
  const queueForPanel = useMemo(
    () => filterQueuedMessagesForPanel(mergedQueueForPanel, turns),
    [mergedQueueForPanel, turnsKey],
  );
  const showQueuePanel = queueForPanel.length > 0;
  const queuedMessageIdsForThread = useMemo(() => {
    if (queueForPanel.length === 0) return new Set<string>();
    const ids = new Set<string>();
    for (const message of queueForPanel) {
      const mid = idToString(message.id);
      if (mid) ids.add(mid);
    }
    return ids;
  }, [queueForPanel]);
  useEffect(() => {
    if (pendingMessages.length === 0) return;
    const realIds = new Set(messages.map((m) => idToString(m.id)));
    setPendingMessages((prev) => {
      if (prev.length === 0) return prev;
      const next = prev.filter((entry) => {
        const pid = idToString(entry.message.id);
        return pid ? !realIds.has(pid) : true;
      });
      return next.length === prev.length ? prev : next;
    });
  }, [messagesKey, pendingMessages.length, messages]);
  useEffect(() => {
    if (pendingQueueMessages.length === 0) return;
    const realIds = new Set(queue.map((m) => idToString(m.id)));
    setPendingQueueMessages((prev) => {
      if (prev.length === 0) return prev;
      const next = prev.filter((entry) => {
        const pid = idToString(entry.message.id);
        return pid ? !realIds.has(pid) : true;
      });
      return next.length === prev.length ? prev : next;
    });
  }, [queue, pendingQueueMessages.length]);

  const displayMessages = useMemo(
    () => mergeMessagesForView(messages, pendingMessages),
    [messagesKey, pendingMessages],
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
  const displayNowMs = needsNowMs ? nowMs : Date.now();

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

  const stopDictation = useCallback(async (opts?: { awaitFinal?: boolean }): Promise<string> => {
    const awaitFinal = opts?.awaitFinal === true;
    const ws = dictationWsRef.current;
    const mic = dictationMicRef.current;
    const base = dictationBaseRef.current;
    const committed = dictationCommittedRef.current;
    const interim = dictationInterimRef.current;

    const hasActiveDictation =
      dictationRecording || !!mic || (ws ? ws.readyState !== WebSocket.CLOSED : false) || !!base || !!committed || !!interim;

    if (!hasActiveDictation) return input;

    setDictationRecording(false);
    dictationMicRef.current = null;

    try {
      await mic?.stop();
    } catch { }

    let finalizeWaiter = dictationFinalizeWaiterRef.current;
    if (ws && ws.readyState !== WebSocket.CLOSED) {
      if (!finalizeWaiter) {
        let resolve: () => void = () => { };
        const promise = new Promise<void>((res) => {
          resolve = res;
        });
        finalizeWaiter = { promise, resolve };
        dictationFinalizeWaiterRef.current = finalizeWaiter;
      }
    }

    try {
      ws?.send(JSON.stringify({ type: "stop" }));
    } catch { }

    if (awaitFinal && finalizeWaiter) {
      await Promise.race([
        finalizeWaiter.promise,
        new Promise<void>((resolve) => window.setTimeout(resolve, 8000)),
      ]);
    }

    const next = appendSegment(
      appendSegment(dictationBaseRef.current, dictationCommittedRef.current),
      dictationInterimRef.current,
    );
    if (next !== input) setInput(next);
    if (awaitFinal) {
      dictationSuppressUpdatesRef.current = true;
      dictationBaseRef.current = "";
      dictationCommittedRef.current = "";
      dictationInterimRef.current = "";
    }
    dictationInterimRef.current = "";
    dictationReadyRef.current = false;
    dictationAudioStartedRef.current = false;

    return next;
  }, [dictationRecording, input, setInput]);

  const startDictation = useCallback(async () => {
    setDictationError(null);

    let settings = dictationSettings;
    if (!settings) {
      try {
        const s = await getSettings();
        settings = s.dictation ?? null;
        setDictationSettings(settings);
      } catch (e: any) {
        setDictationError(e?.message ?? "Failed to load dictation settings.");
        return;
      }
    }

    const enabled = Boolean(settings?.enabled) && settings?.provider === "livekit_inference";
    if (!enabled) {
      setDictationError("Dictation is disabled. Configure it in Settings.");
      return;
    }
    const existing = dictationWsRef.current;
    if (existing && existing.readyState !== WebSocket.CLOSED) return;
    if (dictationRecording) return;

    const token = (() => {
      try {
        return sessionStorage.getItem("ctxAuthToken");
      } catch {
        return null;
      }
    })();
    const wsBase = resolveDaemonWsBaseUrl();
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";
    const ws = new WebSocket(`${wsBase}/api/dictation/livekit/stream${qs}`);
    ws.binaryType = "arraybuffer";
    dictationWsRef.current = ws;
    dictationFinalizeWaiterRef.current = null;
    dictationSuppressUpdatesRef.current = false;

    dictationBaseRef.current = input;
    dictationCommittedRef.current = "";
    dictationInterimRef.current = "";
    dictationAudioBytesRef.current = 0;
    dictationAudioChunksRef.current = 0;
    dictationReadyRef.current = false;
    dictationAudioStartedRef.current = false;
    dictationTranscriptMsgsRef.current = 0;

    const openPromise = new Promise<void>((resolve, reject) => {
      ws.addEventListener("open", () => resolve(), { once: true });
      ws.addEventListener("error", () => reject(new Error("Failed to connect to dictation stream.")), { once: true });
    });

    ws.addEventListener("message", (ev) => {
      void parseWsJson((ev as MessageEvent).data).then((data) => {
        if (!data) return;
        const t = String(data.type ?? "");
        if (t === "ready") {
          dictationReadyRef.current = true;
          return;
        } else if (t === "audio_started") {
          dictationAudioStartedRef.current = true;
          return;
        } else if (t === "interim") {
          dictationTranscriptMsgsRef.current += 1;
          dictationInterimRef.current = String(data.text ?? "");
        } else if (t === "final") {
          dictationTranscriptMsgsRef.current += 1;
          dictationCommittedRef.current = appendSegment(dictationCommittedRef.current, String(data.text ?? ""));
          dictationInterimRef.current = "";
        } else if (t === "done") {
          dictationFinalizeWaiterRef.current?.resolve();
          dictationFinalizeWaiterRef.current = null;
          try {
            ws.close();
          } catch { }
          return;
        } else if (t === "error") {
          setDictationError(String(data.message ?? "Dictation error"));
          dictationFinalizeWaiterRef.current?.resolve();
          dictationFinalizeWaiterRef.current = null;
          stopDictation().catch(() => { });
          return;
        } else {
          return;
        }

        if (dictationSuppressUpdatesRef.current) return;
        const base = dictationBaseRef.current;
        const committed = dictationCommittedRef.current;
        const interim = dictationInterimRef.current;
        setInput(appendSegment(appendSegment(base, committed), interim));
      });
    });

    ws.addEventListener("close", () => {
      dictationWsRef.current = null;
      setDictationRecording(false);
      dictationFinalizeWaiterRef.current?.resolve();
      dictationFinalizeWaiterRef.current = null;
    });

    try {
      await openPromise;
      setDictationRecording(true);
      dictationMicRef.current = await startMicPcmStream({
        onPcmChunk: (pcm16) => {
          dictationAudioChunksRef.current += 1;
          dictationAudioBytesRef.current += pcm16.byteLength;
          if (ws.readyState === WebSocket.OPEN) ws.send(pcm16);
        },
        onError: (err) => {
          setDictationError(err.message);
          stopDictation().catch(() => { });
        },
      });
    } catch (e: any) {
      setDictationError(e?.message ?? String(e));
      try {
        ws.close();
      } catch { }
      dictationWsRef.current = null;
      setDictationRecording(false);
    }
  }, [dictationSettings, dictationRecording, input, stopDictation]);

  const stopDictationRef = useRef(stopDictation);
  useEffect(() => {
    stopDictationRef.current = stopDictation;
  }, [stopDictation]);

  useEffect(() => {
    return () => {
      stopDictationRef.current().catch(() => { });
    };
  }, []);

  useEffect(() => {
    if (!dictationDebugEnabled) return;
    if (!dictationRecording) {
      setDictationDebugText(null);
      return;
    }
    const timer = window.setInterval(() => {
      const ws = dictationWsRef.current;
      const wsState = ws ? ws.readyState : -1;
      setDictationDebugText(
        `Dictation debug\nws_state=${wsState} ready=${dictationReadyRef.current} audio_started=${dictationAudioStartedRef.current}\naudio_chunks=${dictationAudioChunksRef.current} audio_bytes=${dictationAudioBytesRef.current} transcript_msgs=${dictationTranscriptMsgsRef.current}`,
      );
    }, 500);
    return () => window.clearInterval(timer);
  }, [dictationDebugEnabled, dictationRecording]);

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

  useLayoutEffect(() => {
    if (prevSessionForIndexRef.current !== id) {
      prevSessionForIndexRef.current = id;
      setFirstItemIndex(initialVirtuosoIndex);
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
    let indexShift: number | null = null;
    const anchorId =
      latestAnchorIdRef.current ?? prevItems[0]?.id ?? prevItems[prevItems.length - 1]?.id;
    if (anchorId) {
      const prevIndex = prevItems.findIndex((item) => item.id === anchorId);
      const nextIndex = wbListItems.findIndex((item) => item.id === anchorId);
      if (prevIndex >= 0 && nextIndex >= 0) {
        indexShift = nextIndex - prevIndex;
      }
    }
    if (indexShift == null && wbListItems.length > prevItems.length) {
      const prevFirstId = prevItems[0]?.id;
      if (prevFirstId) {
        const startIndex = wbListItems.findIndex((item) => item.id === prevFirstId);
        if (startIndex >= 0) {
          indexShift = startIndex;
        }
      }
      if (indexShift == null) {
        const prevLastId = prevItems[prevItems.length - 1]?.id;
        if (prevLastId) {
          const endIndex = wbListItems.findIndex((item) => item.id === prevLastId);
          if (endIndex >= 0) {
            const startIndex = endIndex - (prevItems.length - 1);
            if (startIndex >= 0) {
              indexShift = startIndex;
            }
          }
        }
      }
      if (indexShift == null) {
        indexShift = wbListItems.length - prevItems.length;
      }
    }
    if (indexShift != null && indexShift !== 0) {
      setFirstItemIndex((prev) => prev - indexShift);
    }
    prevItemsRef.current = wbListItems;
  }, [id, initialVirtuosoIndex, wbListItems]);

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
      const hasVirtuosoState = Boolean(scrollState?.virtuosoState);
      const shouldUseVirtuosoRestore = hasVirtuosoState && !preserveScrollOnFocus && scrollState?.stickToBottom === false;
      restorePendingRef.current = !shouldUseVirtuosoRestore;
      didInitialScrollRef.current = false;
      restoringScrollRef.current = false;
      const hasScrolledAway = scrollState?.stickToBottom === false;
      const nextStickToBottom = !hasScrolledAway;
      stickToBottomRef.current = nextStickToBottom;
      setStickToBottom(nextStickToBottom);
      if (nextStickToBottom) {
        liveScrollTopRef.current = null;
      }
      restoreRetryRef.current = 0;
      if (shouldUseVirtuosoRestore) {
        restoringScrollRef.current = true;
        setRestoreInProgress(true);
        const timer = window.setTimeout(() => {
          restoringScrollRef.current = false;
          setRestoreInProgress(false);
        }, 200);
        return () => window.clearTimeout(timer);
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
    const state = scrollState ?? { stickToBottom: true, anchorItemId: null, scrollTop: null };
    setAtBottom(state.stickToBottom);
    const restoreAnchorId = !state.stickToBottom ? (state.anchorItemId ?? null) : null;
    const restoreScrollTop = !state.stickToBottom ? (state.scrollTop ?? null) : null;
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
      if (restoreScrollTop !== null) {
        const target = Math.max(0, restoreScrollTop);
        markAutoScroll();
        if (handle) {
          handle.scrollTo({ top: target });
        } else if (el) {
          el.scrollTop = target;
          if (Math.abs(el.scrollTop - target) > 2 && restoreRetryRef.current < 3) {
            restoreRetryRef.current += 1;
            requestAnimationFrame(attemptRestore);
            return;
          }
        }
      } else if (restoreAnchorId) {
        const idx = items.findIndex((it) => it?.id === restoreAnchorId);
        const lastIndex = firstItemIndex + items.length - 1;
        if (idx >= 0) {
          markAutoScroll();
          handle?.scrollToIndex({ index: firstItemIndex + idx, align: "start" });
        } else {
          markAutoScroll();
          handle?.scrollToIndex({ index: lastIndex, align: "end" });
        }
      } else {
        const lastIndex = firstItemIndex + items.length - 1;
        markAutoScroll();
        handle?.scrollToIndex({ index: lastIndex, align: "end" });
      }
      finalizeRestore();
    };

    requestAnimationFrame(() => {
      restoreRetryRef.current = 0;
      attemptRestore();
    });
  }, [
    id,
    isActive,
    preserveScrollOnFocus,
    scrollState?.anchorItemId,
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

  const acpSessionInfo = useMemo(() => {
    return [...events]
      .reverse()
      .find((e) => e.event_type === "init" && (e.payload_json?.models || e.payload_json?.modes));
  }, [eventsKey]);

  const acpModels = entry?.acpModels ?? acpSessionInfo?.payload_json?.models;
  const acpCurrentModelId =
    entry?.acpCurrentModelId ?? acpModels?.currentModelId ?? acpModels?.current_model_id;
  const modelOptions = useMemo(() => {
    const list =
      acpModels?.availableModels ??
      acpModels?.available_models ??
      acpModels?.available_models ??
      [];
    if (!Array.isArray(list)) return [];
    return list
      .map((m: any) => ({
        id: m.modelId ?? m.model_id ?? m.id,
        name: m.name ?? (m.modelId ?? m.model_id ?? m.id),
      }))
      .filter((m: any) => typeof m.id === "string" && m.id.length > 0);
  }, [acpModels]);
  const modelOptionIds = useMemo(() => new Set(modelOptions.map((m) => String(m.id))), [modelOptions]);

  const currentModelId = useMemo(() => {
    const sessionModelId = String(session?.model_id ?? "").trim();
    const acpModelId = String(acpCurrentModelId ?? "").trim();
    if (!sessionModelId || sessionModelId === "default") {
      return acpModelId || sessionModelId;
    }
    if (modelOptionIds.size > 0 && !modelOptionIds.has(sessionModelId)) {
      return acpModelId || sessionModelId;
    }
    return sessionModelId || acpModelId;
  }, [acpCurrentModelId, modelOptionIds, session?.model_id]);

  const restoreStateFrom =
    preserveScrollOnFocus || scrollState?.stickToBottom !== false || !restorePendingRef.current
      ? undefined
      : (scrollState?.virtuosoState ?? undefined) as StateSnapshot | undefined;

  const jumpToLatestWorkbench = useCallback(() => {
    if (wbListItems.length > 0) {
      stickToBottomRef.current = true;
      setStickToBottom(true);
      setAtBottom(true);
      latestAnchorIdRef.current = null;
      liveScrollTopRef.current = null;
      persistScroll({ stickToBottom: true, anchorItemId: null, scrollTop: null });
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
    jumpToLatestWorkbench();
  }, [stickToBottom, restoreInProgress, preserveScrollOnFocus, isActive, wbListItems.length, jumpToLatestWorkbench]);
  const handleWorkbenchRangeChanged = useCallback(
    (range: { startIndex: number }) => {
      if (restoringScrollRef.current) return;
      if (atBottom) return;
      const dataIndex = range.startIndex - firstItemIndex;
      const item = wbListItems[dataIndex];
      if (!item) return;
      latestAnchorIdRef.current = item.id ?? null;
    },
    [atBottom, firstItemIndex, wbListItems],
  );

  const handleStartReached = useCallback(() => {
    if (!hasMoreTurns) return;
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

  const sendNow = async () => {
    if (!id) return;
    if (sendBusy) return;
    const text = (dictationRecording ? await stopDictation({ awaitFinal: true }) : input).trim();
    if (!text) return;
    const attachmentsToSend = draftAttachments;
    const shouldQueue = hasActiveTurn;
    const optimisticId = createClientMessageId();
    const optimisticMessage: Message = {
      id: optimisticId,
      session_id: id,
      task_id: session?.task_id ?? "",
      turn_id: null,
      turn_sequence: null,
      role: "user",
      content: text,
      attachments: attachmentsToSend,
      delivery: shouldQueue ? "queued" : "immediate",
      created_at: new Date().toISOString(),
    };
    setSendBusy(true);
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
      setSendBusy(false);
    }
  };

  const onRemoveQueued = async (messageId: string) => {
    if (!id) return;
    if (!messageId) return;
    if (queueActionBusy) return;
    setQueueActionBusyId(messageId);
    setSendError(null);
    try {
      await deleteMessage(messageId);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== messageId),
      );
    } catch (e: any) {
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
    setQueueActionBusyId(mid);
    setSendError(null);
    try {
      await deleteMessage(mid);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== mid),
      );
    } catch (e: any) {
      setSendError(e?.message ? String(e.message) : String(e));
    } finally {
      setQueueActionBusyId(null);
    }
  };

  const onSendQueuedNow = async (message: Message) => {
    if (!id) return;
    if (queueActionBusy || sendBusy) return;
    const mid = idToString(message.id);
    if (!mid) return;
    const attachments = getQueuedAttachments(message);
    const content = message.content ?? "";
    setQueueActionBusyId(mid);
    setSendError(null);
    try {
      await interruptSession(id);
    } catch (e: any) {
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
      setSendError(e?.message ? String(e.message) : String(e));
      setQueueActionBusyId(null);
      return;
    }
    setSendBusy(true);

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
      setSendBusy(false);
      setQueueActionBusyId(null);
    }
  };

  const acpAvailableCommands = useMemo<SlashCommandDescriptor[]>(() => {
    const last = [...events].reverse().find((e) => {
      const update = e.payload_json?.acp_update;
      return update?.sessionUpdate === "available_commands_update";
    });
    const update = last?.payload_json?.acp_update ?? {};
    const list = update.availableCommands ?? update.available_commands ?? [];
    if (!Array.isArray(list)) return [];
    return list
      .map((c: any) => ({
        name: String(c?.name ?? "").replace(/^\//, ""),
        description: typeof c?.description === "string" ? c.description : undefined,
      }))
      .filter((c: any) => typeof c.name === "string" && c.name.length > 0);
  }, [eventsKey]);

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
    return [{ name: "compact", description: "Summarize conversation to save context" }];
  }, [session?.provider_id]);

  const slashCommands = acpAvailableCommands.length > 0 ? acpAvailableCommands : fallbackSlashCommands;

  const virtuosoStyle = useMemo(() => ({ flex: 1, minHeight: 0 } as const), []);
  const workbenchViewportBy = useMemo(() => ({ top: 1000, bottom: 1000 }), []);
  const followOutput = useCallback(
    (_isAtBottom: boolean) => {
      if (!isActive) return false;
      return stickToBottomRef.current ? "auto" : false;
    },
    [isActive],
  );

  const wrapperClass = "wb-session-view";
  const leftClass = "wb-session-left";

  const dropScopeRef = useRef<HTMLDivElement | null>(null);
  const composerStackRef = useRef<HTMLDivElement | null>(null);
  const composerHeightRef = useRef<string | null>(null);

  useLayoutEffect(() => {
    const root = dropScopeRef.current;
    const stack = composerStackRef.current;
    if (!root || !stack) return;
    const updateComposerHeight = () => {
      const nextHeightPx = stack.offsetHeight;
      const prevHeightPx =
        composerHeightRef.current != null ? Number.parseFloat(composerHeightRef.current) : null;
      if (prevHeightPx != null && Math.abs(nextHeightPx - prevHeightPx) < 2) return;
      const nextHeight = `${nextHeightPx}px`;
      if (composerHeightRef.current === nextHeight) return;
      composerHeightRef.current = nextHeight;
      root.style.setProperty("--wb-composer-height", nextHeight);
    };
    updateComposerHeight();
    const observer = new ResizeObserver(() => updateComposerHeight());
    observer.observe(stack);
    return () => {
      observer.disconnect();
      composerHeightRef.current = null;
      root.style.removeProperty("--wb-composer-height");
    };
  }, []);

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
              scheduleScrollbarUpdate();
            } else {
              scrollbarLastScrollTopRef.current = null;
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
            scrollbarLastScrollTopRef.current = nextScrollTop;
            if (trusted && didScroll) showScrollbarTemporarily();
            scheduleScrollbarUpdate();
            const userIntent = hasUserIntent || scrollbarDraggingRef.current;
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
            }
            if (!didInitialScrollRef.current) didInitialScrollRef.current = true;
            if (scrollSyncRafRef.current != null) return;
            scrollSyncRafRef.current = window.requestAnimationFrame(() => {
              scrollSyncRafRef.current = null;
              const el = scrollerRef.current;
              if (!el) return;
              const scrollTop = el.scrollTop;
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
              const persistStick = nextStickToBottom;
              if (!userScroll && persistStick) return;
              const next = {
                stickToBottom: persistStick,
                anchorItemId: persistStick ? null : latestAnchorIdRef.current,
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
      List: forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>((props, ref) => {
        const rawPaddingBottom = props.style?.paddingBottom;
        const basePaddingBottom =
          rawPaddingBottom == null || rawPaddingBottom === ""
            ? "0px"
            : typeof rawPaddingBottom === "number"
              ? `${rawPaddingBottom}px`
              : String(rawPaddingBottom);
        const listPaddingBottom = `calc(${basePaddingBottom} + 8px + var(--wb-composer-height, 0px))`;
        return (
          <div
            {...props}
            ref={(node) => {
              listRef.current = node;
              if (typeof ref === "function") ref(node);
              else if (ref) (ref as MutableRefObject<HTMLDivElement | null>).current = node;
            }}
            role="list"
            className={`wb-thread-list ${props.className ?? ""}`}
            style={{
              ...props.style,
              paddingLeft: "var(--wb-session-padding-inline)",
              paddingRight: "var(--wb-session-padding-inline)",
              paddingBottom: listPaddingBottom,
            }}
          />
        );
      }),
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
        <div className="wb-session-bottom" ref={composerStackRef}>
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
