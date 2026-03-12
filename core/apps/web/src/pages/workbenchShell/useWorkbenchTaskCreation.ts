import { useEffect, useMemo, useRef, useState, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { flushSync } from "react-dom";
import {
  createSession,
  createTask,
  getWorkspaceExecutionConfig,
  idToString,
  postMessage,
  type ExecutionEnvironment,
  type Message,
  MessageAttachment,
  type ProviderOptions,
  type ProviderStatus,
  type Session,
  type SessionSnapshotSummary,
  type SessionTurn,
  type Task,
} from "../../api/client";
import type { DraftHarness, ProviderAuthSummaryTrigger, WorkbenchModeId } from "../../components/WorkbenchComposer";
import type { SessionSupervisor } from "../../state/sessionSupervisor";
import { errorMessage } from "../../utils/errorMessage";
import { randomUuid } from "../../utils/randomUuid";
import type { WorkbenchStore } from "../../workbench/store";
import type { OptimisticFocus, OptimisticTaskSummary } from "../WorkbenchPage.types";
import { deriveTaskTitle, modelIdsFromOptions } from "../WorkbenchPage.utils";
import { buildOptimisticUserMessage } from "../SessionPage.optimisticMessage";

type UseWorkbenchTaskCreationArgs = {
  workspaceId: string;
  draftPrompt: string;
  setNewTaskDraft: (value: { text: string; modeId: WorkbenchModeId }) => void;
  draftAttachments: MessageAttachment[];
  setDraftAttachments: Dispatch<SetStateAction<MessageAttachment[]>>;
  draftHarness: DraftHarness | null;
  providersById: Record<string, ProviderStatus | undefined>;
  providerOptions: Record<string, ProviderOptions | undefined>;
  ensureProviderAuthSummary: (
    providerId: string,
    opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger },
  ) => Promise<ProviderOptions | undefined>;
  dictationRecording: boolean;
  stopDictation: (opts?: { awaitFinal?: boolean }) => Promise<string>;
  focusTask: (taskId: string, sessionId?: string | null) => void;
  workbenchStore: Pick<WorkbenchStore, "getNavToken" | "flushDraft">;
  optimisticStartingTaskRef: MutableRefObject<OptimisticTaskSummary | null>;
  setOptimisticTasks: Dispatch<SetStateAction<OptimisticTaskSummary[]>>;
  setOptimisticFocus: Dispatch<SetStateAction<OptimisticFocus | null>>;
  supervisor: Pick<SessionSupervisor, "setSession" | "setTurns" | "setMessages">;
  newTaskDraftKey: string;
  onStartError: (message: string | null) => void;
};

