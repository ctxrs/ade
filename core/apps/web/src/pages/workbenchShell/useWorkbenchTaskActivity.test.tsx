import React from "react";
import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Session, SessionSnapshotSummary } from "../../api/client";
import type { SessionCacheEntry, SessionSupervisorSnapshot } from "../../state/sessionSupervisor";
import type { WorkspaceActiveSnapshotItem, WorkspaceActiveSnapshotState } from "../../state/workspaceActiveSnapshotStore";
import { WORKBENCH_TASK_IDLE_EVENT, type WorkbenchTaskIdleDetail } from "../../utils/updaterEvents";
import type { OptimisticTaskSummary } from "../WorkbenchPage.types";
import {
  deriveProviderIdsByTaskFromSessions,
  deriveTaskLiveInfo,
  deriveWarmSessionIds,
  isWorkbenchTaskUnread,
  resolveWorkbenchActiveSessionId,
  useWorkbenchTaskActivity,
} from "./useWorkbenchTaskActivity";

const now = "2026-03-09T00:00:00.000Z";

const makeSession = (
  sessionId: string,
  taskId: string,
  status: Session["status"],
  providerId = "codex",
): Session => ({
  id: sessionId,
  task_id: taskId,
  workspace_id: "workspace-1",
  worktree_id: "worktree-1",
  provider_id: providerId,
  model_id: "gpt-5",
  title: `Session ${sessionId}`,
  agent_role: "implementer",
  status,
  created_at: now,
  updated_at: now,
});

const makeSessionSummary = (
  session: Session,
  overrides?: Partial<SessionSnapshotSummary>,
): SessionSnapshotSummary => ({
  session,
  last_message_at: now,
  last_message_preview: "preview",
  last_event_seq: 1,
  state_rev: 1,
  activity: { is_working: false },
  unread: false,
  ...overrides,
});

const makeTaskSummary = ({
  taskId,
  sessions,
  primarySessionId,
  assistantSeenAt = null,
  lastAssistantMessageAt = now,
}: {
  taskId: string;
  sessions: SessionSnapshotSummary[];
  primarySessionId: string;
  assistantSeenAt?: string | null;
  lastAssistantMessageAt?: string | null;
}): WorkspaceActiveSnapshotItem => ({
  id: taskId,
  task: {
    id: taskId,
    workspace_id: "workspace-1",
    title: `Task ${taskId}`,
    status: "running",
    created_at: now,
    updated_at: now,
    last_activity_at: now,
    archived_at: null,
    assistant_seen_at: assistantSeenAt,
    last_assistant_message_at: lastAssistantMessageAt,
    primary_session_id: primarySessionId,
  },
  sessions,
  primarySessionId,
  primarySessionHead: null,
  sort_at: now,
  sortAtMs: Date.parse(now),
});

const makeSessionEntry = ({
  session,
  messageCreatedAt = now,
  updatedAtMs = Date.parse(now),
}: {
  session: Session;
  messageCreatedAt?: string;
  updatedAtMs?: number;
}): SessionCacheEntry => ({
  sessionId: session.id,
  loadState: "live",
  session,
  turns: [],
  turnToolsByTurnId: {},
  turnToolsLoading: [],
  toolSummaries: [],
  toolSummariesReady: false,
  hasMoreTurns: false,
  events: [],
  messages: [
    {
      id: `${session.id}-message`,
      session_id: session.id,
      task_id: session.task_id,
      role: "assistant",
      content: "done",
      delivery: "immediate",
      created_at: messageCreatedAt,
    },
  ],
  artifacts: [],
  artifactsLoading: false,
  subagentInvocations: [],
  subagentInvocationsLoading: false,
  stateLoaded: false,
  stateLoading: false,
  queue: [],
  loading: false,
  subscribed: true,
  updatedAtMs,
});

const makeSessionSnapshot = (sessions: Record<string, SessionCacheEntry>): SessionSupervisorSnapshot => ({
  connection: "connected",
  sessions,
});

const makeWorkspaceSnapshot = (
  tasksById: Record<string, WorkspaceActiveSnapshotItem>,
  activeIds: string[],
): WorkspaceActiveSnapshotState => ({
  workspaceId: "workspace-1",
  initialized: true,
  connection: "connected",
  tasksById,
  activeIds,
  archivedIds: [],
  totalActive: activeIds.length,
  totalArchived: 0,
  archivedRev: 0,
  worktreeVcsById: {},
  fetchState: { active: "idle", archived: "idle" },
  hasMoreActive: false,
  hasMoreArchived: false,
  archivedLoaded: false,
});

type HarnessProps = Parameters<typeof useWorkbenchTaskActivity>[0];

