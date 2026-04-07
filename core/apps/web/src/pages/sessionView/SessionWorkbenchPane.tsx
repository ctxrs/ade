import {
  type CSSProperties,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
  useMemo,
} from "react";
import type {
  DataWithScrollModifier,
  ItemLocation,
  ListScrollLocation,
  ShortSizeAlign,
  VirtuosoMessageListMethods,
} from "@virtuoso.dev/message-list";
import {
  Message,
  Session,
  SessionEvent,
  submitAskUserQuestion,
  type MessageAttachment,
  type SubagentInvocation,
} from "../../api/client";
import { AskUserQuestionCard } from "../../components/AskUserQuestionCard";
import { DictationOnboardingModal } from "../../components/dictation/DictationOnboardingModal";
import {
  WorkbenchComposer as UnifiedWorkbenchComposer,
  type ContextWindowInfo,
  type WorkbenchModeId,
} from "../../components/WorkbenchComposer";
import {
  AssistantEntry,
  ThreadItemView,
  WorkbenchThoughtRow,
  WorkbenchToolGroupRow,
  WorkbenchToolRow,
  WorkbenchTurnHeaderView,
  WorkbenchTurnStatusRow,
} from "../sessionThread/SessionThreadItemViews";
import { HARNESS_CATALOG } from "../../utils/harnessCatalog";
import type {
  DictationOnboardingCloudDraft,
  DictationOnboardingState,
} from "../../utils/useDictationController";
import type { SlashCommandDescriptor } from "../../state/useComposerAutocomplete";
import type { SessionViewVerbosity } from "../../state/uiStateStore";
import { markdownToPlainText } from "../SessionPage.helpers";
import type { WorkbenchMessageListContext } from "../SessionPage.thread";
import type {
  AskUserQuestionAnswerState,
  ThreadItem,
  WorkbenchListItem,
} from "../SessionPage.types";
import { resolveWorkbenchMessageExpanded } from "../sessionMessageListItemIdentity";
import { SessionThreadPane } from "../sessionThread/SessionThreadPane";
import { ProviderGuardBanner } from "./ProviderGuardBanner";
import { SessionAuthBanner } from "./SessionAuthBanner";
import { SessionDebugPanel } from "./SessionDebugPanel";
import { SessionQueuePanel } from "./SessionQueuePanel";
import { SessionSubagentInvocationsCard } from "./SessionSubagentInvocationsCard";

type ProviderGuardNotice = {
  kind: string;
  stage?: string | null;
  provider?: string | null;
  pid?: number | null;
  killAtMs?: number | null;
  limitHighMb?: number | null;
  limitMaxMb?: number | null;
  memoryMb?: number | null;
  systemUsedMb?: number | null;
  systemTotalMb?: number | null;
  message?: string | null;
} | null;

type SessionErrorState = {
  provider?: string | null;
  message: string;
} | null;

type AuthUiState = {
  status: string;
  provider?: string | null;
  message?: string | null;
  methods: Array<{ id: string; name: string }>;
};

