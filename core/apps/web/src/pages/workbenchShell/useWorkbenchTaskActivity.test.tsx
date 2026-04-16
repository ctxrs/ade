import React from "react";
import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Session, SessionHeadSnapshot, SessionSnapshotSummary, SessionTurn } from "../../api/client";
import { SessionSupervisorProvider, type SessionCacheEntry, type SessionSupervisorSnapshot } from "../../state/sessionSupervisor";
import type { WorkspaceActiveSnapshotItem, WorkspaceActiveSnapshotState } from "../../state/workspaceActiveSnapshotStore";
import { WORKBENCH_TASK_IDLE_EVENT, type WorkbenchTaskIdleDetail } from "../../utils/updaterEvents";
import type { OptimisticTaskSummary } from "./WorkbenchPage.types";
import {
  canRenderWorkbenchActiveSession,
  deriveProviderIdsByTaskFromSessions,
  deriveActiveTaskSessionIds,
  deriveWorkbenchTaskStatusKind,
  deriveTaskLiveInfo,
  deriveWarmSessionIds,
  isPrimarySessionRunning,
  isWorkbenchTaskUnread,
  resolveRenderableWorkbenchActiveSessionId,
  resolveWorkbenchActiveSessionId,
  selectWorkbenchTaskLiveState,
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
  activity: { is_working: false, last_turn_status: null },
  unread: false,
  ...overrides,
});

const makeTaskSummary = ({
  taskId,
  sessions,
  primarySessionId,
  primarySessionHead = null,
  assistantSeenAt = null,
  lastAssistantMessageAt = now,
}: {
  taskId: string;
  sessions: SessionSnapshotSummary[];
  primarySessionId: string;
  primarySessionHead?: SessionHeadSnapshot | null;
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
  primarySessionHead,
  sort_at: now,
  sortAtMs: Date.parse(now),
});

const makeSessionEntry = ({
  session,
  turns = [],
  messageCreatedAt = now,
  hasMoreTurns = false,
  updatedAtMs = Date.parse(now),
  freshness = "authoritative",
  activity,
  lastEventSeq,
  projectionRev,
  stateRev,
}: {
  session: Session;
  turns?: SessionTurn[];
  messageCreatedAt?: string;
  hasMoreTurns?: boolean;
  updatedAtMs?: number;
  freshness?: SessionCacheEntry["freshness"];
  activity?: SessionCacheEntry["activity"];
  lastEventSeq?: number;
  projectionRev?: number;
  stateRev?: number;
}): SessionCacheEntry => ({
  sessionId: session.id,
  loadState: "live",
  freshness,
  session,
  activity,
  turns,
  turnToolsByTurnId: {},
  turnToolsLoading: [],
  toolSummaries: [],
  toolSummariesReady: false,
  hasMoreTurns,
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
  lastEventSeq,
  projectionRev,
  stateRev,
  updatedAtMs,
});

