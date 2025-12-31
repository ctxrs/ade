import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
} from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Virtuoso, VirtuosoHandle, type StateSnapshot } from "react-virtuoso";
import { Link, useParams } from "react-router-dom";
import {
  cancelSession,
  deleteMessage,
  DictationSettings,
  getDaemonBaseUrl,
  blobUrl,
  Message,
  MessageAttachment,
  postMessage,
  Session,
  SessionEvent,
  SessionTurn,
  SessionTurnTool,
  setSessionMode,
  setSessionModel,
  authenticateSession,
  getSettings,
  idToString,
  interruptSession,
  submitAskUserQuestion,
  uploadBlob,
} from "../api/client";
import { useOpenSession, useSessionCacheSnapshot, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { WorkspaceCatchupProvider, useWorkspaceCatchupStore } from "../state/workspaceCatchupStore";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import { Check, Copy } from "lucide-react";
import { DiffReviewPane } from "../components/DiffReviewPane";
import { WorktreeBootstrapSnackbar } from "../components/WorktreeBootstrapSnackbar";
import { AskUserQuestionModal } from "../components/AskUserQuestionModal";
import { ComposerAutocompleteMenu } from "../components/ComposerAutocompleteMenu";
import { useComposerAutocomplete, type SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import { shouldSendOnEnter } from "../utils/keyboard";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import { WorkbenchComposer as UnifiedWorkbenchComposer, type WorkbenchModeId } from "../components/WorkbenchComposer";
import { startMicPcmStream } from "../utils/micPcmStream";
import { parseWsJson } from "../utils/wsJson";
import { buildModelCatalog, composeModelId, formatEffortLabel, parseModelId } from "../utils/modelEffort";
import { imageFilesToBlobRefAttachments, imageFilesToInlineAttachments } from "../utils/messageAttachments";
import { registerDropScope } from "../utils/dragDropScopes";
import { desktopGetDeepLinkToken, desktopOpenFile, desktopOpenPath, isDesktopApp } from "../utils/desktop";

type ThreadItem =
  | {
    kind: "message";
    id: string;
    role: "user" | "assistant";
    content: string;
    attachments: MessageAttachment[];
    created_at: string;
    delivery?: "immediate" | "queued";
  }
  | {
    kind: "spacer";
    id: string;
    created_at: string;
  }
  | {
    kind: "assistant";
    id: string;
    turn_id: string;
    created_at: string;
    content: string;
    thought: string;
    is_complete: boolean;
    thought_seconds?: number;
  }
  | {
    kind: "thought";
    id: string;
    turn_id: string;
    created_at: string;
    content: string;
  }
  | {
    kind: "tool_group";
    id: string;
    turn_id: string;
    created_at: string;
    updated_at: string;
    tool_total: number;
    tool_pending: number;
    tool_running: number;
    tool_completed: number;
    tool_failed: number;
    tools: Array<Extract<ThreadItem, { kind: "tool" }>>;
    thought: string;
  }
  | {
    kind: "tool";
    id: string;
    created_at: string;
    updated_at: string;
    tool_call_id: string;
    tool_kind: string;
    title: string;
    status: string;
    locations: Array<{ path?: string; range?: any }>;
    input: any;
    output_text: string;
    raw: any;
    updates_seen: number;
    has_details?: boolean;
  };

type WorkbenchTurnHeader = {
  id: string;
  content: string;
  plain_text: string;
  attachments: MessageAttachment[];
  created_at: string;
};

type WorkbenchListItem =
  | ThreadItem
  | {
      kind: "turn_header";
      id: string;
      header: WorkbenchTurnHeader;
    };

type ScrollbarDragState = {
  pointerId: number;
  startY: number;
  startScrollTop: number;
  trackHeight: number;
  thumbHeight: number;
  scrollHeight: number;
  clientHeight: number;
};

function imageAttachmentSrc(a: MessageAttachment): string {
  return a.kind === "image_ref" ? blobUrl(a.blob_id) : `data:${a.mime_type};base64,${a.data_base64}`;
}

function attachmentDisplayName(name?: string | null) {
  const n = String(name ?? "").trim();
  if (!n) return "image";
  return n.split(/[\\/]/).pop() || "image";
}

function appendSegment(base: string, addition: string): string {
  const trimmed = addition.trim();
  if (!trimmed) return base;
  if (!base) return trimmed;
  const needsSpace = /\S$/.test(base) && !/^[,.;!?]/.test(trimmed);
  return `${base}${needsSpace ? " " : ""}${trimmed}`;
}

function markdownToPlainText(input: string): string {
  if (!input) return "";
  let text = input.replace(/\r/g, "");
  text = text.replace(/```[a-zA-Z0-9_-]*\n/g, "");
  text = text.replace(/```/g, "");
  text = text.replace(/`([^`]*)`/g, "$1");
  text = text.replace(/!\[([^\]]*)\]\([^)]+\)/g, "$1");
  text = text.replace(/\[([^\]]+)\]\([^)]+\)/g, "$1");
  text = text
    .split("\n")
    .map((line) => line.replace(/^\s*(?:[#>*+-]|\d+\.)\s+/, ""))
    .join("\n");
  text = text.replace(/\n{3,}/g, "\n\n");
  return text.trim();
}
type WorkbenchThreadView = {
  groups: Array<{
    key: string;
    header: WorkbenchTurnHeader | null;
    items: ThreadItem[];
  }>;
  debugEvents: SessionEvent[];
};

export default function SessionPage() {
  const { id } = useParams<{ id: string }>();
  if (!id) return null;
  return <SessionPageWithCatchup sessionId={id} />;
}

function SessionPageWithCatchup({ sessionId }: { sessionId: string }) {
  const entry = useSessionEntry(sessionId);
  const workspaceId = entry?.session ? idToString(entry.session.workspace_id) : null;
  if (!workspaceId) {
    return <SessionView sessionId={sessionId} variant="legacy" showDiffPane />;
  }
  return (
    <WorkspaceCatchupProvider workspaceId={workspaceId}>
      <SessionPageCatchupBridge sessionId={sessionId} />
    </WorkspaceCatchupProvider>
  );
}

function SessionPageCatchupBridge({ sessionId }: { sessionId: string }) {
  const supervisor = useSessionSupervisor();
  const workspaceCatchupStore = useWorkspaceCatchupStore();
  useEffect(() => {
    supervisor.bindWorkspaceCatchupStore(workspaceCatchupStore);
    return () => supervisor.bindWorkspaceCatchupStore(null);
  }, [supervisor, workspaceCatchupStore]);
  return (
    <>
      <WorktreeBootstrapSnackbar />
      <SessionView sessionId={sessionId} variant="legacy" showDiffPane />
    </>
  );
}

export type SessionViewVariant = "legacy" | "workbench";

export function SessionView({
  sessionId,
  isActive = true,
  variant,
  showDiffPane,
  draft,
  draftUpdatedAtMs,
  onDraftChange,
  onDraftPersistNow,
  onModeChange,
  scrollState,
  onScrollStateChange,
}: {
  sessionId: string;
  isActive?: boolean;
  variant: SessionViewVariant;
  showDiffPane: boolean;
  draft?: { text: string; modeId: WorkbenchModeId } | null;
  draftUpdatedAtMs?: number | null;
  onDraftChange?: ((text: string) => void) | null;
  onDraftPersistNow?: (() => void | Promise<void>) | null;
  onModeChange?: ((modeId: WorkbenchModeId) => void) | null;
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
}) {
  const id = sessionId;
  const supervisor = useSessionSupervisor();
  const supervisorSnap = useSessionCacheSnapshot();
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
  const [inputInternal, setInputInternal] = useState("");
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const [dropActive, setDropActive] = useState(false);
  const [workbenchModeInternal, setWorkbenchModeInternal] = useState<WorkbenchModeId>("default");
  const [sendBusy, setSendBusy] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [fileOpenError, setFileOpenError] = useState<string | null>(null);
  const [deepLinkToken, setDeepLinkToken] = useState<string | null>(null);
  const deepLinkTokenTimerRef = useRef<number | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [hasNewActivity, setHasNewActivity] = useState(false);
  const lastActivityCountRef = useRef(0);
  const [authMethodId, setAuthMethodId] = useState<string>("");
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [optimisticAskAnswered, setOptimisticAskAnswered] = useState<Record<string, boolean>>({});
  const [expandedTurnHeaders, setExpandedTurnHeaders] = useState<Record<string, boolean>>({});
  const [expandedTurnDetailsById, setExpandedTurnDetailsById] = useState<Record<string, boolean>>({});
  const [expandedThoughtByAssistantId, setExpandedThoughtByAssistantId] = useState<Record<string, boolean>>({});
  const [expandedToolById, setExpandedToolById] = useState<Record<string, boolean>>({});
  const virtuosoRef = useRef<VirtuosoHandle | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const didInitialScrollRef = useRef(false);
  const lastScrollPersistedRef = useRef<{
    stickToBottom: boolean;
    anchorItemId: string | null;
    scrollTop: number | null;
    virtuosoState?: unknown | null;
  } | null>(null);
  const liveScrollTopRef = useRef<number | null>(null);
  const scrollSyncRafRef = useRef<number | null>(null);
  const userScrolledRef = useRef(false);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
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

  const input = variant === "workbench" ? (draft?.text ?? "") : inputInternal;
  const setInput = useCallback(
    (next: string) => {
      if (variant === "workbench") {
        onDraftChange?.(next);
        return;
      }
      setInputInternal(next);
    },
    [onDraftChange, variant],
  );
  const workbenchMode = variant === "workbench" ? (draft?.modeId ?? "default") : workbenchModeInternal;
  const setWorkbenchMode = useCallback(
    (next: WorkbenchModeId) => {
      if (variant === "workbench") {
        onModeChange?.(next);
        return;
      }
      setWorkbenchModeInternal(next);
    },
    [onModeChange, variant],
  );

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

  const handleScrollbarTrackPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      if (event.target === scrollbarThumbRef.current) return;
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
    [scheduleScrollbarUpdate, showScrollbarTemporarily],
  );

  const handleScrollbarThumbPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
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
    [setScrollbarActiveState, updateScrollbar],
  );

  const handleScrollbarThumbPointerMove = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const drag = scrollbarDragRef.current;
      const scroller = scrollerRef.current;
      if (!drag || !scroller || drag.pointerId !== event.pointerId) return;
      const maxScrollTop = Math.max(drag.scrollHeight - drag.clientHeight, 0);
      const maxThumbTop = Math.max(drag.trackHeight - drag.thumbHeight, 1);
      const delta = event.clientY - drag.startY;
      const nextScrollTop = drag.startScrollTop + (delta / maxThumbTop) * maxScrollTop;
      scroller.scrollTop = Math.min(maxScrollTop, Math.max(0, nextScrollTop));
      scheduleScrollbarUpdate();
    },
    [scheduleScrollbarUpdate],
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
    if (variant !== "workbench") return;
    const scroller = scrollerRef.current;
    if (!scroller) return;
    const observer = new ResizeObserver(() => scheduleScrollbarUpdate());
    observer.observe(scroller);
    return () => observer.disconnect();
  }, [variant, scheduleScrollbarUpdate]);

  useEffect(() => {
    return () => {
      if (scrollbarHideTimerRef.current) window.clearTimeout(scrollbarHideTimerRef.current);
      if (scrollbarRafRef.current != null) window.cancelAnimationFrame(scrollbarRafRef.current);
    };
  }, []);


  const handleFileOpenError = useCallback((message: string | null) => {
    setFileOpenError(message);
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    let cancelled = false;

    const scheduleRefresh = (expiresAtMs: number) => {
      if (deepLinkTokenTimerRef.current) {
        window.clearTimeout(deepLinkTokenTimerRef.current);
        deepLinkTokenTimerRef.current = null;
      }
      const now = Date.now();
      const leadTime = 60_000;
      const delay = Math.max(expiresAtMs - now - leadTime, 10_000);
      deepLinkTokenTimerRef.current = window.setTimeout(() => {
        refresh();
      }, delay);
    };

    const refresh = () => {
      desktopGetDeepLinkToken()
        .then((token) => {
          if (cancelled) return;
          setDeepLinkToken(token.token);
          scheduleRefresh(token.expires_at_ms);
        })
        .catch(() => {});
    };

    refresh();
    return () => {
      cancelled = true;
      if (deepLinkTokenTimerRef.current) {
        window.clearTimeout(deepLinkTokenTimerRef.current);
        deepLinkTokenTimerRef.current = null;
      }
    };
  }, []);

  useOpenSession(id ?? "", { watchDiff: true });
  const refreshAll = useCallback(async () => {
    if (!id) return;
    await supervisor.refreshQueue(id);
    supervisor.refreshSession(id, { watchDiff: true });
  }, [id, supervisor]);

  useLayoutEffect(() => {
    restorePendingRef.current = true;
    didInitialScrollRef.current = false;
    lastScrollPersistedRef.current = null;
    latestAnchorIdRef.current = null;
    userScrolledRef.current = false;
    liveScrollTopRef.current = null;
    restoringScrollRef.current = false;
    if (scrollSyncRafRef.current != null) {
      window.cancelAnimationFrame(scrollSyncRafRef.current);
      scrollSyncRafRef.current = null;
    }
    setAtBottom(true);
    setHasNewActivity(false);
    lastActivityCountRef.current = 0;
    setExpandedTurnHeaders({});
    setExpandedThoughtByAssistantId({});
    setExpandedToolById({});
    setSendError(null);
    setFileOpenError(null);
    setAuthMethodId("");
    setAuthError(null);
    setOptimisticAskAnswered({});
    if (variant === "workbench") {
    }
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
    };
  }, []);

  useEffect(() => {
    return () => {
      if (!onScrollStateChange) return;
      if (!userScrolledRef.current) return;
      const el = scrollerRef.current;
      if (!el) return;
      const scrollTop = el.scrollTop;
      const remaining = el.scrollHeight - (scrollTop + el.clientHeight);
      const nearBottom = remaining <= 16;
      persistScroll({
        stickToBottom: nearBottom,
        anchorItemId: nearBottom ? null : latestAnchorIdRef.current,
        scrollTop: nearBottom ? null : scrollTop,
        virtuosoState: lastScrollPersistedRef.current?.virtuosoState ?? undefined,
      });
    };
  }, [id, onScrollStateChange, persistScroll]);

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
  const worktreeId = session ? idToString(session.worktree_id) : null;
  const turns = entry?.turns ?? [];
  const turnToolsByTurnId = entry?.turnToolsByTurnId ?? {};
  const turnToolsLoading = entry?.turnToolsLoading ?? [];
  const toolSummariesReady = entry?.toolSummariesReady ?? false;
  const hasMoreTurns = entry?.hasMoreTurns ?? false;
  const events: SessionEvent[] = entry?.events ?? [];
  const messages: Message[] = entry?.messages ?? [];
  const queue: Message[] = entry?.queue ?? [];
  const diff = entry?.diff ?? "";
  const eventsKey = `${entry?.lastEventSeq ?? 0}:${events.length}`;
  const turnsKey = deriveTurnsKey(turns);
  const messagesKey = deriveMessagesKey(messages);
  const streamConnected = supervisorSnap.connection === "connected";

  const interruptBanner = useMemo(() => {
    const last = [...events].reverse().find((e) => e.event_type === "turn_interrupted");
    if (!last) return null;
    return `Interrupted at ${new Date(last.created_at).toLocaleTimeString()}.`;
  }, [eventsKey]);

  const askUserQuestion = useMemo(() => {
    const answered = new Set<string>();
    for (const ev of events) {
      if (ev.event_type !== "notice") continue;
      if (ev.payload_json?.kind !== "ask_user_question_answered") continue;
      const toolCallId = String(ev.payload_json?.tool_call_id ?? "").trim();
      if (toolCallId) answered.add(toolCallId);
    }
    for (const toolCallId of Object.keys(optimisticAskAnswered)) {
      if (toolCallId) answered.add(toolCallId);
    }

    for (let i = events.length - 1; i >= 0; i--) {
      const ev = events[i];
      if (ev.event_type !== "notice") continue;
      if (ev.payload_json?.kind !== "ask_user_question") continue;
      const toolCallId = String(ev.payload_json?.tool_call_id ?? "").trim();
      if (!toolCallId || answered.has(toolCallId)) continue;
      return { toolCallId, input: ev.payload_json?.input ?? null };
    }
    return null;
  }, [eventsKey, optimisticAskAnswered]);

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
        return sessionStorage.getItem("contextAuthToken");
      } catch {
        return null;
      }
    })();
    const base = getDaemonBaseUrl();
    const wsBase = base
      ? base.startsWith("https://")
        ? base.replace(/^https:\/\//, "wss://")
        : base.replace(/^http:\/\//, "ws://")
      : `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}`;
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

  const legacyThreadView = useMemo(() => buildThreadViewModel(events), [eventsKey]);
  const workbenchThreadView = useMemo(() => {
    if (turns.length === 0) {
      return { groups: [], debugEvents: [] };
    }
    return buildWorkbenchThreadViewModelFromTurns(
      turns,
      messages,
      toolSummariesReady ? turnToolsByTurnId : {},
      events,
    );
    // messages are canonical for turn headers; include in memo key
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [turnsKey, messagesKey, toolSummariesReady ? turnToolsByTurnId : null, eventsKey, turns.length]);

  const debugEvents = variant === "workbench" ? workbenchThreadView.debugEvents : legacyThreadView.debugEvents;
  const emptyThreadItems = useMemo(() => [] as ThreadItem[], []);
  const emptyWorkbenchGroups = useMemo(() => [] as WorkbenchThreadView["groups"], []);
  const threadItems = variant === "workbench" ? emptyThreadItems : legacyThreadView.items;
  const wbGroups = variant === "workbench" ? workbenchThreadView.groups : emptyWorkbenchGroups;
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

  useEffect(() => {
    if (variant !== "workbench") return;
    scheduleScrollbarUpdate();
  }, [variant, scheduleScrollbarUpdate, wbListItems.length]);

  useLayoutEffect(() => {
    if (variant !== "workbench") return;
    updateScrollbar();
  }, [variant, updateScrollbar, wbListItems.length]);

  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  const wasActiveRef = useRef(isActive);
  const virtuosoPersistTimerRef = useRef<number | null>(null);
  const [restoreInProgress, setRestoreInProgress] = useState(false);
  useLayoutEffect(() => {
    if (isActive && !wasActiveRef.current) {
      const hasVirtuosoState = Boolean(scrollState?.virtuosoState);
      restorePendingRef.current = !hasVirtuosoState;
      didInitialScrollRef.current = false;
      restoringScrollRef.current = false;
      userScrolledRef.current = false;
      liveScrollTopRef.current = null;
      if (hasVirtuosoState) {
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
  }, [isActive, scrollState?.virtuosoState]);

  useEffect(() => {
    if (!isActive) setRestoreInProgress(false);
  }, [isActive]);

  useLayoutEffect(() => {
    if (!isActive) return;
    const items = variant === "workbench" ? wbListItems : threadItems;
    if (items.length === 0) return;
    if (scrollState?.virtuosoState) {
      restorePendingRef.current = false;
      return;
    }
    if (!restorePendingRef.current) return;
    if (userScrolledRef.current) {
      restorePendingRef.current = false;
      didInitialScrollRef.current = true;
      return;
    }

    const state = scrollState ?? { stickToBottom: true, anchorItemId: null, scrollTop: null };
    setAtBottom(state.stickToBottom);
    const restoreAnchorId = !state.stickToBottom ? (state.anchorItemId ?? null) : null;
    const restoreScrollTop = !state.stickToBottom ? (state.scrollTop ?? null) : null;

    restoringScrollRef.current = true;
    setRestoreInProgress(true);
    requestAnimationFrame(() => {
      const el = scrollerRef.current;
      if (!el) {
        restoringScrollRef.current = false;
        setRestoreInProgress(false);
        return;
      }
      if (restoreScrollTop !== null) {
        const target = Math.max(0, restoreScrollTop);
        virtuosoRef.current?.scrollTo({ top: target });
        el.scrollTop = target;
      } else if (restoreAnchorId) {
        const idx = items.findIndex((it) => it?.id === restoreAnchorId);
        if (idx >= 0) {
          virtuosoRef.current?.scrollToIndex({ index: idx, align: "start" });
        } else {
          virtuosoRef.current?.scrollToIndex({ index: items.length - 1, align: "end" });
        }
      } else {
        virtuosoRef.current?.scrollToIndex({ index: items.length - 1, align: "end" });
      }
      restorePendingRef.current = false;
      didInitialScrollRef.current = true;
      if (restoreCooldownRef.current) window.clearTimeout(restoreCooldownRef.current);
      restoreCooldownRef.current = window.setTimeout(() => {
        restoreCooldownRef.current = null;
        restoringScrollRef.current = false;
        setRestoreInProgress(false);
      }, 200);
    });
  }, [
    id,
    isActive,
    scrollState?.anchorItemId,
    scrollState?.stickToBottom,
    scrollState?.scrollTop,
    scrollState?.virtuosoState,
    threadItems.length,
    variant,
    wbListItems.length,
  ]);

  const contextIndicator = useMemo(() => {
    const fromTurns = [...turns].reverse().find((t) => t.metrics_json);
    if (fromTurns?.metrics_json) {
      return fromTurns.metrics_json as {
        context_tokens_estimate: number;
        remaining_fraction: number;
      };
    }
    const done = [...events]
      .reverse()
      .find((e) => e.event_type === "done" && e.payload_json?.context_window);
    if (!done) return null;
    return done.payload_json.context_window as {
      context_tokens_estimate: number;
      remaining_fraction: number;
    };
  }, [eventsKey, turnsKey]);

  const planEntries = useMemo(() => {
    const last = [...events].reverse().find((e) => e.event_type === "plan");
    const update = last?.payload_json?.acp_update ?? last?.payload_json;
    const entries = update?.entries ?? [];
    return Array.isArray(entries) ? (entries as any[]) : [];
  }, [eventsKey]);

  const authUi = useMemo(() => deriveAuthUi(events), [eventsKey]);

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

  const acpModels = acpSessionInfo?.payload_json?.models;
  const acpModes = acpSessionInfo?.payload_json?.modes;

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

  const modeOptions = useMemo(() => {
    const list =
      acpModes?.availableModes ??
      acpModes?.available_modes ??
      acpModes?.available_modes ??
      [];
    if (!Array.isArray(list)) return [];
    return list
      .map((m: any) => ({
        id: m.id,
        name: m.name ?? m.id,
      }))
      .filter((m: any) => typeof m.id === "string" && m.id.length > 0);
  }, [acpModes]);

  const currentModelId =
    session?.model_id ??
    acpModels?.currentModelId ??
    acpModels?.current_model_id ??
    "";
  const currentModeId =
    acpModes?.currentModeId ??
    acpModes?.current_mode_id ??
    "";

  const modelCatalog = useMemo(() => buildModelCatalog(modelOptions), [modelOptions]);
  const parsedModel = useMemo(() => parseModelId(currentModelId, modelCatalog), [currentModelId, modelCatalog]);
  const currentBase = parsedModel.base || modelCatalog.baseIds[0] || "";
  const currentEffort = parsedModel.effort;
  const effortOptions = modelCatalog.effortsByBase[currentBase] ?? [];

  const pickDefaultEffort = useCallback((efforts: string[]) => {
    if (efforts.includes("medium")) return "medium";
    return efforts[0] ?? null;
  }, []);

  const deriveFullModelIdForBase = useCallback(
    (base: string, preferredEffort: string | null) => {
      const efforts = modelCatalog.effortsByBase[base] ?? [];
      if (efforts.length === 0) return base;
      const eff = preferredEffort && efforts.includes(preferredEffort) ? preferredEffort : pickDefaultEffort(efforts);
      if (!eff) return base;
      return modelCatalog.fullIdByBaseEffort[base]?.[eff] ?? composeModelId(base, eff);
    },
    [modelCatalog, pickDefaultEffort],
  );

  const threadActivityCount = variant === "workbench" ? wbListItems.length : threadItems.length;

  useEffect(() => {
    if (!didInitialScrollRef.current) return;
    const prev = lastActivityCountRef.current;
    lastActivityCountRef.current = threadActivityCount;
    if (atBottom) return;
    if (prev === 0) return;
    if (threadActivityCount > prev) setHasNewActivity(true);
  }, [threadActivityCount, atBottom]);

  const followOutput = restoreInProgress ? false : atBottom ? "auto" : false;
  const restoreStateFrom = (scrollState?.virtuosoState ?? undefined) as StateSnapshot | undefined;

  const handleAtBottomStateChange = useCallback(
    (isAtBottom: boolean) => {
      if (restoringScrollRef.current) return;
      setAtBottom(isAtBottom);
      if (isAtBottom) setHasNewActivity(false);
    },
    [setAtBottom, setHasNewActivity],
  );

  const handleWorkbenchRangeChanged = useCallback(
    (range: { startIndex: number }) => {
      if (restoringScrollRef.current) return;
      if (atBottom) return;
      const item = wbListItems[range.startIndex];
      if (!item) return;
      latestAnchorIdRef.current = item.id ?? null;
    },
    [atBottom, wbListItems],
  );

  const handleLegacyRangeChanged = useCallback(
    (range: { startIndex: number }) => {
      if (restoringScrollRef.current) return;
      if (atBottom) return;
      const item = threadItems[range.startIndex];
      if (!item) return;
      latestAnchorIdRef.current = item.id ?? null;
    },
    [atBottom, threadItems],
  );

  const handleStartReached = useCallback(() => {
    if (!hasMoreTurns) return;
    supervisor.loadMoreTurns(id);
  }, [hasMoreTurns, id, supervisor]);

  const jumpToLatestWorkbench = useCallback(() => {
    if (wbListItems.length > 0) {
      virtuosoRef.current?.scrollToIndex({ index: wbListItems.length - 1, align: "end" });
    }
  }, [virtuosoRef, wbListItems.length]);

  const jumpToLatestLegacy = useCallback(() => {
    if (threadItems.length > 0) {
      virtuosoRef.current?.scrollToIndex({ index: threadItems.length - 1, align: "end" });
    }
  }, [virtuosoRef, threadItems.length]);

  const handleDiffUpdated = useCallback(
    (next: string) => {
      if (!id) return;
      supervisor.setDiff(id, next);
    },
    [id, supervisor],
  );

  const handleFileSaved = useCallback(() => {
    if (!id) return;
    supervisor.refreshSession(id, { watchDiff: true });
  }, [id, supervisor]);

  const sendNow = async () => {
    if (!id) return;
    if (sendBusy) return;
    const text = (dictationRecording ? await stopDictation({ awaitFinal: true }) : input).trim();
    if (!text) return;
    setSendBusy(true);
    setSendError(null);
    try {
      await postMessage(id, text, undefined, draftAttachments);
      // Refresh Messages immediately so user turns render without waiting for a `done` event.
      await supervisor.refreshQueue(id);
      supervisor.refreshSession(id, { watchDiff: true });
      setInput("");
      setDraftAttachments([]);
      try {
        await onDraftPersistNow?.();
      } catch {
        // best-effort
      }
    } catch (e: any) {
      setSendError(e?.message ? String(e.message) : String(e));
    } finally {
      setSendBusy(false);
    }
  };

  const onSend = async (e: React.FormEvent) => {
    e.preventDefault();
    await sendNow();
  };

  const onRemoveQueued = async (messageId: string) => {
    await deleteMessage(messageId);
    supervisor.refreshQueue(id ?? "");
  };

  const insertIntoComposer = (text: string) => {
    const el = textareaRef.current;
    if (!el) {
      setInput(`${input}${text}`);
      return;
    }
    const start = el.selectionStart ?? input.length;
    const end = el.selectionEnd ?? input.length;
    const next = input.slice(0, start) + text + input.slice(end);
    setInput(next);
    requestAnimationFrame(() => {
      el.focus();
      const cursor = start + text.length;
      el.setSelectionRange(cursor, cursor);
      requestAnimationFrame(() => composerAutocomplete.syncFromDom());
    });
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

  const composerAutocomplete = useComposerAutocomplete({
    sessionId: id ?? null,
    workspaceId: null,
    value: input,
    setValue: setInput,
    textareaRef,
    slashCommands,
  });

  const workbenchVirtuosoStyle = useMemo(() => ({ flex: 1, minHeight: 0 } as const), []);
  const legacyVirtuosoStyle = useMemo(() => ({ height: "70vh" } as const), []);
  const virtuosoStyle = variant === "workbench" ? workbenchVirtuosoStyle : legacyVirtuosoStyle;
  const workbenchViewportBy = useMemo(() => ({ top: 1000, bottom: 1000 }), []);

  const wrapperClass = variant === "workbench" ? "wb-session-view" : "page split";
  const leftClass = variant === "workbench" ? "wb-session-left" : "left";

  const dropScopeRef = useRef<HTMLDivElement | null>(null);

  const onDropFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      const next =
        variant === "workbench"
          ? await imageFilesToInlineAttachments(files)
          : await imageFilesToBlobRefAttachments(files);
      if (next.length === 0) return;
      setDraftAttachments((prev) => [...prev, ...next]);
    },
    [variant],
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
    if (item.kind === "assistant") {
      const thoughtExpanded =
        variant === "workbench" ? false : expandedThoughtByAssistantId[item.id] ?? false;
      return (
        <AssistantEntry
          id={item.id}
          content={item.content}
          thought={item.thought}
          thoughtSeconds={item.thought_seconds}
          isComplete={item.is_complete}
          variant={variant}
          thoughtExpanded={thoughtExpanded}
          onToggleThought={() =>
            setExpandedThoughtByAssistantId((prev) => ({ ...prev, [item.id]: !thoughtExpanded }))
          }
          worktreeId={worktreeId}
          onFileOpenError={handleFileOpenError}
          linkToken={deepLinkToken}
        />
      );
    }
    if (item.kind === "tool_group") {
      const expanded = expandedTurnDetailsById[item.turn_id] ?? false;
      const toolsLoading = turnToolsLoading.includes(item.turn_id);
      return (
        <WorkbenchToolGroupRow
          item={item}
          variant={variant}
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
          variant={variant}
          expanded={toolExpanded}
          onToggle={() => setExpandedToolById((prev) => ({ ...prev, [item.id]: !toolExpanded }))}
        />
      );
    }
    return (
      <ThreadItemView
        item={item}
        variant={variant}
        worktreeId={worktreeId}
        onFileOpenError={handleFileOpenError}
        linkToken={deepLinkToken}
      />
    );
  }, [
    deepLinkToken,
    expandedThoughtByAssistantId,
    expandedToolById,
    expandedTurnDetailsById,
    handleFileOpenError,
    id,
    supervisor,
    turnToolsLoading,
    variant,
    worktreeId,
  ]);

  const workbenchItemContent = useCallback(
    (_: number, item: WorkbenchListItem) => {
      if (!item) return <div style={{ height: 1 }} />;
      if ((item as any).kind === "turn_header") {
        const header = (item as Extract<WorkbenchListItem, { kind: "turn_header" }>).header;
        const isLong = header.plain_text.split("\n").length > 4 || header.plain_text.length > 280;
        const expanded = expandedTurnHeaders[header.id] ?? !isLong;
        return (
          <WorkbenchTurnHeaderView
            header={header}
            expanded={expanded}
            onToggle={() => setExpandedTurnHeaders((prev) => ({ ...prev, [header.id]: !expanded }))}
          />
        );
      }
      const content = renderThreadItem(item as ThreadItem);
      return <div className="wb-thread-indent">{content}</div>;
    },
    [expandedTurnHeaders, renderThreadItem],
  );

  const threadItemContent = useCallback(
    (_: number, item: ThreadItem) => renderThreadItem(item),
    [renderThreadItem],
  );

  const workbenchComponents = useMemo(
    () => ({
      Scroller: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div
          {...props}
          ref={(node) => {
            scrollerRef.current = node;
            if (node) {
              scrollbarLastScrollTopRef.current = node.scrollTop;
              scheduleScrollbarUpdate();
            } else {
              scrollbarLastScrollTopRef.current = null;
            }
            if (typeof ref === "function") ref(node);
            else if (ref) (ref as React.MutableRefObject<HTMLDivElement | null>).current = node;
          }}
          className={`wb-thread-scroller ${props.className ?? ""}`}
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
            scrollbarLastScrollTopRef.current = nextScrollTop;
            if (trusted && didScroll) showScrollbarTemporarily();
            scheduleScrollbarUpdate();
            if (restoringScrollRef.current && !trusted) return;
            if (restoringScrollRef.current && trusted) {
              restoringScrollRef.current = false;
              if (restoreCooldownRef.current) {
                window.clearTimeout(restoreCooldownRef.current);
                restoreCooldownRef.current = null;
              }
            }
            if (!trusted && !userScrolledRef.current) return;
            if (trusted) userScrolledRef.current = true;
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
              const nearBottom = remaining <= 16;
              setAtBottom(nearBottom);
              if (nearBottom) setHasNewActivity(false);
              const next = {
                stickToBottom: nearBottom,
                anchorItemId: nearBottom ? null : latestAnchorIdRef.current,
                scrollTop: nearBottom ? null : scrollTop,
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
      List: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div {...props} ref={ref} role="list" className={`wb-thread-list ${props.className ?? ""}`} />
      )),
      Item: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div {...props} ref={ref} role="listitem" />
      )),
    }),
    [persistScroll, scheduleScrollbarUpdate, showScrollbarTemporarily],
  );

  const threadComponents = useMemo(
    () => ({
      Scroller: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div
          {...props}
          ref={(node) => {
            const prev = scrollerRef.current;
            scrollerRef.current = node;
            if (typeof ref === "function") ref(node);
            else if (ref) (ref as React.MutableRefObject<HTMLDivElement | null>).current = node;
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
            if (restoringScrollRef.current && !trusted) return;
            if (restoringScrollRef.current && trusted) {
              restoringScrollRef.current = false;
              if (restoreCooldownRef.current) {
                window.clearTimeout(restoreCooldownRef.current);
                restoreCooldownRef.current = null;
              }
            }
            if (!trusted && !userScrolledRef.current) return;
            if (trusted) userScrolledRef.current = true;
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
              const nearBottom = remaining <= 16;
              setAtBottom(nearBottom);
              if (nearBottom) setHasNewActivity(false);
              const next = {
                stickToBottom: nearBottom,
                anchorItemId: nearBottom ? null : latestAnchorIdRef.current,
                scrollTop: nearBottom ? null : scrollTop,
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
      List: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div {...props} ref={ref} role="list" />
      )),
      Item: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div {...props} ref={ref} role="listitem" />
      )),
    }),
    [persistScroll],
  );

  return (
    <div
      className={`${wrapperClass} ctx-drop-scope`}
      ref={dropScopeRef}
    >
      {dropActive && (
        <div className="ctx-drop-overlay" aria-hidden="true">
          <div className="ctx-drop-overlay-text">Drop image to attach</div>
        </div>
      )}
      <AskUserQuestionModal
        open={Boolean(askUserQuestion)}
        input={askUserQuestion?.input}
        onCancel={async () => {
          if (!askUserQuestion) return;
          await submitAskUserQuestion(id, askUserQuestion.toolCallId, "cancelled", {});
          setOptimisticAskAnswered((prev) => ({ ...prev, [askUserQuestion.toolCallId]: true }));
        }}
        onSubmit={async (answers) => {
          if (!askUserQuestion) return;
          await submitAskUserQuestion(id, askUserQuestion.toolCallId, "submitted", answers);
          setOptimisticAskAnswered((prev) => ({ ...prev, [askUserQuestion.toolCallId]: true }));
        }}
      />
      <div className={leftClass}>
        {entry?.error && (
          <div className="banner">
            <span className="error">{entry.error}</span>
          </div>
        )}
        {showDebug && variant === "workbench" && (
          <div className="wb-muted" style={{ fontFamily: "var(--mono)" }}>
            debug: events={events.length} messages={messages.length} userMessages={messages.filter((m) => m.role === "user").length} items={wbListItems.length}
          </div>
        )}
        {session && variant === "legacy" && (
          <div className="header">
            <div className="row" style={{ justifyContent: "space-between", alignItems: "center" }}>
              <Link to={`/tasks/${idToString(session.task_id)}`}>← Task</Link>
              <div className="row" style={{ alignItems: "center" }}>
                {showDebug ? (
                  <a className="muted" href={window.location.pathname}>
                    Hide debug
                  </a>
                ) : (
                  <a className="muted" href={`${window.location.pathname}?debug=1`}>
                    Debug
                  </a>
                )}
                {perfEnabled ? (
                  <a className="muted" href={window.location.pathname}>
                    Perf off
                  </a>
                ) : (
                  <a className="muted" href={`${window.location.pathname}?perf=1`}>
                    Perf
                  </a>
                )}
                {threadItems.length > 0 && (
                  <button
                    type="button"
                    onClick={() =>
                      virtuosoRef.current?.scrollToIndex({
                        index: threadItems.length - 1,
                        align: "end",
                      })
                    }
                  >
                    Jump to latest
                  </button>
                )}
              </div>
            </div>
            <div className="muted">
              {session.provider_id} / {session.model_id} ·{" "}
              {contextIndicator ? (
                <>
                  {contextIndicator.context_tokens_estimate} tokens ·{" "}
                  {Math.round(contextIndicator.remaining_fraction * 100)}% remaining
                </>
              ) : (
                <span title="Tokenizer/model window unknown">Unknown</span>
              )}
              {!streamConnected && <span title="Live stream disconnected; polling for updates."> · Polling</span>}
            </div>
          </div>
        )}

        {(modelOptions.length > 0 || modeOptions.length > 0) && id && variant === "legacy" && (
          <div className="card">
            {modelCatalog.baseIds.length > 0 && (
              <label>
                Model
                <select
                  value={currentBase}
                  onChange={async (e) => {
                    const nextBase = e.target.value;
                    const next = deriveFullModelIdForBase(nextBase, currentEffort);
                    const updated = await setSessionModel(id, next);
                    supervisor.setSession(updated);
                  }}
                >
                  {modelCatalog.baseIds.map((b) => (
                    <option key={b} value={b}>
                      {modelCatalog.displayNameByBase[b] ?? b}
                    </option>
                  ))}
                </select>
              </label>
            )}

            {effortOptions.length > 0 && (
              <label>
                Effort
                <select
                  value={currentEffort ?? pickDefaultEffort(effortOptions) ?? ""}
                  onChange={async (e) => {
                    const nextEff = e.target.value || "";
                    const next = deriveFullModelIdForBase(currentBase, nextEff || null);
                    const updated = await setSessionModel(id, next);
                    supervisor.setSession(updated);
                  }}
                >
                  {effortOptions.map((eff) => (
                    <option key={eff} value={eff}>
                      {formatEffortLabel(eff)}
                    </option>
                  ))}
                </select>
              </label>
            )}

            {modeOptions.length > 0 && (
              <label>
                Mode
                <select
                  value={currentModeId}
                  onChange={async (e) => {
                    await setSessionMode(id, e.target.value);
                    supervisor.refreshSession(id, { watchDiff: true });
                  }}
                >
                  {modeOptions.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
        )}

        {interruptBanner && <div className="banner">{interruptBanner}</div>}

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
                  {authBusy ? "Authenticating…" : "Authenticate"}
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

        {/* Zed-style: plan moves into the bottom activity bar; keep hidden above thread. */}

        {queue.length > 0 && (
          <div className="queue-panel card">
            <div className="row">
              <strong>Queued messages ({queue.length})</strong>
            </div>
            <ul className="sublist">
              {queue.map((m) => {
                const mid = idToString(m.id);
                return (
                  <li key={mid} className="row">
                    <span className="muted">{m.content}</span>
                    <button type="button" onClick={() => onRemoveQueued(mid)}>
                      Remove
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        )}

        {showDebug && debugEvents.length > 0 && (
          <DebugPanel events={debugEvents} />
        )}

        {variant === "workbench" ? (
          <WorkbenchThreadStack
            virtuosoStyle={virtuosoStyle}
            data={wbListItems}
            virtuosoRef={virtuosoRef}
            followOutput={followOutput}
            restoreStateFrom={restoreStateFrom}
            increaseViewportBy={workbenchViewportBy}
            onStartReached={handleStartReached}
            onAtBottomStateChange={handleAtBottomStateChange}
            onRangeChanged={handleWorkbenchRangeChanged}
            components={workbenchComponents}
            itemContent={workbenchItemContent}
            hasNewActivity={hasNewActivity}
            onJumpToLatest={jumpToLatestWorkbench}
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
        ) : (
          <LegacyThreadStack
            virtuosoStyle={virtuosoStyle}
            data={threadItems}
            virtuosoRef={virtuosoRef}
            followOutput={followOutput}
            restoreStateFrom={restoreStateFrom}
            onAtBottomStateChange={handleAtBottomStateChange}
            onRangeChanged={handleLegacyRangeChanged}
            components={threadComponents}
            itemContent={threadItemContent}
            hasNewActivity={hasNewActivity}
            onJumpToLatest={jumpToLatestLegacy}
          />
        )}

        {variant === "legacy" && <ActivityBar planEntries={planEntries} diffText={diff} />}

        {variant === "workbench" ? (
          <>
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
              sendDisabled={sendBusy || !input.trim()}
              sendDisabledReason={sendBusy ? "Sending…" : !input.trim() ? "Enter a message." : null}
              onInterrupt={id ? () => interruptSession(id) : null}
              modeId={workbenchMode}
              setModeId={setWorkbenchMode}
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
              envLabel={session?.env_target === "local" ? "Local" : "Worktree"}
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
          </>
        ) : (
          <form onSubmit={onSend} className="composer">
            {sendError && <div className="banner">{sendError}</div>}
            {fileOpenError && <div className="banner">{fileOpenError}</div>}
            <div className="row">
              <button
                type="button"
                onClick={() => {
                  insertIntoComposer("@");
                }}
              >
                @ File
              </button>
              <button type="button" onClick={() => insertIntoComposer("/")} title="Slash commands">
                /
              </button>
              <label className="file-btn">
                Image
                <input
                  type="file"
                  accept="image/*"
                  multiple
                  onChange={async (e) => {
                    const files = Array.from(e.target.files ?? []);
                    const next = await imageFilesToBlobRefAttachments(files);
                    setDraftAttachments((prev) => [...prev, ...next]);
                    e.target.value = "";
                  }}
                />
              </label>
            </div>
            {draftAttachments.length > 0 && (
              <div className="card">
                <div className="muted">Attachments</div>
                <div className="row" style={{ flexWrap: "wrap" }}>
                  {draftAttachments.map((a, idx) => {
                    if (a.kind !== "image" && a.kind !== "image_ref") return null;
                    const src =
                      a.kind === "image_ref"
                        ? blobUrl(a.blob_id)
                        : `data:${a.mime_type};base64,${a.data_base64}`;
                    return (
                      <div key={idx} className="thumb">
                        <img src={src} alt={a.name ?? `image-${idx}`} />
                        <button
                          type="button"
                          onClick={() =>
                            setDraftAttachments((prev) => prev.filter((_, i) => i !== idx))
                          }
                        >
                          Remove
                        </button>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}
            <textarea
              ref={textareaRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder="Send a message… (/ commands, @ file refs)"
              onKeyDown={(e) => {
                if (composerAutocomplete.onKeyDown(e)) return;
                if (shouldSendOnEnter(e)) {
                  e.preventDefault();
                  sendNow();
                }
              }}
              onKeyUp={() => composerAutocomplete.syncFromDom()}
              onClick={() => composerAutocomplete.syncFromDom()}
              onSelect={() => composerAutocomplete.syncFromDom()}
              onPaste={async (e) => {
                const files = Array.from(e.clipboardData?.files ?? []);
                const images = files.filter((f) => (f.type || "").startsWith("image/"));
                if (images.length === 0) return;
                e.preventDefault();
                const next: MessageAttachment[] = [];
                for (const f of images) {
                  const uploaded = await uploadBlob(f);
                  next.push({
                    kind: "image_ref",
                    blob_id: uploaded.blob_id,
                    mime_type: uploaded.mime_type,
                    name: uploaded.name ?? f.name,
                  });
                }
                setDraftAttachments((prev) => [...prev, ...next]);
              }}
            />
            <ComposerAutocompleteMenu
              open={composerAutocomplete.open}
              loading={composerAutocomplete.loading}
              items={composerAutocomplete.items}
              activeIndex={composerAutocomplete.activeIndex}
              onPick={composerAutocomplete.pick}
              onHoverIndex={(i) => composerAutocomplete.setActiveIndex(i)}
              anchorRect={composerAutocomplete.anchorRect}
              anchorInputRect={composerAutocomplete.anchorInputRect}
              inlineFallback={composerAutocomplete.inlineFallback}
            />
            <div className="row">
              <button type="submit">Send</button>
              <button type="button" onClick={() => id && cancelSession(id)}>
                Cancel
              </button>
              <button type="button" onClick={() => id && interruptSession(id)}>
                Interrupt
              </button>
            </div>
          </form>
        )}

        <div className="sr-only" aria-live="polite">
          {session && (atBottom ? "Agent output updating." : "New agent activity.")}
        </div>
      </div>

      {showDiffPane && (
        <div className="right">
          <DiffReviewPane
            diff={diff}
            trackId={session ? idToString(session.track_id) : ""}
            sessionId={id || undefined}
            onDiffUpdated={handleDiffUpdated}
            onFileSaved={handleFileSaved}
          />
        </div>
      )}
    </div>
  );
}

type WorkbenchThreadStackProps = {
  virtuosoStyle: CSSProperties;
  data: WorkbenchListItem[];
  virtuosoRef: React.MutableRefObject<VirtuosoHandle | null>;
  followOutput: false | "auto";
  restoreStateFrom?: StateSnapshot;
  increaseViewportBy: { top: number; bottom: number };
  onStartReached: () => void;
  onAtBottomStateChange: (isAtBottom: boolean) => void;
  onRangeChanged: (range: any) => void;
  components: any;
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  hasNewActivity: boolean;
  onJumpToLatest: () => void;
  scrollbarActive: boolean;
  scrollbarDragging: boolean;
  scrollbarNeeded: boolean;
  scrollbarTrackRef: React.MutableRefObject<HTMLDivElement | null>;
  scrollbarThumbRef: React.MutableRefObject<HTMLDivElement | null>;
  onScrollbarMouseLeave: () => void;
  onScrollbarTrackPointerDown: (event: PointerEvent<HTMLDivElement>) => void;
  onScrollbarThumbPointerDown: (event: PointerEvent<HTMLDivElement>) => void;
  onScrollbarThumbPointerMove: (event: PointerEvent<HTMLDivElement>) => void;
  onScrollbarThumbPointerUp: (event: PointerEvent<HTMLDivElement>) => void;
  scheduleScrollbarUpdate: () => void;
};

const WorkbenchThreadStack = memo(function WorkbenchThreadStack({
  virtuosoStyle,
  data,
  virtuosoRef,
  followOutput,
  restoreStateFrom,
  increaseViewportBy,
  onStartReached,
  onAtBottomStateChange,
  onRangeChanged,
  components,
  itemContent,
  hasNewActivity,
  onJumpToLatest,
  scrollbarActive,
  scrollbarDragging,
  scrollbarNeeded,
  scrollbarTrackRef,
  scrollbarThumbRef,
  onScrollbarMouseLeave,
  onScrollbarTrackPointerDown,
  onScrollbarThumbPointerDown,
  onScrollbarThumbPointerMove,
  onScrollbarThumbPointerUp,
  scheduleScrollbarUpdate,
}: WorkbenchThreadStackProps) {
  return (
    <div className="thread-stack wb-thread-stack" onMouseLeave={onScrollbarMouseLeave}>
      <Virtuoso
        style={virtuosoStyle}
        data={data}
        ref={(node) => {
          virtuosoRef.current = node;
        }}
        followOutput={followOutput}
        defaultItemHeight={56}
        increaseViewportBy={increaseViewportBy}
        computeItemKey={(index, item) => item?.id ?? `i:${index}`}
        restoreStateFrom={restoreStateFrom}
        startReached={onStartReached}
        atBottomStateChange={onAtBottomStateChange}
        rangeChanged={onRangeChanged}
        components={components}
        itemContent={itemContent}
      />

      <div
        className={`wb-scrollbar${scrollbarActive ? " is-active" : ""}${scrollbarDragging ? " is-dragging" : ""}${scrollbarNeeded ? "" : " is-hidden"}`}
        aria-hidden="true"
      >
        <div
          className="wb-scrollbar-track"
          ref={(node) => {
            scrollbarTrackRef.current = node;
            if (node) scheduleScrollbarUpdate();
          }}
          onPointerDown={onScrollbarTrackPointerDown}
        >
          <div
            className="wb-scrollbar-thumb"
            ref={(node) => {
              scrollbarThumbRef.current = node;
              if (node) scheduleScrollbarUpdate();
            }}
            onPointerDown={onScrollbarThumbPointerDown}
            onPointerMove={onScrollbarThumbPointerMove}
            onPointerUp={onScrollbarThumbPointerUp}
            onPointerCancel={onScrollbarThumbPointerUp}
          />
        </div>
      </div>

      {hasNewActivity && (
        <button
          type="button"
          className="new-activity-overlay"
          aria-label="Jump to latest"
          title="Jump to latest"
          onClick={onJumpToLatest}
        >
          ↓
        </button>
      )}
    </div>
  );
});

type LegacyThreadStackProps = {
  virtuosoStyle: CSSProperties;
  data: ThreadItem[];
  virtuosoRef: React.MutableRefObject<VirtuosoHandle | null>;
  followOutput: false | "auto";
  restoreStateFrom?: StateSnapshot;
  onAtBottomStateChange: (isAtBottom: boolean) => void;
  onRangeChanged: (range: any) => void;
  components: any;
  itemContent: (index: number, item: ThreadItem) => ReactNode;
  hasNewActivity: boolean;
  onJumpToLatest: () => void;
};

const LegacyThreadStack = memo(function LegacyThreadStack({
  virtuosoStyle,
  data,
  virtuosoRef,
  followOutput,
  restoreStateFrom,
  onAtBottomStateChange,
  onRangeChanged,
  components,
  itemContent,
  hasNewActivity,
  onJumpToLatest,
}: LegacyThreadStackProps) {
  return (
    <div className="thread-stack">
      <Virtuoso
        style={virtuosoStyle}
        data={data}
        ref={(node) => {
          virtuosoRef.current = node;
        }}
        followOutput={followOutput}
        defaultItemHeight={56}
        computeItemKey={(index, item) => item?.id ?? `i:${index}`}
        restoreStateFrom={restoreStateFrom}
        atBottomStateChange={onAtBottomStateChange}
        rangeChanged={onRangeChanged}
        components={components}
        itemContent={itemContent}
      />

      {hasNewActivity && (
        <button
          type="button"
          className="new-activity-overlay"
          aria-label="Jump to latest"
          title="Jump to latest"
          onClick={onJumpToLatest}
        >
          ↓
        </button>
      )}
    </div>
  );
});

function ThreadItemView({
  item,
  variant,
  worktreeId,
  onFileOpenError,
  linkToken,
}: {
  item: ThreadItem;
  variant: SessionViewVariant;
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  linkToken: string | null;
}) {
  switch (item.kind) {
    case "message":
      return (
        <CollapsibleMessage
          id={item.id}
          role={item.role}
          content={item.content}
          attachments={item.attachments}
          delivery={item.delivery}
          worktreeId={worktreeId}
          onFileOpenError={onFileOpenError}
          linkToken={linkToken}
        />
      );
    case "assistant":
    case "tool":
      // Workbench uses specialized renderers; legacy never reaches here.
      return variant === "workbench" ? null : null;
    case "tool_group":
      return null;
    default:
      return null;
  }
}

function WorkbenchTurnHeaderView({
  header,
  expanded,
  onToggle,
}: {
  header: WorkbenchTurnHeader;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      className={`wb-turn-header ${expanded ? "wb-turn-header-expanded" : "wb-turn-header-collapsed"}`}
      onClick={onToggle}
      aria-expanded={expanded}
    >
      <div className="wb-turn-header-bubble">
        <div className="wb-turn-header-content">
          {header.plain_text.split("\n").map((line, idx, list) => (
            <span key={`${header.id}-${idx}`}>
              {line}
              {idx < list.length - 1 ? <br /> : null}
            </span>
          ))}
        </div>
        {header.attachments.length > 0 && (
          <div className="wb-turn-header-attachments" aria-label="Attachments">
            {header.attachments.map((a, idx) => {
              if (a.kind !== "image" && a.kind !== "image_ref") return null;
              const src = imageAttachmentSrc(a);
              const name = attachmentDisplayName(a.name);
              return <img key={idx} className="wb-turn-header-attachment-img" src={src} alt={name} title={name} />;
            })}
          </div>
        )}
      </div>
    </button>
  );
}

function WorkbenchComposer({
  session,
  modelOptions,
  effortOptions,
  currentModelId,
  autocomplete,
  onSetModel,
  onSetEffort,
  textareaRef,
  input,
  setInput,
  onSend,
  onInterrupt,
  onInsertAtFile,
  onInsertSlash,
  attachments,
  setAttachments,
}: {
  session: Session | null;
  modelOptions: Array<{ id: string; name: string }>;
  effortOptions: string[];
  currentModelId: string;
  autocomplete: ReturnType<typeof useComposerAutocomplete>;
  onSetModel: (modelId: string) => Promise<void>;
  onSetEffort: (effort: string) => Promise<void>;
  textareaRef: { current: HTMLTextAreaElement | null };
  input: string;
  setInput: (next: string) => void;
  onSend: () => Promise<void>;
  onInterrupt: () => void;
  onInsertAtFile: () => void;
  onInsertSlash: () => void;
  attachments: MessageAttachment[];
  setAttachments: React.Dispatch<React.SetStateAction<MessageAttachment[]>>;
}) {
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "0px";
    const next = Math.min(220, Math.max(28, el.scrollHeight));
    el.style.height = `${next}px`;
  }, [input, textareaRef]);

  const providerLabel = useMemo(() => {
    const p = String(session?.provider_id ?? "agent");
    if (p === "codex") return "Codex";
    if (p === "claude") return "Claude";
    if (p === "gemini") return "Gemini";
    return p.slice(0, 1).toUpperCase() + p.slice(1);
  }, [session?.provider_id]);

  const currentEffort = String(currentModelId).split("/")[1] ?? "";

  return (
    <div className="wb-composer">
      {attachments.length > 0 && (
        <div className="wb-composer-attachments">
          {attachments.map((a, idx) => (
            <button
              key={idx}
              type="button"
              className="wb-attach-chip"
              onClick={() => setAttachments((prev) => prev.filter((_, i) => i !== idx))}
              title="Remove attachment"
            >
              {a.kind === "image" || a.kind === "image_ref" ? (a.name ?? "image") : "attachment"} ×
            </button>
          ))}
        </div>
      )}

      <textarea
        ref={textareaRef}
        className="wb-composer-input"
        value={input}
        onChange={(e) => setInput(e.target.value)}
        placeholder="Ask follow-ups in the worktree"
        onKeyDown={(e) => {
          if (autocomplete.onKeyDown(e)) return;
          if (shouldSendOnEnter(e)) {
            e.preventDefault();
            onSend();
          }
        }}
        onKeyUp={() => autocomplete.syncFromDom()}
        onClick={() => autocomplete.syncFromDom()}
        onSelect={() => autocomplete.syncFromDom()}
        onPaste={async (e) => {
          const files = Array.from(e.clipboardData?.files ?? []);
          const images = files.filter((f) => (f.type || "").startsWith("image/"));
          if (images.length === 0) return;
          e.preventDefault();
          const next: MessageAttachment[] = [];
          for (const f of images) {
            const uploaded = await uploadBlob(f);
            next.push({
              kind: "image_ref",
              blob_id: uploaded.blob_id,
              mime_type: uploaded.mime_type,
              name: uploaded.name ?? f.name,
            });
          }
          setAttachments((prev) => [...prev, ...next]);
        }}
      />

      <ComposerAutocompleteMenu
        open={autocomplete.open}
        loading={autocomplete.loading}
        items={autocomplete.items}
        activeIndex={autocomplete.activeIndex}
        onPick={autocomplete.pick}
        onHoverIndex={(i) => autocomplete.setActiveIndex(i)}
        anchorRect={autocomplete.anchorRect}
        anchorInputRect={autocomplete.anchorInputRect}
        inlineFallback={autocomplete.inlineFallback}
      />

      <div className="wb-composer-footer">
        <div className="wb-composer-left">
          <div className="wb-composer-pill" title="Harness">
            ∞ {providerLabel}
          </div>

          {modelOptions.length > 0 && (
            <div className="wb-composer-pill" title="Model">
              <select
                className="wb-composer-select"
                value={currentModelId}
                onChange={(e) => onSetModel(e.target.value)}
              >
                {modelOptions.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            </div>
          )}

          {effortOptions.length > 0 && (
            <div className="wb-composer-pill" title="Effort">
              <select
                className="wb-composer-select"
                value={currentEffort}
                onChange={(e) => onSetEffort(e.target.value)}
              >
                {effortOptions.map((eff) => (
                  <option key={eff} value={eff}>
                    {formatEffortLabel(eff)}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>

        <div className="wb-composer-right">
          <button type="button" className="wb-composer-icon" onClick={onInterrupt} title="Interrupt">
            ■
          </button>
          <button type="button" className="wb-composer-icon" onClick={onInsertAtFile} title="Insert @file">
            @
          </button>
          <button type="button" className="wb-composer-icon" onClick={onInsertSlash} title="Slash commands">
            /
          </button>
          <button
            type="button"
            className="wb-composer-icon"
            onClick={() => fileInputRef.current?.click()}
            title="Attach image"
          >
            ☐
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept="image/*"
            multiple
            style={{ display: "none" }}
            onChange={async (e) => {
              const files = Array.from(e.target.files ?? []);
              const next = await imageFilesToBlobRefAttachments(files);
              setAttachments((prev) => [...prev, ...next]);
              e.target.value = "";
            }}
          />
        </div>
      </div>
    </div>
  );
}

function CollapsibleMessage({
  id,
  role,
  content,
  attachments,
  delivery,
  worktreeId,
  onFileOpenError,
  linkToken,
}: {
  id: string;
  role: "user" | "assistant";
  content: string;
  attachments: MessageAttachment[];
  delivery?: "immediate" | "queued";
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  linkToken: string | null;
}) {
  const lines = (content || "").split("\n");
  const isLong = lines.length > 20 || content.length > 1500;
  const [expanded, setExpanded] = useState(!isLong);
  const shown = expanded ? content : lines.slice(0, 20).join("\n");

  return (
    <div className={`msg ${role}`}>
      <div className="role">{role}</div>
      <div id={`msg-${id}`}>
        <Markdown
          content={shown}
          linkifyFiles={role === "assistant"}
          worktreeId={worktreeId}
          onFileOpenError={onFileOpenError}
          linkToken={linkToken}
        />
      </div>
      {attachments?.length > 0 && (
        <div className="attachments">
          {attachments.map((a, idx) => {
            if (a.kind !== "image" && a.kind !== "image_ref") return null;
            const src =
              a.kind === "image_ref"
                ? blobUrl(a.blob_id)
                : `data:${a.mime_type};base64,${a.data_base64}`;
            return (
              <img
                key={idx}
                className="attachment-img"
                src={src}
                alt={a.name ?? `image-${idx}`}
              />
            );
          })}
        </div>
      )}
      {delivery === "queued" && <span className="badge">Queued</span>}
      {isLong && (
        <button
          type="button"
          className="link"
          aria-expanded={expanded}
          aria-controls={`msg-${id}`}
          onClick={() => setExpanded((e) => !e)}
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}

function AssistantEntry({
  id,
  content,
  thought,
  thoughtSeconds,
  isComplete,
  variant,
  thoughtExpanded,
  onToggleThought,
  worktreeId,
  onFileOpenError,
  linkToken,
}: {
  id: string;
  content: string;
  thought: string;
  thoughtSeconds?: number;
  isComplete: boolean;
  variant: SessionViewVariant;
  thoughtExpanded: boolean;
  onToggleThought: () => void;
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  linkToken: string | null;
}) {
  const [showThought, setShowThought] = useState(false);
  const show = variant === "workbench" ? false : showThought;
  const toggle = variant === "workbench" ? onToggleThought : () => setShowThought((s) => !s);

  if (variant === "workbench") {
    return (
      <div className="wb-assistant-entry">
        <div className="wb-assistant-body">
          <Markdown
            content={content}
            linkifyFiles
            worktreeId={worktreeId}
            onFileOpenError={onFileOpenError}
            linkToken={linkToken}
          />
        </div>
      </div>
    );
  }

  return (
    <div className="msg assistant">
      <div className="row">
        <div className="role">assistant</div>
        <span className={`pill ${isComplete ? "ok" : "run"}`}>
          {isComplete ? "complete" : "streaming"}
        </span>
      </div>
      <div id={`msg-${id}`}>
        <Markdown
          content={content}
          linkifyFiles
          worktreeId={worktreeId}
          onFileOpenError={onFileOpenError}
          linkToken={linkToken}
        />
      </div>
      {thought.trim() && (
        <div className="thinking">
          <button
            type="button"
            className="thinking-header"
            aria-expanded={show}
            aria-controls={`thought-${id}`}
            onClick={toggle}
          >
            <span className="thinking-title">Thinking</span>
            <span className="thinking-chev">{show ? "▴" : "▾"}</span>
          </button>
          {show && (
            <pre id={`thought-${id}`} className="thought">
              {thought}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function WorkbenchToolRow({
  item,
  variant,
  expanded,
  onToggle,
}: {
  item: Extract<ThreadItem, { kind: "tool" }>;
  variant: SessionViewVariant;
  expanded: boolean;
  onToggle: () => void;
}) {
  if (variant !== "workbench") return <ToolCard item={item} />;

  const kind = String(item.tool_kind ?? "").toLowerCase();
  const pathFromLoc = item.locations?.[0]?.path;
  const title = String(item.title ?? "").trim();
  const summary = toolSummaryLine(kind, item.input);

  const normalizeWorktreePath = (p?: string) => {
    const s = String(p ?? "").trim();
    if (!s) return "";
    // Strip the Context worktree prefix.
    return s.replace(
      /\/home\/[^/]+\/\.context\/worktrees\/[0-9a-f-]+\/[0-9a-f-]+\//g,
      "",
    );
  };

  const shortPath = (p?: string) => {
    const s0 = normalizeWorktreePath(p);
    const s = String(s0 ?? "").trim();
    if (!s) return "";
    const parts = s.split(/[\\/]/).filter(Boolean);
    if (parts.length <= 2) return s;
    return `${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
  };

  const shortCommand = (raw: string) => {
    let cmd = String(raw ?? "").trim();
    if (!cmd) return "";
    cmd = cmd.replace(/^\/bin\/bash\s+-lc\s+/, "");
    cmd = cmd.replace(/^bash\s+-lc\s+/, "");
    cmd = cmd.replace(/^set\s+-euo\s+pipefail\s*;?\s*/i, "");
    cmd = cmd.replace(/^\s*&&\s*/, "");
    cmd = cmd.replace(/^"(.+)"$/, "$1");
    cmd = cmd.replace(
      /\/home\/[^/]+\/\.context\/worktrees\/[0-9a-f-]+\/[0-9a-f-]+\//g,
      "",
    );
    const mSupercat = cmd.match(/(?:^|\s)(\.?\/?scripts\/supercat\.sh)\s+([^\s&;]+)/);
    if (mSupercat) return `./scripts/supercat.sh ${shortPath(mSupercat[2])}`;
    const mReadMany =
      cmd.match(/rg\s+--files\s+([^\s|&;]+)\s*\|\s*sort\b[\s\S]*xargs[\s\S]*\bcat\b/) ??
      cmd.match(/find\s+([^\s|&;]+)\s+.*xargs[\s\S]*\bcat\b/);
    if (mReadMany) return `Read ${shortPath(mReadMany[1])}`;
    const mCat = cmd.match(/(?:^|[;&|]\s*)cat\s+([^\s|&;]+)(?:\s|$)/);
    if (mCat) return `Read ${shortPath(mCat[1])}`;
    const mLs = cmd.match(/(?:^|[;&|]\s*)(?:ls|ls\s+-la|ls\s+-l)\s+([^\s|&;]+)(?:\s|$)/);
    if (mLs) return `Explored ${shortPath(mLs[1])}`;
    const mRg = cmd.match(/(?:^|[;&|]\s*)rg\s+([^\s]+)\s+([^\s|&;]+)(?:\s|$)/);
    if (mRg) return `Searched ${truncateMiddle(mRg[1], 60)}`;
    const first = cmd.split(/\s+/).slice(0, 4).join(" ");
    return truncateMiddle(first, 80);
  };

  const makeParts = (verb: string, rest?: string) => {
    const trimmedRest = String(rest ?? "").trim();
    return {
      verb,
      rest: trimmedRest,
      label: trimmedRest ? `${verb} ${trimmedRest}` : verb,
    };
  };

  const parsePrefixed = (value: string, verbs: string[]) => {
    const trimmed = String(value ?? "").trim();
    if (!trimmed) return null;
    for (const verb of verbs) {
      if (trimmed === verb) return makeParts(verb);
      if (trimmed.startsWith(`${verb} `)) return makeParts(verb, trimmed.slice(verb.length + 1));
    }
    return null;
  };

  const labelParts = (() => {
    const parsed = Array.isArray((item.input as any)?.parsed_cmd) ? ((item.input as any).parsed_cmd as any[]) : [];
    if (parsed.length > 0) {
      const c0 = parsed[0] ?? {};
      if (c0.type === "list_files" && c0.path) return makeParts("Explored", shortPath(c0.path));
      if (c0.type === "read_file" && c0.path) return makeParts("Read", shortPath(c0.path));
    }

    if (kind === "search") {
      const q = String(item.input?.query ?? item.input?.pattern ?? item.input?.text ?? summary ?? "").trim();
      return q ? makeParts("Searched", truncateMiddle(q, 90)) : makeParts("Searched");
    }
    if (kind === "execute") {
      const cmd = Array.isArray(item.input?.command) ? item.input.command.join(" ") : item.input?.command;
      const short = shortCommand(cmd ?? "");
      const parsedShort = parsePrefixed(short, ["Read", "Explored", "Searched", "Wrote", "Edited"]);
      if (parsedShort) return parsedShort;
      return short ? makeParts("Run", short) : makeParts("Run");
    }
    if (kind === "read_file" || kind === "read") {
      const p = pathFromLoc ?? summary;
      return p ? makeParts("Read", shortPath(p)) : makeParts("Read");
    }
    if (kind === "write" || kind === "edit") {
      const p = summary || pathFromLoc;
      const verb = kind === "write" ? "Wrote" : "Edited";
      return p ? makeParts(verb, shortPath(p)) : makeParts(verb);
    }
    if (kind === "error") return makeParts("Error");

    const fallbackLabel = normalizeWorktreePath(title) || humanToolKind(item.tool_kind);
    const parsedLabel = parsePrefixed(fallbackLabel, [
      "Read",
      "Explored",
      "Searched",
      "Wrote",
      "Edited",
      "Run",
    ]);
    if (parsedLabel) return parsedLabel;
    const fallbackVerb = humanToolKind(item.tool_kind);
    if (fallbackLabel && fallbackLabel !== fallbackVerb) {
      return makeParts(fallbackVerb, fallbackLabel);
    }
    return makeParts(fallbackVerb);
  })();

  const { verb, rest, label } = labelParts;
  const hasDetails = item.has_details ?? !!(item.input || item.output_text?.trim());

  return (
    <div className="wb-tool-row">
      <button
        type="button"
        className={`wb-event-row ${expanded ? "wb-event-row-expanded" : ""}`}
        onClick={hasDetails ? onToggle : undefined}
        aria-expanded={hasDetails ? expanded : undefined}
        title={label}
      >
        <span className="wb-event-text">
          <span className="wb-tool-verb">{verb}</span>
          {rest ? <span className="wb-tool-rest"> {rest}</span> : null}
        </span>
      </button>
      {hasDetails && expanded && (
        <div className="wb-tool-details">
          {item.input && (
            <div className="wb-tool-section">
              <div className="wb-tool-section-title">Input</div>
              <pre className="wb-tool-pre">{formatToolInput(item.tool_kind, item.input)}</pre>
            </div>
          )}
          {!!item.output_text?.trim() && (
            <div className="wb-tool-section">
              <div className="wb-tool-section-title">Output</div>
              {looksLikeMarkdown(item.output_text) ? (
                <div className="wb-tool-markdown">
                  <Markdown content={item.output_text} />
                </div>
              ) : (
                <pre className="wb-tool-pre">{item.output_text}</pre>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function WorkbenchThoughtRow({ item }: { item: Extract<ThreadItem, { kind: "thought" }> }) {
  return <div className="wb-thought-row">{item.content}</div>;
}

function WorkbenchToolGroupRow({
  item,
  variant,
  expanded,
  onToggle,
  toolsLoading,
  onRequestTools,
  onToggleTool,
  expandedToolById,
}: {
  item: Extract<ThreadItem, { kind: "tool_group" }>;
  variant: SessionViewVariant;
  expanded: boolean;
  onToggle: () => void;
  toolsLoading: boolean;
  onRequestTools: () => void;
  onToggleTool: (id: string) => void;
  expandedToolById: Record<string, boolean>;
}) {
  if (variant !== "workbench") return null;

  const total = Math.max(item.tool_total ?? 0, item.tools.length);
  const parts: string[] = [];
  if (total > 0) {
    parts.push(`${total} tool${total === 1 ? "" : "s"}`);
  }
  if ((item.tool_running ?? 0) > 0) {
    parts.push(`${item.tool_running} running`);
  }
  if ((item.tool_failed ?? 0) > 0) {
    parts.push(`${item.tool_failed} failed`);
  }
  if (parts.length === 0 && item.thought.trim()) {
    parts.push("Thought");
  }
  const label = parts.join(" · ") || "Activity";
  const hasDetails = total > 0 || item.thought.trim().length > 0;

  useEffect(() => {
    if (!expanded) return;
    if (total > 0 && item.tools.length === 0 && !toolsLoading) {
      onRequestTools();
    }
  }, [expanded, total, item.tools.length, toolsLoading, onRequestTools]);

  return (
    <div className="wb-tool-group">
      <button
        type="button"
        className={`wb-event-row ${expanded ? "wb-event-row-expanded" : ""}`}
        onClick={hasDetails ? onToggle : undefined}
        aria-expanded={hasDetails ? expanded : undefined}
        title={label}
      >
        <span className="wb-event-text">{label}</span>
        {hasDetails && <span className="wb-event-chev">{expanded ? "▴" : "▾"}</span>}
      </button>
      {hasDetails && expanded && (
        <div className="wb-tool-group-body">
          {total > 0 && item.tools.length === 0 && toolsLoading && (
            <div className="wb-tool-loading">Loading tools…</div>
          )}
          {item.tools.map((tool) => (
            <WorkbenchToolRow
              key={tool.id}
              item={tool}
              variant={variant}
              expanded={expandedToolById[tool.id] ?? false}
              onToggle={() => onToggleTool(tool.id)}
            />
          ))}
          {item.thought.trim() && (
            <div className="wb-tool-thought">
              <div className="wb-tool-section-title">Thought</div>
              <pre className="wb-tool-pre">{item.thought}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ToolCard({ item }: { item: Extract<ThreadItem, { kind: "tool" }> }) {
  const [expanded, setExpanded] = useState(false);
  const isRunning = item.status === "in_progress" || item.status === "pending";
  const isFailed = item.status === "failed";
  const hasOutput = item.output_text.trim().length > 0;
  const shouldDefaultOpen = item.tool_kind === "execute" && isRunning;
  const isOpen = expanded || shouldDefaultOpen;
  const summary = useMemo(() => toolSummaryLine(item.tool_kind, item.input), [item.tool_kind, item.input]);

  return (
    <div className={`tool-card ${isOpen ? "expanded" : ""}`}>
      <button
        type="button"
        className="tool-header"
        onClick={() => setExpanded((e) => !e)}
        aria-expanded={isOpen}
        aria-controls={`tool-${item.id}`}
      >
        <div className="tool-header-left">
          <span className={`tool-icon kind-${item.tool_kind}`}>{toolKindIcon(item.tool_kind)}</span>
          <div className="tool-title-wrap">
            <div className="tool-title">{item.title}</div>
            <div className="tool-subtitle">
              <span className={`pill ${isFailed ? "err" : isRunning ? "run" : "ok"}`}>
                {humanToolStatus(item.status)}
              </span>
              {item.locations?.length === 1 && item.locations[0]?.path && (
                <span className="muted tool-path">{item.locations[0].path}</span>
              )}
              {summary && <span className="muted tool-summary">{summary}</span>}
            </div>
          </div>
        </div>
        <div className="tool-header-right">
          <span className="muted">{new Date(item.updated_at).toLocaleTimeString()}</span>
          <span className="thinking-chev">{isOpen ? "▴" : "▾"}</span>
        </div>
      </button>

      {isOpen && (
        <div id={`tool-${item.id}`} className="tool-body">
          {item.input && (
            <div className="tool-section">
              <div className="tool-section-title">Input</div>
              <pre className="tool-pre">{formatToolInput(item.tool_kind, item.input)}</pre>
            </div>
          )}
          {hasOutput && (
            <div className="tool-section">
              <div className="tool-section-title">Output</div>
              {looksLikeMarkdown(item.output_text) ? (
                <div className="tool-markdown">
                  <Markdown content={item.output_text} />
                </div>
              ) : (
                <pre className="tool-pre tool-output">{item.output_text}</pre>
              )}
            </div>
          )}
          <details className="tool-raw">
            <summary className="link">
              Raw event {item.updates_seen > 1 ? `(updated ${item.updates_seen}×)` : ""}
            </summary>
            <pre className="json">{JSON.stringify(item.raw, null, 2)}</pre>
          </details>
        </div>
      )}
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

function PlanPanel({ entries }: { entries: any[] }) {
  const [expanded, setExpanded] = useState(false);
  const counts = entries.reduce(
    (acc, e) => {
      const status = e?.status ?? "pending";
      if (status === "completed") acc.completed += 1;
      else if (status === "in_progress") acc.in_progress += 1;
      else acc.pending += 1;
      return acc;
    },
    { pending: 0, in_progress: 0, completed: 0 },
  );

  return (
    <div className="plan-bar card">
      <div className="row">
        <strong>Plan</strong>
        <span className="muted">
          {counts.in_progress} in progress · {counts.pending} pending ·{" "}
          {counts.completed} done
        </span>
      </div>
      <button type="button" onClick={() => setExpanded((e) => !e)}>
        {expanded ? "Hide plan" : "Show plan"}
      </button>
      {expanded && (
        <ul className="sublist">
          {entries.map((e, idx) => (
            <li key={idx} className={`plan-item ${e.status ?? ""}`}>
              <span className="muted">{e.status ?? "pending"}</span> {e.content}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function ActivityBar({ planEntries, diffText }: { planEntries: any[]; diffText: string }) {
  const [planOpen, setPlanOpen] = useState(false);
  const [editsOpen, setEditsOpen] = useState(false);

  const planStats = useMemo(() => {
    const counts = planEntries.reduce(
      (acc, e) => {
        const status = e?.status ?? "pending";
        if (status === "completed") acc.completed += 1;
        else if (status === "in_progress") acc.in_progress += 1;
        else acc.pending += 1;
        return acc;
      },
      { pending: 0, in_progress: 0, completed: 0 },
    );
    const current = planEntries.find((e) => e?.status === "in_progress") ?? null;
    return { ...counts, current };
  }, [planEntries]);

  const editedFiles = useMemo(() => extractEditedFiles(diffText), [diffText]);

  if (planEntries.length === 0 && editedFiles.length === 0) return null;

  return (
    <div className="activity-bar">
      {planEntries.length > 0 && (
        <div className="activity-section">
          <button type="button" className="activity-summary" onClick={() => setPlanOpen((v) => !v)}>
            <span className="activity-title">Plan</span>
            {planStats.current && !planOpen ? (
              <span className="muted activity-current">
                Current: {String(planStats.current.content ?? "").trim() || "in progress"}
              </span>
            ) : (
              <span className="muted">
                {planStats.in_progress} in progress · {planStats.pending} pending · {planStats.completed} done
              </span>
            )}
            {planStats.pending > 0 && !planOpen && (
              <span className="muted activity-right">{planStats.pending} left</span>
            )}
            <span className="thinking-chev">{planOpen ? "▴" : "▾"}</span>
          </button>
          {planOpen && (
            <ul className="activity-list">
              {planEntries.map((e, idx) => (
                <li key={idx} className={`plan-item ${e.status ?? ""}`}>
                  <span className={`plan-dot ${e.status ?? ""}`} />
                  <span className="muted">{e.status ?? "pending"}</span>{" "}
                  <span className="activity-item-text">{e.content}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {editedFiles.length > 0 && (
        <div className="activity-section">
          <button type="button" className="activity-summary" onClick={() => setEditsOpen((v) => !v)}>
            <span className="activity-title">Edits</span>
            <span className="muted">{editedFiles.length} file{editedFiles.length === 1 ? "" : "s"}</span>
            <span className="thinking-chev">{editsOpen ? "▴" : "▾"}</span>
          </button>
          {editsOpen && (
            <ul className="activity-list">
              {editedFiles.map((f) => (
                <li key={f} className="activity-item-text">
                  {f}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

function extractEditedFiles(diffText: string): string[] {
  const text = String(diffText ?? "");
  const files = new Set<string>();
  for (const line of text.split("\n")) {
    const m = /^diff --git a\/(.+?) b\/(.+)$/.exec(line);
    if (m) {
      files.add(m[2]);
      continue;
    }
    const m2 = /^\+\+\+ b\/(.+)$/.exec(line);
    if (m2) files.add(m2[1]);
  }
  return [...files].slice(0, 200);
}

type MdastNode = {
  type?: string;
  children?: MdastNode[];
  [key: string]: unknown;
};

type FileRef = {
  path: string;
  line?: number;
  col?: number;
};

type ParsedContextOpen = {
  worktreeId?: string;
  file?: string;
  path?: string;
  line?: number;
  col?: number;
};

function isAbsolutePath(path: string): boolean {
  if (!path) return false;
  if (path.startsWith("/") || path.startsWith("\\")) return true;
  return /^[A-Za-z]:[\\/]/.test(path);
}

function buildContextOpenUrl(worktreeId: string, ref: FileRef, token?: string | null): string {
  const params = new URLSearchParams();
  params.set("v", "1");
  params.set("openWith", "editor");
  if (isAbsolutePath(ref.path)) {
    params.set("path", ref.path);
  } else {
    params.set("worktreeId", worktreeId);
    params.set("file", ref.path);
  }
  if (typeof ref.line === "number") params.set("line", String(ref.line));
  if (typeof ref.col === "number") params.set("col", String(ref.col));
  if (token) params.set("token", token);
  return `context://open?${params.toString()}`;
}

function parseContextOpenUrl(href: string): ParsedContextOpen | null {
  try {
    const url = new URL(href);
    if (url.protocol !== "context:") return null;
    if (url.hostname !== "open") return null;
    const worktreeId = url.searchParams.get("worktreeId") ?? "";
    const file = url.searchParams.get("file") ?? "";
    const path = url.searchParams.get("path") ?? "";
    if (!worktreeId && !path) return null;
    if (worktreeId && !file) return null;
    const line = url.searchParams.get("line");
    const col = url.searchParams.get("col");
    const parsedLine = line ? Number.parseInt(line, 10) : undefined;
    const parsedCol = col ? Number.parseInt(col, 10) : undefined;
    const normalizedLine = parsedLine && parsedLine > 0 ? parsedLine : undefined;
    const normalizedCol = parsedCol && parsedCol > 0 ? parsedCol : undefined;
    return {
      worktreeId: worktreeId || undefined,
      file: file || undefined,
      path: path || undefined,
      line: Number.isFinite(normalizedLine ?? NaN) ? normalizedLine : undefined,
      col: Number.isFinite(normalizedCol ?? NaN) ? normalizedCol : undefined,
    };
  } catch {
    return null;
  }
}

function looksLikeFilePath(path: string): boolean {
  if (!path) return false;
  if (path === "." || path === "..") return false;
  if (path.includes("://")) return false;
  const hasSlash = /[\\/]/.test(path);
  const hasExt = /\.[A-Za-z0-9][A-Za-z0-9_-]*$/.test(path);
  return hasSlash || hasExt;
}

function parseFileRefToken(raw: string): FileRef | null {
  let path = raw;
  let line: number | undefined;
  let col: number | undefined;

  const hashMatch = raw.match(/^(.*)#L(\d+)(?:C(\d+))?$/);
  if (hashMatch) {
    path = hashMatch[1];
    line = Number.parseInt(hashMatch[2], 10);
    if (hashMatch[3]) col = Number.parseInt(hashMatch[3], 10);
  } else {
    const colonMatch = raw.match(/^(.*?)(?::(\d+)(?::(\d+))?)$/);
    if (colonMatch) {
      path = colonMatch[1];
      line = Number.parseInt(colonMatch[2], 10);
      if (colonMatch[3]) col = Number.parseInt(colonMatch[3], 10);
    }
  }

  if (!looksLikeFilePath(path)) return null;
  return {
    path,
    line: Number.isFinite(line ?? NaN) ? line : undefined,
    col: Number.isFinite(col ?? NaN) ? col : undefined,
  };
}

function splitFileRefs(text: string, worktreeId: string, linkToken?: string | null): MdastNode[] {
  const parts = text.split(/(\s+)/);
  const nodes: MdastNode[] = [];
  const leadingPunct = new Set(["(", "{", "[", "\"", "'", "`", "<"]);
  const trailingPunct = new Set([")", "]", "}", "\"", "'", "`", ",", ".", ";", "!", "?"]);

  for (const part of parts) {
    if (!part) continue;
    if (part.trim() === "") {
      nodes.push({ type: "text", value: part });
      continue;
    }

    let candidate = part;
    let prefix = "";
    let suffix = "";
    while (candidate && leadingPunct.has(candidate[0])) {
      prefix += candidate[0];
      candidate = candidate.slice(1);
    }
    while (candidate && trailingPunct.has(candidate[candidate.length - 1])) {
      suffix = candidate[candidate.length - 1] + suffix;
      candidate = candidate.slice(0, -1);
    }

    const ref = parseFileRefToken(candidate);
    if (!ref) {
      nodes.push({ type: "text", value: part });
      continue;
    }

    if (prefix) nodes.push({ type: "text", value: prefix });
    nodes.push({
      type: "link",
      url: buildContextOpenUrl(worktreeId, ref, linkToken),
      children: [{ type: "text", value: candidate }],
    });
    if (suffix) nodes.push({ type: "text", value: suffix });
  }

  return nodes;
}

function FencedCodeBlock({
  codeString,
  lang,
}: {
  codeString: string;
  lang?: string;
}) {
  const [copied, setCopied] = useState(false);
  const resetTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    };
  }, []);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(codeString);
    } catch {
      return;
    }

    setCopied(true);
    if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    resetTimerRef.current = window.setTimeout(() => setCopied(false), 1000);
  }, [codeString]);

  return (
    <div className="codeblock">
      <div className="codeblock-toolbar">
        <button
          type="button"
          className="wb-icon codeblock-copy"
          aria-label={copied ? "Copied" : "Copy code"}
          title={copied ? "Copied" : "Copy"}
          onClick={() => void handleCopy()}
        >
          {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        </button>
      </div>
      <div className="codeblock-body">
        <SyntaxHighlighter
          style={oneDark}
          language={lang}
          PreTag="div"
          customStyle={{
            margin: 0,
            background: "transparent",
            padding: "28px 12px 12px",
            fontSize: "12px",
            lineHeight: 1.45,
            width: "100%",
            maxWidth: "100%",
            boxSizing: "border-box",
            overflowX: "auto",
          }}
          codeTagProps={{ style: { fontFamily: "var(--mono)" } }}
        >
          {codeString}
        </SyntaxHighlighter>
      </div>
    </div>
  );
}

function remarkLinkifyFileRefs(opts: { worktreeId: string; token?: string | null }) {
  return (tree: MdastNode) => {
    const walk = (node: MdastNode) => {
      if (!node || typeof node !== "object") return;
      if (node.type && ["code", "inlineCode", "link", "linkReference"].includes(node.type)) return;
      if (!Array.isArray(node.children)) return;

      const next: MdastNode[] = [];
      for (const child of node.children) {
        if (child?.type === "text" && typeof (child as any).value === "string") {
          next.push(...splitFileRefs(String((child as any).value), opts.worktreeId, opts.token));
          continue;
        }
        walk(child as MdastNode);
        next.push(child as MdastNode);
      }
      node.children = next;
    };

    walk(tree);
  };
}

function remarkNormalizeCursorMarkdown() {
  return (tree: MdastNode) => {
    const walk = (node: MdastNode) => {
      if (node?.type === "listItem" && Array.isArray(node.children)) {
        const maybeInlineFromCode = (codeNode: MdastNode): string | null => {
          if (codeNode?.type !== "code") return null;
          const lang = typeof codeNode.lang === "string" ? codeNode.lang : undefined;
          const raw = typeof codeNode.value === "string" ? codeNode.value : "";
          const trimmed = raw.replace(/[\r\n]+$/, "");
          if ((!lang || lang === "code") && trimmed.length > 0 && !trimmed.includes("\n")) return trimmed;
          return null;
        };

        // Cursor sometimes emits list items that are a single one-line fenced code block.
        if (node.children.length === 1) {
          const trimmed = maybeInlineFromCode(node.children[0] as MdastNode);
          if (trimmed) {
            node.children = [
              {
                type: "paragraph",
                children: [{ type: "inlineCode", value: trimmed }],
              },
            ];
          }
        }

        // Some models emit list items as:
        //   - code
        //     ```code
        //     .github/
        //     ```
        // Normalize that to a single inlineCode row.
        if (node.children.length === 2) {
          const [p, codeNode] = node.children as MdastNode[];
          if (p?.type === "paragraph" && Array.isArray((p as any).children)) {
            const kids = (p as any).children as MdastNode[];
            if (
              kids.length === 1 &&
              kids[0]?.type === "text" &&
              String((kids[0] as any).value ?? "").trim().toLowerCase() === "code"
            ) {
              const trimmed = maybeInlineFromCode(codeNode);
              if (trimmed) {
                node.children = [
                  {
                    type: "paragraph",
                    children: [{ type: "inlineCode", value: trimmed }],
                  },
                ];
              }
            }
          }
        }
      }

      if (!Array.isArray(node.children)) return;
      for (let i = 0; i < node.children.length; i++) {
        const child = node.children[i] as MdastNode;
        if (
          child?.type === "paragraph" &&
          Array.isArray(child.children) &&
          child.children.length === 1 &&
          (child.children[0] as any)?.type === "text"
        ) {
          const raw = String((child.children[0] as any)?.value ?? "");
          const trimmed = raw.trim();
          if (/^(⸻|—{3,}|-{3,}|_{3,}|\*{3,})$/.test(trimmed)) {
            node.children[i] = { type: "thematicBreak" };
            continue;
          }
        }
        walk(child);
      }
    };

    walk(tree);
  };
}

function Markdown({
  content,
  linkifyFiles = false,
  worktreeId = null,
  onFileOpenError,
  linkToken,
}: {
  content: string;
  linkifyFiles?: boolean;
  worktreeId?: string | null;
  onFileOpenError?: (message: string | null) => void;
  linkToken?: string | null;
}) {
  const remarkPlugins: any[] = [remarkGfm, remarkNormalizeCursorMarkdown];
  if (linkifyFiles && worktreeId) {
    remarkPlugins.push([remarkLinkifyFileRefs, { worktreeId, token: linkToken }]);
  }

  return (
    <ReactMarkdown
      remarkPlugins={remarkPlugins}
      urlTransform={(url) => (url.startsWith("context://") ? url : defaultUrlTransform(url))}
      components={{
        a({ href, children, className, ...rest }) {
          const isContextOpen = typeof href === "string" && href.startsWith("context://open?");
          if (!isContextOpen) {
            return (
              <a href={href} className={className} {...rest}>
                {children}
              </a>
            );
          }

          const handleClick = async (event: MouseEvent<HTMLAnchorElement>) => {
            if (!isDesktopApp()) {
              return;
            }
            event.preventDefault();
            if (!event.metaKey && !event.ctrlKey) return;
            if (!href) return;
            const parsed = parseContextOpenUrl(href);
            if (!parsed) {
              onFileOpenError?.("Couldn't parse file link.");
              return;
            }
            try {
              if (parsed.worktreeId && parsed.file) {
                await desktopOpenFile({
                  worktree_id: parsed.worktreeId,
                  path: parsed.file,
                  line: parsed.line ?? null,
                  col: parsed.col ?? null,
                });
              } else if (parsed.path) {
                await desktopOpenPath({
                  path: parsed.path,
                  line: parsed.line ?? null,
                  col: parsed.col ?? null,
                });
              } else {
                throw new Error("Missing file reference.");
              }
              onFileOpenError?.(null);
            } catch (e: any) {
              onFileOpenError?.(e?.message ?? String(e));
            }
          };

          const combinedClassName = [className, "ctx-file-link"].filter(Boolean).join(" ");
          return (
            <a
              href={href}
              className={combinedClassName}
              title="Cmd/Ctrl+Click to open in editor"
              onClick={handleClick}
              {...rest}
            >
              {children}
            </a>
          );
        },
        pre({ children }) {
          return <>{children}</>;
        },
        code({ inline, className, children }: { inline?: boolean; className?: string; children?: ReactNode }) {
          const match = /language-([A-Za-z0-9_-]+)/.exec(className || "");
          const rawLang = match?.[1];
          const lang = rawLang && rawLang !== "code" ? rawLang : undefined;
          const codeString = String(children ?? "").replace(/[\r\n]+$/, "");
          if (inline) {
            return <code className={className}>{children}</code>;
          }

          if (!lang && !codeString.includes("\n")) {
            return <code className={className}>{codeString}</code>;
          }

          return <FencedCodeBlock codeString={codeString} lang={lang} />;
        },
      }}
    >
      {content}
    </ReactMarkdown>
  );
}

export function deriveMessagesKey(messages: Message[]): string {
  if (messages.length === 0) return "0";
  const last = messages[messages.length - 1];
  const lastId = idToString(last?.id);
  const lastUpdated = last?.created_at ?? "";
  const contentHash = hashString(String(last?.content ?? ""));
  return `${messages.length}:${lastId}:${lastUpdated}:${contentHash}`;
}

function hashString(value: string): string {
  let hash = 5381;
  for (let i = 0; i < value.length; i += 1) {
    hash = ((hash << 5) + hash) ^ value.charCodeAt(i);
  }
  return (hash >>> 0).toString(36);
}

function deriveTurnsKey(turns: SessionTurn[]): string {
  if (turns.length === 0) return "0";
  const first = turns[0];
  const last = turns[turns.length - 1];
  return `${turns.length}:${first.start_seq ?? ""}:${last.start_seq ?? ""}:${last.updated_at ?? ""}`;
}

export function buildWorkbenchThreadViewModel(
  turns: SessionTurn[],
  messages: Message[],
  toolsByTurnId: Record<string, SessionTurnTool[]>,
  events: SessionEvent[],
): WorkbenchThreadView {
  if (turns.length > 0) {
    return buildWorkbenchThreadViewModelFromTurns(turns, messages, toolsByTurnId, events);
  }
  return buildWorkbenchThreadViewModelFromEvents(events, messages);
}

function shouldRenderThoughtChunk(ev: SessionEvent): boolean {
  const payload = ev.payload_json ?? {};
  const meta =
    payload?.acp_update?._meta ??
    payload?.acp_update?.meta ??
    payload?._meta ??
    payload?.meta ??
    {};
  if (meta?.heartbeat === true) return false;
  const reasoningKind = meta?.codex?.reasoning_kind ?? meta?.codex?.reasoningKind;
  if (reasoningKind === "summary") return false;
  return true;
}

function buildWorkbenchThreadViewModelFromTurns(
  turns: SessionTurn[],
  messages: Message[],
  toolsByTurnId: Record<string, SessionTurnTool[]>,
  events: SessionEvent[],
): WorkbenchThreadView {
  const debugEvents: SessionEvent[] = [];
  const groups: WorkbenchThreadView["groups"] = [];

  const messageById = new Map<string, Message>();
  const messagesByTurnId = new Map<string, Message[]>();
  for (const m of messages) {
    const mid = idToString(m.id);
    if (mid) messageById.set(mid, m);
    const turnId = idToString(m.turn_id);
    if (turnId) {
      const list = messagesByTurnId.get(turnId) ?? [];
      list.push(m);
      messagesByTurnId.set(turnId, list);
    }
  }

  for (const turn of turns) {
    const turnId = idToString(turn.turn_id) || `turn-${turn.started_at}`;
    const userMessageId = turn.user_message_id ? idToString(turn.user_message_id) : "";

    const userMessage = userMessageId ? messageById.get(userMessageId) : undefined;

    const header: WorkbenchTurnHeader | null = userMessage
      ? {
        id: userMessageId || turnId,
        content: userMessage.content ?? "",
        plain_text: markdownToPlainText(userMessage.content ?? ""),
        attachments: Array.isArray((userMessage as any).attachments)
          ? ((userMessage as any).attachments as MessageAttachment[])
          : [],
        created_at: userMessage.created_at,
      }
      : null;

    const tools = (toolsByTurnId[turnId] ?? []).map((tool) => {
      const toolKind = String(tool.tool_kind ?? "tool");
      const title = String(tool.title ?? humanToolKind(toolKind));
      const summaryOnly = (tool as any).summary_only === true;
      const hasDetails =
        !summaryOnly && (tool.input_json != null || String(tool.output_text ?? "").trim().length > 0);
      return {
        kind: "tool",
        id: `tool-${turnId}-${tool.tool_call_id}`,
        tool_call_id: tool.tool_call_id,
        created_at: tool.created_at,
        updated_at: tool.updated_at ?? tool.created_at,
        tool_kind: toolKind,
        title,
        status: String(tool.status ?? "pending"),
        locations: [],
        input: tool.input_json ?? null,
        output_text: String(tool.output_text ?? ""),
        raw: tool,
        updates_seen: 1,
        has_details: hasDetails,
      } satisfies Extract<ThreadItem, { kind: "tool" }>;
    });

    // Tool summaries are supplied by the session head (toolsByTurnId).
    // Avoid rebuilding tool rows from events here to keep workbench switching fast.

    const thought = String(turn.thought_partial ?? "");
    const hasThought = thought.trim().length > 0;
    const hasToolDetails = tools.length > 0;

    const assistantMessages = (messagesByTurnId.get(turnId) ?? [])
      .filter((m) => m.role === "assistant")
      .slice()
      .sort((a, b) => {
        const sa = Number(a.turn_sequence ?? Number.NaN);
        const sb = Number(b.turn_sequence ?? Number.NaN);
        if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
        if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
        if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
        return String(a.created_at).localeCompare(String(b.created_at));
      });

    type TimelineEntry = {
      item: ThreadItem;
      created_at: string;
      kind: "assistant" | "tool";
      turn_sequence?: number;
    };

    const timeline: TimelineEntry[] = [];
    for (const m of assistantMessages) {
      timeline.push({
        item: {
          kind: "assistant",
          id: `assistant-${turnId}-${m.turn_sequence ?? m.created_at}`,
          turn_id: turnId,
          created_at: m.created_at,
          content: m.content ?? "",
          thought: "",
          is_complete: true,
        },
        created_at: m.created_at,
        kind: "assistant",
        turn_sequence: Number(m.turn_sequence ?? Number.NaN),
      });
    }

    const pendingContent = String(turn.assistant_partial ?? "");
    if (pendingContent.trim().length > 0) {
      timeline.push({
        item: {
          kind: "assistant",
          id: `assistant-${turnId}-pending`,
          turn_id: turnId,
          created_at: turn.updated_at ?? turn.started_at,
          content: pendingContent,
          thought: "",
          is_complete: false,
        },
        created_at: turn.updated_at ?? turn.started_at,
        kind: "assistant",
        turn_sequence: Number.MAX_SAFE_INTEGER,
      });
    }

    for (const tool of tools) {
      timeline.push({
        item: tool,
        created_at: tool.created_at,
        kind: "tool",
      });
    }

    timeline.sort((a, b) => {
      const tcmp = String(a.created_at).localeCompare(String(b.created_at));
      if (tcmp !== 0) return tcmp;
      if (a.kind === "assistant" && b.kind === "assistant") {
        const sa = Number(a.turn_sequence ?? Number.NaN);
        const sb = Number(b.turn_sequence ?? Number.NaN);
        if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
      }
      if (a.kind === "assistant" && b.kind === "tool") return -1;
      if (a.kind === "tool" && b.kind === "assistant") return 1;
      return String(a.item.id).localeCompare(String(b.item.id));
    });

    const items: ThreadItem[] = timeline.map((entry) => entry.item);

    if (!hasToolDetails && (turn.tool_total ?? 0) > 0) {
      items.unshift({
        kind: "tool_group",
        id: `tool-group-${turnId}`,
        turn_id: turnId,
        created_at: turn.started_at,
        updated_at: turn.updated_at,
        tool_total: turn.tool_total ?? 0,
        tool_pending: turn.tool_pending ?? 0,
        tool_running: turn.tool_running ?? 0,
        tool_completed: turn.tool_completed ?? 0,
        tool_failed: turn.tool_failed ?? 0,
        tools: [],
        thought: "",
      });
    }

    if (hasThought) {
      const thoughtItem: ThreadItem = {
        kind: "thought",
        id: `thought-${turnId}`,
        turn_id: turnId,
        created_at: turn.updated_at ?? turn.started_at,
        content: thought,
      };
      let insertAt = -1;
      for (let i = items.length - 1; i >= 0; i--) {
        if (items[i]?.kind === "assistant") {
          insertAt = i;
          break;
        }
      }
      if (insertAt >= 0) {
        items.splice(insertAt, 0, thoughtItem);
      } else {
        items.push(thoughtItem);
      }
    }

    if (items.length === 0) {
      items.push({ kind: "spacer", id: `spacer-${turnId}`, created_at: turn.started_at });
    }

    groups.push({ key: `turn-${turnId}`, header, items });
  }

  return { groups, debugEvents };
}

function buildToolItemsFromEventsForTurn(
  events: SessionEvent[],
  turnId: string,
): Array<Extract<ThreadItem, { kind: "tool" }>> {
  const toolById = new Map<string, Extract<ThreadItem, { kind: "tool" }>>();

  for (const ev of events) {
    const evTurnId = idToString((ev as any).turn_id);
    if (evTurnId !== turnId) continue;
    if (!["tool_call", "tool_call_update", "tool_result"].includes(String(ev.event_type))) continue;

    const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
    const toolCallId =
      String(
        ev.payload_json?.tool_call_id ??
          update?.toolCallId ??
          update?.tool_call_id ??
          update?.rawInput?.call_id ??
          update?.raw_input?.call_id ??
          update?.toolCall?.rawInput?.call_id ??
          "",
      ).trim();
    if (!toolCallId) continue;

    let tool = toolById.get(toolCallId);
    if (!tool) {
      tool = {
        kind: "tool",
        id: `tool-${turnId}-${toolCallId}`,
        tool_call_id: toolCallId,
        created_at: ev.created_at,
        updated_at: ev.created_at,
        tool_kind: "tool",
        title: "Tool",
        status: "pending",
        locations: [],
        input: null,
        output_text: "",
        raw: null,
        updates_seen: 0,
        has_details: true,
      };
      toolById.set(toolCallId, tool);
    }

    tool.updated_at = ev.created_at;
    tool.updates_seen += 1;
    tool.raw = ev.payload_json ?? tool.raw;

    const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
    if (nextKind) tool.tool_kind = nextKind;

    const nextTitle = String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
    if (nextTitle) tool.title = nextTitle;
    else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

    const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
    if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
    else if (ev.event_type === "tool_result") tool.status = "completed";

    const locs = Array.isArray(update?.locations) ? update.locations : [];
    tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

    const input = update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? null;
    if (input != null) tool.input = input;

    const nextOutput =
      update?.outputText ??
      update?.output_text ??
      update?.toolCall?.outputText ??
      update?.toolCall?.output_text ??
      update?.result ??
      null;
    if (typeof nextOutput === "string" && nextOutput.trim()) {
      tool.output_text = mergeStreamingText(tool.output_text, nextOutput);
    }
  }

  return Array.from(toolById.values()).sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
}

function buildWorkbenchThreadViewModelFromEvents(events: SessionEvent[], messages: Message[]): WorkbenchThreadView {
  type ToolItem = Extract<ThreadItem, { kind: "tool" }>;
  type TurnGroup = {
    key: string;
    header: WorkbenchTurnHeader | null;
    first_at: string;
    toolItems: ToolItem[];
    toolById: Map<string, ToolItem>;
    assistant: Extract<ThreadItem, { kind: "assistant" }> | null;
    thought_first_at: string | null;
    thought_last_at: string | null;
    assistant_first_at: string | null;
    assistant_complete_at: string | null;
  };

  const debugEvents: SessionEvent[] = [];

  const userMessages = messages
    .filter((m) => m.role === "user")
    .slice()
    .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

  const assistantMessages = messages
    .filter((m) => m.role === "assistant")
    .slice()
    .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

  const ensureTool = (g: TurnGroup, toolCallId: string, createdAt: string) => {
    const existing = g.toolById.get(toolCallId);
    if (existing) return existing;
    const t: ToolItem = {
      kind: "tool",
      id: `tool-${toolCallId}`,
      tool_call_id: toolCallId,
      created_at: createdAt,
      updated_at: createdAt,
      tool_kind: "tool",
      title: "Tool",
      status: "pending",
      locations: [],
      input: null,
      output_text: "",
      raw: null,
      updates_seen: 0,
      has_details: true,
    };
    g.toolById.set(toolCallId, t);
    g.toolItems.push(t);
    return t;
  };

  // If messages haven't been refreshed yet (common in the Workbench "Start" flow), fall back to
  // grouping by `user_message` events so streamed assistant/tool updates still render.
  if (userMessages.length === 0) {
    const userEvents = events
      .filter((e) => e.event_type === "user_message")
      .slice()
      .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

    const eventsInRangeExclusive = (startIso: string, endIso: string | null) => {
      const start = Date.parse(startIso);
      const end = endIso ? Date.parse(endIso) : Number.POSITIVE_INFINITY;
      return events.filter((e) => {
        const t = Date.parse(String(e.created_at));
        if (!Number.isFinite(t) || !Number.isFinite(start)) return false;
        return t >= start && t < end;
      });
    };

    const groups: WorkbenchThreadView["groups"] = [];

    if (userEvents.length === 0) {
      // As a last resort, show any tool activity even without a user turn anchor.
      const g: TurnGroup = {
        key: "no-user-messages",
        header: null,
        first_at: events[0]?.created_at ?? new Date().toISOString(),
        toolItems: [],
        toolById: new Map(),
        assistant: null,
        thought_first_at: null,
        thought_last_at: null,
        assistant_first_at: null,
        assistant_complete_at: null,
      };
      let thought = "";
      let thoughtAt: string | null = null;
      for (const ev of events) {
        if (ev.event_type === "thought_chunk") {
          const fragment = String(ev.payload_json?.content_fragment ?? "");
          if (fragment) {
            thought += fragment;
            thoughtAt = thoughtAt ?? ev.created_at;
          }
        }
        const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
        const toolCallId =
          String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
        if (!toolCallId) continue;
        ensureTool(g, toolCallId, ev.created_at);
      }
      const items: ThreadItem[] = [];
      if (thought.trim()) {
        items.push({
          kind: "thought",
          id: `thought-${g.key}`,
          turn_id: g.key,
          created_at: thoughtAt ?? g.first_at,
          content: thought,
        });
      }
      items.push(...g.toolItems);
      if (items.length === 0) items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
      groups.push({ key: g.key, header: g.header, items });
      return { groups, debugEvents };
    }

    for (let i = 0; i < userEvents.length; i++) {
      const u = userEvents[i];
      const nextUser = userEvents[i + 1] ?? null;
      const mid =
        String(u.payload_json?.message_id ?? "").trim() ||
        (idToString(u.id) || `msg-${u.created_at}`);

      const header: WorkbenchTurnHeader = {
        id: mid,
        content: String(u.payload_json?.content ?? ""),
        plain_text: markdownToPlainText(String(u.payload_json?.content ?? "")),
        attachments: Array.isArray(u.payload_json?.attachments)
          ? (u.payload_json.attachments as MessageAttachment[])
          : [],
        created_at: u.created_at,
      };

      const g: TurnGroup = {
        key: `m-${mid}`,
        header,
        first_at: u.created_at,
        toolItems: [],
        toolById: new Map(),
        assistant: null,
        thought_first_at: null,
        thought_last_at: null,
        assistant_first_at: null,
        assistant_complete_at: null,
      };

      const evs = eventsInRangeExclusive(u.created_at, nextUser?.created_at ?? null);

      for (const ev of evs) {
        const eventId = idToString(ev.id) || `${ev.created_at}`;
        if (ev.created_at < g.first_at) g.first_at = ev.created_at;

        switch (ev.event_type) {
          case "error": {
            const message = String(ev.payload_json?.message ?? "Error");
            const provider = String(ev.payload_json?.provider ?? "").trim();
            const output = provider ? `${message}\nprovider: ${provider}` : message;
            const tool = ensureTool(g, `error-${eventId}`, ev.created_at);
            tool.tool_kind = "error";
            tool.title = "Error";
            tool.status = "failed";
            tool.updated_at = ev.created_at;
            tool.updates_seen += 1;
            tool.input = ev.payload_json ?? null;
            tool.output_text = output;
            tool.raw = ev;
            break;
          }
          case "assistant_chunk": {
            const fragment = String(ev.payload_json?.content_fragment ?? "");
            if (!fragment) break;
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                turn_id: g.key,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.assistant_first_at = g.assistant_first_at ?? ev.created_at;
            g.assistant.content += fragment;
            break;
          }
          case "assistant_complete": {
            const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                turn_id: g.key,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.assistant_complete_at = ev.created_at;
            if (full) g.assistant.content = full;
            g.assistant.is_complete = true;
            break;
          }
          case "thought_chunk": {
            if (!shouldRenderThoughtChunk(ev)) break;
            const fragment = String(ev.payload_json?.content_fragment ?? "");
            if (!fragment) break;
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                turn_id: g.key,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.thought_first_at = g.thought_first_at ?? ev.created_at;
            g.thought_last_at = ev.created_at;
            g.assistant.thought += fragment;
            break;
          }
          case "tool_call":
          case "tool_call_update":
          case "tool_result": {
            const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
            const toolCallId =
              String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
            if (!toolCallId) {
              debugEvents.push(ev);
              break;
            }
            const tool = ensureTool(g, toolCallId, ev.created_at);
            tool.updated_at = ev.created_at;
            tool.updates_seen += 1;
            tool.raw = ev.payload_json;

            const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
            if (nextKind) tool.tool_kind = nextKind;

            const nextTitle = String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
            if (nextTitle) tool.title = nextTitle;
            else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

            const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
            if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
            else if (ev.event_type === "tool_result") tool.status = "completed";

            const locs = Array.isArray(update?.locations) ? update.locations : [];
            tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

            const rawInput =
              update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? null;
            if (rawInput != null) tool.input = rawInput;

            const output =
              update?.outputText ??
              update?.output_text ??
              update?.toolCall?.outputText ??
              update?.toolCall?.output_text ??
              update?.result ??
              null;
            if (typeof output === "string") tool.output_text = output;

            break;
          }
          default: {
            break;
          }
        }
      }

      const items: ThreadItem[] = [];
      items.push(...g.toolItems);
      if (g.assistant?.thought.trim()) {
        items.push({
          kind: "thought",
          id: `thought-${g.key}`,
          turn_id: g.key,
          created_at: g.thought_first_at ?? g.assistant_first_at ?? g.first_at,
          content: g.assistant.thought,
        });
      }
      if (g.assistant) {
        items.push({
          ...g.assistant,
          thought_seconds: (() => {
            if (!g.thought_first_at) return undefined;
            const start = Date.parse(g.thought_first_at);
            const endRaw = g.assistant_first_at ?? g.assistant_complete_at ?? g.thought_last_at;
            if (!endRaw) return undefined;
            const end = Date.parse(endRaw);
            if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 1;
            return Math.max(1, Math.round((end - start) / 1000));
          })(),
        });
      }
      if (items.length === 0) {
        items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
      }

      groups.push({ key: g.key, header: g.header, items });
    }

    return { groups, debugEvents };
  }

  const eventsInRange = (startIso: string, endIso: string | null) => {
    const start = Date.parse(startIso);
    const end = endIso ? Date.parse(endIso) : Number.POSITIVE_INFINITY;
    return events.filter((e) => {
      const t = Date.parse(String(e.created_at));
      return Number.isFinite(t) && t >= start && t <= end;
    });
  };

  const groups: WorkbenchThreadView["groups"] = [];

  for (let i = 0; i < userMessages.length; i++) {
    const u = userMessages[i];
    const nextUser = userMessages[i + 1] ?? null;

    const mid = idToString(u.id) || `msg-${u.created_at}`;
    const g: TurnGroup = {
      key: `m-${mid}`,
      header: {
        id: mid,
        content: u.content ?? "",
        plain_text: u.content ?? "",
        attachments: Array.isArray((u as any).attachments) ? ((u as any).attachments as MessageAttachment[]) : [],
        created_at: u.created_at,
      },
      first_at: u.created_at,
      toolItems: [],
      toolById: new Map(),
      assistant: null,
      thought_first_at: null,
      thought_last_at: null,
      assistant_first_at: null,
      assistant_complete_at: null,
    };

    const assistant = assistantMessages.find((a) => {
      const ta = Date.parse(String(a.created_at));
      const tu = Date.parse(String(u.created_at));
      if (!Number.isFinite(ta) || !Number.isFinite(tu) || ta <= tu) return false;
      if (!nextUser) return true;
      const tn = Date.parse(String(nextUser.created_at));
      return !Number.isFinite(tn) || ta < tn;
    });

    const endAt = assistant?.created_at ?? nextUser?.created_at ?? null;
    const evs = eventsInRange(u.created_at, endAt);

    for (const ev of evs) {
      const eventId = idToString(ev.id) || `${ev.created_at}`;
      if (ev.created_at < g.first_at) g.first_at = ev.created_at;

      switch (ev.event_type) {
        case "error": {
          const message = String(ev.payload_json?.message ?? "Error");
          const provider = String(ev.payload_json?.provider ?? "").trim();
          const output = provider ? `${message}\nprovider: ${provider}` : message;
          const tool = ensureTool(g, `error-${eventId}`, ev.created_at);
          tool.tool_kind = "error";
          tool.title = "Error";
          tool.status = "failed";
          tool.updated_at = ev.created_at;
          tool.updates_seen += 1;
          tool.input = ev.payload_json ?? null;
          tool.output_text = output;
          tool.raw = ev;
          break;
        }
        case "assistant_chunk": {
          const fragment = String(ev.payload_json?.content_fragment ?? "");
          if (!fragment) break;
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              turn_id: g.key,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.assistant_first_at = g.assistant_first_at ?? ev.created_at;
          g.assistant.content += fragment;
          break;
        }
        case "assistant_complete": {
          const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              turn_id: g.key,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.assistant_complete_at = ev.created_at;
          if (full) g.assistant.content = full;
          g.assistant.is_complete = true;
          break;
        }
          case "thought_chunk": {
            if (!shouldRenderThoughtChunk(ev)) break;
            const fragment = String(ev.payload_json?.content_fragment ?? "");
            if (!fragment) break;
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
              id: `assistant-${g.key}`,
              turn_id: g.key,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.thought_first_at = g.thought_first_at ?? ev.created_at;
          g.thought_last_at = ev.created_at;
          g.assistant.thought += fragment;
          break;
        }
        case "tool_call":
        case "tool_call_update":
        case "tool_result": {
          const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
          const toolCallId =
            String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
          if (!toolCallId) {
            debugEvents.push(ev);
            break;
          }
          const tool = ensureTool(g, toolCallId, ev.created_at);
          tool.updated_at = ev.created_at;
          tool.updates_seen += 1;
          tool.raw = ev.payload_json;

          const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
          if (nextKind) tool.tool_kind = nextKind;

          const nextTitle = String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
          if (nextTitle) tool.title = nextTitle;
          else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

          const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
          if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
          else if (ev.event_type === "tool_result") tool.status = "completed";

          const locs = Array.isArray(update?.locations) ? update.locations : [];
          tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

          const rawInput =
            update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? null;
          if (rawInput != null) tool.input = rawInput;

          const output =
            update?.outputText ??
            update?.output_text ??
            update?.toolCall?.outputText ??
            update?.toolCall?.output_text ??
            update?.result ??
            null;
          if (typeof output === "string") tool.output_text = output;

          break;
        }
        default: {
          break;
        }
      }
    }

    if (!g.assistant && assistant) {
      g.assistant = {
        kind: "assistant",
        id: `assistant-${g.key}`,
        turn_id: g.key,
        created_at: assistant.created_at,
        content: assistant.content ?? "",
        thought: "",
        is_complete: true,
      };
      g.assistant_complete_at = assistant.created_at;
    } else if (g.assistant && assistant && !g.assistant.content) {
      g.assistant.content = assistant.content ?? "";
      g.assistant.is_complete = true;
      g.assistant_complete_at = assistant.created_at;
    }

    const items: ThreadItem[] = [];
    items.push(...g.toolItems);
    if (g.assistant?.thought.trim()) {
      items.push({
        kind: "thought",
        id: `thought-${g.key}`,
        turn_id: g.key,
        created_at: g.thought_first_at ?? g.assistant_first_at ?? g.first_at,
        content: g.assistant.thought,
      });
    }
    if (g.assistant) {
      items.push({
        ...g.assistant,
        thought_seconds: (() => {
          if (!g.thought_first_at) return undefined;
          const start = Date.parse(g.thought_first_at);
          const endRaw = g.assistant_first_at ?? g.assistant_complete_at ?? g.thought_last_at;
          if (!endRaw) return undefined;
          const end = Date.parse(endRaw);
          if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 1;
          return Math.max(1, Math.round((end - start) / 1000));
        })(),
      });
    }
    if (items.length === 0) {
      items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
    }

    groups.push({ key: g.key, header: g.header, items });
  }

  // If there are no user messages (should be rare), fall back to an event-only group.
  if (groups.length === 0) {
    const g: TurnGroup = {
      key: "no-user-messages",
      header: null,
      first_at: events[0]?.created_at ?? new Date().toISOString(),
      toolItems: [],
      toolById: new Map(),
      assistant: null,
      thought_first_at: null,
      thought_last_at: null,
      assistant_first_at: null,
      assistant_complete_at: null,
    };
    for (const ev of events) {
      const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
      const toolCallId =
        String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
      if (!toolCallId) continue;
      ensureTool(g, toolCallId, ev.created_at);
    }
    const items: ThreadItem[] = [...g.toolItems];
    if (items.length === 0) items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
    groups.push({ key: g.key, header: g.header, items });
  }

  return { groups, debugEvents };
}

function buildThreadViewModel(events: SessionEvent[]): {
  items: ThreadItem[];
  debugEvents: SessionEvent[];
} {
  const items: ThreadItem[] = [];
  const debugEvents: SessionEvent[] = [];

  const assistantByTurn = new Map<string, Extract<ThreadItem, { kind: "assistant" }>>();
  const toolById = new Map<string, Extract<ThreadItem, { kind: "tool" }>>();

  const upsertAssistant = (turnId: string, createdAt: string) => {
    const existing = assistantByTurn.get(turnId);
    if (existing) return existing;
    const item: Extract<ThreadItem, { kind: "assistant" }> = {
      kind: "assistant",
      id: `assistant-${turnId}`,
      turn_id: turnId,
      created_at: createdAt,
      content: "",
      thought: "",
      is_complete: false,
    };
    assistantByTurn.set(turnId, item);
    items.push(item);
    return item;
  };

  const upsertTool = (toolCallId: string, createdAt: string) => {
    const existing = toolById.get(toolCallId);
    if (existing) return existing;
    const item: Extract<ThreadItem, { kind: "tool" }> = {
      kind: "tool",
      id: `tool-${toolCallId}`,
      tool_call_id: toolCallId,
      created_at: createdAt,
      updated_at: createdAt,
      tool_kind: "tool",
      title: "Tool",
      status: "pending",
      locations: [],
      input: null,
      output_text: "",
      raw: null,
      updates_seen: 0,
      has_details: true,
    };
    toolById.set(toolCallId, item);
    items.push(item);
    return item;
  };

  for (const ev of events) {
    const id = idToString(ev.id) || `${ev.created_at}`;
    const turnId = idToString((ev as any).turn_id) || "no-turn";

    switch (ev.event_type) {
      case "user_message": {
        items.push({
          kind: "message",
          id,
          role: "user",
          content: ev.payload_json?.content ?? "",
          attachments: Array.isArray(ev.payload_json?.attachments)
            ? (ev.payload_json.attachments as MessageAttachment[])
            : [],
          created_at: ev.created_at,
        });
        break;
      }
      case "error": {
        const message = String(ev.payload_json?.message ?? "Error");
        const provider = String(ev.payload_json?.provider ?? "").trim();
        const output = provider ? `${message}\nprovider: ${provider}` : message;
        items.push({
          kind: "tool",
          id: `error-${id}`,
          tool_call_id: `error-${id}`,
          created_at: ev.created_at,
          updated_at: ev.created_at,
          tool_kind: "error",
          title: "Error",
          status: "failed",
          locations: [],
          input: ev.payload_json ?? null,
          output_text: output,
          raw: ev,
          updates_seen: 1,
          has_details: true,
        });
        break;
      }
      case "assistant_chunk": {
        const fragment = String(ev.payload_json?.content_fragment ?? "");
        if (!fragment) break;
        const item = upsertAssistant(turnId, ev.created_at);
        item.content += fragment;
        break;
      }
      case "assistant_complete": {
        const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
        const item = upsertAssistant(turnId, ev.created_at);
        // Place completed assistant responses after any preceding tool activity.
        item.created_at = ev.created_at;
        if (full) item.content = full;
        item.is_complete = true;
        break;
      }
      case "thought_chunk": {
        if (!shouldRenderThoughtChunk(ev)) break;
        const fragment = String(ev.payload_json?.content_fragment ?? "");
        if (!fragment) break;
        const item = upsertAssistant(turnId, ev.created_at);
        item.thought += fragment;
        break;
      }
      case "tool_call":
      case "tool_call_update":
      case "tool_result": {
        const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
        const toolCallId =
          String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
        if (!toolCallId) {
          debugEvents.push(ev);
          break;
        }
        const tool = upsertTool(toolCallId, ev.created_at);
        tool.updated_at = ev.created_at;
        tool.updates_seen += 1;
        tool.raw = ev.payload_json;

        const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
        if (nextKind) tool.tool_kind = nextKind;

        const nextTitle =
          String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
        if (nextTitle) tool.title = nextTitle;
        else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

        const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
        if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
        else if (ev.event_type === "tool_result") tool.status = "completed";

        const locs = Array.isArray(update?.locations) ? update.locations : [];
        tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

        const input = update?.rawInput ?? update?.toolCall?.input ?? update?.args ?? null;
        if (input) tool.input = input;

        const nextOutput = extractToolOutputText(update);
        if (nextOutput) {
          tool.output_text = mergeStreamingText(tool.output_text, nextOutput);
        }
        break;
      }
      case "plan":
        break;
      case "init":
      case "notice":
      case "auth_required":
      case "done":
      case "interrupt_requested":
      case "turn_interrupted":
      case "input_queued":
        debugEvents.push(ev);
        break;
      default:
        break;
    }
  }

  const itemRank = (it: ThreadItem): number => {
    if (it.kind === "message") return 0;
    if (it.kind === "tool") return 1;
    return 2; // assistant
  };

  items.sort((a, b) => {
    const ta = a.created_at;
    const tb = b.created_at;
    const tcmp = String(ta).localeCompare(String(tb));
    if (tcmp !== 0) return tcmp;
    const rcmp = itemRank(a) - itemRank(b);
    if (rcmp !== 0) return rcmp;
    return String(a.id).localeCompare(String(b.id));
  });

  return { items, debugEvents };
}

function extractToolOutputText(update: any): string {
  const raw = update?.rawOutput?.aggregated_output ?? update?.rawOutput?.output ?? null;
  if (typeof raw === "string" && raw.trim()) return raw;

  const blocks = Array.isArray(update?.content) ? update.content : [];
  const parts: string[] = [];
  for (const b of blocks) {
    const c = b?.content ?? b;
    const t = c?.text;
    if (typeof t === "string") parts.push(t);
  }
  return parts.join("").trim();
}

function mergeStreamingText(prev: string, next: string): string {
  const p = prev ?? "";
  const n = next ?? "";
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
}

function normalizeToolStatus(status: string, eventType: string): string {
  const s = status.toLowerCase();
  if (s === "inprogress") return "in_progress";
  if (s === "in_progress") return "in_progress";
  if (s === "running") return "in_progress";
  if (s === "pending" || s === "queued") return "pending";
  if (s === "completed" || s === "complete" || s === "ok" || s === "succeeded") return "completed";
  if (s === "failed" || s === "error") return "failed";
  if (eventType === "tool_result") return "completed";
  return s || "pending";
}

function humanToolStatus(status: string): string {
  switch (status) {
    case "pending":
      return "Pending";
    case "in_progress":
      return "Running";
    case "completed":
      return "Done";
    case "failed":
      return "Failed";
    default:
      return status || "Unknown";
  }
}

function humanToolKind(kind: string): string {
  const k = (kind || "").toLowerCase();
  if (k === "execute") return "Run Command";
  if (k === "search") return "Search";
  if (k === "read") return "Read File";
  if (k === "edit" || k === "write") return "Edit File";
  if (k === "fetch") return "Fetch";
  if (k === "think") return "Think";
  if (k === "error") return "Error";
  return kind || "Tool";
}

function toolKindIcon(kind: string): string {
  const k = (kind || "").toLowerCase();
  if (k === "execute") return "⌘";
  if (k === "search") return "⌕";
  if (k === "read") return "⟲";
  if (k === "edit" || k === "write") return "✎";
  if (k === "fetch") return "⇣";
  if (k === "think") return "…";
  if (k === "error") return "!";
  return "▦";
}

function formatToolInput(toolKind: string, input: any): string {
  const k = (toolKind || "").toLowerCase();
  if (k === "execute") {
    const cmd = Array.isArray(input?.command) ? input.command.join(" ") : input?.command;
    const cwd = input?.cwd;
    const out: string[] = [];
    if (cwd) out.push(`cwd: ${cwd}`);
    if (cmd) out.push(`cmd: ${cmd}`);
    return out.join("\n") || JSON.stringify(input, null, 2);
  }
  if (typeof input === "string") return input;
  return JSON.stringify(input, null, 2);
}

function toolSummaryLine(toolKind: string, input: any): string {
  const k = (toolKind || "").toLowerCase();
  if (k === "execute") {
    const cmd = Array.isArray(input?.command) ? input.command.join(" ") : input?.command;
    return cmd ? truncateMiddle(String(cmd), 120) : "";
  }
  if (k === "search") {
    const q = input?.query ?? input?.pattern ?? input?.text;
    return q ? truncateMiddle(String(q), 120) : "";
  }
  if (k === "read" || k === "edit" || k === "write") {
    const path = input?.path ?? input?.file ?? input?.filename;
    return path ? truncateMiddle(String(path), 120) : "";
  }
  return "";
}

function truncateMiddle(text: string, maxLen: number): string {
  const s = String(text ?? "");
  if (s.length <= maxLen) return s;
  const head = Math.max(10, Math.floor(maxLen * 0.6));
  const tail = Math.max(10, maxLen - head - 3);
  return `${s.slice(0, head)}...${s.slice(-tail)}`;
}

function looksLikeMarkdown(text: string): boolean {
  const t = String(text ?? "");
  if (t.includes("```")) return true;
  if (/^#{1,6}\s/m.test(t)) return true;
  if (/^\s*[-*]\s+/m.test(t)) return true;
  if (/\[[^\]]+\]\([^)]+\)/.test(t)) return true;
  return false;
}

function mergeEvents(prev: SessionEvent[], incoming: SessionEvent[]): SessionEvent[] {
  const map = new Map<string, SessionEvent>();
  for (const ev of prev) {
    const key = idToString(ev.id) || `${ev.created_at}-${ev.event_type}`;
    map.set(key, ev);
  }
  for (const ev of incoming) {
    const key = idToString(ev.id) || `${ev.created_at}-${ev.event_type}`;
    map.set(key, ev);
  }
  return [...map.values()].sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
}

type AuthMethodOption = { id: string; name: string };

type AuthUi = {
  status: "unknown" | "required" | "failed" | "authenticated";
  provider?: string;
  message?: string;
  methods: AuthMethodOption[];
};

function deriveAuthUi(events: SessionEvent[]): AuthUi {
  const fromMethodsValue = (value: any): AuthMethodOption[] => {
    const list = Array.isArray(value) ? value : [];
    return list
      .map((m: any) => ({
        id: m?.methodId ?? m?.method_id ?? m?.id,
        name: m?.name ?? m?.label ?? (m?.methodId ?? m?.method_id ?? m?.id),
      }))
      .filter((m: any) => typeof m.id === "string" && m.id.length > 0)
      .map((m: any) => ({ id: String(m.id), name: String(m.name ?? m.id) }));
  };

  let status: AuthUi["status"] = "unknown";
  let provider: string | undefined;
  let message: string | undefined;
  let methods: AuthMethodOption[] = [];

  const lastInit = [...events].reverse().find((e) => e.event_type === "init");
  const initMethods =
    lastInit?.payload_json?.auth_methods ??
    lastInit?.payload_json?.authMethods ??
    lastInit?.payload_json?.auth_methods;
  const initMethodOptions = fromMethodsValue(initMethods);

  for (const ev of events) {
    if (ev.event_type === "auth_required") {
      status = "required";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
      methods = fromMethodsValue(ev.payload_json?.auth_methods ?? ev.payload_json?.authMethods);
      continue;
    }

    if (ev.event_type !== "notice") continue;
    const kind = ev.payload_json?.kind;
    if (kind === "auth_required") {
      status = "required";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
      methods = fromMethodsValue(ev.payload_json?.auth_methods ?? ev.payload_json?.authMethods);
    }
    if (kind === "auth_failed") {
      status = "failed";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
    }
    if (kind === "auth_finished") {
      status = "authenticated";
      provider = ev.payload_json?.provider;
      message = undefined;
      methods = [];
    }
  }

  if ((status === "required" || status === "failed") && methods.length === 0) {
    methods = initMethodOptions;
  }

  return { status, provider, message, methods };
}