type SessionWorkbenchPaneProps = {
  id: string;
  entryLoadState: string | undefined;
  entryError: string | null | undefined;
  session: Session | null;
  sessionError: SessionErrorState;
  sessionLoadIssues: Array<{ key: "state" | "subagentInvocations"; message: string }>;
  dropActive: boolean;
  dropScopeRef: MutableRefObject<HTMLDivElement | null>;
  listItems: WorkbenchListItem[];
  liveTailItems: WorkbenchListItem[];
  events: SessionEvent[];
  messages: Message[];
  worktreeId: string | null;
  handleFileOpenError: (message: string | null) => void;
  activeAskToolCallId: string | null;
  expandedTurnHeaders: Record<string, boolean>;
  setExpandedTurnHeaders: Dispatch<SetStateAction<Record<string, boolean>>>;
  expandedTurnDetailsById: Record<string, boolean>;
  setExpandedTurnDetailsById: Dispatch<SetStateAction<Record<string, boolean>>>;
  expandedToolById: Record<string, boolean>;
  setExpandedToolById: Dispatch<SetStateAction<Record<string, boolean>>>;
  expandedMessageById: Record<string, boolean>;
  setExpandedMessageById: Dispatch<SetStateAction<Record<string, boolean>>>;
  turnToolsLoading: string[];
  verbosity: SessionViewVerbosity;
  setOptimisticAskAnswers: Dispatch<
    SetStateAction<Record<string, AskUserQuestionAnswerState>>
  >;
  onRequestTurnTools: (turnId: string) => void;
  showDebug: boolean;
  debugEvents: SessionEvent[];
  authUi: AuthUiState;
  authMethodId: string;
  onAuthMethodChange: (value: string) => void;
  authBusy: boolean;
  authError: string | null;
  onAuthenticate: () => Promise<void>;
  onRetrySessionLoads: () => void;
  subagentInvocations: SubagentInvocation[];
  onOpenChildSession: (childSessionId: string) => void;
  style: CSSProperties;
  itemIdentity: (item: WorkbenchListItem) => unknown;
  itemKey: (item: WorkbenchListItem) => string;
  increaseViewportBy: number;
  initialData: WorkbenchListItem[];
  initialLocation: ItemLocation;
  dataState?: DataWithScrollModifier<WorkbenchListItem>;
  context: WorkbenchMessageListContext;
  onScroll: (location: ListScrollLocation) => void;
  onRenderedDataChange: (range: WorkbenchListItem[]) => void;
  methodsRef: MutableRefObject<
    VirtuosoMessageListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null
  >;
  licenseKey: string;
  shortSizeAlign: ShortSizeAlign;
  queueForPanel: Message[];
  pendingQueueMessageIdSet: Set<string>;
  queueActionBusy: boolean;
  sendBusy: boolean;
  onSendQueuedNow: (message: Message) => Promise<void>;
  onEditQueued: (message: Message) => Promise<void>;
  onRemoveQueued: (messageId: string) => Promise<void>;
  input: string;
  setInput: (next: string) => void;
  slashCommands: SlashCommandDescriptor[];
  draftAttachments: MessageAttachment[];
  setDraftAttachments: Dispatch<SetStateAction<MessageAttachment[]>>;
  onAttachmentError: (message: string | null) => void;
  sendNow: () => Promise<void>;
  hasDraftContent: boolean;
  hasActiveTurn: boolean;
  atBottom: boolean;
  setVerbosityPref: (next: SessionViewVerbosity) => void;
  workbenchMode: WorkbenchModeId;
  setWorkbenchMode: (next: WorkbenchModeId) => void;
  contextWindow: ContextWindowInfo | null;
  dictationRecording: boolean;
  onToggleRecording: () => void;
  onInterruptSession: (() => Promise<void>) | null;
  sendError: string | null;
  fileOpenError: string | null;
  dictationDebugText: string | null;
  dictationError: string | null;
  dictationOnboarding: DictationOnboardingState | null;
  dismissDictationOnboarding: () => void;
  backDictationOnboarding: () => void;
  chooseDictationOnboardingLocal: () => void;
  chooseDictationOnboardingCloud: () => void;
  updateDictationOnboardingCloud: (
    patch: Partial<DictationOnboardingCloudDraft>,
  ) => void;
  submitDictationOnboardingCloud: () => Promise<void>;
  submitDictationOnboardingLocal: () => Promise<void>;
  providerGuardNotice: ProviderGuardNotice;
  providerGuardHeading: string;
  providerGuardMessage: string;
  providerGuardProviderLabel?: string;
  providerGuardPidLabel: string | null;
  providerGuardMemoryLimitMb?: number | null;
  providerGuardLimitLabel: string;
  providerGuardActionBusy: boolean;
  providerGuardActionError: string | null;
  canRaiseProviderGuard: boolean;
  onRaiseProviderGuardLimit: () => Promise<void>;
  onDisableProviderGuard: () => Promise<void>;
  formatMemoryMb: (value?: number | null) => string;
  availableModels: Array<{ id: string; name?: string }>;
  currentModelId: string;
  onSetModelId: (next: string) => Promise<void>;
  modelSwitchError: string | null;
};

