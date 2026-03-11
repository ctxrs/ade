import React, { useEffect } from "react";
import { act, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Session, SessionSnapshotSummary, Task } from "../../api/client";
import type { WorkspaceActiveSnapshotItem } from "../../state/workspaceActiveSnapshotStore";
import type { OptimisticTaskSummary } from "../WorkbenchPage.types";
import { useWorkbenchOptimisticTasks } from "./useWorkbenchOptimisticTasks";

const now = "2026-03-10T00:00:00.000Z";

type HookValue = ReturnType<typeof useWorkbenchOptimisticTasks>;

function makeSession(sessionId: string, taskId: string): Session {
  return {
    id: sessionId,
    task_id: taskId,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    provider_id: "codex",
    model_id: "gpt-5",
    title: `Session ${sessionId}`,
    agent_role: "assistant",
    status: "starting",
    created_at: now,
    updated_at: now,
  };
}

function makeTask(taskId: string, sessionId: string): Task {
  return {
    id: taskId,
    workspace_id: "workspace-1",
    title: `Task ${taskId}`,
    status: "running",
    created_at: now,
    updated_at: now,
    last_activity_at: now,
    primary_session_id: sessionId,
    has_active_session: true,
  };
}

function makeSessionSummary(sessionId: string, taskId: string): SessionSnapshotSummary {
  return {
    session: makeSession(sessionId, taskId),
    last_message_at: now,
    last_message_preview: "preview",
    activity: { is_working: true, last_turn_status: "running" },
    unread: false,
  };
}

function makeOptimisticTask(
  taskId = "task-1",
  sessionId = "session-1",
  localStatus: OptimisticTaskSummary["localStatus"] = "starting",
): OptimisticTaskSummary {
  const base: WorkspaceActiveSnapshotItem = {
    id: taskId,
    task: makeTask(taskId, sessionId),
    sessions: [makeSessionSummary(sessionId, taskId)],
    primarySessionHead: null,
    primarySessionId: sessionId,
    sort_at: now,
    sortAtMs: Date.parse(now),
    providerIds: ["codex"],
  };
  return {
    ...base,
    localStatus,
    localPrompt: "Write docs",
    localMessageId: "message-1",
    localError: null,
  };
}

function requireValue(value: HookValue | null): HookValue {
  if (!value) throw new Error("hook value not ready");
  return value;
}

function Harness({
  activeTaskId,
  activeTaskIdFromTab,
  tasksById,
  onChange,
}: {
  activeTaskId: string | null;
  activeTaskIdFromTab: string | null;
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
  onChange: (value: HookValue) => void;
}) {
  const value = useWorkbenchOptimisticTasks({ activeTaskId, activeTaskIdFromTab, tasksById });
  useEffect(() => {
    onChange(value);
  }, [onChange, value]);
  return null;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("useWorkbenchOptimisticTasks", () => {
  it("bridges the active task from optimisticStartingTaskRef before optimistic state commits", async () => {
    let current: HookValue | null = null;
    const optimistic = makeOptimisticTask();
    const { rerender } = render(
      <Harness
        activeTaskId="task-1"
        activeTaskIdFromTab="task-1"
        tasksById={{}}
        onChange={(value) => {
          current = value;
        }}
      />,
    );

    act(() => {
      requireValue(current).optimisticStartingTaskRef.current = optimistic;
    });

    rerender(
      <Harness
        activeTaskId="task-1"
        activeTaskIdFromTab="task-1"
        tasksById={{}}
        onChange={(value) => {
          current = value;
        }}
      />,
    );

    expect(requireValue(current).activeTaskSummary).toMatchObject({
      id: "task-1",
      localStatus: "starting",
    });
  });

  it("clears optimisticStartingTaskRef once optimistic state contains the task", async () => {
    let current: HookValue | null = null;
    const optimistic = makeOptimisticTask();
    const { rerender } = render(
      <Harness
        activeTaskId="task-1"
        activeTaskIdFromTab="task-1"
        tasksById={{}}
        onChange={(value) => {
          current = value;
        }}
      />,
    );

    act(() => {
      requireValue(current).optimisticStartingTaskRef.current = optimistic;
    });

    rerender(
      <Harness
        activeTaskId="task-1"
        activeTaskIdFromTab="task-1"
        tasksById={{}}
        onChange={(value) => {
          current = value;
        }}
      />,
    );

    await act(async () => {
      requireValue(current).setOptimisticTasks([optimistic]);
    });

    await waitFor(() => {
      expect(requireValue(current).optimisticStartingTaskRef.current).toBeNull();
    });
    expect(requireValue(current).activeTaskSummary).toMatchObject({
      id: "task-1",
      localStatus: "starting",
    });
  });

  it("clears optimisticStartingTaskRef when focus moves to another task", async () => {
    let current: HookValue | null = null;
    const optimistic = makeOptimisticTask();
    const { rerender } = render(
      <Harness
        activeTaskId="task-1"
        activeTaskIdFromTab="task-1"
        tasksById={{}}
        onChange={(value) => {
          current = value;
        }}
      />,
    );

    act(() => {
      requireValue(current).optimisticStartingTaskRef.current = optimistic;
    });

    rerender(
      <Harness
        activeTaskId="task-2"
        activeTaskIdFromTab="task-2"
        tasksById={{}}
        onChange={(value) => {
          current = value;
        }}
      />,
    );

    await waitFor(() => {
      expect(requireValue(current).optimisticStartingTaskRef.current).toBeNull();
    });
  });
});
