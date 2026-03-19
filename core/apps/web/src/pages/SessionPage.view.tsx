import {
  type SetStateAction,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  deleteMessage,
  Message,
  type MessageAttachment,
  postMessage,
  Session,
  SessionEvent,
  type SessionTurn,
  type SessionTurnTool,
  type SubagentInvocation,
  setSessionModel,
  authenticateSession,
  type ProviderOptions,
  idToString,
  interruptSession,
  uploadBlob,
} from "../api/client";
import { useOpenSession, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { loadSessionViewPrefsV1, saveSessionViewPrefsV1, type SessionViewVerbosity } from "../state/uiStateStore";
import { type SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import { useRafCoalesced } from "../components/hooks/useRafCoalesced";
import { type ContextWindowInfo, type WorkbenchModeId } from "../components/WorkbenchComposer";
import { useFeatureGate } from "../utils/analytics";
import { useDictationController } from "../utils/useDictationController";
import { useWorkbenchStore } from "../workbench/store";
import { buildModelsFromProviderOptions } from "../components/workbenchComposer/WorkbenchComposer.utils";
import { deriveProtocolSlashCommands } from "../utils/protocolSlashCommands";
import { VIRTUOSO_MESSAGE_LIST_LICENSE_KEY } from "../config/licenses";
import { randomUuid } from "../utils/randomUuid";
import type { AskUserQuestionAnswerState, WorkbenchListItem } from "./SessionPage.types";
import {
  buildPendingTurns,
  deriveAuthUi,
  deriveMessagesKey,
  deriveProviderGuardNotice,
  deriveSessionError,
  deriveTurnsKey,
  filterQueuedMessagesForPanel,
  filterTurnsForQueuedMessages,
  mergeMessagesForView,
  mergeQueuedMessagesForPanel,
  normalizeContextWindowMetrics,
} from "./SessionPage.workbenchViewModel";
import { buildOptimisticUserMessage } from "./SessionPage.optimisticMessage";
import { useSessionMessageListController } from "./useSessionMessageListController";
import { useWorkbenchThreadViewModelController } from "./useWorkbenchThreadViewModelController";
import { errorMessage } from "../utils/errorMessage";
import { hasSessionActiveTurn } from "../utils/sessionActivity";
import { defaultSessionVerbosityForProvider } from "./sessionVerbosity";
import { appendSegment } from "./SessionPage.helpers";
import {
  getWorkbenchListItemKey,
  getWorkbenchListItemSizeCacheKey,
  getWorkbenchMessageListLayoutRevision,
} from "./sessionMessageListItemIdentity";
import { isSameContextWindow } from "./sessionView/estimateHeuristics";
import { PendingMessageEntry, shouldDropPendingMessage } from "./sessionView/pendingMessages";
import { getQueuedAttachments } from "./sessionView/SessionQueuePanel";
import { SessionWorkbenchPane } from "./sessionView/SessionWorkbenchPane";
import {
  shouldFreezeInitialThreadProjection,
  shouldReleaseInitialThreadProjection,
  shouldRestoreBottomAnchorAfterProjectionRelease,
} from "./sessionView/initialThreadProjection";
import { useSessionImageDropScope } from "./sessionView/useSessionImageDropScope";
import { useSessionProviderGuard } from "./sessionView/useSessionProviderGuard";
import { useSharedSessionProviderOptions } from "./sessionView/useSharedSessionProviderOptions";
import { useStableAskUserQuestionAnswers } from "./sessionView/useStableAskUserQuestionAnswers";
import { composeModelId, parseModelId } from "../utils/modelEffort";

const SCROLLBACK_INCREASE_VIEWPORT_BY_PX = 240;
const INITIAL_THREAD_PROJECTION_QUIET_MS = 250;

type ThreadProjectionInputs = {
  loaded: boolean;
  turns: SessionTurn[];
  turnsStamp: string;
  messages: Message[];
  messagesStamp: string;
  events: SessionEvent[];
  eventsStamp: string;
  toolsByTurnId: Record<string, SessionTurnTool[]>;
  toolSummariesReady: boolean;
  projectionRev: number;
};

export function SessionView({
  sessionId,
  isActive = true,
  autoOpenSession = true,
  sessionMode = "active",
  draft,
  onDraftChange,
  onDraftAttachmentsChange,
  onDraftPersistNow,
  onModeChange,
}: {
  sessionId: string;
  isActive?: boolean;
  sessionMode?: "active" | "archived";
  draft?: { text: string; modeId: WorkbenchModeId; attachments?: MessageAttachment[] } | null;
  onDraftChange?: ((text: string) => void) | null;
  onDraftAttachmentsChange?: ((attachments: MessageAttachment[]) => void) | null;
  onDraftPersistNow?: (() => void | Promise<void>) | null;
  onModeChange?: ((modeId: WorkbenchModeId) => void) | null;
  autoOpenSession?: boolean;
}) {
  const id = sessionId;
  const supervisor = useSessionSupervisor();
  const workbenchStore = useWorkbenchStore();
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
  const messageListLicenseKey = VIRTUOSO_MESSAGE_LIST_LICENSE_KEY;
  const perfStartRef = useRef<number>(0);
  const [verbosity, setVerbosity] = useState<SessionViewVerbosity>("default");
  const [inputInternal, setInputInternal] = useState("");
  const [draftAttachmentsInternal, setDraftAttachmentsInternal] = useState<MessageAttachment[]>([]);
  const [workbenchModeInternal, setWorkbenchModeInternal] = useState<WorkbenchModeId>("default");
  const [sendBusy, setSendBusy] = useState(false);
  const sendBusyRef = useRef(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [queueActionBusyId, setQueueActionBusyId] = useState<string | null>(null);
  const [pendingMessages, setPendingMessages] = useState<PendingMessageEntry[]>([]);
  const [pendingQueueMessages, setPendingQueueMessages] = useState<PendingMessageEntry[]>([]);
  const [optimisticQueueRemovalIds, setOptimisticQueueRemovalIds] = useState<string[]>([]);
  const [fileOpenError, setFileOpenError] = useState<string | null>(null);
  const [modelSwitchError, setModelSwitchError] = useState<string | null>(null);
  const [optimisticModelId, setOptimisticModelId] = useState<string | null>(null);
  const [modifierDown, setModifierDown] = useState(false);
  const [atBottom, setAtBottom] = useState(true);
  const [authMethodId, setAuthMethodId] = useState<string>("");
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [optimisticAskAnswers, setOptimisticAskAnswers] = useState<Record<string, AskUserQuestionAnswerState>>({});
  const [expandedTurnHeaders, setExpandedTurnHeaders] = useState<Record<string, boolean>>({});
  const [expandedTurnDetailsById, setExpandedTurnDetailsById] = useState<Record<string, boolean>>({});
  const [expandedToolById, setExpandedToolById] = useState<Record<string, boolean>>({});
  const [expandedMessageById, setExpandedMessageById] = useState<Record<string, boolean>>({});
  const [lastContextWindow, setLastContextWindow] = useState<ContextWindowInfo | null>(null);
  const draftAttachments = draft?.attachments ?? draftAttachmentsInternal;
  const setDraftAttachments = useCallback(
    (next: SetStateAction<MessageAttachment[]>) => {
      if (draft) {
        const resolved = typeof next === "function" ? next(draft.attachments ?? []) : next;
        onDraftAttachmentsChange?.(resolved);
        return;
      }
      setDraftAttachmentsInternal(next);
    },
    [draft, onDraftAttachmentsChange],
  );
  const { dropScopeRef, dropActive } = useSessionImageDropScope({ setDraftAttachments });

  useEffect(() => {
    setPendingMessages([]);
    setPendingQueueMessages([]);
    setOptimisticQueueRemovalIds([]);
    setDraftAttachmentsInternal([]);
    setSendError(null);
    setFileOpenError(null);
    setModelSwitchError(null);
    setOptimisticModelId(null);
    setOptimisticAskAnswers({});
    setExpandedTurnHeaders({});
    setExpandedTurnDetailsById({});
    setExpandedToolById({});
    setExpandedMessageById({});
    setLastContextWindow(null);
    setAuthMethodId("");
    setAuthBusy(false);
    setAuthError(null);
    setAtBottom(true);
  }, [id]);

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

  const handleFileOpenError = useCallback((message: string | null) => {
    setFileOpenError(message);
  }, []);

  useOpenSession(autoOpenSession ? id ?? "" : "", { watchDiff: true, mode: sessionMode });
  const refreshAll = useCallback(async () => {
    if (!id) return;
    supervisor.refreshSession(id, { watchDiff: true });
  }, [id, supervisor]);

  const {
    dictationRecording,
    dictationError,
    dictationDebugText,
    dictationOnboarding,
    dismissDictationOnboarding,
    backDictationOnboarding,
    chooseDictationOnboardingLocal,
    chooseDictationOnboardingCloud,
    updateDictationOnboardingCloud,
    submitDictationOnboardingLocal,
    submitDictationOnboardingCloud,
    startDictation,
    stopDictation,
  } = useDictationController({
    text: input,
    setText: setInput,
    appendSegment,
  });

  const entry = useSessionEntry(id ?? "");
  const session: Session | null = entry?.session ?? null;
  useEffect(() => {
    let cancelled = false;
    loadSessionViewPrefsV1()
      .then((prefs) => {
        if (cancelled) return;
        if (prefs?.verbosity) {
          setVerbosity(prefs.verbosity);
          return;
        }
        setVerbosity(defaultSessionVerbosityForProvider(session?.provider_id));
      })
      .catch(() => {
        if (cancelled) return;
        setVerbosity(defaultSessionVerbosityForProvider(session?.provider_id));
      });
    return () => {
      cancelled = true;
    };
  }, [id, session?.provider_id]);
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
  const turnsRev = entry?.turnsRev ?? 0;
  const messagesRev = entry?.messagesRev ?? 0;
  const eventsRev = entry?.eventsRev ?? 0;
  const subagentInvocations: SubagentInvocation[] = entry?.subagentInvocations ?? [];
  const eventsStamp = `${eventsRev}:${entry?.lastEventSeq ?? 0}:${events.length}`;
  const turnsKey = useMemo(() => deriveTurnsKey(turns), [turns, turnsRev]);
  const messagesKey = useMemo(() => deriveMessagesKey(messages), [messages, messagesRev]);
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
    const msg = errorMessage(error);
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
  const pendingQueueMessageIdSet = useMemo(() => {
    return new Set(
      pendingQueueMessages
        .map((entry) => idToString(entry.message.id))
        .filter((messageId): messageId is string => !!messageId),
    );
  }, [pendingQueueMessages]);
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
    setPendingMessages((prev) => {
      if (prev.length === 0) return prev;
      const next = prev.filter((entry) => {
        return !shouldDropPendingMessage(entry.message, realIds);
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
        return !shouldDropPendingMessage(entry.message, realIds);
      });
      return next.length === prev.length ? prev : next;
    });
  }, [queue, pendingQueueMessages.length]);

  const displayMessages = useMemo(
    () => mergeMessagesForView(messages, pendingMessages, queuedMessageIdsToShow),
    [messagesKey, pendingMessages, queuedMessageIdsToShow],
  );
  const displayMessagesKey = useMemo(() => deriveMessagesKey(displayMessages), [displayMessages]);
  const pendingTurns = useMemo(
    () => buildPendingTurns(turns, displayMessages),
    [turnsKey, displayMessagesKey],
  );
  const displayTurns = useMemo(
    () => (pendingTurns.length > 0 ? [...turns, ...pendingTurns] : turns),
    [turnsKey, pendingTurns],
  );
  const displayTurnsKey = useMemo(() => deriveTurnsKey(displayTurns), [displayTurns]);
  const displayTurnsForThread = useMemo(
    () => filterTurnsForQueuedMessages(displayTurns, queuedMessageIdsForThread),
    [displayTurns, queuedMessageIdsForThread],
  );
  const displayTurnsForThreadKey = useMemo(
    () => deriveTurnsKey(displayTurnsForThread),
    [displayTurnsForThread],
  );
  const threadProjectionLoaded = Boolean(entry?.stateLoaded);
  const rawThreadProjection = useMemo(
    (): ThreadProjectionInputs => ({
      loaded: threadProjectionLoaded,
      turnsStamp: `${turnsRev}:${displayTurnsForThreadKey}`,
      turns: displayTurnsForThread,
      messagesStamp: `${messagesRev}:${displayMessagesKey}`,
      messages: displayMessages,
      eventsStamp,
      events,
      toolsByTurnId: turnToolsByTurnId,
      toolSummariesReady,
      projectionRev: entry?.projectionRev ?? 0,
    }),
    [
      displayMessages,
      displayMessagesKey,
      displayTurnsForThread,
      displayTurnsForThreadKey,
      entry?.projectionRev,
      events,
      eventsStamp,
      messagesRev,
      threadProjectionLoaded,
      toolSummariesReady,
      turnToolsByTurnId,
      turnsRev,
    ],
  );
  const coalescedThreadProjection = useRafCoalesced(rawThreadProjection);
  const initialThreadProjectionRef = useRef<{ sessionId: string; projection: ThreadProjectionInputs } | null>(null);
  const [initialThreadProjectionSettledSessionId, setInitialThreadProjectionSettledSessionId] = useState<string | null>(
    null,
  );
  const [initialThreadProjectionReleasedSessionId, setInitialThreadProjectionReleasedSessionId] = useState<string | null>(
    null,
  );
  if (
    initialThreadProjectionRef.current == null ||
    initialThreadProjectionRef.current.sessionId !== id ||
    (!initialThreadProjectionRef.current.projection.loaded && rawThreadProjection.loaded)
  ) {
    initialThreadProjectionRef.current = {
      sessionId: id,
      projection: rawThreadProjection,
    };
  }
  const initialThreadProjection =
    initialThreadProjectionRef.current?.sessionId === id
      ? initialThreadProjectionRef.current.projection
      : rawThreadProjection;
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
    () => hasSessionActiveTurn(entry?.activity),
    [entry?.activity],
  );
  const sessionIsAuthoritative = entry?.freshness === "authoritative";
  const sessionProjectionReady =
    entry?.loadState === "live" &&
    toolSummariesReady &&
    entry?.freshness === "authoritative";
  useEffect(() => {
    if (!initialThreadProjection.loaded || !sessionProjectionReady) {
      setInitialThreadProjectionReleasedSessionId((current) => (current === id ? null : current));
      return;
    }
    setInitialThreadProjectionSettledSessionId((current) => (current === id ? current : id));
    if (
      !shouldReleaseInitialThreadProjection({
        loaded: initialThreadProjection.loaded,
        sessionId: id,
        settledSessionId: initialThreadProjectionSettledSessionId,
        releasedSessionId: initialThreadProjectionReleasedSessionId,
      })
    ) {
      return;
    }
    const timeoutId = window.setTimeout(() => {
      setInitialThreadProjectionReleasedSessionId((current) => (current === id ? current : id));
    }, INITIAL_THREAD_PROJECTION_QUIET_MS);
    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [
    id,
    initialThreadProjection.loaded,
    initialThreadProjectionReleasedSessionId,
    initialThreadProjectionSettledSessionId,
    rawThreadProjection.eventsStamp,
    rawThreadProjection.messagesStamp,
    rawThreadProjection.projectionRev,
    rawThreadProjection.turnsStamp,
    sessionProjectionReady,
  ]);
  const freezeInitialThreadProjection = shouldFreezeInitialThreadProjection({
    loaded: initialThreadProjection.loaded,
    sessionId: id,
    releasedSessionId: initialThreadProjectionReleasedSessionId,
  });
  const threadProjection = freezeInitialThreadProjection ? initialThreadProjection : coalescedThreadProjection;
  const threadProjectionSource: "initial" | "coalesced" =
    freezeInitialThreadProjection ? "initial" : "coalesced";
  const previousThreadProjectionSourceRef = useRef<"initial" | "coalesced">(threadProjectionSource);
  const previousAtBottomRef = useRef(atBottom);
  const queuedMessagesEnabled = useFeatureGate("queued_messages_enabled", false);
  const sessionError = useMemo(
    () => deriveSessionError(turns, events),
    [turnsKey, eventsStamp],
  );
  const providerGuardNotice = useMemo(
    () => deriveProviderGuardNotice(events),
    [eventsStamp],
  );

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
  }, [eventsStamp, optimisticAskAnswers]);

  const askUserQuestionAnswers = useStableAskUserQuestionAnswers({
    events,
    optimisticAskAnswers,
    eventsStamp,
  });

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

  const { view: workbenchThreadView, listItems: threadListItems } = useWorkbenchThreadViewModelController({
    sessionId: id,
    turnsStamp: threadProjection.turnsStamp,
    messagesStamp: threadProjection.messagesStamp,
    eventsStamp: threadProjection.eventsStamp,
    verbosity,
    turns: threadProjection.turns,
    messages: threadProjection.messages,
    events: threadProjection.events,
    toolsByTurnId: threadProjection.toolsByTurnId,
    toolSummariesReady: threadProjection.toolSummariesReady,
    askUserQuestionAnswers,
    enableDebugEvents: showDebug,
  });

  const debugEvents = workbenchThreadView.debugEvents;
  const wbListItems = threadListItems;
  const listItems = wbListItems;
  const messageListItemIdentity = useCallback((item: WorkbenchListItem) => item.id, []);
  const messageListItemKey = useCallback(
    (item: WorkbenchListItem) =>
      getWorkbenchListItemKey(item, {
        expandedTurnHeaders,
        expandedTurnDetailsById,
        expandedToolById,
        expandedMessageById,
        turnToolsLoading,
      }, { verbosity }),
    [
      expandedMessageById,
      expandedToolById,
      expandedTurnDetailsById,
      expandedTurnHeaders,
      turnToolsLoading,
      verbosity,
    ],
  );
  const messageListItemSizeCacheKey = useCallback(
    (item: WorkbenchListItem) =>
      getWorkbenchListItemSizeCacheKey(item, {
        expandedTurnHeaders,
        expandedTurnDetailsById,
        expandedToolById,
        expandedMessageById,
        turnToolsLoading,
      }, { verbosity }),
    [
      expandedMessageById,
      expandedToolById,
      expandedTurnDetailsById,
      expandedTurnHeaders,
      turnToolsLoading,
      verbosity,
    ],
  );
  const messageListLayoutRevision = useMemo(
    () =>
      getWorkbenchMessageListLayoutRevision(
        {
          expandedTurnHeaders,
          expandedTurnDetailsById,
          expandedToolById,
          expandedMessageById,
          turnToolsLoading,
        },
        { verbosity },
      ),
    [
      expandedMessageById,
      expandedToolById,
      expandedTurnDetailsById,
      expandedTurnHeaders,
      turnToolsLoading,
      verbosity,
    ],
  );

  const {
    methodsRef: messageListMethodsRef,
    context: messageListContext,
    initialData: messageListInitialData,
    initialLocation: messageListInitialLocation,
    onScroll: handleMessageListScroll,
    onRenderedDataChange: handleRenderedDataChange,
  } = useSessionMessageListController({
    sessionId: id,
    isActive,
    loaded: Boolean(entry?.stateLoaded),
    listItems,
    canLoadOlder: Boolean(id && hasMoreTurns),
    loadOlder: async () => {
      if (!id) return;
      await supervisor.loadMoreTurns(id);
    },
    layoutRevision: messageListLayoutRevision,
    itemSizeCacheKey: messageListItemSizeCacheKey,
    showDebug,
    onAtBottomChange: setAtBottom,
  });

  // MessageList integration is now handled by `useSessionMessageListController`.

  useLayoutEffect(() => {
    const previousSource = previousThreadProjectionSourceRef.current;
    const wasAtBottom = previousAtBottomRef.current;
    previousThreadProjectionSourceRef.current = threadProjectionSource;
    previousAtBottomRef.current = atBottom;
    if (
      !shouldRestoreBottomAnchorAfterProjectionRelease({
        previousSource,
        nextSource: threadProjectionSource,
        wasAtBottom,
      })
    ) {
      return;
    }
    messageListMethodsRef.current?.scrollToItem({
      index: "LAST",
      align: "end",
      behavior: "auto",
    });
  }, [atBottom, messageListMethodsRef, threadProjectionSource]);

  const authUi = useMemo(() => deriveAuthUi(events), [eventsStamp]);
  const {
    providerGuardActionError,
    providerGuardActionBusy,
    providerGuardMemoryLimitMb,
    providerGuardHeading,
    providerGuardMessage,
    providerGuardLimitLabel,
    providerGuardProviderLabel,
    providerGuardPidLabel,
    canRaiseProviderGuard,
    raiseProviderGuardLimit,
    disableProviderGuard,
  } = useSessionProviderGuard({
    providerGuardNotice,
    sessionProviderId: session?.provider_id,
  });

  useEffect(() => {
    if (authMethodId) return;
    if (authUi.methods.length > 0) {
      setAuthMethodId(authUi.methods[0].id);
    }
  }, [authUi.methods, authMethodId]);

  const sharedProviderOptions = useSharedSessionProviderOptions(session);

  const modelOptions = useMemo(() => {
    const parsed = buildModelsFromProviderOptions(sharedProviderOptions);
    if (parsed.length > 0) return parsed;
    const fallbackId = composeModelId(
      String(session?.model_id ?? ""),
      session?.reasoning_effort ?? null,
    );
    return fallbackId ? [{ id: fallbackId, name: fallbackId }] : [];
  }, [session?.model_id, session?.reasoning_effort, sharedProviderOptions]);
  const currentModelId = useMemo(() => {
    return composeModelId(
      String(session?.model_id ?? ""),
      session?.reasoning_effort ?? null,
    );
  }, [session?.model_id, session?.reasoning_effort]);
  const displayedModelId = optimisticModelId ?? currentModelId;

  const setSendBusySafe = (next: boolean) => {
    sendBusyRef.current = next;
    setSendBusy(next);
  };

  const formatMemoryMb = (value?: number | null): string => {
    if (!Number.isFinite(value)) return "—";
    const mb = value as number;
    const gb = mb / 1024;
    if (gb >= 1) {
      const precision = gb >= 10 ? 0 : 1;
      return `${gb.toFixed(precision)} GB`;
    }
    return `${Math.round(mb)} MB`;
  };

  const sendNow = async () => {
    if (!id) return;
    if (sendBusyRef.current) return;
    if (hasActiveTurn && !queuedMessagesEnabled && sessionIsAuthoritative) {
      setSendError("A turn is already running. Stop it or wait for it to finish.");
      return;
    }
    setSendBusySafe(true);
    let text = "";
    try {
      text = (dictationRecording ? await stopDictation({ awaitFinal: true }) : input).trim();
    } catch (e: unknown) {
      setSendError(errorMessage(e));
      setSendBusySafe(false);
      return;
    }
    if (!text) {
      setSendBusySafe(false);
      return;
    }
    const attachmentsToSend = draftAttachments.slice();
    const shouldQueue = hasActiveTurn && queuedMessagesEnabled && sessionIsAuthoritative;
    const requestedDelivery = shouldQueue ? "queued" : undefined;
    const messageId = randomUuid();
    const turnId = randomUuid();
    const optimisticMessage: Message = buildOptimisticUserMessage({
      messageId,
      sessionId: id,
      taskId: String(session?.task_id ?? ""),
      turnId,
      content: text,
      attachments: attachmentsToSend,
      delivery: shouldQueue ? "queued" : "immediate",
    });
    setSendError(null);
    if (shouldQueue) {
      setPendingQueueMessages((prev) => [...prev, { clientId: messageId, message: optimisticMessage }]);
    } else {
      setPendingMessages((prev) => [...prev, { clientId: messageId, message: optimisticMessage }]);
    }
    setAtBottom(true);
    setInput("");
    setDraftAttachments([]);
    try {
      const posted = await postMessage(id, text, requestedDelivery, attachmentsToSend, {
        id: messageId,
        turn_id: turnId,
      });
      if (shouldQueue) {
        setPendingQueueMessages((prev) =>
          prev.map((entry) => (entry.clientId === messageId ? { ...entry, message: posted } : entry)),
        );
      } else {
        setPendingMessages((prev) =>
          prev.map((entry) => (entry.clientId === messageId ? { ...entry, message: posted } : entry)),
        );
      }
      try {
        await onDraftPersistNow?.();
      } catch {
        // best-effort
      }
    } catch (e: unknown) {
      if (shouldQueue) {
        setPendingQueueMessages((prev) => prev.filter((entry) => entry.clientId !== messageId));
      } else {
        setPendingMessages((prev) => prev.filter((entry) => entry.clientId !== messageId));
      }
      setInput(text);
      setDraftAttachments(attachmentsToSend);
      setSendError(errorMessage(e));
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
    } catch (e: unknown) {
      if (!shouldKeepQueueRemovalOnError(e)) {
        rollbackOptimisticQueueRemoval(messageId);
      }
      setSendError(errorMessage(e));
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
    } catch (e: unknown) {
      if (!shouldKeepQueueRemovalOnError(e)) {
        rollbackOptimisticQueueRemoval(mid);
      }
      setSendError(errorMessage(e));
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
    } catch (e: unknown) {
      rollbackOptimisticQueueRemoval(mid);
      setSendError(errorMessage(e));
      setQueueActionBusyId(null);
      return;
    }
    try {
      await deleteMessage(mid);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== mid),
      );
    } catch (e: unknown) {
      if (!shouldKeepQueueRemovalOnError(e)) {
        rollbackOptimisticQueueRemoval(mid);
      }
      setSendError(errorMessage(e));
      setQueueActionBusyId(null);
      return;
    }
    setSendBusySafe(true);

    const messageId = randomUuid();
    const turnId = randomUuid();
    const optimisticMessage: Message = buildOptimisticUserMessage({
      messageId,
      sessionId: id,
      taskId: String(session?.task_id ?? ""),
      turnId,
      content,
      attachments,
      delivery: "immediate",
    });
    setPendingMessages((prev) => [...prev, { clientId: messageId, message: optimisticMessage }]);
    setAtBottom(true);

    try {
      const posted = await postMessage(id, content, "immediate", attachments, {
        id: messageId,
        turn_id: turnId,
      });
      setPendingMessages((prev) =>
        prev.map((entry) => (entry.clientId === messageId ? { ...entry, message: posted } : entry)),
      );
    } catch (e: unknown) {
      setPendingMessages((prev) => prev.filter((entry) => entry.clientId !== messageId));
      setSendError(errorMessage(e));
    } finally {
      setSendBusySafe(false);
      setQueueActionBusyId(null);
    }
  };

  const slashCommands = useMemo<SlashCommandDescriptor[]>(
    () =>
      deriveProtocolSlashCommands({
        providerId: entry?.session?.provider_id,
        commands: entry?.acpCommands,
        slashCommands: entry?.acpSlashCommands,
      }),
    [entry?.acpCommands, entry?.acpSlashCommands, entry?.session?.provider_id],
  );

  const virtuosoStyle = useMemo(() => ({ flex: 1, minHeight: 0 } as const), []);
  // Note: we intentionally do not overscan (`increaseViewportBy`) for the session thread.
  // Large overscan amplifies prepend stabilization error for unknown-height items.
  const handleAuthenticate = useCallback(async () => {
    if (!id) return;
    setAuthBusy(true);
    setAuthError(null);
    try {
      await authenticateSession(id, authMethodId);
      await refreshAll();
    } catch (error: unknown) {
      setAuthError(errorMessage(error));
    } finally {
      setAuthBusy(false);
    }
  }, [authMethodId, id, refreshAll]);

  const handleInterruptSession = useCallback(async () => {
    await interruptSession(id);
  }, [id]);

  const handleToggleRecording = useCallback(() => {
    if (dictationRecording) {
      stopDictation().catch(() => {});
      return;
    }
    startDictation().catch(() => {});
  }, [dictationRecording, startDictation, stopDictation]);

  const handleSetModelId = useCallback(async (next: string) => {
    setModelSwitchError(null);
    setOptimisticModelId(next);
    try {
      const parsed = parseModelId(next);
      const updated = await setSessionModel(id, parsed.base || next, parsed.effort);
      supervisor.setSession(updated);
      setOptimisticModelId(null);
    } catch (error: unknown) {
      setOptimisticModelId(null);
      setModelSwitchError(errorMessage(error));
    }
  }, [id, supervisor]);

  return (
    <SessionWorkbenchPane
      id={id}
      entryLoadState={entry?.loadState}
      entryError={entry?.error}
      session={session}
      sessionError={sessionError}
      dropActive={dropActive}
      dropScopeRef={dropScopeRef}
      listItems={listItems}
      liveTailItems={[]}
      events={events}
      messages={messages}
      worktreeId={worktreeId}
      handleFileOpenError={handleFileOpenError}
      modifierDown={modifierDown}
      activeAskToolCallId={activeAskToolCallId}
      expandedTurnHeaders={expandedTurnHeaders}
      setExpandedTurnHeaders={setExpandedTurnHeaders}
      expandedTurnDetailsById={expandedTurnDetailsById}
      setExpandedTurnDetailsById={setExpandedTurnDetailsById}
      expandedToolById={expandedToolById}
      setExpandedToolById={setExpandedToolById}
      expandedMessageById={expandedMessageById}
      setExpandedMessageById={setExpandedMessageById}
      turnToolsLoading={turnToolsLoading}
      verbosity={verbosity}
      setOptimisticAskAnswers={setOptimisticAskAnswers}
      onRequestTurnTools={(turnId) => {
        supervisor.loadTurnTools(id, turnId);
      }}
      showDebug={showDebug}
      debugEvents={debugEvents}
      authUi={authUi}
      authMethodId={authMethodId}
      onAuthMethodChange={setAuthMethodId}
      authBusy={authBusy}
      authError={authError}
      onAuthenticate={handleAuthenticate}
      subagentInvocations={subagentInvocations}
      onOpenChildSession={openChildSession}
      style={virtuosoStyle}
      itemIdentity={messageListItemIdentity}
      itemKey={messageListItemKey}
      increaseViewportBy={SCROLLBACK_INCREASE_VIEWPORT_BY_PX}
      initialData={messageListInitialData}
      initialLocation={messageListInitialLocation}
      context={messageListContext}
      onScroll={handleMessageListScroll}
      onRenderedDataChange={handleRenderedDataChange}
      methodsRef={messageListMethodsRef}
      licenseKey={messageListLicenseKey}
      shortSizeAlign={atBottom ? "bottom" : "top"}
      queueForPanel={queueForPanel}
      pendingQueueMessageIdSet={pendingQueueMessageIdSet}
      queueActionBusy={queueActionBusy}
      sendBusy={sendBusy}
      onSendQueuedNow={onSendQueuedNow}
      onEditQueued={onEditQueued}
      onRemoveQueued={onRemoveQueued}
      input={input}
      setInput={setInput}
      slashCommands={slashCommands}
      draftAttachments={draftAttachments}
      setDraftAttachments={setDraftAttachments}
      sendNow={sendNow}
      hasDraftContent={hasDraftContent}
      hasActiveTurn={hasActiveTurn}
      atBottom={atBottom}
      setVerbosityPref={setVerbosityPref}
      workbenchMode={workbenchMode}
      setWorkbenchMode={setWorkbenchMode}
      contextWindow={contextWindow}
      dictationRecording={dictationRecording}
      onToggleRecording={handleToggleRecording}
      onInterruptSession={handleInterruptSession}
      sendError={sendError}
      fileOpenError={fileOpenError}
      dictationDebugText={dictationDebugText}
      dictationError={dictationError}
      dictationOnboarding={dictationOnboarding}
      dismissDictationOnboarding={dismissDictationOnboarding}
      backDictationOnboarding={backDictationOnboarding}
      chooseDictationOnboardingLocal={chooseDictationOnboardingLocal}
      chooseDictationOnboardingCloud={chooseDictationOnboardingCloud}
      updateDictationOnboardingCloud={updateDictationOnboardingCloud}
      submitDictationOnboardingCloud={submitDictationOnboardingCloud}
      submitDictationOnboardingLocal={submitDictationOnboardingLocal}
      providerGuardNotice={providerGuardNotice}
      providerGuardHeading={providerGuardHeading}
      providerGuardMessage={providerGuardMessage}
      providerGuardProviderLabel={providerGuardProviderLabel}
      providerGuardPidLabel={providerGuardPidLabel}
      providerGuardMemoryLimitMb={providerGuardMemoryLimitMb}
      providerGuardLimitLabel={providerGuardLimitLabel}
      providerGuardActionBusy={providerGuardActionBusy}
      providerGuardActionError={providerGuardActionError}
      canRaiseProviderGuard={canRaiseProviderGuard}
      onRaiseProviderGuardLimit={raiseProviderGuardLimit}
      onDisableProviderGuard={disableProviderGuard}
      formatMemoryMb={formatMemoryMb}
      availableModels={modelOptions}
      currentModelId={displayedModelId}
      onSetModelId={handleSetModelId}
      modelSwitchError={modelSwitchError}
    />
  );
}
