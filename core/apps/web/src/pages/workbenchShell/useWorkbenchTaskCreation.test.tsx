import React, { useEffect, useMemo, useRef, useState } from "react";
import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageAttachment, ProviderOptions, ProviderStatus, Session, Task } from "../../api/client";
import { createSession, createTask, getWorkspaceExecutionConfig, postMessage } from "../../api/client";
import type { DraftHarness, ProviderAuthSummaryTrigger } from "../../components/WorkbenchComposer";
import type { SessionSupervisor } from "../../state/sessionSupervisor";
import type { WorkspaceActiveSnapshotItem } from "../../state/workspaceActiveSnapshotStore";
import type { WorkbenchStore } from "../../workbench/store";
import type { OptimisticFocus } from "../WorkbenchPage.types";
import { useWorkbenchOptimisticTasks } from "./useWorkbenchOptimisticTasks";
import { useWorkbenchTaskCreation } from "./useWorkbenchTaskCreation";

vi.mock("../../api/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../api/client")>();
  return {
    ...original,
    createTask: vi.fn(),
    createSession: vi.fn(),
    getWorkspaceExecutionConfig: vi.fn(),
    postMessage: vi.fn(),
  };
});

vi.mock("../../utils/randomUuid", () => ({
  randomUuid: vi.fn(),
}));

const mockedCreateTask = vi.mocked(createTask);
const mockedCreateSession = vi.mocked(createSession);
const mockedGetWorkspaceExecutionConfig = vi.mocked(getWorkspaceExecutionConfig);
const mockedPostMessage = vi.mocked(postMessage);

const now = "2026-03-10T00:00:00.000Z";

type FlowValue = {
  startNewTask: () => Promise<void>;
  startBusy: boolean;
  activeTaskId: string | null;
  activeTaskSummary: ReturnType<typeof useWorkbenchOptimisticTasks>["activeTaskSummary"];
  optimisticTasks: ReturnType<typeof useWorkbenchOptimisticTasks>["optimisticTasks"];
  optimisticFailureBySessionId: ReturnType<typeof useWorkbenchOptimisticTasks>["optimisticFailureBySessionId"];
  optimisticStartingTaskRef: ReturnType<typeof useWorkbenchOptimisticTasks>["optimisticStartingTaskRef"];
  optimisticFocus: OptimisticFocus | null;
};

function makeProviderOptions(overrides: Partial<ProviderOptions> = {}): ProviderOptions {
  return {
    provider_id: "codex",
    workspace_id: "workspace-1",
    supports_load: false,
    auth_required: false,
    has_active_auth: true,
    auth_mode: "subscription",
    probed_at: now,
    ...overrides,
  };
}

function makeProviderStatus(overrides: Partial<ProviderStatus> = {}): ProviderStatus {
  return {
    provider_id: "codex",
    installed: true,
    health: "ok",
    diagnostics: [],
    details: {},
    usability: {
      usable: true,
      status: "ready",
      blocking_provider_ids: [],
      recommended_action: "none",
    },
    ...overrides,
  };
}

function makeTask(taskId: string, sessionId: string): Task {
  return {
    id: taskId,
    workspace_id: "workspace-1",
    title: "Write docs",
    status: "running",
    created_at: now,
    updated_at: now,
    last_activity_at: now,
    primary_session_id: sessionId,
    has_active_session: true,
  };
}

function makeSession(sessionId: string, taskId: string): Session {
  return {
    id: sessionId,
    task_id: taskId,
    workspace_id: "workspace-1",
    worktree_id: "",
    provider_id: "codex",
    model_id: "gpt-5",
    title: "Session 1",
    agent_role: "assistant",
    status: "starting",
    execution_environment: "container_disk_isolated",
    created_at: now,
    updated_at: now,
  };
}

function requireValue(value: FlowValue | null): FlowValue {
  if (!value) throw new Error("flow value not ready");
  return value;
}