export function useWorkbenchTaskCreation({
  workspaceId,
  draftPrompt,
  setNewTaskDraft,
  draftAttachments,
  setDraftAttachments,
  draftHarness,
  providersById,
  providerOptions,
  ensureProviderAuthSummary,
  dictationRecording,
  stopDictation,
  focusTask,
  workbenchStore,
  optimisticStartingTaskRef,
  setOptimisticTasks,
  setOptimisticFocus,
  supervisor,
  newTaskDraftKey,
  onStartError,
}: UseWorkbenchTaskCreationArgs) {
  const [startBusy, setStartBusy] = useState(false);
  const lastDraftProviderIdRef = useRef<string | null>(draftHarness?.providerId ?? null);

  const startBlockedReason = useMemo(() => {
    if (draftPrompt.trim().length === 0) return "Enter a prompt to start.";
    if (startBusy) return "Starting…";
    if (!draftHarness) return "Select a harness to start.";
    const missing =
      !(providersById[draftHarness.providerId]?.installed === true
        && providersById[draftHarness.providerId]?.health === "ok");
    if (missing) {
      const diag = providersById[draftHarness.providerId]?.diagnostics?.[0];
      return diag
        ? `Harness “${draftHarness.providerId}” unavailable: ${diag}`
        : `Harness “${draftHarness.providerId}” unavailable.`;
    }
    return null;
  }, [draftHarness, draftPrompt, startBusy, providersById]);

  useEffect(() => {
    const nextProviderId = draftHarness?.providerId ?? null;
    if (lastDraftProviderIdRef.current === nextProviderId) return;
    lastDraftProviderIdRef.current = nextProviderId;
    onStartError(null);
  }, [draftHarness?.providerId, onStartError]);

  const resolveSessionModelId = async (
    providerId: string,
    selectedModelId: string | null | undefined,
  ): Promise<string> => {
    const explicitModelId = selectedModelId?.trim();
    if (explicitModelId) return explicitModelId;

    const cachedModelId = modelIdsFromOptions(providerOptions[providerId])[0];
    if (cachedModelId) return cachedModelId;

    let refreshedOptions: ProviderOptions | undefined;
    try {
      refreshedOptions = await ensureProviderAuthSummary(providerId, {
        force: true,
        trigger: "explicit",
      });
    } catch (e: unknown) {
      throw new Error(`Failed to load models for harness “${providerId}”: ${errorMessage(e)}`);
    }

    const refreshedModelId = modelIdsFromOptions(refreshedOptions ?? providerOptions[providerId])[0];
    if (refreshedModelId) return refreshedModelId;

    throw new Error(`Harness “${providerId}” did not provide a model. Refresh provider settings and try again.`);
  };

  const startNewTask = async () => {
    if (!workspaceId) return;
    const prompt = (dictationRecording ? await stopDictation({ awaitFinal: true }) : draftPrompt).trim();
    if (!prompt) return;
    if (startBusy) return;
    if (startBlockedReason && !startBlockedReason.startsWith("Starting")) {
      onStartError(startBlockedReason);
      return;
    }
    setStartBusy(true);
    onStartError(null);

    const nowIso = new Date().toISOString();
    const title = deriveTaskTitle(prompt);
    const attachmentsToSend = draftAttachments.slice();
    const primaryTrack = draftHarness;
    if (!primaryTrack) {
      setStartBusy(false);
      onStartError("Select a harness to start.");
      return;
    }
    let resolvedModelId: string;
    try {
      resolvedModelId = await resolveSessionModelId(primaryTrack.providerId, primaryTrack.modelId);
    } catch (e: unknown) {
      setStartBusy(false);
      onStartError(errorMessage(e));
      return;
    }
    const optimisticTaskId = randomUuid();
    const optimisticSessionId = randomUuid();
    const optimisticMessageId = randomUuid();
    const optimisticTurnId = randomUuid();
    let executionEnvironment: ExecutionEnvironment;
    try {
      executionEnvironment = (await getWorkspaceExecutionConfig(workspaceId)).environment;
    } catch (e: unknown) {
      setStartBusy(false);
      onStartError(errorMessage(e));
      return;
    }

    const optimisticTask: Task = {
      id: optimisticTaskId,
      workspace_id: workspaceId,
      title,
      status: "running",
      primary_session_id: optimisticSessionId,
      created_at: nowIso,
      updated_at: nowIso,
      last_activity_at: nowIso,
      has_active_session: true,
    };

    const optimisticSession: Session = {
      id: optimisticSessionId,
      task_id: optimisticTaskId,
      workspace_id: workspaceId,
      worktree_id: "",
      provider_id: primaryTrack.providerId,
      model_id: resolvedModelId,
      title: "Session 1",
      agent_role: "assistant",
      status: "starting",
      execution_environment: executionEnvironment,
      created_at: nowIso,
      updated_at: nowIso,
    };

    const optimisticSummary: SessionSnapshotSummary = {
      session: optimisticSession,
      last_message_at: nowIso,
      last_message_preview: prompt.slice(0, 160),
      activity: { is_working: true, last_turn_status: "running" },
      unread: false,
    };

    const optimisticItem: OptimisticTaskSummary = {
      id: optimisticTaskId,
      task: optimisticTask,
      sessions: [optimisticSummary],
      primarySessionHead: null,
      primarySessionId: optimisticSessionId,
      sort_at: nowIso,
      sortAtMs: Date.parse(nowIso) || Date.now(),
      providerIds: [primaryTrack.providerId],
      localStatus: "starting",
      localPrompt: prompt,
      localMessageId: optimisticMessageId,
    };

    const optimisticMessage: Message = buildOptimisticUserMessage({
      messageId: optimisticMessageId,
      sessionId: optimisticSessionId,
      taskId: optimisticTaskId,
      turnId: optimisticTurnId,
      content: prompt,
      attachments: attachmentsToSend,
      delivery: "immediate",
      createdAt: nowIso,
    });
    const optimisticTurn: SessionTurn = {
      turn_id: optimisticTurnId,
      session_id: optimisticSessionId,
      run_id: null,
      user_message_id: optimisticMessageId,
      status: "running",
      start_seq: null,
      end_seq: null,
      started_at: nowIso,
      updated_at: nowIso,
      assistant_partial: null,
      thought_partial: null,
      metrics_json: null,
      tool_total: 0,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 0,
      tool_failed: 0,
    };

    flushSync(() => {
      optimisticStartingTaskRef.current = optimisticItem;
      setOptimisticTasks((prev) => [optimisticItem, ...prev]);
      focusTask(optimisticTaskId, optimisticSessionId);
      setOptimisticFocus({
        taskId: optimisticTaskId,
        sessionId: optimisticSessionId,
        navToken: workbenchStore.getNavToken(),
      });
      supervisor.setSession(optimisticSession);
      supervisor.setTurns(optimisticSessionId, [optimisticTurn], { replace: true });
      supervisor.setMessages(optimisticSessionId, [optimisticMessage], { replace: true });
    });

    setNewTaskDraft({ text: "", modeId: "default" });
    await workbenchStore.flushDraft(newTaskDraftKey);
    setDraftAttachments([]);

    let currentTaskId = optimisticTaskId;
    let primaryMessagePosted = false;

    try {
      const task = await createTask(workspaceId, title, undefined, {
        create_default_session: false,
        id: optimisticTaskId,
      });
      const taskId = idToString(task.id);
      if (!taskId) throw new Error("Task creation failed.");

      if (taskId !== optimisticTaskId) {
        throw new Error("Task creation returned an unexpected id.");
      }

      currentTaskId = taskId;
      setOptimisticTasks((prev) =>
        prev.map((item) => {
          if (item.id !== currentTaskId) return item;
          const nextTask: Task = { ...task, primary_session_id: item.primarySessionId ?? null };
          const nextSessions = item.sessions.map((summary) => ({
            ...summary,
            session: {
              ...summary.session,
              task_id: currentTaskId,
              workspace_id: task.workspace_id ?? summary.session.workspace_id,
            },
          }));
          return {
            ...item,
            task: nextTask,
            sessions: nextSessions,
            sort_at: task.created_at ?? item.sort_at,
            sortAtMs: Date.parse(task.created_at ?? item.sort_at ?? "") || item.sortAtMs,
          };
        }),
      );

      const installed =
        providersById[primaryTrack.providerId]?.installed === true
        && providersById[primaryTrack.providerId]?.health === "ok";
      if (!installed) {
        const diag = providersById[primaryTrack.providerId]?.diagnostics?.[0];
        throw new Error(
          diag
            ? `Harness “${primaryTrack.providerId}” unavailable: ${diag}`
            : `Harness “${primaryTrack.providerId}” unavailable.`,
        );
      }
      const clientSessionId = optimisticSessionId;
      const messageId = optimisticMessageId;
      const turnId = optimisticTurnId;
      const shouldSendInitialPrompt = attachmentsToSend.length === 0;
      const session = await createSession(currentTaskId, primaryTrack.providerId, resolvedModelId, {
        execution_environment: executionEnvironment,
        id: clientSessionId,
        initial_message_id: messageId,
        initial_turn_id: turnId,
        ...(shouldSendInitialPrompt ? { initial_prompt: prompt } : {}),
      });
      const sessionId = idToString(session.id);
      if (!sessionId) throw new Error("Session creation failed.");
      if (sessionId !== clientSessionId) {
        throw new Error("Session creation returned an unexpected id.");
      }
      supervisor.setSession(session);
      setOptimisticTasks((prev) =>
        prev.map((item) => {
          if (item.id !== currentTaskId) return item;
          const nextSessions = item.sessions.map((summary) => {
            if (idToString(summary.session.id) !== sessionId) return summary;
            const nextSummary: SessionSnapshotSummary = {
              ...summary,
              session,
              last_message_at: nowIso,
              last_message_preview: summary.last_message_preview ?? prompt.slice(0, 160),
              activity: { is_working: true, last_turn_status: "running" },
            };
            return nextSummary;
          });
          return {
            ...item,
            sessions: nextSessions,
            primarySessionId: sessionId,
            task: { ...item.task, primary_session_id: sessionId },
          };
        }),
      );

      if (shouldSendInitialPrompt) {
        primaryMessagePosted = true;
      } else {
        const posted = await postMessage(sessionId, prompt, "immediate", attachmentsToSend, {
          id: messageId,
          turn_id: turnId,
        });
        supervisor.setMessages(sessionId, [posted]);
        primaryMessagePosted = true;
      }
      setOptimisticTasks((prev) =>
        prev.map((item) =>
          item.id === currentTaskId && item.localStatus === "starting"
            ? { ...item, localStatus: "synced" }
            : item,
        ),
      );

      if (!primaryMessagePosted) {
        throw new Error("Failed to start the first session.");
      }
    } catch (e: unknown) {
      const message = errorMessage(e);
      setOptimisticTasks((prev) =>
        prev.map((item) =>
          item.id === currentTaskId ? { ...item, localStatus: "failed", localError: message } : item,
        ),
      );
      onStartError(message);
    } finally {
      setStartBusy(false);
    }
  };

  return {
    startBusy,
    startBlockedReason,
    startNewTask,
  };
}
