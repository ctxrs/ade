import {
  useCallback,
  useEffect,
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
  type SubagentInvocation,
  authenticateSession,
  type ProviderOptions,
  idToString,
  interruptSession,
  uploadBlob,
} from "../../api/client";
import { useOpenSession, useSessionEntry, useSessionSupervisor } from "../../state/sessionSupervisor";
import {
  selectSessionThreadProjection,
  selectSessionQueuePanelMessages,
} from "../../state/sessionThreadProjection/selectors";
import { loadSessionViewPrefsV1, saveSessionViewPrefsV1, type SessionViewVerbosity } from "../../state/uiStateStore";
import { type SlashCommandDescriptor } from "../../state/useComposerAutocomplete";
import { type ContextWindowInfo, type WorkbenchModeId } from "../../components/WorkbenchComposer";
import { useFeatureGate } from "../../utils/analytics";
import { useDictationController } from "../../utils/useDictationController";
import { useWorkbenchStore } from "../../workbench/store";
import { buildModelsFromProviderOptions } from "../../components/workbenchComposer/WorkbenchComposer.utils";
import { deriveProtocolSlashCommands } from "../../utils/protocolSlashCommands";
import { VIRTUOSO_MESSAGE_LIST_LICENSE_KEY } from "../../config/licenses";
import { randomUuid } from "../../utils/randomUuid";
import type { AskUserQuestionAnswerState, WorkbenchListItem } from "./SessionPage.types";
import {
  deriveAuthUi,
  deriveProviderGuardNotice,
  deriveSessionError,
  normalizeContextWindowMetrics,
} from "../workbenchViewModel/SessionPage.workbenchViewModel";
import { buildOptimisticUserMessage } from "./SessionPage.optimisticMessage";
import { useSessionTranscriptController } from "../useSessionTranscriptController";
import { useWorkbenchThreadViewModelController } from "../useWorkbenchThreadViewModelController";
import { errorMessage } from "../../utils/errorMessage";
import { hasSessionActiveTurn } from "../../utils/sessionActivity";
import { defaultSessionVerbosityForProvider } from "../sessionVerbosity";
import { appendSegment } from "./SessionPage.helpers";
import { type WorkbenchMessageListUiState } from "../sessionMessageListItemIdentity";
import { isSameContextWindow } from "./estimateHeuristics";
import { getQueuedAttachments } from "./SessionQueuePanel";
import { collectSessionLoadIssues } from "./sessionLoadIssues";
import { SessionWorkbenchPane } from "./SessionWorkbenchPane";
import { useSessionDraftAttachments } from "./useSessionDraftAttachments";
import { useSessionProviderGuard } from "./useSessionProviderGuard";
import { recordSessionThreadProjectionDebugEntry } from "../sessionThreadProjectionDebug";
import { useSharedSessionProviderOptions } from "./useSharedSessionProviderOptions";
import { useStableAskUserQuestionAnswers } from "./useStableAskUserQuestionAnswers";
import { composeModelId } from "../../utils/modelEffort";
import { useSessionModelSwitcher } from "./useSessionModelSwitcher";
import { noteSessionTranscriptWarmVerbosity } from "../sessionThread/sessionTranscriptWarmState";

const SCROLLBACK_INCREASE_VIEWPORT_BY_PX = 240;

function buildModelsFromAcpMeta(models: unknown): Array<{ id: string; name?: string }> {
  if (!models || typeof models !== "object") return [];
  const list = "models" in models ? models.models : undefined;
  if (!Array.isArray(list)) return [];
  const parsed: Array<{ id: string; name?: string }> = [];
  for (const item of list) {
    if (!item || typeof item !== "object") continue;
    const id = "id" in item && typeof item.id === "string" ? item.id : "";
    if (!id) continue;
    const name = "name" in item && typeof item.name === "string" ? item.name : undefined;
    parsed.push(name ? { id, name } : { id });
  }
  return parsed;
}