const makeTurn = (sessionId: string, status: SessionTurn["status"]): SessionTurn => ({
  turn_id: `${sessionId}-${status}`,
  session_id: sessionId,
  run_id: null,
  user_message_id: `${sessionId}-message`,
  status,
  start_seq: 1,
  end_seq: status === "running" ? null : 2,
  started_at: now,
  updated_at: now,
  assistant_partial: null,
  thought_partial: null,
  metrics_json: null,
  tool_total: 0,
  tool_pending: 0,
  tool_running: 0,
  tool_completed: 0,
  tool_failed: 0,
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
  liveSnapshotApplied: true,
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

function renderHarness(props: HarnessProps) {
  return render(
    <SessionSupervisorProvider>
      <Harness {...props} />
    </SessionSupervisorProvider>,
  );
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
              {
                last_message_at: "2026-03-09T00:00:05.000Z",
                activity: { is_working: true, last_turn_status: "running" },
              },
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

  it("preserves the primary_session fallback when task.primary_session_id is absent", () => {
    const primarySession = makeSession("session-legacy", "task-1", "active");
    const primarySummary = makeSessionSummary(primarySession, {
      activity: { is_working: true, last_turn_status: "running" },
      last_message_at: "2026-03-09T00:00:07.000Z",
    });
    const legacyTaskSummary = {
      ...makeTaskSummary({
        taskId: "task-1",
        primarySessionId: "",
        sessions: [primarySummary],
      }),
      task: {
        ...makeTaskSummary({
          taskId: "task-1",
          primarySessionId: "",
          sessions: [primarySummary],
        }).task,
        primary_session_id: "",
      },
      primary_session: primarySummary,
    } as WorkspaceActiveSnapshotItem & { primary_session: SessionSnapshotSummary };

    const { primarySessionId, activeTaskSessionIds } = deriveActiveTaskSessionIds(legacyTaskSummary);
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: { "task-1": legacyTaskSummary },
      optimisticTasks: [],
      sessions: {},
    });

    expect(primarySessionId).toBe("session-legacy");
    expect(activeTaskSessionIds).toEqual(["session-legacy"]);
    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
  });

  it("prioritizes the active tab session ahead of the task primary session", () => {
    const primarySession = makeSession("session-primary", "task-1", "active");
    const secondarySession = makeSession("session-secondary", "task-1", "active");
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "session-primary",
      sessions: [
        makeSessionSummary(primarySession),
        makeSessionSummary(secondarySession),
      ],
    });

    const { primarySessionId, activeTaskSessionIds } = deriveActiveTaskSessionIds(
      taskSummary,
      "session-secondary",
    );

    expect(primarySessionId).toBe("session-primary");
    expect(activeTaskSessionIds).toEqual(["session-secondary", "session-primary"]);
  });

  it("does not treat a user follow-up timestamp as a new unread assistant message", () => {
    const primarySession = makeSession("session-1", "task-1", "completed");
    const tasksById = {
      "task-1": makeTaskSummary({
        taskId: "task-1",
        primarySessionId: "session-1",
        sessions: [
          makeSessionSummary(primarySession, {
            last_message_at: "2026-03-09T00:00:09.000Z",
          }),
        ],
        assistantSeenAt: "2026-03-09T00:00:07.000Z",
        lastAssistantMessageAt: "2026-03-09T00:00:05.000Z",
      }),
    };
    const primaryEntry = {
      ...makeSessionEntry({
        session: primarySession,
        messageCreatedAt: "2026-03-09T00:00:05.000Z",
      }),
      messages: [
        {
          id: "assistant-message",
          session_id: primarySession.id,
          task_id: primarySession.task_id,
          role: "assistant" as const,
          content: "done",
          delivery: "immediate" as const,
          created_at: "2026-03-09T00:00:05.000Z",
        },
        {
          id: "user-message",
          session_id: primarySession.id,
          task_id: primarySession.task_id,
          role: "user" as const,
          content: "follow up",
          delivery: "immediate" as const,
          created_at: "2026-03-09T00:00:09.000Z",
        },
      ],
    };

    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById,
      optimisticTasks: [],
      sessions: {
        "session-1": primaryEntry,
      },
    });

    expect(taskLiveInfo.lastAssistantMsByTask["task-1"]).toBe(Date.parse("2026-03-09T00:00:05.000Z"));
    expect(isWorkbenchTaskUnread({ taskId: "task-1", tasksById, taskLiveInfo })).toBe(false);
  });

  it("falls back to the primary session summary timestamp when no live entry is open", () => {
    const primarySession = makeSession("session-1", "task-1", "completed");
    const tasksById = {
      "task-1": makeTaskSummary({
        taskId: "task-1",
        primarySessionId: "session-1",
        sessions: [
          makeSessionSummary(primarySession, {
            last_message_at: "2026-03-09T00:00:08.000Z",
          }),
        ],
        assistantSeenAt: "2026-03-09T00:00:06.000Z",
        lastAssistantMessageAt: "2026-03-09T00:00:05.000Z",
      }),
    };

    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById,
      optimisticTasks: [],
      sessions: {},
    });

    expect(taskLiveInfo.lastAssistantMsByTask["task-1"]).toBe(Date.parse("2026-03-09T00:00:08.000Z"));
    expect(isWorkbenchTaskUnread({ taskId: "task-1", tasksById, taskLiveInfo })).toBe(true);
  });

  it("prefers a newer summary timestamp over stale non-authoritative live cache", () => {
    const primarySession = makeSession("session-1", "task-1", "completed");
    const tasksById = {
      "task-1": makeTaskSummary({
        taskId: "task-1",
        primarySessionId: "session-1",
        sessions: [
          makeSessionSummary(primarySession, {
            last_message_at: "2026-03-09T00:00:08.000Z",
          }),
        ],
        assistantSeenAt: "2026-03-09T00:00:06.000Z",
        lastAssistantMessageAt: "2026-03-09T00:00:05.000Z",
      }),
    };

    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById,
      optimisticTasks: [],
      sessions: {
        "session-1": makeSessionEntry({
          session: primarySession,
          messageCreatedAt: "2026-03-09T00:00:07.000Z",
          freshness: "bootstrap",
        }),
      },
    });

    expect(taskLiveInfo.lastAssistantMsByTask["task-1"]).toBe(Date.parse("2026-03-09T00:00:08.000Z"));
    expect(isWorkbenchTaskUnread({ taskId: "task-1", tasksById, taskLiveInfo })).toBe(true);
  });

  it("uses the authoritative primary session entry status for task errors when present", () => {
    const primarySession = makeSession("session-1", "task-1", "completed");
    const failedEntrySession = { ...primarySession, status: "failed" as const };
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
          session: failedEntrySession,
          messageCreatedAt: "2026-03-09T00:00:06.000Z",
        }),
      },
    });

    expect(taskLiveInfo.errorByTask.has("task-1")).toBe(true);
  });

  it("derives live task state when only the primary head is hydrated", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "session-1",
      sessions: [],
      assistantSeenAt: "2026-03-09T00:00:06.000Z",
      lastAssistantMessageAt: "2026-03-09T00:00:05.000Z",
    });
    taskSummary.primarySessionHead = {
      session: primarySession,
      turns: [],
      messages: [
        {
          id: "head-message",
          session_id: primarySession.id,
          task_id: primarySession.task_id,
          role: "assistant",
          content: "still running",
          delivery: "immediate",
          created_at: "2026-03-09T00:00:08.000Z",
        },
      ],
      last_event_seq: 8,
      projection_rev: 8,
      state_rev: 0,
      activity: { is_working: true, last_turn_status: "running" },
      has_more_turns: false,
      has_more_history: false,
    };

    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: { "task-1": taskSummary },
      optimisticTasks: [],
      sessions: {
        "session-1": makeSessionEntry({
          session: primarySession,
          messageCreatedAt: "2026-03-09T00:00:08.000Z",
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
    expect(taskLiveInfo.lastAssistantMsByTask["task-1"]).toBe(Date.parse("2026-03-09T00:00:08.000Z"));
    expect(isWorkbenchTaskUnread({ taskId: "task-1", tasksById: { "task-1": taskSummary }, taskLiveInfo })).toBe(true);
  });

  it("selects task working state from the freshest canonical source", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const selectedState = selectWorkbenchTaskLiveState({
      task: makeTaskSummary({
        taskId: "task-1",
        primarySessionId: primarySession.id,
        sessions: [
          makeSessionSummary(primarySession, {
            last_event_seq: 12,
            projection_rev: 12,
            state_rev: 12,
            activity: { is_working: true, last_turn_status: "running" },
          }),
        ],
        primarySessionHead: {
          session: primarySession,
          turns: [],
          tool_summaries: [],
          messages: [],
          events: [],
          last_event_seq: 8,
          projection_rev: 8,
          state_rev: 8,
          activity: { is_working: false, last_turn_status: "completed" },
          has_more_turns: false,
          has_more_history: false,
        },
      }),
      entryBySessionId: new Map(),
    });

    expect(selectedState).toEqual({
      working: true,
      hasError: false,
      lastAssistantMs: Date.parse(now),
    });
  });

  it("does not let bootstrap cache activity outrank canonical task state in the selector", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const selectedState = selectWorkbenchTaskLiveState({
      task: makeTaskSummary({
        taskId: "task-1",
        primarySessionId: primarySession.id,
        sessions: [
          makeSessionSummary(primarySession, {
            activity: { is_working: false, last_turn_status: "completed" },
            last_event_seq: 10,
            projection_rev: 10,
            state_rev: 10,
          }),
        ],
      }),
      entryBySessionId: new Map([
        [
          primarySession.id,
          makeSessionEntry({
            session: primarySession,
            freshness: "bootstrap",
            activity: { is_working: true, last_turn_status: "running" },
            lastEventSeq: 8,
            projectionRev: 8,
            stateRev: 8,
          }),
        ],
      ]),
    });

    expect(selectedState).toEqual({
      working: false,
      hasError: false,
      lastAssistantMs: Date.parse(now),
    });
  });

  it("treats canonical is_working summaries as working, including queued follow-ups", () => {
    const primarySession = makeSession("session-1", "task-1", "active");

    expect(
      isPrimarySessionRunning({
        primarySessionSummary: makeSessionSummary(primarySession, {
          activity: { is_working: true, last_turn_status: "running" },
        }),
      }),
    ).toBe(true);

    expect(
      isPrimarySessionRunning({
        primarySessionSummary: makeSessionSummary(primarySession, {
          activity: { is_working: true, last_turn_status: "queued" },
        }),
      }),
    ).toBe(true);

    expect(
      isPrimarySessionRunning({
        primarySessionSummary: makeSessionSummary(primarySession, {
          activity: { is_working: false, last_turn_status: "completed" },
        }),
      }),
    ).toBe(false);

    expect(
      isPrimarySessionRunning({
        primarySessionSummary: undefined,
      }),
    ).toBe(false);
  });

  it("prefers canonical head activity over stale summary activity for task working state", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: {
        "task-1": makeTaskSummary({
          taskId: "task-1",
          primarySessionId: primarySession.id,
          sessions: [
            makeSessionSummary(primarySession, {
              activity: { is_working: false, last_turn_status: "completed" },
            }),
          ],
          primarySessionHead: {
            session: primarySession,
            turns: [],
            tool_summaries: [],
            messages: [],
            events: [],
            last_event_seq: 8,
            projection_rev: 8,
            state_rev: 8,
            activity: { is_working: true, last_turn_status: "running" },
            has_more_turns: false,
            has_more_history: false,
          },
        }),
      },
      optimisticTasks: [],
      sessions: {},
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
  });

  it("prefers fresher summary activity over stale canonical head activity", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: {
        "task-1": makeTaskSummary({
          taskId: "task-1",
          primarySessionId: primarySession.id,
          sessions: [
            makeSessionSummary(primarySession, {
              last_event_seq: 10,
              projection_rev: 10,
              state_rev: 10,
              activity: { is_working: true, last_turn_status: "running" },
            }),
          ],
          primarySessionHead: {
            session: primarySession,
            turns: [],
            tool_summaries: [],
            messages: [],
            events: [],
            last_event_seq: 8,
            projection_rev: 8,
            state_rev: 8,
            activity: { is_working: false, last_turn_status: "completed" },
            has_more_turns: false,
            has_more_history: false,
          },
        }),
      },
      optimisticTasks: [],
      sessions: {},
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
  });

  it("prefers fresher summary activity over stale canonical session cache activity", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: {
        "task-1": makeTaskSummary({
          taskId: "task-1",
          primarySessionId: primarySession.id,
          sessions: [
            makeSessionSummary(primarySession, {
              last_event_seq: 10,
              projection_rev: 10,
              state_rev: 10,
              activity: { is_working: true, last_turn_status: "running" },
            }),
          ],
        }),
      },
      optimisticTasks: [],
      sessions: {
        [primarySession.id]: makeSessionEntry({
          session: primarySession,
          freshness: "authoritative",
          activity: { is_working: false, last_turn_status: "completed" },
          lastEventSeq: 8,
          projectionRev: 8,
          stateRev: 8,
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
  });

  it("ignores bootstrap live activity when canonical head is already completed", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: {
        "task-1": makeTaskSummary({
          taskId: "task-1",
          primarySessionId: primarySession.id,
          sessions: [
            makeSessionSummary(primarySession, {
              activity: { is_working: false, last_turn_status: "completed" },
              last_message_at: "2026-03-09T00:00:06.000Z",
            }),
          ],
          primarySessionHead: {
            session: primarySession,
            turns: [],
            tool_summaries: [],
            messages: [],
            events: [],
            last_event_seq: 8,
            projection_rev: 8,
            state_rev: 8,
            activity: { is_working: false, last_turn_status: "completed" },
            has_more_turns: false,
            has_more_history: false,
          },
        }),
      },
      optimisticTasks: [],
      sessions: {
        [primarySession.id]: makeSessionEntry({
          session: primarySession,
          turns: [makeTurn(primarySession.id, "running")],
          messageCreatedAt: "2026-03-09T00:00:08.000Z",
          freshness: "bootstrap",
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(false);
    expect(taskLiveInfo.lastAssistantMsByTask["task-1"]).toBe(Date.parse("2026-03-09T00:00:06.000Z"));
  });

  it("does not let live primary turns override a non-working canonical summary", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: {
        "task-1": makeTaskSummary({
          taskId: "task-1",
          primarySessionId: primarySession.id,
          sessions: [
            makeSessionSummary(primarySession, {
              activity: { is_working: false, last_turn_status: "completed" },
            }),
          ],
        }),
      },
      optimisticTasks: [],
      sessions: {
        [primarySession.id]: makeSessionEntry({
          session: primarySession,
          turns: [makeTurn(primarySession.id, "running")],
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(false);
  });

  it("keeps queued primary summaries working when canonical activity is still working", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: {
        "task-1": makeTaskSummary({
          taskId: "task-1",
          primarySessionId: primarySession.id,
          sessions: [
            makeSessionSummary(primarySession, {
              activity: { is_working: true, last_turn_status: "queued" },
            }),
          ],
        }),
      },
      optimisticTasks: [],
      sessions: {
        [primarySession.id]: makeSessionEntry({
          session: primarySession,
          turns: [makeTurn(primarySession.id, "queued")],
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
  });

  it("falls back to a running summary when the live cache only has older non-running turns", () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: {
        "task-1": makeTaskSummary({
          taskId: "task-1",
          primarySessionId: primarySession.id,
          sessions: [
            makeSessionSummary(primarySession, {
              activity: { is_working: true, last_turn_status: "running" },
            }),
          ],
        }),
      },
      optimisticTasks: [],
      sessions: {
        [primarySession.id]: makeSessionEntry({
          session: primarySession,
          turns: [makeTurn(primarySession.id, "completed")],
          hasMoreTurns: true,
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
  });

  it("ignores optimistic starting state when a real primary running turn exists", () => {
    const primarySession = makeSession("session-1", "task-1", "starting");
    const optimisticTask = {
      ...makeTaskSummary({
        taskId: "task-1",
        primarySessionId: primarySession.id,
        sessions: [
          makeSessionSummary(primarySession, {
            activity: { is_working: true, last_turn_status: "running" },
          }),
        ],
      }),
      localStatus: "starting",
      localPrompt: "ship it",
      localMessageId: "message-1",
    } as OptimisticTaskSummary;

    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: {},
      optimisticTasks: [optimisticTask],
      sessions: {
        [primarySession.id]: makeSessionEntry({
          session: primarySession,
          turns: [makeTurn(primarySession.id, "running")],
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
  });

  it("derives task row status from running, unread, and error only", () => {
    expect(
      deriveWorkbenchTaskStatusKind({
        hasError: false,
        working: false,
        unread: true,
        localStatus: "starting",
      }),
    ).toBe("unread");

    expect(
      deriveWorkbenchTaskStatusKind({
        hasError: false,
        working: true,
        unread: true,
        localStatus: null,
      }),
    ).toBe("working");
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

    expect(
      resolveWorkbenchActiveSessionId({
        activeSessionIdFromTab: "session-stale",
        primarySessionId: "",
        sessions: [
          makeSession("session-1", "task-1", "completed"),
          makeSession("session-2", "task-1", "active"),
        ],
      }),
    ).toBe("session-2");
  });

  it("only exposes a renderable active session once its entry has seeded content or loaded state", () => {
    const session = makeSession("session-1", "task-1", "active");
    const emptyEntry = {
      ...makeSessionEntry({ session }),
      turns: [],
      messages: [],
      events: [],
      queue: [],
      stateLoaded: false,
    };

    expect(canRenderWorkbenchActiveSession(emptyEntry)).toBe(false);
    expect(
      resolveRenderableWorkbenchActiveSessionId({
        activeSessionIdFromTab: "session-1",
        primarySessionId: "",
        sessions: [session],
        sessionEntries: { "session-1": emptyEntry },
      }),
    ).toBeNull();

    expect(
      resolveRenderableWorkbenchActiveSessionId({
        activeSessionIdFromTab: "session-1",
        primarySessionId: "",
        sessions: [session],
        sessionEntries: {
          "session-1": {
            ...emptyEntry,
            stateLoaded: true,
          },
        },
      }),
    ).toBe("session-1");

    expect(
      resolveRenderableWorkbenchActiveSessionId({
        activeSessionIdFromTab: "session-1",
        primarySessionId: "",
        sessions: [session],
        sessionEntries: {
          "session-1": makeSessionEntry({
            session,
            freshness: "bootstrap",
          }),
        },
      }),
    ).toBe("session-1");
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

const makeSupervisor = () => ({
  setActiveTaskSessionIds: vi.fn(),
  setWarmSessionIds: vi.fn(),
  setSubscribedSessionIdsSink: vi.fn(),
  setWorkspaceSnapshotState: vi.fn(),
  setWorkspaceSessionHeads: vi.fn(),
  handleWorkspaceEvent: vi.fn(),
});

const makeWorkspaceSnapshotStore = (snapshot: WorkspaceActiveSnapshotState) => ({
  subscribe: vi.fn(() => () => {}),
  subscribeEvents: vi.fn(() => () => {}),
  getSnapshot: vi.fn(() => snapshot),
  getSessionHeadSnapshot: vi.fn(() => null),
  getSessionHeadsSnapshot: vi.fn(() => ({})),
  setSubscribedSessions: vi.fn(),
  setForegroundSessionId: vi.fn(),
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
          activity: { is_working: true, last_turn_status: "running" },
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
    const workspaceSnapshot = makeWorkspaceSnapshot(tasksById, ["task-1", "task-2"]);
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
    const workbenchStore = makeWorkbenchStore("task-1");
    const idleDetails: WorkbenchTaskIdleDetail[] = [];
    const onIdle = (event: Event) => {
      idleDetails.push((event as CustomEvent<WorkbenchTaskIdleDetail>).detail);
    };

    window.addEventListener(WORKBENCH_TASK_IDLE_EVENT, onIdle as EventListener);
    try {
      renderHarness({
        activeTaskId: "task-1",
        activeSessionIdFromTab: null,
        activeTaskSummary: taskSummary,
        tasksById,
        workspaceSnapshot,
        sessionSnap: makeSessionSnapshot({
          "session-1": makeSessionEntry({ session: activeSession }),
          "session-2": makeSessionEntry({
            session: warmSession,
            updatedAtMs: Date.parse("2026-03-09T00:00:10.000Z"),
          }),
        }),
        optimisticTasks: [] satisfies OptimisticTaskSummary[],
        optimisticTasksById: {},
        supervisor,
        workbenchStore,
        workspaceSnapshotStore,
        markTaskRead: vi.fn(async () => {}),
      });

      await waitFor(() => {
        expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith(["session-1"]);
      });

      expect(supervisor.setWarmSessionIds).toHaveBeenCalledWith(["session-2"]);
      expect(supervisor.setSubscribedSessionIdsSink).toHaveBeenCalledTimes(1);
      expect(supervisor.setWorkspaceSnapshotState).toHaveBeenCalledWith(workspaceSnapshot);
      expect(supervisor.setWorkspaceSessionHeads).toHaveBeenCalledWith({});
      expect(workbenchStore.setActiveSessionForActiveTask).toHaveBeenCalledWith("session-1", { source: "system" });
      expect(workspaceSnapshotStore.setForegroundSessionId).toHaveBeenCalledWith("session-1");
      const subscribedSessionsSink = supervisor.setSubscribedSessionIdsSink.mock.calls[0]?.[0];
      expect(subscribedSessionsSink).toBeTypeOf("function");
      subscribedSessionsSink?.([{ sessionId: "session-1", replay: { kind: "resume", afterSeq: 3 } }]);
      expect(workspaceSnapshotStore.setSubscribedSessions).toHaveBeenCalledWith([
        { sessionId: "session-1", replay: { kind: "resume", afterSeq: 3 } },
      ]);
      expect(idleDetails.at(-1)).toEqual({ allTasksIdle: false });
    } finally {
      window.removeEventListener(WORKBENCH_TASK_IDLE_EVENT, onIdle as EventListener);
    }
  });

  it("falls back to single-head workspace snapshots when the batch getter is unavailable", async () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const secondarySession = makeSession("session-2", "task-1", "completed");
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "session-1",
      sessions: [
        makeSessionSummary(primarySession),
        makeSessionSummary(secondarySession),
      ],
    });
    const workspaceSnapshot = makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]);
    const supervisor = makeSupervisor();
    const secondaryHead = {
      session: secondarySession,
      turns: [],
      events: [],
      messages: [],
      last_event_seq: 3,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    };
    const workspaceSnapshotStore = {
      subscribe: vi.fn(() => () => {}),
      subscribeEvents: vi.fn(() => () => {}),
      getSnapshot: vi.fn(() => workspaceSnapshot),
      getSessionHeadSnapshot: vi.fn((sessionId: string) => (sessionId === "session-2" ? secondaryHead : null)),
      setSubscribedSessions: vi.fn(),
      setForegroundSessionId: vi.fn(),
    };

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: null,
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        "session-1": makeSessionEntry({ session: primarySession }),
        "session-2": makeSessionEntry({ session: secondarySession }),
      }),
      optimisticTasks: [] satisfies OptimisticTaskSummary[],
      optimisticTasksById: {},
      supervisor,
      workbenchStore: makeWorkbenchStore("task-1"),
      workspaceSnapshotStore,
      markTaskRead: vi.fn(async () => {}),
    });

    await waitFor(() => {
      expect(supervisor.setWorkspaceSessionHeads).toHaveBeenCalledWith({ "session-2": secondaryHead });
    });
  });

  it("syncs workspace session heads before snapshot state so remembered non-primary sessions can seed immediately", async () => {
    const primarySession = makeSession("session-1", "task-1", "active");
    const secondarySession = makeSession("session-2", "task-1", "completed");
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "session-1",
      sessions: [
        makeSessionSummary(primarySession),
        makeSessionSummary(secondarySession),
      ],
    });
    const workspaceSnapshot = makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]);
    const supervisor = makeSupervisor();
    const secondaryHead = {
      session: secondarySession,
      turns: [],
      events: [],
      messages: [],
      last_event_seq: 7,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
    };
    const workspaceSnapshotStore = {
      subscribe: vi.fn(() => () => {}),
      subscribeEvents: vi.fn(() => () => {}),
      getSnapshot: vi.fn(() => workspaceSnapshot),
      getSessionHeadSnapshot: vi.fn(() => null),
      getSessionHeadsSnapshot: vi.fn(() => ({ "session-2": secondaryHead })),
      setSubscribedSessions: vi.fn(),
      setForegroundSessionId: vi.fn(),
    };

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: "session-2",
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        "session-1": makeSessionEntry({ session: primarySession }),
        "session-2": makeSessionEntry({ session: secondarySession }),
      }),
      optimisticTasks: [] satisfies OptimisticTaskSummary[],
      optimisticTasksById: {},
      supervisor,
      workbenchStore: makeWorkbenchStore("task-1", "session-2"),
      workspaceSnapshotStore,
      markTaskRead: vi.fn(async () => {}),
    });

    await waitFor(() => {
      expect(supervisor.setWorkspaceSessionHeads).toHaveBeenCalledWith({ "session-2": secondaryHead });
      expect(supervisor.setWorkspaceSnapshotState).toHaveBeenCalledWith(workspaceSnapshot);
    });

    expect(
      supervisor.setWorkspaceSessionHeads.mock.invocationCallOrder[0],
    ).toBeLessThan(supervisor.setWorkspaceSnapshotState.mock.invocationCallOrder[0]);
  });

  it("marks the active task read once it is idle and unread", async () => {
    const session = makeSession("session-1", "task-1", "completed");
    const markTaskRead = vi.fn(async () => {});
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
    const workspaceSnapshot = makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]);
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
    const workbenchStore = makeWorkbenchStore("task-1");

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: null,
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        "session-1": makeSessionEntry({
          session,
          messageCreatedAt: "2026-03-09T00:00:06.000Z",
        }),
      }),
      optimisticTasks: [] satisfies OptimisticTaskSummary[],
      optimisticTasksById: {},
      supervisor,
      workbenchStore,
      workspaceSnapshotStore,
      markTaskRead,
    });

    await waitFor(() => {
      expect(markTaskRead).toHaveBeenCalledWith("task-1");
    });
    expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith(["session-1"]);
    expect(workbenchStore.setActiveSessionForActiveTask).toHaveBeenCalledWith("session-1", { source: "system" });
    expect(workspaceSnapshotStore.setForegroundSessionId).toHaveBeenCalledWith("session-1");
  });

  it("does not mark optimistic tasks read", async () => {
    const session = makeSession("session-1", "task-1", "completed");
    const markTaskRead = vi.fn(async () => {});
    const workspaceSnapshot = makeWorkspaceSnapshot({}, ["task-1"]);
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
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

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: "session-1",
      activeTaskSummary: taskSummary,
      tasksById: {},
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        "session-1": makeSessionEntry({ session }),
      }),
      optimisticTasks: [taskSummary],
      optimisticTasksById: { "task-1": taskSummary },
      supervisor,
      workbenchStore,
      workspaceSnapshotStore,
      markTaskRead,
    });

    await waitFor(() => {
      expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith(["session-1"]);
    });
    expect(markTaskRead).not.toHaveBeenCalled();
    expect(workbenchStore.setActiveSessionForActiveTask).not.toHaveBeenCalled();
  });

  it("revalidates a stale remembered session id against available task sessions", async () => {
    const olderSession = makeSession("session-1", "task-1", "completed");
    const activeSession = makeSession("session-2", "task-1", "active");
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "",
      sessions: [
        makeSessionSummary(olderSession, {
          last_message_at: "2026-03-09T00:00:02.000Z",
        }),
        makeSessionSummary(activeSession, {
          last_message_at: "2026-03-09T00:00:03.000Z",
          activity: { is_working: true, last_turn_status: "running" },
        }),
      ],
    });
    const workspaceSnapshot = makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]);
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
    const workbenchStore = makeWorkbenchStore("task-1", "session-stale");

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: "session-stale",
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        "session-1": makeSessionEntry({ session: olderSession }),
        "session-2": makeSessionEntry({ session: activeSession }),
      }),
      optimisticTasks: [] satisfies OptimisticTaskSummary[],
      optimisticTasksById: {},
      supervisor,
      workbenchStore,
      workspaceSnapshotStore,
      markTaskRead: vi.fn(async () => {}),
    });

    await waitFor(() => {
      expect(workbenchStore.setActiveSessionForActiveTask).toHaveBeenCalledWith("session-2", { source: "system" });
    });
  });

  it("preserves the remembered tab session id until task summaries hydrate", async () => {
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "",
      sessions: [],
    });
    const workspaceSnapshot = {
      ...makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]),
      initialized: false,
      fetchState: { active: "loading" as const, archived: "idle" as const },
    };
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
    const workbenchStore = makeWorkbenchStore("task-1", "session-from-tab");

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: "session-from-tab",
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({}),
      optimisticTasks: [] satisfies OptimisticTaskSummary[],
      optimisticTasksById: {},
      supervisor,
      workbenchStore,
      workspaceSnapshotStore,
      markTaskRead: vi.fn(async () => {}),
    });

    await waitFor(() => {
      expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith(["session-from-tab"]);
    });
    expect(workspaceSnapshotStore.setForegroundSessionId).toHaveBeenCalledWith("session-from-tab");
    expect(workbenchStore.setActiveSessionForActiveTask).not.toHaveBeenCalled();
  });

  it("preserves the optimistic session during the server handoff until the task summary publishes a session", async () => {
    const optimisticSessionId = "session-optimistic";
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "",
      sessions: [],
    });
    const workspaceSnapshot = makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]);
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
    const workbenchStore = makeWorkbenchStore("task-1", optimisticSessionId);
    const optimisticTask = {
      ...makeTaskSummary({
        taskId: "task-1",
        primarySessionId: optimisticSessionId,
        sessions: [makeSessionSummary(makeSession(optimisticSessionId, "task-1", "active"))],
      }),
      localStatus: "synced",
      localPrompt: "hello",
      localMessageId: "message-1",
    } as OptimisticTaskSummary;

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: optimisticSessionId,
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        [optimisticSessionId]: makeSessionEntry({
          session: makeSession(optimisticSessionId, "task-1", "active"),
        }),
      }),
      optimisticTasks: [optimisticTask],
      optimisticTasksById: { "task-1": optimisticTask },
      supervisor,
      workbenchStore,
      workspaceSnapshotStore,
      markTaskRead: vi.fn(async () => {}),
    });

    await waitFor(() => {
      expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith([optimisticSessionId]);
    });
    expect(workspaceSnapshotStore.setForegroundSessionId).toHaveBeenCalledWith(optimisticSessionId);
    expect(workbenchStore.setActiveSessionForActiveTask).not.toHaveBeenCalledWith(null, { source: "system" });
  });

  it("falls back to the primary session head when task summaries are still sessionless", async () => {
    const session = makeSession("session-1", "task-1", "active");
    const taskSummary = {
      ...makeTaskSummary({
        taskId: "task-1",
        primarySessionId: "",
        sessions: [],
      }),
      task: {
        ...makeTaskSummary({
          taskId: "task-1",
          primarySessionId: "",
          sessions: [],
        }).task,
        primary_session_id: null,
      },
      primarySessionId: session.id,
      primarySessionHead: {
        session,
        turns: [],
        events: [],
        messages: [],
        last_event_seq: 1,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
    };
    const workspaceSnapshot = makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]);
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
    const workbenchStore = makeWorkbenchStore("task-1");

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: null,
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        "session-1": makeSessionEntry({ session }),
      }),
      optimisticTasks: [] satisfies OptimisticTaskSummary[],
      optimisticTasksById: {},
      supervisor,
      workbenchStore,
      workspaceSnapshotStore,
      markTaskRead: vi.fn(async () => {}),
    });

    await waitFor(() => {
      expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith(["session-1"]);
      expect(workbenchStore.setActiveSessionForActiveTask).toHaveBeenCalledWith("session-1", {
        source: "system",
      });
    });
  });

  it("waits to switch the visible task session until the candidate session is renderable", async () => {
    const session = makeSession("session-1", "task-1", "active");
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: session.id,
      sessions: [makeSessionSummary(session)],
    });
    const workspaceSnapshot = makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]);
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
    const workbenchStore = makeWorkbenchStore("task-1");
    const rendered = renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: null,
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        [session.id]: {
          ...makeSessionEntry({ session }),
          turns: [],
          messages: [],
          events: [],
          queue: [],
          stateLoaded: false,
          loadState: "pending_hydration",
        },
      }),
      optimisticTasks: [] satisfies OptimisticTaskSummary[],
      optimisticTasksById: {},
      supervisor,
      workbenchStore,
      workspaceSnapshotStore,
      markTaskRead: vi.fn(async () => {}),
    });

    await waitFor(() => {
      expect(supervisor.setActiveTaskSessionIds).toHaveBeenCalledWith([session.id]);
    });
    expect(workbenchStore.setActiveSessionForActiveTask).not.toHaveBeenCalledWith(session.id, { source: "system" });

    rendered.rerender(
      <SessionSupervisorProvider>
        <Harness
          activeTaskId="task-1"
          activeSessionIdFromTab={null}
          activeTaskSummary={taskSummary}
          tasksById={{ "task-1": taskSummary }}
          workspaceSnapshot={workspaceSnapshot}
          sessionSnap={makeSessionSnapshot({
            [session.id]: makeSessionEntry({ session }),
          })}
          optimisticTasks={[] satisfies OptimisticTaskSummary[]}
          optimisticTasksById={{}}
          supervisor={supervisor}
          workbenchStore={workbenchStore}
          workspaceSnapshotStore={workspaceSnapshotStore}
          markTaskRead={vi.fn(async () => {})}
        />
      </SessionSupervisorProvider>,
    );

    await waitFor(() => {
      expect(workbenchStore.setActiveSessionForActiveTask).toHaveBeenCalledWith(session.id, {
        source: "system",
      });
    });
  });

  it("derives live task info from the primary session fallback when task summaries are still sessionless", () => {
    const session = makeSession("session-1", "task-1", "active");
    const sessionSummary = makeSessionSummary(session, {
      activity: { is_working: true, last_turn_status: "running" },
    });
    const taskSummary = {
      ...makeTaskSummary({
        taskId: "task-1",
        primarySessionId: "",
        sessions: [sessionSummary],
      }),
      task: {
        ...makeTaskSummary({
          taskId: "task-1",
          primarySessionId: "",
          sessions: [sessionSummary],
        }).task,
        primary_session_id: null,
      },
      primarySessionId: session.id,
      primarySessionHead: {
        session,
        turns: [],
        events: [],
        messages: [],
        last_event_seq: 1,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
    };

    const taskLiveInfo = deriveTaskLiveInfo({
      tasksById: { "task-1": taskSummary },
      optimisticTasks: [],
      sessions: {
        [session.id]: makeSessionEntry({
          session,
          turns: [makeTurn(session.id, "running")],
          messageCreatedAt: "2026-03-09T00:00:06.000Z",
        }),
      },
    });

    expect(taskLiveInfo.workingByTask.has("task-1")).toBe(true);
    expect(taskLiveInfo.lastAssistantMsByTask["task-1"]).toBe(Date.parse("2026-03-09T00:00:06.000Z"));
  });

  it("clears a stale remembered session id when an active task is still sessionless after hydration completes", async () => {
    const taskSummary = makeTaskSummary({
      taskId: "task-1",
      primarySessionId: "",
      sessions: [],
    });
    const workspaceSnapshot = makeWorkspaceSnapshot({ "task-1": taskSummary }, ["task-1"]);
    const supervisor = makeSupervisor();
    const workspaceSnapshotStore = makeWorkspaceSnapshotStore(workspaceSnapshot);
    const workbenchStore = makeWorkbenchStore("task-1", "session-stale");

    renderHarness({
      activeTaskId: "task-1",
      activeSessionIdFromTab: "session-stale",
      activeTaskSummary: taskSummary,
      tasksById: { "task-1": taskSummary },
      workspaceSnapshot,
      sessionSnap: makeSessionSnapshot({
        "session-stale": {
          ...makeSessionEntry({ session: makeSession("session-stale", "task-1", "active") }),
          loadState: "live",
        },
      }),
      optimisticTasks: [] satisfies OptimisticTaskSummary[],
      optimisticTasksById: {},
      supervisor,
      workbenchStore,
      workspaceSnapshotStore,
      markTaskRead: vi.fn(async () => {}),
    });

    await waitFor(() => {
      expect(workbenchStore.setActiveSessionForActiveTask).toHaveBeenCalledWith(null, { source: "system" });
    });
  });
});