export function SessionWorkbenchPane({
  id,
  entryLoadState,
  entryError,
  session,
  sessionError,
  sessionLoadIssues,
  dropActive,
  dropScopeRef,
  listItems,
  liveTailItems,
  events,
  messages,
  worktreeId,
  handleFileOpenError,
  activeAskToolCallId,
  expandedTurnHeaders,
  setExpandedTurnHeaders,
  expandedTurnDetailsById,
  setExpandedTurnDetailsById,
  expandedToolById,
  setExpandedToolById,
  expandedMessageById,
  setExpandedMessageById,
  turnToolsLoading,
  verbosity,
  setOptimisticAskAnswers,
  onRequestTurnTools,
  showDebug,
  debugEvents,
  authUi,
  authMethodId,
  onAuthMethodChange,
  authBusy,
  authError,
  onAuthenticate,
  onRetrySessionLoads,
  subagentInvocations,
  onOpenChildSession,
  style,
  itemIdentity,
  itemKey,
  increaseViewportBy,
  initialData,
  initialLocation,
  dataState,
  context,
  onScroll,
  onRenderedDataChange,
  methodsRef,
  licenseKey,
  shortSizeAlign,
  queueForPanel,
  pendingQueueMessageIdSet,
  queueActionBusy,
  sendBusy,
  onSendQueuedNow,
  onEditQueued,
  onRemoveQueued,
  input,
  setInput,
  slashCommands,
  draftAttachments,
  setDraftAttachments,
  onAttachmentError,
  sendNow,
  hasDraftContent,
  hasActiveTurn,
  atBottom,
  setVerbosityPref,
  workbenchMode,
  setWorkbenchMode,
  contextWindow,
  dictationRecording,
  onToggleRecording,
  onInterruptSession,
  sendError,
  fileOpenError,
  dictationDebugText,
  dictationError,
  dictationOnboarding,
  dismissDictationOnboarding,
  backDictationOnboarding,
  chooseDictationOnboardingLocal,
  chooseDictationOnboardingCloud,
  updateDictationOnboardingCloud,
  submitDictationOnboardingCloud,
  submitDictationOnboardingLocal,
  providerGuardNotice,
  providerGuardHeading,
  providerGuardMessage,
  providerGuardProviderLabel,
  providerGuardPidLabel,
  providerGuardMemoryLimitMb,
  providerGuardLimitLabel,
  providerGuardActionBusy,
  providerGuardActionError,
  canRaiseProviderGuard,
  onRaiseProviderGuardLimit,
  onDisableProviderGuard,
  formatMemoryMb,
  availableModels,
  currentModelId,
  onSetModelId,
  modelSwitchError,
}: SessionWorkbenchPaneProps) {
  const messageListContext = useMemo(
    () => ({
      ...context,
      expandedTurnHeaders,
      expandedTurnDetailsById,
      expandedToolById,
    }),
    [context, expandedToolById, expandedTurnDetailsById, expandedTurnHeaders],
  );
  const renderThreadItem = (item: ThreadItem) => {
    if (item.kind === "spacer") {
      return <div style={{ height: 1 }} />;
    }
    if (item.kind === "thought") {
      return <WorkbenchThoughtRow item={item} />;
    }
    if (item.kind === "turn_status") {
      return <WorkbenchTurnStatusRow item={item} />;
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
          onToggle={() => {
            setExpandedTurnDetailsById((prev) => ({
              ...prev,
              [item.turn_id]: !expanded,
            }));
          }}
          onRequestTools={() => onRequestTurnTools(item.turn_id)}
          onToggleTool={(toolId) => {
            setExpandedToolById((prev) => ({ ...prev, [toolId]: !prev[toolId] }));
          }}
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
          onToggle={() => {
            setExpandedToolById((prev) => ({ ...prev, [item.id]: !toolExpanded }));
          }}
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
        messageExpanded={
          item.kind === "message" ? resolveWorkbenchMessageExpanded(item, expandedMessageById) : undefined
        }
        onToggleMessageExpanded={
          item.kind === "message"
            ? (expanded) => {
                setExpandedMessageById((prev) => ({ ...prev, [item.id]: expanded }));
              }
            : undefined
        }
      />
    );
  };

  const workbenchItemContent = (_: number, item: WorkbenchListItem) => {
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
            onToggle={() => {
              setExpandedTurnHeaders((prev) => ({ ...prev, [header.id]: !expanded }));
            }}
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
  };
  const liveTailCount = liveTailItems.length;
  const totalVisibleThreadItems = listItems.length + liveTailCount;

  const harness = HARNESS_CATALOG.find((candidate) => candidate.id === (session?.provider_id ?? ""));

  return (
    <div
      className="wb-session-view ctx-drop-scope"
      ref={dropScopeRef}
      data-testid="session-view"
      data-session-id={id}
      data-thread-count={totalVisibleThreadItems}
    >
      {dropActive ? (
        <div className="ctx-drop-overlay" aria-hidden="true">
          <div className="ctx-drop-overlay-text">Drop image to attach</div>
        </div>
      ) : null}
      <div className="wb-session-left">
        {entryLoadState === "fatal" && entryError ? (
          <div className="banner">
            <span className="error">{entryError}</span>
          </div>
        ) : null}
        {sessionError ? (
          <div className="banner" role="alert">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <strong>Error</strong>
              {sessionError.provider ? <span className="muted">{sessionError.provider}</span> : null}
            </div>
            <div className="error" style={{ whiteSpace: "pre-wrap" }}>
              {sessionError.message}
            </div>
          </div>
        ) : null}
        {modelSwitchError ? (
          <div className="banner" role="alert">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <strong>Model Switch Failed</strong>
            </div>
            <div className="error" style={{ whiteSpace: "pre-wrap" }}>
              {modelSwitchError}
            </div>
          </div>
        ) : null}
        {sessionLoadIssues.length > 0 ? (
          <div className="banner wb-session-load-issues" role="alert" data-testid="workbench-session-load-issues">
            <div className="wb-session-load-issues-title">Some session details failed to load.</div>
            {sessionLoadIssues.map((issue) => (
              <div key={issue.key}>{issue.message}</div>
            ))}
            <div>
              <button type="button" onClick={onRetrySessionLoads}>Retry</button>
            </div>
          </div>
        ) : null}
        <ProviderGuardBanner
          heading={providerGuardHeading}
          message={providerGuardMessage}
          providerLabel={providerGuardProviderLabel}
          pidLabel={providerGuardPidLabel}
          memoryLabel={
            providerGuardNotice?.memoryMb != null
              ? `Memory ${formatMemoryMb(providerGuardNotice.memoryMb)}${
                  providerGuardMemoryLimitMb != null
                    ? ` / ${formatMemoryMb(providerGuardMemoryLimitMb)} (${providerGuardLimitLabel})`
                    : ""
                }`
              : null
          }
          systemLabel={
            providerGuardNotice?.systemUsedMb != null &&
            providerGuardNotice.systemTotalMb != null
              ? `System ${formatMemoryMb(providerGuardNotice.systemUsedMb)} / ${formatMemoryMb(providerGuardNotice.systemTotalMb)}`
              : null
          }
          notice={providerGuardNotice}
          actionBusy={providerGuardActionBusy}
          actionError={providerGuardActionError}
          canRaiseLimit={canRaiseProviderGuard}
          onRaiseLimit={onRaiseProviderGuardLimit}
          onDisableGuard={onDisableProviderGuard}
        />
        {showDebug ? (
          <div className="wb-muted" style={{ fontFamily: "var(--mono)" }}>
            debug: events={events.length} messages={messages.length} userMessages=
            {messages.filter((message) => message.role === "user").length} historyItems={listItems.length} liveItems=
            {liveTailCount}
          </div>
        ) : null}
        <SessionAuthBanner
          visible={authUi.status === "required" || authUi.status === "failed"}
          status={authUi.status}
          provider={authUi.provider ?? session?.provider_id}
          message={authUi.message}
          methods={authUi.methods}
          authMethodId={authMethodId}
          onAuthMethodChange={onAuthMethodChange}
          authBusy={authBusy}
          authError={authError}
          onAuthenticate={onAuthenticate}
        />
        <SessionSubagentInvocationsCard
          subagentInvocations={subagentInvocations}
          onOpenChildSession={onOpenChildSession}
        />
        {showDebug && debugEvents.length > 0 ? <SessionDebugPanel events={debugEvents} /> : null}

        <SessionThreadPane
          sessionId={id}
          style={style}
          initialData={initialData}
          itemContent={workbenchItemContent}
          itemIdentity={itemIdentity}
          itemKey={itemKey}
          increaseViewportBy={increaseViewportBy}
          initialLocation={initialLocation}
          dataState={dataState}
          context={messageListContext}
          onScroll={onScroll}
          onRenderedDataChange={onRenderedDataChange}
          methodsRef={methodsRef}
          licenseKey={licenseKey}
          shortSizeAlign={shortSizeAlign}
        >
          {liveTailCount > 0 ? (
            <div className="wb-thread-live-tail" role="list" aria-label="Live turn">
              {liveTailItems.map((item, index) => (
                <div key={item.id} role="listitem" className="wb-thread-live-tail-row" data-thread-item-id={item.id}>
                  {workbenchItemContent(index, item)}
                </div>
              ))}
            </div>
          ) : null}
          <SessionQueuePanel
            queue={queueForPanel}
            pendingQueueMessageIdSet={pendingQueueMessageIdSet}
            queueActionBusy={queueActionBusy}
            sendBusy={sendBusy}
            onSendQueuedNow={onSendQueuedNow}
            onEditQueued={onEditQueued}
            onRemoveQueued={onRemoveQueued}
          />
          <UnifiedWorkbenchComposer
            variant="activeSession"
            value={input}
            setValue={setInput}
            placeholder="@ for context, / for commands"
            inputDisabled={dictationRecording}
            sessionIdForAutocomplete={id}
            slashCommands={slashCommands}
            attachments={draftAttachments}
            setAttachments={setDraftAttachments}
            onAttachmentError={onAttachmentError}
            onSend={sendNow}
            sendDisabled={sendBusy || !hasDraftContent}
            sendDisabledReason={
              sendBusy ? "Sending..." : !hasDraftContent ? "Enter a message." : null
            }
            onInterrupt={onInterruptSession}
            isWorking={hasActiveTurn}
            verbosity={verbosity}
            onSetVerbosity={setVerbosityPref}
            modeId={workbenchMode}
            setModeId={setWorkbenchMode}
            contextWindow={contextWindow}
            recording={dictationRecording}
            onToggleRecording={onToggleRecording}
            harnessLabel={harness?.label ?? (session?.provider_id ?? "Provider")}
            harnessLogoSrc={harness?.logoSrc}
            harnessLogoInvert={harness?.invertInDark}
            harnessLogoInvertInLight={harness?.invertInLight}
            availableModels={availableModels}
            currentModelId={currentModelId}
            onSetModelId={onSetModelId}
          />
          {sendError ? <div className="wb-banner">{sendError}</div> : null}
          {fileOpenError ? <div className="wb-banner">{fileOpenError}</div> : null}
          {dictationDebugText ? <div className="wb-banner">{dictationDebugText}</div> : null}
          {dictationError ? <div className="wb-banner">{dictationError}</div> : null}
          <DictationOnboardingModal
            state={dictationOnboarding}
            onClose={dismissDictationOnboarding}
            onBack={backDictationOnboarding}
            onChooseLocal={chooseDictationOnboardingLocal}
            onChooseCloud={chooseDictationOnboardingCloud}
            onCloudChange={updateDictationOnboardingCloud}
            onSubmitCloud={() => {
              void submitDictationOnboardingCloud();
            }}
            onSubmitLocal={() => {
              void submitDictationOnboardingLocal();
            }}
          />
        </SessionThreadPane>

        <div className="sr-only" aria-live="polite">
          {session && (atBottom ? "Agent output updating." : "New agent activity.")}
        </div>
      </div>
    </div>
  );
}
