import {
  Fragment,
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
import { Virtuoso, VirtuosoHandle, type IndexLocationWithAlign, type StateSnapshot } from "react-virtuoso";
import {
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
import { Check, Copy } from "lucide-react";
import { AskUserQuestionModal } from "../components/AskUserQuestionModal";
import { type SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import {
  WorkbenchComposer as UnifiedWorkbenchComposer,
  type ContextWindowInfo,
  type WorkbenchModeId,
} from "../components/WorkbenchComposer";
import { startMicPcmStream } from "../utils/micPcmStream";
import { parseWsJson } from "../utils/wsJson";
import { imageFilesToInlineAttachments } from "../utils/messageAttachments";
import { registerDropScope } from "../utils/dragDropScopes";
import { copyTextToClipboard } from "../utils/clipboard";
import {
  type FileRef,
  isAbsolutePath,
  parseFileRefToken,
  parseUrlToken,
  splitWhitespaceTokens,
} from "../utils/codeTokenLinks";
import { desktopOpenFile, desktopOpenPath, isDesktopApp } from "../utils/desktop";
import { usePinnedScrollManager } from "./usePinnedScrollManager";

type ThreadItem =
  | {
    kind: "message";
    id: string;
    role: "user" | "assistant" | "system";
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
    kind: "turn_status";
    id: string;
    turn_id: string;
    created_at: string;
    status: SessionTurn["status"];
    started_at: string;
    updated_at: string;
    custom_status?: string | null;
    status_text?: string | null;
    assistant_messages_content?: string;
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

function appendFragment(base: string, fragment: string): string {
  const b = base ?? "";
  const f = fragment ?? "";
  if (!b) return f;
  if (!f) return b;
  if (f.startsWith(b)) return f;
  if (b.endsWith(f)) return b;
  return `${b}${f}`;
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

function buildCustomStatusByTurnId(events: SessionEvent[]): Map<string, string> {
  const normalize = (value: unknown): string | null => {
    const t = String(value ?? "").trim();
    return t ? t : null;
  };

  const extractNoticeStatusText = (ev: SessionEvent): string | null => {
    if (ev.event_type !== "notice") return null;
    const payload = ev.payload_json ?? {};
    const meta =
      payload?.acp_update?._meta ??
      payload?.acp_update?.meta ??
      payload?._meta ??
      payload?.meta ??
      {};
    return (
      normalize(meta?.statusText) ??
      normalize(meta?.status_text) ??
      normalize(payload?.statusText) ??
      normalize(payload?.status_text)
    );
  };

  const toolStatusVerb = (kind: string): string | null => {
    const k = String(kind ?? "").trim().toLowerCase();
    if (k === "search") return "Searching";
    if (k === "read" || k === "read_file") return "Reading";
    if (k === "execute") return "Running";
    if (k === "write" || k === "edit") return "Writing";
    return null;
  };

  const deriveToolStatusText = (update: any): string | null => {
    const kind = String(update?.kind ?? update?.tool_kind ?? update?.toolKind ?? "").trim();
    const verb = toolStatusVerb(kind);
    if (!verb) return null;
    if (verb === "Searching") {
      const query = normalize(update?.input?.query ?? update?.input?.q);
      if (query) return `${verb} ${query}`;
    }
    const title = normalize(update?.title);
    if (title) return `${verb} ${title}`;
    return verb;
  };

  const isActiveToolStatus = (status: unknown): boolean => {
    const s = String(status ?? "").trim().toLowerCase();
    return s === "pending" || s === "queued" || s === "running" || s === "in_progress" || s === "inprogress";
  };

  const sorted = events
    .slice()
    .sort((a, b) => {
      const sa = typeof (a as any).seq === "number" ? ((a as any).seq as number) : Number.NaN;
      const sb = typeof (b as any).seq === "number" ? ((b as any).seq as number) : Number.NaN;
      if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
      if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
      if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
      return String(a.created_at).localeCompare(String(b.created_at));
    });

  const noticeByTurn = new Map<string, { order: number; text: string }>();
  const toolsByTurn = new Map<string, Map<string, { order: number; status: string; text: string | null }>>();

  let order = 0;
  for (const ev of sorted) {
    order += 1;
    const turnId = idToString((ev as any).turn_id);
    if (!turnId) continue;

    const noticeText = extractNoticeStatusText(ev);
    if (noticeText) {
      noticeByTurn.set(turnId, { order, text: noticeText });
      continue;
    }

    if (ev.event_type !== "tool_call" && ev.event_type !== "tool_call_update" && ev.event_type !== "tool_result") {
      continue;
    }

    const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
    const toolCallId =
      normalize(
        ev.payload_json?.tool_call_id ??
          update?.toolCallId ??
          update?.tool_call_id ??
          update?.rawInput?.call_id ??
          update?.raw_input?.call_id ??
          update?.toolCall?.rawInput?.call_id ??
          "",
      ) ?? "";
    if (!toolCallId) continue;

    const status = String(update?.status ?? update?.tool_status ?? update?.toolStatus ?? "").trim();
    const text = deriveToolStatusText(update);
    const perTurn = toolsByTurn.get(turnId) ?? new Map<string, { order: number; status: string; text: string | null }>();
    perTurn.set(toolCallId, { order, status, text });
    toolsByTurn.set(turnId, perTurn);
  }

  const out = new Map<string, string>();
  const allTurnIds = new Set<string>([...noticeByTurn.keys(), ...toolsByTurn.keys()]);
  for (const turnId of allTurnIds) {
    const perTurnTools = toolsByTurn.get(turnId);
    let bestTool: { order: number; text: string } | null = null;
    if (perTurnTools) {
      for (const tool of perTurnTools.values()) {
        if (!tool.text) continue;
        if (!isActiveToolStatus(tool.status)) continue;
        if (!bestTool || tool.order > bestTool.order) bestTool = { order: tool.order, text: tool.text };
      }
    }
    if (bestTool) {
      out.set(turnId, bestTool.text);
      continue;
    }
    const notice = noticeByTurn.get(turnId);
    if (notice?.text) out.set(turnId, notice.text);
  }

  return out;
}


function parseIsoMs(value?: string | null): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function coerceNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number.parseFloat(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function normalizeContextWindowMetrics(metrics: any): ContextWindowInfo | null {
  if (!metrics || typeof metrics !== "object") return null;
  const windowTokens =
    coerceNumber(metrics.context_window_tokens) ??
    coerceNumber(metrics.context_window_size) ??
    coerceNumber(metrics.context_window) ??
    coerceNumber(metrics.context_size) ??
    coerceNumber(metrics.window_tokens) ??
    coerceNumber(metrics.max_context_tokens) ??
    coerceNumber(metrics.max_tokens);
  if (!windowTokens || windowTokens <= 0) return null;

  const inputTokens =
    coerceNumber(metrics.total_input_tokens) ??
    coerceNumber(metrics.input_tokens) ??
    coerceNumber(metrics.prompt_tokens) ??
    coerceNumber(metrics.input);
  const outputTokens =
    coerceNumber(metrics.total_output_tokens) ??
    coerceNumber(metrics.output_tokens) ??
    coerceNumber(metrics.completion_tokens) ??
    coerceNumber(metrics.output);
  const contextTokensEstimate =
    coerceNumber(metrics.context_tokens_estimate) ??
    coerceNumber(metrics.context_tokens);

  let usedTokens: number | null = null;
  if (inputTokens != null || outputTokens != null) {
    usedTokens = (inputTokens ?? 0) + (outputTokens ?? 0);
  } else if (contextTokensEstimate != null) {
    usedTokens = contextTokensEstimate;
  }

  let remainingTokens =
    coerceNumber(metrics.remaining_tokens_estimate) ??
    coerceNumber(metrics.remaining_tokens);
  let remainingFraction =
    coerceNumber(metrics.remaining_fraction) ??
    coerceNumber(metrics.remaining_pct) ??
    coerceNumber(metrics.remaining_percent);

  if (remainingFraction != null && remainingFraction > 1) {
    remainingFraction = remainingFraction <= 100 ? remainingFraction / 100 : null;
  }
  if (remainingFraction != null) {
    remainingFraction = Math.max(0, Math.min(1, remainingFraction));
  }

  if (remainingTokens == null && usedTokens != null) {
    remainingTokens = Math.max(0, windowTokens - usedTokens);
  }
  if (usedTokens == null && remainingTokens != null) {
    usedTokens = Math.max(0, windowTokens - remainingTokens);
  }
  if (remainingFraction == null && usedTokens != null) {
    remainingFraction = Math.max(0, Math.min(1, 1 - usedTokens / windowTokens));
  }
  if (usedTokens == null && remainingFraction != null) {
    usedTokens = Math.max(0, Math.round(windowTokens * (1 - remainingFraction)));
  }

  return {
    windowTokens,
    usedTokens: usedTokens ?? undefined,
    remainingTokens: remainingTokens ?? undefined,
    remainingFraction: remainingFraction ?? undefined,
  };
}

function formatElapsedMs(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);

  if (hours > 0) {
    return `${hours}h ${String(minutes).padStart(2, "0")}m`;
  }
  if (minutes > 0) {
    return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
  }
  return `${seconds}s`;
}

function humanTurnStatus(status: SessionTurn["status"]): string {
  switch (status) {
    case "completed":
      return "Completed";
    case "interrupted":
      return "Interrupted";
    case "failed":
      return "Error";
    case "queued":
    case "running":
    default:
      return "Working";
  }
}

function humanToolStatus(status: string): string {
  const s = String(status ?? "").trim().toLowerCase();
  switch (s) {
    case "pending":
    case "queued":
      return "Queued";
    case "running":
    case "in_progress":
    case "inprogress":
      return "Running";
    case "completed":
    case "complete":
    case "ok":
    case "success":
    case "succeeded":
      return "Completed";
    case "failed":
    case "error":
      return "Failed";
    default:
      return status ? String(status) : "";
  }
}

function toolKindIcon(kind: string): string {
  const k = String(kind ?? "").trim().toLowerCase();
  if (k === "execute") return "$";
  if (k === "read" || k === "read_file") return "R";
  if (k === "search" || k === "list" || k === "list_files") return "S";
  if (k === "write" || k === "edit" || k === "apply_patch") return "W";
  return "·";
}

type WorkbenchThreadView = {
  groups: Array<{
    key: string;
    header: WorkbenchTurnHeader | null;
    items: ThreadItem[];
  }>;
  debugEvents: SessionEvent[];
};

export function SessionView({
  sessionId,
  isActive = true,
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
}) {
  const id = sessionId;
  const supervisor = useSessionSupervisor();
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
  const [fileOpenError, setFileOpenError] = useState<string | null>(null);
  const [modifierDown, setModifierDown] = useState(false);
  const [atBottom, setAtBottom] = useState(true);
  const [stickToBottom, setStickToBottom] = useState(true);
  const stickToBottomRef = useRef(true);
  const [authMethodId, setAuthMethodId] = useState<string>("");
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [optimisticAskAnswered, setOptimisticAskAnswered] = useState<Record<string, boolean>>({});
  const [expandedTurnHeaders, setExpandedTurnHeaders] = useState<Record<string, boolean>>({});
  const [expandedTurnDetailsById, setExpandedTurnDetailsById] = useState<Record<string, boolean>>({});
  const [expandedToolById, setExpandedToolById] = useState<Record<string, boolean>>({});
  const virtuosoRef = useRef<VirtuosoHandle | null>(null);
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
  const [nowMs, setNowMs] = useState(() => Date.now());

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

  useOpenSession(id ?? "", { watchDiff: true });
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
    setOptimisticAskAnswered({});
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
  const worktreeId = session ? idToString(session.worktree_id) : null;
  const turns = entry?.turns ?? [];
  const turnToolsByTurnId = entry?.turnToolsByTurnId ?? {};
  const turnToolsLoading = entry?.turnToolsLoading ?? [];
  const toolSummariesReady = entry?.toolSummariesReady ?? false;
  const hasMoreTurns = entry?.hasMoreTurns ?? false;
  const events: SessionEvent[] = entry?.events ?? [];
  const messages: Message[] = entry?.messages ?? [];
  const queue: Message[] = entry?.queue ?? [];
  const eventsKey = `${entry?.lastEventSeq ?? 0}:${events.length}`;
  const turnsKey = deriveTurnsKey(turns);
  const messagesKey = deriveMessagesKey(messages);
  const contextWindow = useMemo<ContextWindowInfo | null>(() => {
    let latestMetrics: any = null;
    let latestAt = -1;
    for (const turn of turns) {
      if (!turn.metrics_json) continue;
      const updatedAt = parseIsoMs(turn.updated_at) ?? 0;
      if (updatedAt >= latestAt) {
        latestAt = updatedAt;
        latestMetrics = turn.metrics_json;
      }
    }
    return normalizeContextWindowMetrics(latestMetrics);
  }, [turnsKey]);
  const hasActiveTurn = useMemo(
    () => turns.some((turn) => turn.status === "running" || turn.status === "queued"),
    [turnsKey],
  );
  const sessionError = useMemo(
    () => deriveSessionError(turns, events),
    [turnsKey, eventsKey],
  );

  useEffect(() => {
    if (!hasActiveTurn) return;
    setNowMs(Date.now());
    const timer = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(timer);
  }, [hasActiveTurn]);

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
        return sessionStorage.getItem("ctxAuthToken");
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
  const virtuosoPersistTimerRef = useRef<number | null>(null);
  const scrollSyncKey = useMemo(
    () => `${eventsKey}:${messagesKey}:${turnsKey}`,
    [eventsKey, messagesKey, turnsKey],
  );
  const [restoreInProgress, setRestoreInProgress] = useState(false);
  const { initialTopMostItemIndex, markAutoScroll, scheduleAutoScroll } = usePinnedScrollManager({
    isActive,
    preserveScrollOnFocus,
    bottomThresholdPx,
    userIntentWindowMs,
    itemsLength: wbListItems.length,
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
    if (isActive && !wasActiveRef.current) {
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
  }, [isActive, preserveScrollOnFocus, scrollState?.stickToBottom, scrollState?.virtuosoState]);

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
        handle?.scrollTo({ top: target });
        if (el) el.scrollTop = target;
        if (el && Math.abs(el.scrollTop - target) > 2 && restoreRetryRef.current < 3) {
          restoreRetryRef.current += 1;
          requestAnimationFrame(attemptRestore);
          return;
        }
      } else if (restoreAnchorId) {
        const idx = items.findIndex((it) => it?.id === restoreAnchorId);
        if (idx >= 0) {
          markAutoScroll();
          handle?.scrollToIndex({ index: idx, align: "start" });
        } else {
          markAutoScroll();
          handle?.scrollToIndex({ index: items.length - 1, align: "end" });
        }
      } else {
        markAutoScroll();
        handle?.scrollToIndex({ index: items.length - 1, align: "end" });
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
    wbListItems.length,
  ]);

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
    preserveScrollOnFocus || scrollState?.stickToBottom !== false
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
      const item = wbListItems[range.startIndex];
      if (!item) return;
      latestAnchorIdRef.current = item.id ?? null;
    },
    [atBottom, wbListItems],
  );

  const handleStartReached = useCallback(() => {
    if (!hasMoreTurns) return;
    supervisor.loadMoreTurns(id);
  }, [hasMoreTurns, id, supervisor]);


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
      stickToBottomRef.current = true;
      setStickToBottom(true);
      liveScrollTopRef.current = null;
      pendingScrollToBottomRef.current = true;
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

  const onRemoveQueued = async (messageId: string) => {
    await deleteMessage(messageId);
    supervisor.refreshQueue(id ?? "");
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
      return <WorkbenchTurnStatusRow item={item} nowMs={nowMs} />;
    }
    if (item.kind === "assistant") {
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
    return (
      <ThreadItemView
        item={item}
        worktreeId={worktreeId}
        onFileOpenError={handleFileOpenError}
        modifierDown={modifierDown}
      />
    );
  }, [
    expandedToolById,
    expandedTurnDetailsById,
    handleFileOpenError,
    modifierDown,
    id,
    nowMs,
    supervisor,
    nowMs,
    turnToolsLoading,
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

  const workbenchComponents = useMemo(
    () => ({
      Scroller: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
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
            else if (ref) (ref as React.MutableRefObject<HTMLDivElement | null>).current = node;
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
      List: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div
          {...props}
          ref={(node) => {
            listRef.current = node;
            if (typeof ref === "function") ref(node);
            else if (ref) (ref as React.MutableRefObject<HTMLDivElement | null>).current = node;
          }}
          role="list"
          className={`wb-thread-list ${props.className ?? ""}`}
        />
      )),
      Footer: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
        <div
          {...props}
          ref={(node) => {
            bottomSentinelRef.current = node;
            setBottomSentinelNode((prev) => (prev === node ? prev : node));
            if (typeof ref === "function") ref(node);
            else if (ref) (ref as React.MutableRefObject<HTMLDivElement | null>).current = node;
          }}
          aria-hidden="true"
          data-bottom-sentinel="true"
          style={{ height: 1, minHeight: 1, ...props.style }}
        />
      )),
      Item: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
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

        <WorkbenchThreadStack
          virtuosoStyle={virtuosoStyle}
          data={wbListItems}
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

        <div className="sr-only" aria-live="polite">
          {session && (atBottom ? "Agent output updating." : "New agent activity.")}
        </div>
      </div>

    </div>
  );
}

type WorkbenchThreadStackProps = {
  virtuosoStyle: CSSProperties;
  data: WorkbenchListItem[];
  virtuosoRef: React.MutableRefObject<VirtuosoHandle | null>;
  initialTopMostItemIndex?: IndexLocationWithAlign | number;
  restoreStateFrom?: StateSnapshot;
  increaseViewportBy: { top: number; bottom: number };
  onStartReached: () => void;
  onRangeChanged: (range: any) => void;
  components: any;
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  showJumpToLatest: boolean;
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
  initialTopMostItemIndex,
  restoreStateFrom,
  increaseViewportBy,
  onStartReached,
  onRangeChanged,
  components,
  itemContent,
  showJumpToLatest,
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
        initialTopMostItemIndex={initialTopMostItemIndex}
        defaultItemHeight={56}
        increaseViewportBy={increaseViewportBy}
        computeItemKey={(index, item) => item?.id ?? `i:${index}`}
        restoreStateFrom={restoreStateFrom}
        startReached={onStartReached}
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

      {showJumpToLatest && (
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
  worktreeId,
  onFileOpenError,
  modifierDown,
}: {
  item: ThreadItem;
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  modifierDown: boolean;
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
          modifierDown={modifierDown}
        />
      );
    case "assistant":
    case "tool":
      return null;
    case "tool_group":
    case "turn_status":
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
  const [copied, setCopied] = useState(false);
  const resetTimerRef = useRef<number | null>(null);

  useEffect(() => {
    if (!copied) return;
    if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    resetTimerRef.current = window.setTimeout(() => {
      setCopied(false);
      resetTimerRef.current = null;
    }, 1000);
    return () => {
      if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    };
  }, [copied]);

  const handleCopy = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation();
      const content = header.content ?? "";
      if (!content.trim()) return;
      const ok = await copyTextToClipboard(content);
      if (!ok) return;
      setCopied(true);
    },
    [header.content]
  );

  const handleClick = () => {
    const selection = window.getSelection()?.toString() ?? "";
    if (selection.trim()) return;
    onToggle();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onToggle();
  };

  const hasContent = (header.content ?? "").trim().length > 0;

  return (
    <div
      className={`wb-turn-header ${expanded ? "wb-turn-header-expanded" : "wb-turn-header-collapsed"}`}
      role="button"
      tabIndex={0}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      aria-expanded={expanded}
    >
      <div className="wb-turn-header-bubble">
        {hasContent && (
          <button
            type="button"
            className="wb-turn-header-copy"
            aria-label={copied ? "Copied" : "Copy message"}
            title={copied ? "Copied" : "Copy message"}
            onClick={handleCopy}
          >
            {copied ? <Check size={12} aria-hidden="true" /> : <Copy size={12} aria-hidden="true" />}
          </button>
        )}
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
  modifierDown,
}: {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  attachments: MessageAttachment[];
  delivery?: "immediate" | "queued";
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  modifierDown: boolean;
}) {
  const lines = (content || "").split("\n");
  const isLong = lines.length > 20 || content.length > 1500;
  const [expanded, setExpanded] = useState(!isLong);
  const shown = expanded ? content : lines.slice(0, 20).join("\n");

  return (
    <div className={`msg ${role}`}>
      <div className="role">{role}</div>
      <div id={`msg-${id}`}>
        <div className={modifierDown ? "markdown-modifier" : undefined}>
          <MemoMarkdown
            content={shown}
            linkifyFiles={role === "assistant"}
            worktreeId={worktreeId}
            onFileOpenError={onFileOpenError}
          />
        </div>
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
  content,
  worktreeId,
  onFileOpenError,
  modifierDown,
}: {
  content: string;
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  modifierDown: boolean;
}) {
  return (
    <div className="wb-assistant-entry">
      <div className="wb-assistant-body">
        <div className={modifierDown ? "markdown-modifier" : undefined}>
          <MemoMarkdown
            content={content}
            linkifyFiles
            worktreeId={worktreeId}
            onFileOpenError={onFileOpenError}
          />
        </div>
      </div>
    </div>
  );
}