function Harness(props: HarnessProps) {
  useWorkbenchTaskActivity(props);
  return null;
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("useWorkbenchTaskActivity helpers", () => {
  it("prefers running sessions when deriving warm subscriptions", () => {
    const warmIds = deriveWarmSessionIds({
      activeTaskSessionIds: ["session-active"],
      activeIds: ["task-running", "task-idle"],
      tasksById: {
        "task-running": makeTaskSummary({
          taskId: "task-running",
          primarySessionId: "session-running",
          sessions: [
            makeSessionSummary(
              makeSession("session-running", "task-running", "active"),
              { last_message_at: "2026-03-09T00:00:05.000Z", activity: { is_working: true } },
            ),
          ],
        }),
        "task-idle": makeTaskSummary({
          taskId: "task-idle",
          primarySessionId: "session-idle",
          sessions: [
            makeSessionSummary(
              makeSession("session-idle", "task-idle", "completed"),
              { last_message_at: "2026-03-09T00:00:10.000Z" },
            ),
          ],
        }),
      },
    });

    expect(warmIds).toEqual(["session-running", "session-idle"]);
  });

  it("derives task live info and unread state from primary sessions only", () => {
    const primarySession = makeSession("session-1", "task-1", "completed");
    const subagentSession = {
      ...makeSession("session-subagent", "task-1", "failed", "claude-crp"),
      parent_session_id: "session-1",
      relationship: "sub_agent",
    } as Session;
    const tasksById = {
      "task-1": makeTaskSummary({
        taskId: "task-1",
        primarySessionId: "session-1",
        sessions: [
          makeSessionSummary(primarySession, {
            last_message_at: "2026-03-09T00:00:05.000Z",
          }),
        ],
      }),
    };
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById,
      optimisticTasks: [],
      sessions: {
        "session-1": makeSessionEntry({
          session: primarySession,
          messageCreatedAt: "2026-03-09T00:00:06.000Z",
        }),
        "session-subagent": makeSessionEntry({
          session: subagentSession,
          messageCreatedAt: "2026-03-09T00:00:07.000Z",
          updatedAtMs: Date.parse("2026-03-09T00:00:07.000Z"),
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.size).toBe(0);
    expect(taskLiveInfo.errorByTask.size).toBe(0);
    expect(taskLiveInfo.lastAssistantMsByTask["task-1"]).toBe(Date.parse("2026-03-09T00:00:06.000Z"));
    expect(isWorkbenchTaskUnread({ taskId: "task-1", tasksById, taskLiveInfo })).toBe(true);
  });

  it("orders task provider ids by recent session activity", () => {
    const providerIdsByTask = deriveProviderIdsByTaskFromSessions({
      "session-1": makeSessionEntry({
        session: makeSession("session-1", "task-1", "completed", "codex"),
        updatedAtMs: 10,
      }),
      "session-2": makeSessionEntry({
        session: makeSession("session-2", "task-1", "completed", "claude-crp"),
        updatedAtMs: 20,
      }),
      "session-3": makeSessionEntry({
        session: makeSession("session-3", "task-1", "completed", "codex"),
        updatedAtMs: 30,
      }),
    });

    expect(providerIdsByTask["task-1"]).toEqual(["codex", "claude-crp"]);
  });

  it("resolves the visible workbench session from primary, tab, then inferred session order", () => {
    expect(
      resolveWorkbenchActiveSessionId({
        activeSessionIdFromTab: "session-from-tab",
        primarySessionId: "session-primary",
        sessions: [makeSession("session-secondary", "task-1", "completed")],
      }),
    ).toBe("session-primary");

    expect(
      resolveWorkbenchActiveSessionId({
        activeSessionIdFromTab: "session-from-tab",
        primarySessionId: "",
        sessions: [],
      }),
    ).toBe("session-from-tab");
  });
});

const makeWorkbenchStore = (taskId: string, sessionId: string | null = null) => ({
  getActiveTab: vi.fn(() => ({
    id: "tab-1",
    kind: "task" as const,
    ref: { taskId, sessionId },
  })),
  setActiveSessionForActiveTask: vi.fn(),
});

describe("useWorkbenchTaskActivity", () => {
  it("publishes warm-session ownership and idle status from the extracted seam", async () => {
    const activeSession = makeSession("session-1", "task-1", "active");
    const warmSession = makeSession("session-2", "task-2", "completed", "claude-crp");
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "session-1",
      sessions: [
        makeSessionSummary(activeSession, {
          activity: { is_working: true },
        }),
      ],
    });
    const warmTaskSummary = makeTaskSummary({
      taskId: "task-2",
      primarySessionId: "session-2",
      sessions: [
        makeSessionSummary(warmSession, {
          last_message_at: "2026-03-09T00:00:10.000Z",
        }),
      ],
    });
    const tasksById = { "task-1": taskSummary, "task-2": warmTaskSummary };
    const supervisor = {
      setActiveTaskSessionIds: vi.fn(),
      setWarmSessionIds: vi.fn(),
    };
    const workspaceSnapshotStore = {
      setForegroundTaskId: vi.fn(),
    };
    const workbenchStore = makeWorkbenchStore("task-1");
    const idleDetails: WorkbenchTaskIdleDetail[] = [];
    const onIdle = (event: Event) => {
      idleDetails.push((event as CustomEvent<WorkbenchTaskIdleDetail>).detail);
    };

    window.addEventListener(WORKBENCH_TASK_IDLE_EVENT, onIdle as EventListener);
    try {
      render(
        <Harness
          activeTaskId="task-1"
          activeSessionIdFromTab={null}
          activeTaskSummary={taskSummary}
          tasksById={tasksById}
          workspaceSnapshot={makeWorkspaceSnapshot(tasksById, ["task-1", "task-2"])}
          sessionSnap={makeSessionSnapshot({
            "session-1": makeSessionEntry({ session: activeSession }),
            "session-2": makeSessionEntry({
              session: warmSession,
              updatedAtMs: Date.parse("2026-03-09T00:00:10.000Z"),
            }),
          })}
          optimisticTasks={[] satisfies OptimisticTaskSummary[]}
          optimisticTasksById={{}}
          supervisor={supervisor}
          workbenchStore={workbenchStore}
          workspaceSnapshotStore={workspaceSnapshotStore}
          markTaskRead={vi.fn(async () => {})}
        />,
      );

      await waitFor(() => {
        expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith(["session-1"]);
      });

      expect(supervisor.setWarmSessionIds).toHaveBeenCalledWith(["session-2"]);
      expect(workbenchStore.setActiveSessionForActiveTask).toHaveBeenCalledWith("session-1", { source: "system" });
      expect(workspaceSnapshotStore.setForegroundTaskId).toHaveBeenCalledWith("task-1");
      expect(idleDetails.at(-1)).toEqual({ allTasksIdle: false });
    } finally {
      window.removeEventListener(WORKBENCH_TASK_IDLE_EVENT, onIdle as EventListener);
    }
  });

  it("marks the active task read once it is idle and unread", async () => {
    const session = makeSession("session-1", "task-1", "completed");
    const markTaskRead = vi.fn(async () => {});
    const supervisor = {
      setActiveTaskSessionIds: vi.fn(),
      setWarmSessionIds: vi.fn(),
    };
    const workspaceSnapshotStore = {
      setForegroundTaskId: vi.fn(),
    };
    const workbenchStore = makeWorkbenchStore("task-1");
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "session-1",
      sessions: [
        makeSessionSummary(session, {
          last_message_at: "2026-03-09T00:00:05.000Z",
        }),
      ],
      assistantSeenAt: "2026-03-09T00:00:01.000Z",
      lastAssistantMessageAt: "2026-03-09T00:00:05.000Z",
    });

    render(
      <Harness
        activeTaskId="task-1"
        activeSessionIdFromTab={null}
        activeTaskSummary={taskSummary}
        tasksById={{ "task-1": taskSummary }}
        workspaceSnapshot={makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"])}
        sessionSnap={makeSessionSnapshot({
          "session-1": makeSessionEntry({
            session,
            messageCreatedAt: "2026-03-09T00:00:06.000Z",
          }),
        })}
        optimisticTasks={[] satisfies OptimisticTaskSummary[]}
        optimisticTasksById={{}}
        supervisor={supervisor}
        workbenchStore={workbenchStore}
        workspaceSnapshotStore={workspaceSnapshotStore}
        markTaskRead={markTaskRead}
      />,
    );

    await waitFor(() => {
      expect(markTaskRead).toHaveBeenCalledWith("task-1");
    });
    expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith(["session-1"]);
    expect(workbenchStore.setActiveSessionForActiveTask).toHaveBeenCalledWith("session-1", { source: "system" });
    expect(workspaceSnapshotStore.setForegroundTaskId).toHaveBeenCalledWith("task-1");
  });

  it("does not mark optimistic tasks read", async () => {
    const session = makeSession("session-1", "task-1", "completed");
    const markTaskRead = vi.fn(async () => {});
    const supervisor = {
      setActiveTaskSessionIds: vi.fn(),
      setWarmSessionIds: vi.fn(),
    };
    const workspaceSnapshotStore = {
      setForegroundTaskId: vi.fn(),
    };
    const workbenchStore = makeWorkbenchStore("task-1", "session-1");
    const taskSummary = {
      ...makeTaskSummary({
        taskId: "task-1",
        primarySessionId: "session-1",
        sessions: [makeSessionSummary(session)],
      }),
      localStatus: "starting",
      localPrompt: "hello",
      localMessageId: "message-1",
    } as OptimisticTaskSummary;

    render(
      <Harness
        activeTaskId="task-1"
        activeSessionIdFromTab="session-1"
        activeTaskSummary={taskSummary}
        tasksById={{}}
        workspaceSnapshot={makeWorkspaceSnapshot({}, ["task-1"])}
        sessionSnap={makeSessionSnapshot({
          "session-1": makeSessionEntry({ session }),
        })}
        optimisticTasks={[taskSummary]}
        optimisticTasksById={{ "task-1": taskSummary }}
        supervisor={supervisor}
        workbenchStore={workbenchStore}
        workspaceSnapshotStore={workspaceSnapshotStore}
        markTaskRead={markTaskRead}
      />,
    );

    await waitFor(() => {
      expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith(["session-1"]);
    });
    expect(markTaskRead).not.toHaveBeenCalled();
    expect(workbenchStore.setActiveSessionForActiveTask).not.toHaveBeenCalled();
  });
});