function Harness({
  prompt = "Write docs",
  onChange,
  onStartError,
  draftHarness = { providerId: "codex", modelId: "gpt-5" },
  initialAttachments,
  providerOptionsById,
  providersByIdProp,
  ensureProviderAuthSummary,
}: {
  prompt?: string;
  onChange: (value: FlowValue) => void;
  onStartError: (message: string | null) => void;
  draftHarness?: DraftHarness | null;
  initialAttachments?: MessageAttachment[];
  providerOptionsById?: Record<string, ProviderOptions>;
  providersByIdProp?: Record<string, ProviderStatus>;
  ensureProviderAuthSummary?: (
    providerId: string,
    opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger },
  ) => Promise<ProviderOptions | undefined>;
}) {
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const [activeTaskIdFromTab, setActiveTaskIdFromTab] = useState<string | null>(null);
  const [tasksById] = useState<Record<string, WorkspaceActiveSnapshotItem>>({});
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>(() => initialAttachments ?? []);
  const [optimisticFocus, setOptimisticFocus] = useState<OptimisticFocus | null>(null);
  const optimistic = useWorkbenchOptimisticTasks({
    activeTaskId,
    activeTaskIdFromTab,
    tasksById,
  });

  const resolvedProviderOptions = useMemo<Record<string, ProviderOptions>>(
    () => providerOptionsById ?? { codex: makeProviderOptions() },
    [providerOptionsById],
  );
  const resolvedProvidersById = useMemo<Record<string, ProviderStatus>>(
    () => providersByIdProp ?? { codex: makeProviderStatus() },
    [providersByIdProp],
  );

  const supervisor = useMemo<Pick<SessionSupervisor, "setSession" | "setTurns" | "setMessages">>(
    () => ({
      setSession: vi.fn(),
      setTurns: vi.fn(),
      setMessages: vi.fn(),
    }),
    [],
  );
  const workbenchStore = useMemo<Pick<WorkbenchStore, "getNavToken" | "flushDraft">>(
    () => ({
      getNavToken: () => 7,
      flushDraft: vi.fn(async () => {}),
    }),
    [],
  );
  const setNewTaskDraft = useRef<
    (value: { text: string; modeId: "default" | "research" | "plan" | "review" }) => void
  >(() => undefined);

  const { startBusy, startNewTask } = useWorkbenchTaskCreation({
    workspaceId: "workspace-1",
    draftPrompt: prompt,
    setNewTaskDraft: setNewTaskDraft.current,
    draftAttachments,
    setDraftAttachments,
    draftHarness,
    providersById: resolvedProvidersById,
    providerOptions: resolvedProviderOptions,
    ensureProviderAuthSummary:
      ensureProviderAuthSummary
      ?? (async (providerId) => resolvedProviderOptions[providerId]),
    dictationRecording: false,
    stopDictation: async () => prompt,
    focusTask: (taskId) => {
      setActiveTaskId(taskId);
      setActiveTaskIdFromTab(taskId);
    },
    workbenchStore,
    optimisticStartingTaskRef: optimistic.optimisticStartingTaskRef,
    setOptimisticTasks: optimistic.setOptimisticTasks,
    setOptimisticFocus,
    supervisor,
    newTaskDraftKey: "draft-1",
    onStartError,
  });

  useEffect(() => {
    onChange({
      startNewTask,
      startBusy,
      activeTaskId,
      activeTaskSummary: optimistic.activeTaskSummary,
      optimisticTasks: optimistic.optimisticTasks,
      optimisticFailureBySessionId: optimistic.optimisticFailureBySessionId,
      optimisticStartingTaskRef: optimistic.optimisticStartingTaskRef,
      optimisticFocus,
    });
  }, [
    activeTaskId,
    onChange,
    optimistic.activeTaskSummary,
    optimistic.optimisticFailureBySessionId,
    optimistic.optimisticStartingTaskRef,
    optimistic.optimisticTasks,
    optimisticFocus,
    startBusy,
    startNewTask,
  ]);

  return null;
}