export function SessionView({
  sessionId,
  isActive = true,
  autoOpenSession = true,
  sessionMode = "active",
  hideSessionLoadIssuesBanner = false,
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
  hideSessionLoadIssuesBanner?: boolean;
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
  const [workbenchModeInternal, setWorkbenchModeInternal] = useState<WorkbenchModeId>("default");
  const [sendBusy, setSendBusy] = useState(false);
  const sendBusyRef = useRef(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [queueActionBusyId, setQueueActionBusyId] = useState<string | null>(null);
  const [fileOpenError, setFileOpenError] = useState<string | null>(null);
  const [modelSwitchError, setModelSwitchError] = useState<string | null>(null);
  const [optimisticModelId, setOptimisticModelId] = useState<string | null>(null);
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
  const {
    draftAttachmentsInternal,
    setDraftAttachmentsInternal,
    draftAttachments,
    setDraftAttachments,
    dropScopeRef,
    dropActive,
  } = useSessionDraftAttachments({
    draft,
    onDraftAttachmentsChange,
    onError: setSendError,
  });

  useEffect(() => {
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
  const handleRetrySessionLoads = useCallback(() => {
    if (!id) return;
    supervisor.loadSessionState(id, { force: true });
    supervisor.loadSubagentInvocations(id, { force: true });
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
  const supervisorThreadProjection = useMemo(
    () => selectSessionThreadProjection(entry),
    [entry],
  );
  const baseTurns = supervisorThreadProjection.turns;
  const baseMessages = supervisorThreadProjection.messages;
  const baseEvents = supervisorThreadProjection.events;
  const baseTurnsKey = supervisorThreadProjection.turnsStamp;
  const baseEventsStamp = supervisorThreadProjection.eventsStamp;
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

  useEffect(() => {
    noteSessionTranscriptWarmVerbosity(verbosity);
  }, [verbosity]);
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
  const turnToolsLoading = entry?.turnToolsLoading ?? [];
  const hasMoreTurns = entry?.hasMoreTurns ?? false;
  const optimisticQueuedMessages: Message[] = entry?.optimisticQueuedMessages ?? [];
  const optimisticQueueRemovalIds = entry?.optimisticQueueRemovalIds ?? [];
  const subagentInvocations: SubagentInvocation[] = entry?.subagentInvocations ?? [];
  const sessionLoadIssues = useMemo(
    () => collectSessionLoadIssues(entry?.loadErrors),
    [entry?.loadErrors?.state, entry?.loadErrors?.subagentInvocations],
  );
  const markQueueOptimisticallyRemoved = useCallback((messageId: string) => {
    if (!messageId) return;
    supervisor.addOptimisticQueueRemovalId(id, messageId);
  }, [id, supervisor]);
  const rollbackOptimisticQueueRemoval = useCallback((messageId: string) => {
    if (!messageId) return;
    supervisor.removeOptimisticQueueRemovalId(id, messageId);
  }, [id, supervisor]);
  const shouldKeepQueueRemovalOnError = (error: unknown) => {
    const msg = errorMessage(error);
    return msg.startsWith("400") || msg.startsWith("404");
  };
  const queuedMessagesEnabled = useFeatureGate("queued_messages_enabled", false);
  const queueForPanel = useMemo(
    () => (queuedMessagesEnabled ? selectSessionQueuePanelMessages(entry, baseTurns) : []),
    [baseTurns, baseTurnsKey, entry, queuedMessagesEnabled],
  );
  const pendingQueueMessageIdSet = useMemo(() => {
    return new Set(
      optimisticQueuedMessages
        .map((message) => idToString(message.id))
        .filter((messageId): messageId is string => !!messageId),
    );
  }, [optimisticQueuedMessages]);
  const queuedMessageIdsForThread = useMemo(() => {
    if (!queuedMessagesEnabled) return new Set<string>();
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
  }, [optimisticQueueRemovalIds, queueForPanel, queuedMessagesEnabled]);
  const threadProjection = supervisorThreadProjection;
  const computedContextWindow = useMemo<ContextWindowInfo | null>(() => {
    for (let i = baseTurns.length - 1; i >= 0; i -= 1) {
      const metrics = baseTurns[i]?.metrics_json;
      if (!metrics) continue;
      const normalized = normalizeContextWindowMetrics(metrics);
      if (normalized) return normalized;
    }
    return null;
  }, [baseTurns, baseTurnsKey]);
  useEffect(() => {
    if (!computedContextWindow) return;
    setLastContextWindow((prev) =>
      isSameContextWindow(prev, computedContextWindow) ? prev : computedContextWindow,
    );
  }, [computedContextWindow]);
  const contextWindow = computedContextWindow ?? lastContextWindow;
  const latestTurnStatus = useMemo(
    () => baseTurns.at(-1)?.status ?? entry?.turns.at(-1)?.status ?? null,
    [baseTurns, baseTurnsKey, entry?.turns],
  );
  const hasActiveTurn = useMemo(
    () => hasSessionActiveTurn(entry?.activity, latestTurnStatus),
    [entry?.activity, latestTurnStatus],
  );
  const sessionProjectionReady =
    entry?.loadState === "live" &&
    supervisorThreadProjection.toolSummariesReady &&
    ["authoritative", "replica"].includes(String(entry?.freshness ?? ""));
  const threadProjectionSource = "supervisor" as const;
  const sessionError = useMemo(
    () => deriveSessionError(baseTurns, baseEvents),
    [baseEvents, baseEventsStamp, baseTurns, baseTurnsKey],
  );
  const providerGuardNotice = useMemo(
    () => deriveProviderGuardNotice(baseEvents),
    [baseEvents, baseEventsStamp],
  );

  const activeAskToolCallId = useMemo(() => {
    const answered = new Set<string>();
    for (const ev of baseEvents) {
      if (ev.event_type !== "notice") continue;
      if (ev.payload_json?.kind !== "ask_user_question_answered") continue;
      const toolCallId = String(ev.payload_json?.tool_call_id ?? "").trim();
      if (toolCallId) answered.add(toolCallId);
    }
    for (const toolCallId of Object.keys(optimisticAskAnswers)) {
      if (toolCallId) answered.add(toolCallId);
    }

    for (let i = baseEvents.length - 1; i >= 0; i--) {
      const ev = baseEvents[i];
      if (ev.event_type !== "notice") continue;
      if (ev.payload_json?.kind !== "ask_user_question") continue;
      const toolCallId = String(ev.payload_json?.tool_call_id ?? "").trim();
      if (!toolCallId || answered.has(toolCallId)) continue;
      return toolCallId;
    }
    return null;
  }, [baseEvents, baseEventsStamp, optimisticAskAnswers]);

  const askUserQuestionAnswers = useStableAskUserQuestionAnswers({
    events: baseEvents,
    optimisticAskAnswers,
    eventsStamp: baseEventsStamp,
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

  const {
    view: workbenchThreadView,
    listItems: threadListItems,
    projectionRevision,
    lastOp: rawWorkbenchThreadOp,
  } = useWorkbenchThreadViewModelController({
    sessionId: id,
    projectionRev: threadProjection.projectionRev,
    turnsStamp: threadProjection.turnsStamp,
    messagesStamp: threadProjection.messagesStamp,
    eventsStamp: threadProjection.eventsStamp,
    verbosity,
    turns: threadProjection.turns,
    assistantStreamingByTurnId: threadProjection.assistantStreamingByTurnId,
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
  const messageListUiState = useMemo<WorkbenchMessageListUiState>(
    () => ({
      expandedTurnHeaders,
      expandedTurnDetailsById,
      expandedToolById,
      expandedMessageById,
      turnToolsLoading,
      verbosity,
    }),
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
    threadProjectionOp,
    itemIdentity: messageListItemIdentity,
    itemKey: messageListItemKey,
    methodsRef: messageListMethodsRef,
    context: messageListContext,
    initialData: messageListInitialData,
    initialLocation: messageListInitialLocation,
    onScroll: handleMessageListScroll,
    onRenderedDataChange: handleRenderedDataChange,
  } = useSessionTranscriptController({
    sessionId: id,
    isActive,
    loaded: Boolean(entry?.stateLoaded),
    listItems,
    canLoadOlder: Boolean(id && hasMoreTurns),
    loadOlder: async () => {
      if (!id) return;
      await supervisor.loadMoreTurns(id);
    },
    showDebug,
    onAtBottomChange: setAtBottom,
    uiState: messageListUiState,
    workbenchThreadOp: rawWorkbenchThreadOp,
    projectionRevision,
  });

  useEffect(() => {
    if (!showDebug || typeof window === "undefined") return;
    recordSessionThreadProjectionDebugEntry({
      sessionId: id,
      source: threadProjectionSource,
      loaded: Boolean(threadProjection.loaded),
      sessionProjectionReady,
      freshness: entry?.freshness ?? null,
      loadState: entry?.loadState ?? null,
      lastTurnStatus: entry?.activity?.last_turn_status ?? null,
      turnsStamp: threadProjection.turnsStamp,
      messagesStamp: threadProjection.messagesStamp,
      eventsStamp: threadProjection.eventsStamp,
      projectionRev: threadProjection.projectionRev,
      opKind: threadProjectionOp.kind,
      listItemCount: listItems.length,
    });
  }, [
    entry?.activity?.last_turn_status,
    entry?.freshness,
    entry?.loadState,
    id,
    listItems.length,
    sessionProjectionReady,
    showDebug,
    threadProjection.eventsStamp,
    threadProjection.loaded,
    threadProjection.messagesStamp,
    threadProjectionOp.kind,
    threadProjection.projectionRev,
    threadProjection.turnsStamp,
    threadProjectionSource,
  ]);

  // PretextVirtualizer integration is now handled by `usePretextVirtualizerSessionController`.

  const authUi = useMemo(() => deriveAuthUi(baseEvents), [baseEvents, baseEventsStamp]);
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

  const currentModelId = useMemo(() => {
    return composeModelId(
      String(session?.model_id ?? ""),
      session?.reasoning_effort ?? null,
    );
  }, [session?.model_id, session?.reasoning_effort]);
  const acpModelOptions = useMemo(
    () => buildModelsFromAcpMeta(entry?.acpModels),
    [entry?.acpModels],
  );
  const modelOptions = useMemo(() => {
    const parsed = buildModelsFromProviderOptions(sharedProviderOptions);
    if (parsed.length > 0) return parsed;
    if (acpModelOptions.length > 0) return acpModelOptions;
    return currentModelId ? [{ id: currentModelId, name: currentModelId }] : [];
  }, [acpModelOptions, currentModelId, sharedProviderOptions]);
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
    if (hasActiveTurn && !queuedMessagesEnabled) {
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
    const shouldQueue = hasActiveTurn && queuedMessagesEnabled;
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
      supervisor.upsertOptimisticQueuedMessage(id, optimisticMessage);
    } else {
      supervisor.upsertOptimisticThreadMessage(id, optimisticMessage);
    }
    setAtBottom(true);
    setInput("");
    setDraftAttachments([]);
    try {
      const posted = await postMessage(id, text, requestedDelivery, attachmentsToSend, {
        id: messageId,
        turn_id: turnId,
        analytics: {
          providerId: session?.provider_id ?? undefined,
          modelId: currentModelId || undefined,
          reasoningEffort: session?.reasoning_effort ?? null,
          executionEnvironment: session?.execution_environment ?? undefined,
          sessionKind:
            session?.parent_session_id || session?.relationship === "sub_agent"
              ? "subagent"
              : "primary",
        },
      });
      if (shouldQueue) {
        supervisor.upsertOptimisticQueuedMessage(id, posted);
      } else {
        supervisor.upsertOptimisticThreadMessage(id, posted);
      }
      try {
        await onDraftPersistNow?.();
      } catch {
        // best-effort
      }
    } catch (e: unknown) {
      if (shouldQueue) {
        supervisor.removeOptimisticQueuedMessage(id, messageId);
      } else {
        supervisor.removeOptimisticThreadMessage(id, messageId);
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
      await deleteMessage(id, messageId);
      supervisor.removeOptimisticQueuedMessage(id, messageId);
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
      await deleteMessage(id, mid);
      supervisor.removeOptimisticQueuedMessage(id, mid);
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
      await deleteMessage(id, mid);
      supervisor.removeOptimisticQueuedMessage(id, mid);
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
    supervisor.upsertOptimisticThreadMessage(id, optimisticMessage);
    setAtBottom(true);

    try {
      const posted = await postMessage(id, content, "immediate", attachments, {
        id: messageId,
        turn_id: turnId,
        analytics: {
          providerId: session?.provider_id ?? undefined,
          modelId: currentModelId || undefined,
          reasoningEffort: session?.reasoning_effort ?? null,
          executionEnvironment: session?.execution_environment ?? undefined,
          sessionKind:
            session?.parent_session_id || session?.relationship === "sub_agent"
              ? "subagent"
              : "primary",
        },
      });
      supervisor.upsertOptimisticThreadMessage(id, posted);
    } catch (e: unknown) {
      supervisor.removeOptimisticThreadMessage(id, messageId);
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

  const handleSetModelId = useSessionModelSwitcher({
    sessionId: id,
    supervisor,
    setModelSwitchError,
    setOptimisticModelId,
  });

  return (
    <SessionWorkbenchPane
      id={id}
      entryLoadState={entry?.loadState}
      entryError={entry?.error}
      session={session}
      sessionError={sessionError}
      sessionLoadIssues={hideSessionLoadIssuesBanner ? [] : sessionLoadIssues}
      dropActive={dropActive}
      dropScopeRef={dropScopeRef}
      listItems={listItems}
      liveTailItems={[]}
      events={baseEvents}
      messages={baseMessages}
      worktreeId={worktreeId}
      handleFileOpenError={handleFileOpenError}
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
      onRetrySessionLoads={handleRetrySessionLoads}
      subagentInvocations={subagentInvocations}
      onOpenChildSession={openChildSession}
      isActive={isActive}
      style={virtuosoStyle}
      itemIdentity={messageListItemIdentity}
      itemKey={messageListItemKey}
      increaseViewportBy={SCROLLBACK_INCREASE_VIEWPORT_BY_PX}
      initialData={messageListInitialData}
      initialLocation={messageListInitialLocation}
      threadProjectionOp={threadProjectionOp}
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
      onAttachmentError={setSendError}
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