function WorkbenchToolRow({
  item,
  verbosity,
  expanded,
  onToggle,
}: {
  item: Extract<ThreadItem, { kind: "tool" }>;
  verbosity: SessionViewVerbosity;
  expanded: boolean;
  onToggle: () => void;
}) {
  const kind = String(item.tool_kind ?? "").toLowerCase();
  const pathFromLoc = item.locations?.[0]?.path;
  const title = String(item.title ?? "").trim();
  const summary = toolSummaryLine(kind, item.input);

  const normalizeWorktreePath = (p?: string) => {
    const s = String(p ?? "").trim();
    if (!s) return "";
    // Strip the ctx worktree prefix.
    return s.replace(
      /\/home\/[^/]+\/\.ctx\/worktrees\/[0-9a-f-]+\/[0-9a-f-]+\//g,
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
      /\/home\/[^/]+\/\.ctx\/worktrees\/[0-9a-f-]+\/[0-9a-f-]+\//g,
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
    const normalizedTitle = normalizeWorktreePath(title);
    const titlePrefixed = parsePrefixed(normalizedTitle, [
      "Read",
      "Explored",
      "Searched",
      "Wrote",
      "Edited",
      "Run",
      "Fetch",
      "Search",
      "List",
      "Write",
      "Edit",
    ]);
    const titleRest = titlePrefixed?.rest ?? "";

    if (normalizedTitle && normalizedTitle !== "Tool") {
      if (titlePrefixed) return titlePrefixed;
      return makeParts(normalizedTitle);
    }

    const parsed = Array.isArray((item.input as any)?.parsed_cmd) ? ((item.input as any).parsed_cmd as any[]) : [];
    if (parsed.length > 0) {
      const c0 = parsed[0] ?? {};
      if (c0.type === "list_files" && c0.path) return makeParts("Explored", shortPath(c0.path));
      if (c0.type === "read_file" && c0.path) return makeParts("Read", shortPath(c0.path));
      if (c0.type === "search") {
        const q = String(c0.query ?? c0.pattern ?? c0.regex ?? c0.text ?? "").trim();
        if (q) return makeParts("Searched", truncateMiddle(q, 90));
        if (c0.path) return makeParts("Searched", shortPath(c0.path));
        return makeParts("Searched");
      }
    }
    if (kind === "search") {
      const q = String(
        item.input?.query ??
          item.input?.pattern ??
          item.input?.regex ??
          item.input?.text ??
          summary ??
          titleRest ??
          "",
      ).trim();
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
      const p = pathFromLoc ?? summary ?? titleRest;
      return p ? makeParts("Read", shortPath(p)) : makeParts("Read");
    }
    if (kind === "list" || kind === "list_files") {
      const p = summary || pathFromLoc || titleRest;
      return p ? makeParts("Explored", shortPath(p)) : makeParts("Explored");
    }
    if (kind === "write" || kind === "edit" || kind === "apply_patch") {
      const p = summary || pathFromLoc || titleRest;
      const verb = kind === "write" ? "Wrote" : "Edited";
      return p ? makeParts(verb, shortPath(p)) : makeParts(verb);
    }
    if (kind === "fetch" || kind === "http") {
      const p = summary || titleRest;
      return p ? makeParts("Fetch", p) : makeParts("Fetch");
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
  const showOutputPreview = verbosity === "verbose";
  const hasOutputText = !!item.output_text?.trim();
  const hasDetails = !!item.input || (showOutputPreview && hasOutputText);

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
          {rest ? (
            <>
              <span className="wb-tool-sep" aria-hidden="true">
                ·
              </span>
              <span className="wb-tool-rest">{rest}</span>
            </>
          ) : null}
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
          {showOutputPreview && !!item.output_text?.trim() && (
            <div className="wb-tool-section">
              <div className="wb-tool-section-title">Output</div>
              {looksLikeMarkdown(item.output_text) ? (
                <div className="wb-tool-markdown">
                  <MemoMarkdown content={item.output_text} />
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

function WorkbenchTurnStatusRow({
  item,
  nowMs,
}: {
  item: Extract<ThreadItem, { kind: "turn_status" }>;
  nowMs: number;
}) {
  const isRunning = item.status === "running" || item.status === "queued";
  const isCompleted = item.status === "completed";
  const customStatus = String(item.custom_status ?? item.status_text ?? "").trim();
  const statusLabel = isRunning && customStatus ? customStatus : humanTurnStatus(item.status);
  const startMs = parseIsoMs(item.started_at);
  const endMs = isRunning ? nowMs : parseIsoMs(item.updated_at) ?? nowMs;
  const elapsedMs = startMs != null && endMs != null ? Math.max(0, endMs - startMs) : 0;
  const elapsedLabel = formatElapsedMs(elapsedMs);

  const [copied, setCopied] = useState(false);
  const resetTimerRef = useRef<number | null>(null);

  useEffect(() => {
    if (!copied) return;
    if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    resetTimerRef.current = window.setTimeout(() => {
      setCopied(false);
      resetTimerRef.current = null;
    }, 1000);
    return () => {
      if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    };
  }, [copied]);

  const handleCopy = useCallback(async () => {
    const content = item.assistant_messages_content ?? "";
    if (!content.trim()) return;
    const ok = await copyTextToClipboard(content);
    if (!ok) return;
    setCopied(true);
  }, [item.assistant_messages_content]);

  const hasContent = (item.assistant_messages_content ?? "").trim().length > 0;
  const showCopyButton = isCompleted && hasContent;

  return (
    <div className="wb-turn-status">
      <span className="wb-turn-status-label">{statusLabel}</span>
      <span className="wb-turn-status-dot" aria-hidden="true">
        ·
      </span>
      <span className="wb-turn-status-time">{elapsedLabel}</span>
      {showCopyButton && (
        <>
          <span className="wb-turn-status-dot" aria-hidden="true">
            ·
          </span>
          <button
            type="button"
            className="wb-turn-status-copy"
            aria-label={copied ? "Copied" : "Copy response"}
            title={copied ? "Copied" : "Copy response"}
            onClick={() => void handleCopy()}
          >
            {copied ? <Check size={12} aria-hidden="true" /> : <Copy size={12} aria-hidden="true" />}
          </button>
        </>
      )}
    </div>
  );
}

function WorkbenchToolGroupRow({
  item,
  verbosity,
  expanded,
  onToggle,
  toolsLoading,
  onRequestTools,
  onToggleTool,
  expandedToolById,
}: {
  item: Extract<ThreadItem, { kind: "tool_group" }>;
  verbosity: SessionViewVerbosity;
  expanded: boolean;
  onToggle: () => void;
  toolsLoading: boolean;
  onRequestTools: () => void;
  onToggleTool: (id: string) => void;
  expandedToolById: Record<string, boolean>;
}) {
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
              verbosity={verbosity}
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
  const path = item.locations?.length === 1 ? item.locations[0]?.path : null;
  const subtitleItems: ReactNode[] = [
    <span key="status" className={`pill ${isFailed ? "err" : isRunning ? "run" : "ok"}`}>
      {humanToolStatus(item.status)}
    </span>,
  ];
  if (path) subtitleItems.push(<span key="path" className="muted tool-path">{path}</span>);
  if (summary) subtitleItems.push(<span key="summary" className="muted tool-summary">{summary}</span>);

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
              {subtitleItems.map((node, idx) => (
                <Fragment key={idx}>
                  {idx > 0 && (
                    <span className="tool-status-dot" aria-hidden="true">
                      ·
                    </span>
                  )}
                  {node}
                </Fragment>
              ))}
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
                  <MemoMarkdown content={item.output_text} />
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

type MdastNode = {
  type?: string;
  children?: MdastNode[];
  [key: string]: unknown;
};

type ParsedContextOpen = {
  worktreeId?: string;
  file?: string;
  path?: string;
  line?: number;
  col?: number;
};

function parseContextOpenUrl(href: string): ParsedContextOpen | null {
  try {
    const url = new URL(href);
    if (url.protocol !== "ctx:") return null;
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

type CodeTokenOptions = {
  enableLinks: boolean;
  worktreeId: string | null;
  onFileOpenError?: (message: string | null) => void;
  wrapPlainTokens?: boolean;
};

const handleCodeTokenClick = async (
  event: MouseEvent<HTMLElement>,
  ref: FileRef,
  worktreeId: string | null,
  onFileOpenError?: (message: string | null) => void,
) => {
  if (!event.metaKey && !event.ctrlKey) return;
  if (!isDesktopApp()) return;
  if (!worktreeId && !isAbsolutePath(ref.path)) return;
  event.preventDefault();
  event.stopPropagation();

  try {
    if (isAbsolutePath(ref.path)) {
      await desktopOpenPath({
        path: ref.path,
        line: ref.line ?? null,
        col: ref.col ?? null,
      });
    } else {
      await desktopOpenFile({
        worktree_id: worktreeId ?? "",
        path: ref.path,
        line: ref.line ?? null,
        col: ref.col ?? null,
      });
    }
    onFileOpenError?.(null);
  } catch {
    // Ignore failures to keep interaction silent.
  }
};

const handleUrlTokenClick = (event: MouseEvent<HTMLElement>, href: string) => {
  if (!event.metaKey && !event.ctrlKey) {
    event.preventDefault();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  window.open(href, "_blank", "noopener,noreferrer");
};

const buildCodeTokenNodes = (text: string, opts: CodeTokenOptions): ReactNode[] => {
  const parts = splitWhitespaceTokens(text);
  return parts.map((part, idx) => {
    if (!part) return null;
    if (part.trim() === "") return part;
    if (opts.enableLinks) {
      const urlRef = parseUrlToken(part);
      if (urlRef) {
        return (
          <a
            key={`token-${idx}`}
            className="code-token code-token-url"
            href={urlRef.url}
            rel="noreferrer noopener"
            target="_blank"
            onClick={(event) => handleUrlTokenClick(event, urlRef.url)}
          >
            {part}
          </a>
        );
      }

      const ref = parseFileRefToken(part);
      if (ref && (opts.worktreeId || isAbsolutePath(ref.path))) {
        return (
          <span
            key={`token-${idx}`}
            className="code-token code-token-path"
            onClick={(event) => handleCodeTokenClick(event, ref, opts.worktreeId, opts.onFileOpenError)}
          >
            {part}
          </span>
        );
      }
    }

    if (!opts.wrapPlainTokens) return part;
    return (
      <span key={`token-${idx}`} className="code-token">
        {part}
      </span>
    );
  });
};

function TokenizedInlineCode({
  codeString,
  className,
  enableLinks,
  worktreeId,
  onFileOpenError,
}: {
  codeString: string;
  className?: string;
  enableLinks: boolean;
  worktreeId: string | null;
  onFileOpenError?: (message: string | null) => void;
}) {
  const handleDoubleClick = useCallback((event: MouseEvent<HTMLElement>) => {
    if (event.metaKey || event.ctrlKey) return;
    const selection = window.getSelection();
    if (!selection) return;
    const range = document.createRange();
    range.selectNodeContents(event.currentTarget);
    selection.removeAllRanges();
    selection.addRange(range);
  }, []);

  const content = buildCodeTokenNodes(codeString, {
    enableLinks,
    worktreeId,
    onFileOpenError,
    wrapPlainTokens: true,
  });
  return (
    <code className={className} onDoubleClick={handleDoubleClick}>
      {content}
    </code>
  );
}

function FencedCodeBlock({
  codeString,
  enableLinks,
  worktreeId,
  onFileOpenError,
}: {
  codeString: string;
  enableLinks: boolean;
  worktreeId: string | null;
  onFileOpenError?: (message: string | null) => void;
}) {
  const [copied, setCopied] = useState(false);
  const resetTimerRef = useRef<number | null>(null);

  useEffect(() => {
    if (!copied) return;
    if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    resetTimerRef.current = window.setTimeout(() => {
      setCopied(false);
      resetTimerRef.current = null;
    }, 1000);
    return () => {
      if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    };
  }, [copied]);

  const handleCopy = useCallback(async () => {
    const ok = await copyTextToClipboard(codeString);
    if (!ok) return;

    setCopied(true);
  }, [codeString]);

  const content = enableLinks
    ? buildCodeTokenNodes(codeString, { enableLinks, worktreeId, onFileOpenError })
    : codeString;

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
        <pre className="codeblock-pre">
          <code className="codeblock-code">{content}</code>
        </pre>
      </div>
    </div>
  );
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
}: {
  content: string;
  linkifyFiles?: boolean;
  worktreeId?: string | null;
  onFileOpenError?: (message: string | null) => void;
}) {
  const remarkPlugins: any[] = [remarkGfm, remarkNormalizeCursorMarkdown];

  return (
    <div>
      <ReactMarkdown
        remarkPlugins={remarkPlugins}
        urlTransform={(url) =>
          url.startsWith("ctx://")
            ? url
            : defaultUrlTransform(url)
        }
        components={{
          a({ href, children, className, ...rest }) {
            const isContextOpen =
              typeof href === "string" && href.startsWith("ctx://open?");
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
              if (!event.metaKey && !event.ctrlKey) return;
              event.preventDefault();
              if (!href) return;
              const parsed = parseContextOpenUrl(href);
              if (!parsed) return;
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
                  return;
                }
                onFileOpenError?.(null);
              } catch {
                // Ignore failures to keep interaction silent.
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
            const enableLinks = Boolean(linkifyFiles);
            if (inline || (!lang && !codeString.includes("\n"))) {
              return (
                <TokenizedInlineCode
                  codeString={codeString}
                  className={className}
                  enableLinks={enableLinks}
                  worktreeId={worktreeId}
                  onFileOpenError={onFileOpenError}
                />
              );
            }

            return (
              <FencedCodeBlock
                codeString={codeString}
                enableLinks={enableLinks}
                worktreeId={worktreeId}
                onFileOpenError={onFileOpenError}
              />
            );
          },
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}

// Memoize markdown so selection isn't disrupted by unrelated re-renders.
const MemoMarkdown = memo(
  Markdown,
  (prev, next) =>
    prev.content === next.content &&
    prev.linkifyFiles === next.linkifyFiles &&
    prev.worktreeId === next.worktreeId,
);

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

function filterThreadItemsForVerbosity(items: ThreadItem[], verbosity: SessionViewVerbosity): ThreadItem[] {
  if (verbosity === "terse") {
    return items.filter((item) => item.kind !== "tool" && item.kind !== "tool_group" && item.kind !== "thought");
  }
  return items;
}

type SortableThreadGroup = {
  sort_at: string;
  group: WorkbenchThreadView["groups"][number];
};

function buildSystemMessageGroups(messages: Message[]): SortableThreadGroup[] {
  const systemMessages = messages
    .filter((m) => m.role === "system")
    .slice()
    .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
  return systemMessages.map((m, idx) => {
    const id = idToString(m.id) || `system-${idx}`;
    const attachments = Array.isArray((m as any).attachments)
      ? ((m as any).attachments as MessageAttachment[])
      : [];
    return {
      sort_at: m.created_at,
      group: {
        key: `system-${id}`,
        header: null,
        items: [
          {
            kind: "message",
            id,
            role: "system",
            content: m.content ?? "",
            attachments,
            created_at: m.created_at,
            delivery: m.delivery,
          },
        ],
      },
    };
  });
}

function mergeGroupsWithSystemMessages(
  groups: SortableThreadGroup[],
  messages: Message[],
): WorkbenchThreadView["groups"] {
  const systemGroups = buildSystemMessageGroups(messages);
  if (systemGroups.length === 0) {
    return groups.map((g) => g.group);
  }
  const combined = [...groups, ...systemGroups];
  combined.sort((a, b) => {
    const cmp = String(a.sort_at).localeCompare(String(b.sort_at));
    if (cmp !== 0) return cmp;
    return String(a.group.key).localeCompare(String(b.group.key));
  });
  return combined.map((g) => g.group);
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
  if (isStatusUpdateMeta(meta)) return false;
  const reasoningKind = meta?.codex?.reasoning_kind ?? meta?.codex?.reasoningKind;
  if (reasoningKind === "summary") return false;
  return true;
}

type ActivityEntry = {
  item: ThreadItem;
  created_at: string;
  kind: "tool" | "thought";
  order_seq?: number;
};

function ensureToolItem(
  toolById: Map<string, Extract<ThreadItem, { kind: "tool" }>>,
  turnId: string,
  toolCallId: string,
  createdAt: string,
) {
  const existing = toolById.get(toolCallId);
  if (existing) return existing;
  const tool: Extract<ThreadItem, { kind: "tool" }> = {
    kind: "tool",
    id: `tool-${turnId}-${toolCallId}`,
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
  toolById.set(toolCallId, tool);
  return tool;
}

function applyToolUpdateFromEvent(
  tool: Extract<ThreadItem, { kind: "tool" }>,
  ev: SessionEvent,
  update: any,
) {
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

  const input =
    update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? update?.input_preview ?? null;
  if (input != null) tool.input = input;

  const nextOutput = extractToolOutputText(update);
  if (nextOutput) tool.output_text = mergeStreamingText(tool.output_text, nextOutput);
  if (tool.input != null || tool.output_text.trim().length > 0) {
    tool.has_details = true;
  }
}

function buildTurnActivityTimeline(opts: {
  turnId: string;
  turn: SessionTurn;
  tools: Array<Extract<ThreadItem, { kind: "tool" }>>;
  events: SessionEvent[];
}): { activity: ActivityEntry[]; tools: Array<Extract<ThreadItem, { kind: "tool" }>> } {
  const toolById = new Map<string, Extract<ThreadItem, { kind: "tool" }>>();
  for (const tool of opts.tools) {
    toolById.set(tool.tool_call_id, tool);
  }

  const activity: ActivityEntry[] = [];
  const toolInserted = new Set<string>();

  for (const ev of opts.events) {
    if (ev.event_type === "thought_chunk") {
      if (!shouldRenderThoughtChunk(ev)) {
        continue;
      }
      const fragment = String(ev.payload_json?.content_fragment ?? "");
      if (!fragment) {
        continue;
      }
      const last = activity[activity.length - 1];
      if (last && last.item.kind === "thought") {
        (last.item as Extract<ThreadItem, { kind: "thought" }>).content = mergeStreamingText(
          (last.item as Extract<ThreadItem, { kind: "thought" }>).content,
          fragment,
        );
        continue;
      }
      const thoughtItem: Extract<ThreadItem, { kind: "thought" }> = {
        kind: "thought",
        id: `thought-${opts.turnId}-${ev.seq ?? ev.created_at}`,
        turn_id: opts.turnId,
        created_at: ev.created_at,
        content: fragment,
      };
      activity.push({
        item: thoughtItem,
        created_at: ev.created_at,
        kind: "thought",
        order_seq: typeof ev.seq === "number" ? ev.seq : undefined,
      });
      continue;
    }

    if (ev.event_type === "tool_call" || ev.event_type === "tool_call_update" || ev.event_type === "tool_result") {
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
      if (!toolCallId) {
        continue;
      }
      const tool = ensureToolItem(toolById, opts.turnId, toolCallId, ev.created_at);
      applyToolUpdateFromEvent(tool, ev, update);
      if (!toolInserted.has(toolCallId)) {
        activity.push({
          item: tool,
          created_at: tool.created_at,
          kind: "tool",
          order_seq: typeof ev.seq === "number" ? ev.seq : undefined,
        });
        toolInserted.add(toolCallId);
      }
      continue;
    }
  }

  const fallbackThought = String(opts.turn.thought_partial ?? "").trim();
  if (fallbackThought && !activity.some((entry) => entry.item.kind === "thought")) {
    const fallbackItem: Extract<ThreadItem, { kind: "thought" }> = {
      kind: "thought",
      id: `thought-${opts.turnId}-fallback`,
      turn_id: opts.turnId,
      created_at: opts.turn.updated_at ?? opts.turn.started_at,
      content: fallbackThought,
    };
    activity.push({
      item: fallbackItem,
      created_at: fallbackItem.created_at,
      kind: "thought",
    });
  }

  const remainingTools = Array.from(toolById.values()).filter((tool) => !toolInserted.has(tool.tool_call_id));
  remainingTools.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
  for (const tool of remainingTools) {
    activity.push({ item: tool, created_at: tool.created_at, kind: "tool" });
  }

  return { activity, tools: Array.from(toolById.values()) };
}

function buildWorkbenchThreadViewModelFromTurns(
  turns: SessionTurn[],
  messages: Message[],
  toolsByTurnId: Record<string, SessionTurnTool[]>,
  events: SessionEvent[],
): WorkbenchThreadView {
  const debugEvents: SessionEvent[] = [];
  const groups: SortableThreadGroup[] = [];
  const customStatusByTurnId = buildCustomStatusByTurnId(events);

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

  const eventsByTurnId = new Map<string, SessionEvent[]>();
  for (const ev of events) {
    const turnId = idToString(ev.turn_id);
    if (!turnId) continue;
    const list = eventsByTurnId.get(turnId) ?? [];
    list.push(ev);
    eventsByTurnId.set(turnId, list);
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

    let tools = (toolsByTurnId[turnId] ?? []).map((tool) => {
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

    const { activity } = buildTurnActivityTimeline({
      turnId,
      turn,
      tools,
      events: eventsByTurnId.get(turnId) ?? [],
    });

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
      kind: "assistant" | "tool" | "thought";
      order_seq?: number;
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

    for (const entry of activity) {
      timeline.push({
        item: entry.item,
        created_at: entry.created_at,
        kind: entry.kind,
        order_seq: entry.order_seq,
      });
    }

    timeline.sort((a, b) => {
      const aSeq = a.order_seq;
      const bSeq = b.order_seq;
      if (Number.isFinite(aSeq) && Number.isFinite(bSeq) && aSeq !== bSeq) {
        return (aSeq ?? 0) - (bSeq ?? 0);
      }

      const tcmp = String(a.created_at).localeCompare(String(b.created_at));
      if (tcmp !== 0) return tcmp;
      if (a.kind === "assistant" && b.kind === "assistant") {
        const sa = Number(a.turn_sequence ?? Number.NaN);
        const sb = Number(b.turn_sequence ?? Number.NaN);
        if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
      }
      const aRank = a.kind === "assistant" ? 2 : 1;
      const bRank = b.kind === "assistant" ? 2 : 1;
      if (aRank !== bRank) return aRank - bRank;
      return String(a.item.id).localeCompare(String(b.item.id));
    });

    const items: ThreadItem[] = timeline.map((entry) => entry.item);

    if (items.length === 0) {
      items.push({ kind: "spacer", id: `spacer-${turnId}`, created_at: turn.started_at });
    }

    const statusText = customStatusByTurnId.get(turnId) ?? null;
    const assistantMessagesContent = assistantMessages
      .map((m) => m.content ?? "")
      .filter((c) => c.trim().length > 0)
      .join("\n\n");
    items.push({
      kind: "turn_status",
      id: `turn-status-${turnId}`,
      turn_id: turnId,
      created_at: turn.updated_at ?? turn.started_at,
      status: turn.status,
      started_at: turn.started_at,
      updated_at: turn.updated_at ?? turn.started_at,
      custom_status: statusText,
      status_text: statusText,
      assistant_messages_content: assistantMessagesContent,
    });

    groups.push({
      sort_at: header?.created_at ?? turn.started_at,
      group: { key: `turn-${turnId}`, header, items },
    });
  }

  return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
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

    const groups: SortableThreadGroup[] = [];

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
      groups.push({ sort_at: g.first_at, group: { key: g.key, header: g.header, items } });
      return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
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
            const message = extractErrorMessage(ev.payload_json) ?? "Error";
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

      groups.push({ sort_at: g.first_at, group: { key: g.key, header: g.header, items } });
    }

    return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
  }

  const eventsInRange = (startIso: string, endIso: string | null) => {
    const start = Date.parse(startIso);
    const end = endIso ? Date.parse(endIso) : Number.POSITIVE_INFINITY;
    return events.filter((e) => {
      const t = Date.parse(String(e.created_at));
      return Number.isFinite(t) && t >= start && t <= end;
    });
  };

  const groups: SortableThreadGroup[] = [];

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
          const message = extractErrorMessage(ev.payload_json) ?? "Error";
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

    groups.push({ sort_at: g.first_at, group: { key: g.key, header: g.header, items } });
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
    groups.push({ sort_at: g.first_at, group: { key: g.key, header: g.header, items } });
  }

  return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
}

function extractToolOutputText(update: any): string {
  const direct =
    update?.outputText ??
    update?.output_text ??
    update?.output_preview ??
    update?.result ??
    update?.rawOutput?.aggregated_output ??
    update?.rawOutput?.output ??
    null;
  if (typeof direct === "string" && direct.trim()) return direct.trim();

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

function pickFirstString(...values: any[]): string | null {
  for (const v of values) {
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return null;
}

function isNonToolStatus(value: string): boolean {
  const s = value.trim().toLowerCase();
  return ![
    "pending",
    "queued",
    "running",
    "in_progress",
    "completed",
    "failed",
    "error",
    "ok",
    "success",
    "succeeded",
  ].includes(s);
}

function formatElapsedSeconds(totalSeconds: number): string {
  const clamped = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(clamped / 3600);
  const minutes = Math.floor((clamped % 3600) / 60);
  const seconds = clamped % 60;
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function isStatusUpdateMeta(meta: any): boolean {
  if (!meta || typeof meta !== "object") return false;
  const codexMeta = meta?.codex ?? {};
  const reasoningKind = codexMeta?.reasoning_kind ?? codexMeta?.reasoningKind;
  if (reasoningKind === "status") return true;

  const statusText = pickFirstString(
    meta?.status_text,
    meta?.statusText,
    meta?.status_string,
    meta?.statusString,
    codexMeta?.status_text,
    codexMeta?.statusText,
    codexMeta?.status_string,
    codexMeta?.statusString,
  );
  if (statusText) return true;

  const statusValue =
    typeof meta?.status === "string"
      ? meta.status
      : typeof codexMeta?.status === "string"
        ? codexMeta.status
        : null;
  if (statusValue && isNonToolStatus(statusValue)) return true;

  return false;
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

function humanToolKind(kind: string): string {
  const k = (kind || "").toLowerCase();
  if (k === "execute") return "Run Command";
  if (k === "search") return "Search";
  if (k === "read" || k === "read_file") return "Read File";
  if (k === "edit" || k === "write" || k === "apply_patch") return "Edit File";
  if (k === "list" || k === "list_files") return "List Files";
  if (k === "fetch" || k === "http" || k === "curl") return "Fetch";
  if (k === "think") return "Think";
  if (k === "error") return "Error";
  return kind || "Tool";
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

type ToolDiffStats = {
  added?: number;
  removed?: number;
  files?: number;
};

function extractToolDiffStats(input: any): ToolDiffStats | null {
  const raw = input?.diff_stats;
  if (!raw || typeof raw !== "object") return null;
  const added = Number((raw as any).added);
  const removed = Number((raw as any).removed);
  const files = Number((raw as any).files);
  const hasAny =
    Number.isFinite(added) || Number.isFinite(removed) || Number.isFinite(files);
  if (!hasAny) return null;
  return {
    added: Number.isFinite(added) ? added : undefined,
    removed: Number.isFinite(removed) ? removed : undefined,
    files: Number.isFinite(files) ? files : undefined,
  };
}

function formatToolDiffStats(input: any): string {
  const stats = extractToolDiffStats(input);
  if (!stats) return "";
  const parts: string[] = [];
  if (stats.added && stats.added > 0) parts.push(`+${stats.added}`);
  if (stats.removed && stats.removed > 0) parts.push(`-${stats.removed}`);
  if (!parts.length && stats.files && stats.files > 0) {
    parts.push(`${stats.files} files`);
  }
  return parts.length ? `(${parts.join(" ")})` : "";
}

function extractToolPaths(input: any): { paths: string[]; total?: number } {
  const paths: string[] = [];
  const push = (value: unknown) => {
    if (typeof value !== "string") return;
    const trimmed = value.trim();
    if (trimmed) paths.push(trimmed);
  };
  push(input?.path);
  push(input?.file);
  push(input?.filename);
  push(input?.file_path);
  push(input?.filePath);
  push(input?.filepath);
  push(input?.target);
  if (Array.isArray(input?.paths)) input.paths.forEach(push);
  if (Array.isArray(input?.files)) input.files.forEach(push);
  if (Array.isArray(input?.file_paths)) input.file_paths.forEach(push);
  if (Array.isArray(input?.filePaths)) input.filePaths.forEach(push);
  if (Array.isArray(input?.parsed_cmd)) {
    input.parsed_cmd.forEach((cmd: any) => push(cmd?.path));
  }
  const seen = new Set<string>();
  const unique = paths.filter((p) => {
    if (seen.has(p)) return false;
    seen.add(p);
    return true;
  });
  const total =
    typeof input?.paths_total === "number" ? input.paths_total : unique.length;
  return { paths: unique, total };
}

function formatToolPathSummary(input: any): string {
  const { paths, total } = extractToolPaths(input);
  if (!paths.length) return "";
  const more = Math.max(0, (total ?? paths.length) - 1);
  const head = truncateMiddle(paths[0], 120);
  return more > 0 ? `${head} +${more} more` : head;
}

function toolSummaryLine(toolKind: string, input: any): string {
  const k = (toolKind || "").toLowerCase();
  if (k === "execute") {
    const cmd = Array.isArray(input?.command) ? input.command.join(" ") : input?.command;
    return cmd ? truncateMiddle(String(cmd), 120) : "";
  }
  if (k === "search") {
    const q = input?.query ?? input?.pattern ?? input?.regex ?? input?.text;
    const path = formatToolPathSummary(input);
    const query = q ? truncateMiddle(String(q), 120) : "";
    if (query && path) return truncateMiddle(`${query} in ${path}`, 120);
    return query || path;
  }
  if (k === "list" || k === "list_files") {
    return formatToolPathSummary(input);
  }
  if (k === "read" || k === "read_file") {
    return formatToolPathSummary(input);
  }
  if (k === "edit" || k === "write" || k === "apply_patch") {
    const path = formatToolPathSummary(input);
    const stats = formatToolDiffStats(input);
    if (path && stats) return `${path} ${stats}`;
    return path || stats;
  }
  if (k === "fetch" || k === "http" || k === "curl") {
    const method = String(input?.method ?? "GET").trim().toUpperCase();
    const url = input?.url ?? input?.uri ?? input?.href;
    if (url) return truncateMiddle(`${method} ${String(url)}`, 120);
    return method;
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
type SessionErrorInfo = { message: string; provider?: string };

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

function readNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const text = value.trim();
  return text ? text : null;
}

function extractErrorMessage(payload: any): string | null {
  if (!payload) return null;
  const direct =
    readNonEmptyString(payload.message) ??
    readNonEmptyString(payload.error) ??
    readNonEmptyString(payload.error_message) ??
    readNonEmptyString(payload.errorMessage);
  if (direct) return direct;

  const acpError = payload.acp_error ?? payload.acpError;
  if (acpError && typeof acpError === "object") {
    const acpDataText = extractErrorMessageFromObject((acpError as any).data);
    if (acpDataText) return acpDataText;
  }
  const acpErrorText = extractErrorMessageFromObject(acpError);
  if (acpErrorText) return acpErrorText;

  const update = payload.acp_update ?? payload.acpUpdate ?? payload.update;
  const updateText = extractErrorMessageFromObject(update);
  if (updateText) return updateText;

  const meta = update?._meta ?? update?.meta ?? payload._meta ?? payload.meta ?? null;
  return (
    readNonEmptyString(meta?.statusText) ??
    readNonEmptyString(meta?.status_text) ??
    readNonEmptyString(meta?.message) ??
    readNonEmptyString(meta?.error)
  );
}

function extractErrorMessageFromObject(value: any): string | null {
  if (!value) return null;
  if (typeof value === "string") return readNonEmptyString(value);
  if (typeof value !== "object") return null;

  const direct =
    readNonEmptyString(value.message) ??
    readNonEmptyString(value.error_message) ??
    readNonEmptyString(value.errorMessage);
  if (direct) return direct;

  const data = value.data ?? value.details ?? value.detail;
  const dataText =
    readNonEmptyString(data) ??
    readNonEmptyString(data?.message) ??
    readNonEmptyString(data?.error);
  if (dataText) return dataText;

  const nested = value.error ?? value.cause;
  const nestedText =
    typeof nested === "object"
      ? extractErrorMessageFromObject(nested)
      : readNonEmptyString(nested);
  if (nestedText) return nestedText;

  const meta = value._meta ?? value.meta;
  return readNonEmptyString(meta?.statusText) ?? readNonEmptyString(meta?.status_text);
}

function deriveSessionError(
  turns: SessionTurn[],
  events: SessionEvent[],
): SessionErrorInfo | null {
  if (turns.length === 0) return null;
  const lastTurn = turns[turns.length - 1];
  if (lastTurn.status !== "failed") return null;
  const turnId = idToString(lastTurn.turn_id);
  let errorEvent: SessionEvent | null = null;
  for (let i = events.length - 1; i >= 0; i--) {
    const ev = events[i];
    if (ev.event_type !== "error") continue;
    if (turnId && idToString(ev.turn_id) !== turnId) continue;
    errorEvent = ev;
    break;
  }
  if (!errorEvent) {
    return { message: "Harness error." };
  }
  const message = extractErrorMessage(errorEvent.payload_json) ?? "Harness error.";
  const provider =
    readNonEmptyString(errorEvent.payload_json?.provider) ??
    readNonEmptyString(errorEvent.payload_json?.provider_id) ??
    readNonEmptyString(errorEvent.payload_json?.providerId) ??
    undefined;
  return { message, provider };
}