beforeEach(async () => {
  vi.clearAllMocks();
  mockedGetWorkspaceExecutionConfig.mockResolvedValue({
    source: "workspace",
    environment: "container_disk_isolated",
  });
  const { randomUuid } = await import("../../utils/randomUuid");
  vi.mocked(randomUuid).mockImplementationOnce(() => "task-1");
  vi.mocked(randomUuid).mockImplementationOnce(() => "session-1");
  vi.mocked(randomUuid).mockImplementationOnce(() => "message-1");
  vi.mocked(randomUuid).mockImplementationOnce(() => "turn-1");
});

afterEach(() => {
  document.body.innerHTML = "";
});

describe("useWorkbenchTaskCreation optimistic lifecycle", () => {
  it("reconciles a successful start to synced optimistic state", async () => {
    let current: FlowValue | null = null;
    mockedCreateTask.mockResolvedValue(makeTask("task-1", "session-1"));
    mockedCreateSession.mockResolvedValue(makeSession("session-1", "task-1"));
    const onStartError = vi.fn();

    render(
      <Harness
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    await waitFor(() => {
      expect(requireValue(current).optimisticTasks[0]?.localStatus).toBe("synced");
    });
    expect(requireValue(current).activeTaskId).toBe("task-1");
    expect(requireValue(current).optimisticFocus).toMatchObject({
      taskId: "task-1",
      sessionId: "session-1",
      navToken: 7,
    });
    expect(requireValue(current).optimisticStartingTaskRef.current).toBeNull();
    expect(mockedCreateTask).toHaveBeenCalledWith("workspace-1", "New Task", undefined, {
      create_default_session: false,
      id: "task-1",
    });
    expect(mockedGetWorkspaceExecutionConfig).toHaveBeenCalledWith("workspace-1");
    expect(mockedCreateSession).toHaveBeenCalledWith("task-1", "codex", "gpt-5", expect.objectContaining({
      execution_environment: "container_disk_isolated",
    }));
    expect(mockedPostMessage).not.toHaveBeenCalled();
    expect(onStartError).toHaveBeenCalledWith(null);
  });

  it("posts the first message separately when the new-task draft includes attachments", async () => {
    let current: FlowValue | null = null;
    mockedCreateTask.mockResolvedValue(makeTask("task-1", "session-1"));
    mockedCreateSession.mockResolvedValue(makeSession("session-1", "task-1"));
    mockedPostMessage.mockResolvedValue({
      id: "message-1",
      task_id: "task-1",
      session_id: "session-1",
      role: "user",
      content: "Write docs",
      attachments: [
        {
          kind: "image",
          mime_type: "image/png",
          name: "drop.png",
          data_base64: "abc123",
        },
      ],
      delivery: "immediate",
      created_at: now,
      turn_id: "turn-1",
      order_seq: 1,
    });
    const onStartError = vi.fn();
    const attachments: MessageAttachment[] = [
      {
        kind: "image",
        mime_type: "image/png",
        name: "drop.png",
        data_base64: "abc123",
      },
    ];

    render(
      <Harness
        initialAttachments={attachments}
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    await waitFor(() => {
      expect(requireValue(current).optimisticTasks[0]?.localStatus).toBe("synced");
    });
    expect(mockedCreateSession).toHaveBeenCalledWith("task-1", "codex", "gpt-5", expect.objectContaining({
      execution_environment: "container_disk_isolated",
      initial_message_id: "message-1",
      initial_turn_id: "turn-1",
    }));
    expect(mockedCreateSession.mock.calls[0]?.[3]).not.toHaveProperty("initial_prompt");
    expect(mockedPostMessage).toHaveBeenCalledWith("session-1", "Write docs", "immediate", attachments, {
      id: "message-1",
      turn_id: "turn-1",
    });
    expect(onStartError).toHaveBeenCalledWith(null);
  });

  it("splits a combined draft model id into model_id and reasoning_effort for session creation", async () => {
    let current: FlowValue | null = null;
    mockedCreateTask.mockResolvedValue(makeTask("task-1", "session-1"));
    mockedCreateSession.mockResolvedValue({
      ...makeSession("session-1", "task-1"),
      model_id: "gpt-5",
      reasoning_effort: "xhigh",
    });
    const onStartError = vi.fn();

    render(
      <Harness
        draftHarness={{ providerId: "codex", modelId: "gpt-5/xhigh" }}
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    await waitFor(() => {
      expect(requireValue(current).optimisticTasks[0]?.localStatus).toBe("synced");
    });
    expect(mockedCreateSession).toHaveBeenCalledWith("task-1", "codex", "gpt-5", expect.objectContaining({
      execution_environment: "container_disk_isolated",
      reasoning_effort: "xhigh",
    }));
    expect(onStartError).toHaveBeenCalledWith(null);
  });

  it("keeps a failed optimistic task visible with failure metadata", async () => {
    let current: FlowValue | null = null;
    mockedCreateTask.mockRejectedValue(new Error("task create failed"));
    const onStartError = vi.fn();

    render(
      <Harness
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    await waitFor(() => {
      expect(requireValue(current).optimisticTasks[0]?.localStatus).toBe("failed");
    });
    expect(requireValue(current).optimisticTasks[0]).toMatchObject({
      id: "task-1",
      primarySessionId: "session-1",
      localStatus: "failed",
      localError: "task create failed",
    });
    expect(requireValue(current).optimisticFailureBySessionId).toEqual({
      "session-1": { prompt: "Write docs", error: "task create failed" },
    });
    expect(requireValue(current).optimisticStartingTaskRef.current).toBeNull();
    expect(mockedCreateSession).not.toHaveBeenCalled();
    expect(onStartError).toHaveBeenLastCalledWith("task create failed");
  });

  it("fails cleanly when the workspace execution config cannot be loaded", async () => {
    let current: FlowValue | null = null;
    mockedGetWorkspaceExecutionConfig.mockRejectedValue(new Error("config unavailable"));
    const onStartError = vi.fn();

    render(
      <Harness
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    expect(requireValue(current).optimisticTasks).toEqual([]);
    expect(mockedCreateTask).not.toHaveBeenCalled();
    expect(mockedCreateSession).not.toHaveBeenCalled();
    expect(onStartError).toHaveBeenLastCalledWith("config unavailable");
  });

  it("loads the concrete model from provider options before creating the session", async () => {
    let current: FlowValue | null = null;
    mockedCreateTask.mockResolvedValue(makeTask("task-1", "session-1"));
    mockedCreateSession.mockResolvedValue(makeSession("session-1", "task-1"));
    const onStartError = vi.fn();

    const emptyOptions = makeProviderOptions({
      provider_id: "fake",
      has_active_auth: true,
    });
    const refreshedOptions = makeProviderOptions({
      provider_id: "fake",
      has_active_auth: true,
      models: {
        current_model_id: "fake-model",
        models: [{ id: "fake-model" }],
      },
    });

    render(
      <Harness
        draftHarness={{ providerId: "fake", modelId: "" }}
        providerOptionsById={{ fake: emptyOptions }}
        providersByIdProp={{ fake: makeProviderStatus({ provider_id: "fake" }) }}
        ensureProviderAuthSummary={async () => refreshedOptions}
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    await waitFor(() => {
      expect(requireValue(current).optimisticTasks[0]?.localStatus).toBe("synced");
    });
    expect(mockedCreateSession).toHaveBeenCalledWith(
      "task-1",
      "fake",
      "fake-model",
      expect.objectContaining({
        execution_environment: "container_disk_isolated",
      }),
    );
    expect(onStartError).toHaveBeenCalledWith(null);
  });

  it("fails before creating optimistic state when no concrete model can be resolved", async () => {
    let current: FlowValue | null = null;
    const onStartError = vi.fn();

    render(
      <Harness
        draftHarness={{ providerId: "codex", modelId: "" }}
        providerOptionsById={{ codex: makeProviderOptions({ has_active_auth: true }) }}
        ensureProviderAuthSummary={async () => makeProviderOptions({ has_active_auth: true })}
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    expect(requireValue(current).optimisticTasks).toEqual([]);
    expect(mockedCreateTask).not.toHaveBeenCalled();
    expect(mockedCreateSession).not.toHaveBeenCalled();
    expect(onStartError).toHaveBeenLastCalledWith(
      "Harness “codex” did not provide a model. Refresh provider settings and try again.",
    );
  });

  it("clears the previous start error when the selected harness provider changes", async () => {
    let current: FlowValue | null = null;
    const onStartError = vi.fn();

    const view = render(
      <Harness
        draftHarness={{ providerId: "amp", modelId: "" }}
        providerOptionsById={{ amp: makeProviderOptions({ provider_id: "amp", has_active_auth: true }) }}
        providersByIdProp={{ amp: makeProviderStatus({ provider_id: "amp" }) }}
        ensureProviderAuthSummary={async () => makeProviderOptions({ provider_id: "amp", has_active_auth: true })}
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    expect(onStartError).toHaveBeenLastCalledWith(
      "Harness “amp” did not provide a model. Refresh provider settings and try again.",
    );

    view.rerender(
      <Harness
        draftHarness={{ providerId: "kimi", modelId: "kimi-model" }}
        providerOptionsById={{
          kimi: makeProviderOptions({
            provider_id: "kimi",
            has_active_auth: true,
            models: {
              current_model_id: "kimi-model",
              models: [{ id: "kimi-model" }],
            },
          }),
        }}
        providersByIdProp={{ kimi: makeProviderStatus({ provider_id: "kimi" }) }}
        ensureProviderAuthSummary={async () =>
          makeProviderOptions({
            provider_id: "kimi",
            has_active_auth: true,
            models: {
              current_model_id: "kimi-model",
              models: [{ id: "kimi-model" }],
            },
          })}
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await waitFor(() => {
      expect(onStartError).toHaveBeenLastCalledWith(null);
    });
  });

  it("blocks task creation when the selected harness is not usable", async () => {
    let current: FlowValue | null = null;
    const onStartError = vi.fn();

    render(
      <Harness
        draftHarness={{ providerId: "cursor", modelId: "cursor-model" }}
        providerOptionsById={{
          cursor: makeProviderOptions({
            provider_id: "cursor",
            has_active_auth: true,
            models: {
              current_model_id: "cursor-model",
              models: [{ id: "cursor-model" }],
            },
          }),
        }}
        providersByIdProp={{
          cursor: makeProviderStatus({
            provider_id: "cursor",
            installed: false,
            health: "error",
            diagnostics: [
              "provider is not ready until required dependencies are installed: acp-crp-bridge",
            ],
            usability: {
              usable: false,
              status: "blocked",
              reason_code: "missing_dependency",
              reason:
                "provider is not ready until required dependencies are installed: acp-crp-bridge",
              blocking_provider_ids: ["acp-crp-bridge"],
              recommended_action: "resolve_dependency",
            },
          }),
        }}
        onChange={(value) => {
          current = value;
        }}
        onStartError={onStartError}
      />,
    );

    await act(async () => {
      await requireValue(current).startNewTask();
    });

    expect(requireValue(current).optimisticTasks).toEqual([]);
    expect(mockedCreateTask).not.toHaveBeenCalled();
    expect(mockedCreateSession).not.toHaveBeenCalled();
    expect(onStartError).toHaveBeenLastCalledWith(
      "Harness “cursor” unavailable: provider is not ready until required dependencies are installed: acp-crp-bridge",
    );
  });
});
