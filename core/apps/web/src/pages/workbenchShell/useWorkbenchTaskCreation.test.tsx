import React, { useEffect, useMemo, useRef, useState } from "react";
import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageAttachment, ProviderOptions, ProviderStatus, Session, Task } from "../../api/client";
import { createSession, createTask, getWorkspaceExecutionConfig, postMessage } from "../../api/client";
import type { DraftHarness } from "../../components/WorkbenchComposer";
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

function makeProviderOptions(): ProviderOptions {
  return {
    provider_id: "codex",
    workspace_id: "workspace-1",
    supports_load: false,
    auth_required: false,
    has_active_auth: true,
    auth_mode: "subscription",
    probed_at: now,
  };
}

function makeProviderStatus(): ProviderStatus {
  return {
    provider_id: "codex",
    installed: true,
    health: "ok",
    diagnostics: [],
    details: {},
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
}: {
  prompt?: string;
  onChange: (value: FlowValue) => void;
  onStartError: (message: string | null) => void;
}) {
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const [activeTaskIdFromTab, setActiveTaskIdFromTab] = useState<string | null>(null);
  const [tasksById] = useState<Record<string, WorkspaceActiveSnapshotItem>>({});
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const [optimisticFocus, setOptimisticFocus] = useState<OptimisticFocus | null>(null);
  const optimistic = useWorkbenchOptimisticTasks({
    activeTaskId,
    activeTaskIdFromTab,
    tasksById,
  });

  const draftHarness = useMemo<DraftHarness>(() => ({ providerId: "codex", modelId: "gpt-5" }), []);
  const providerOptions = useMemo<Record<string, ProviderOptions>>(() => ({ codex: makeProviderOptions() }), []);
  const providersById = useMemo<Record<string, ProviderStatus>>(() => ({ codex: makeProviderStatus() }), []);

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
    providersById,
    providerOptions,
    ensureProviderAuthSummary: async () => providerOptions.codex,
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
});
