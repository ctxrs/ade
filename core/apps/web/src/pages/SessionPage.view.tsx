import {
  type SetStateAction,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Message,
  type MessageAttachment,
  Session,
  SessionEvent,
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
import type { AskUserQuestionAnswerState, WorkbenchListItem } from "./SessionPage.types";
import {
  buildPendingTurns,
  deriveAuthUi,
  deriveMessagesKey,
  deriveProviderGuardNotice,
  deriveSessionError,
  deriveTurnsKey,
  filterTurnsForQueuedMessages,
  normalizeContextWindowMetrics,
} from "./SessionPage.workbenchViewModel";
import { useSessionMessageListController } from "./useSessionMessageListController";
import { useSessionComposerQueueController } from "./useSessionComposerQueueController";
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
import { SessionWorkbenchPane } from "./sessionView/SessionWorkbenchPane";
import { useSessionImageDropScope } from "./sessionView/useSessionImageDropScope";
import { useSessionProviderGuard } from "./sessionView/useSessionProviderGuard";
import { useSharedSessionProviderOptions } from "./sessionView/useSharedSessionProviderOptions";
import { useStableAskUserQuestionAnswers } from "./sessionView/useStableAskUserQuestionAnswers";
import { composeModelId, parseModelId } from "../utils/modelEffort";

const SCROLLBACK_INCREASE_VIEWPORT_BY_PX = 240;

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
    setDraftAttachmentsInternal([]);
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
  const hasActiveTurn = useMemo(
    () => hasSessionActiveTurn(entry?.activity),
    [entry?.activity],
  );
  const sessionIsAuthoritative = entry?.freshness === "authoritative";
  const queuedMessagesEnabled = useFeatureGate("queued_messages_enabled", false);
  const resolveSendText = useCallback(async () => {
    const text = dictationRecording ? await stopDictation({ awaitFinal: true }) : input;
    return text.trim();
  }, [dictationRecording, input, stopDictation]);
  const {
    sendBusy,
    sendError,
    queueActionBusy,
    queueForPanel,
    displayMessages,
    pendingQueueMessageIdSet,
    queuedMessageIdsForThread,
    sendNow,
    onRemoveQueued,
    onEditQueued,
    onSendQueuedNow,
  } = useSessionComposerQueueController({
    sessionId: id,
    session,
    input,
    setInput,
    draftAttachments,
    setDraftAttachments,
    messages,
    messagesKey,
    queue,
    turns,
    turnsKey,
    hasActiveTurn,
    queuedMessagesEnabled,
    sessionIsAuthoritative,
    resolveSendText,
    setAtBottom,
    onDraftPersistNow,
  });
  const displayMessagesKey = useMemo(() => deriveMessagesKey(displayMessages), [displayMessages]);
  const pendingTurns = useMemo(
    () => buildPendingTurns(turns, displayMessages),
    [displayMessages, displayMessagesKey, turns, turnsKey],
  );
  const displayTurns = useMemo(
    () => (pendingTurns.length > 0 ? [...turns, ...pendingTurns] : turns),
    [pendingTurns, turns, turnsKey],
  );
  const displayTurnsKey = useMemo(() => deriveTurnsKey(displayTurns), [displayTurns]);
  const coalescedEvents = useRafCoalesced(events);
  const coalescedEventsStamp = useRafCoalesced(eventsStamp);
  const coalescedDisplayMessages = useRafCoalesced(displayMessages);
  const coalescedDisplayMessagesKey = useRafCoalesced(displayMessagesKey);
  const coalescedDisplayTurns = useRafCoalesced(displayTurns);
  const coalescedDisplayTurnsKey = useRafCoalesced(displayTurnsKey);
  const displayTurnsForThread = useMemo(
    () => filterTurnsForQueuedMessages(coalescedDisplayTurns, queuedMessageIdsForThread),
    [coalescedDisplayTurns, coalescedDisplayTurnsKey, queuedMessageIdsForThread],
  );
  const displayTurnsForThreadKey = useMemo(
    () => deriveTurnsKey(displayTurnsForThread),
    [displayTurnsForThread],
  );
  const displayTurnsForThreadStamp = `${turnsRev}:${displayTurnsForThreadKey}`;
  const coalescedDisplayMessagesStamp = `${messagesRev}:${coalescedDisplayMessagesKey}`;
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
    turnsStamp: displayTurnsForThreadStamp,
    messagesStamp: coalescedDisplayMessagesStamp,
    eventsStamp: coalescedEventsStamp,
    verbosity,
    turns: displayTurnsForThread,
    messages: coalescedDisplayMessages,
    events: coalescedEvents,
    toolsByTurnId: turnToolsByTurnId,
    toolSummariesReady,
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
    const fromAcp = buildModelsFromProviderOptions({ models: entry?.acpModels } as ProviderOptions);
    if (fromAcp.length > 0) return fromAcp;
    return buildModelsFromProviderOptions(sharedProviderOptions);
  }, [entry?.acpModels, session?.model_id, session?.reasoning_effort, sharedProviderOptions]);
  const currentModelId = useMemo(() => {
    return composeModelId(
      String(session?.model_id ?? ""),
      session?.reasoning_effort ?? null,
    );
  }, [session?.model_id, session?.reasoning_effort]);
  const displayedModelId = optimisticModelId ?? currentModelId;

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
