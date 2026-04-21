import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Message, Session, SessionEvent } from "../api/client";
import { SessionSupervisor, type SessionCacheEntry } from "./sessionSupervisorCore";

const sendDesktopNotification = vi.hoisted(() => vi.fn());
const isAppInForeground = vi.hoisted(() => vi.fn());
const getClientSettingsState = vi.hoisted(() => vi.fn());

vi.mock("../utils/desktopNotifications", () => ({
  sendDesktopNotification,
}));

vi.mock("../utils/windowFocus", () => ({
  isAppInForeground,
}));

vi.mock("./clientSettings", () => ({
  getClientSettingsState,
}));

const baseIso = "2024-01-01T00:00:00.000Z";

type SessionSupervisorInternals = {
  ensureEntry: (sessionId: string) => SessionCacheEntry;
  ensureTurnFromEvent: (entry: SessionCacheEntry, event: SessionEvent) => boolean;
  applyEventToTurns: (entry: SessionCacheEntry, event: SessionEvent) => boolean;
};

const asSupervisorInternals = (supervisor: SessionSupervisor): SessionSupervisorInternals =>
  supervisor as unknown as SessionSupervisorInternals;

const makeWorkspaceSnapshot = (taskTitle: string) => ({
  workspaceId: "workspace-1",
  initialized: true,
  liveSnapshotApplied: true,
  connection: "connected" as const,
  tasksById: {
    "task-1": {
      id: "task-1",
      task: {
        id: "task-1",
        workspace_id: "workspace-1",
        title: taskTitle,
        status: "active",
        primary_session_id: "session-1",
        primary_worktree_id: "worktree-1",
        created_at: baseIso,
        updated_at: baseIso,
      },
      sessions: [],
      sortAtMs: Date.parse(baseIso),
    },
  },
  activeIds: ["task-1"],
  archivedIds: [],
  totalActive: 1,
  totalArchived: 0,
  archivedRev: 0,
  worktreeVcsById: {},
  fetchState: {
    active: "idle" as const,
    archived: "idle" as const,
  },
  hasMoreActive: false,
  hasMoreArchived: false,
  archivedLoaded: true,
});

const setupEntry = () => {
  const supervisor = new SessionSupervisor();
  const internals = asSupervisorInternals(supervisor);
  const entry = internals.ensureEntry("session-1");
  const session: Session = {
    id: "session-1",
    task_id: "task-1",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    provider_id: "codex",
    model_id: "gpt-5",
    agent_role: "implementer",
    status: "active",
    title: "Demo session",
    created_at: baseIso,
  };
  entry.session = session;

  const startEvent: SessionEvent = {
    seq: 1,
    id: "ev-1",
    session_id: "session-1",
    turn_id: "turn-1",
    event_type: "turn_started",
    payload_json: {},
    created_at: baseIso,
  };
  internals.ensureTurnFromEvent(entry, startEvent);

  return { supervisor, entry, turnId: "turn-1" };
};

describe("SessionSupervisor turn notifications", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getClientSettingsState.mockReturnValue({
      loaded: true,
      settings: {
        v: 2,
        desktopNotifications: {
          turnCompleted: true,
          turnFailed: true,
          badgeUnreadCount: true,
        },
      },
    });
    isAppInForeground.mockReturnValue(false);
  });

  it("sends a notification when a turn completes while unfocused", () => {
    const { supervisor, entry, turnId } = setupEntry();
    supervisor.setWorkspaceSnapshotState(makeWorkspaceSnapshot("Implement desktop notifications"));
    entry.messages = [
      {
        id: "message-1",
        session_id: "session-1",
        task_id: "task-1",
        turn_id: turnId,
        role: "assistant",
        content: "## Update\n\nWired Dock badge aggregation across open workspaces.",
        delivery: "immediate",
        created_at: baseIso,
      } satisfies Message,
    ];
    const finishEvent: SessionEvent = {
      seq: 2,
      id: "ev-2",
      session_id: "session-1",
      turn_id: turnId,
      event_type: "turn_finished",
      payload_json: {},
      created_at: baseIso,
    };

    const changed = asSupervisorInternals(supervisor).applyEventToTurns(entry, finishEvent);
    expect(changed).toBe(true);
    expect(sendDesktopNotification).toHaveBeenCalledWith({
      kind: "turn_completed",
      title: "Implement desktop notifications",
      body: "Update Wired Dock badge aggregation across open workspaces.",
      workspaceId: "workspace-1",
      taskId: "task-1",
      sessionId: "session-1",
    });
  });

  it("skips notifications when the app is focused", () => {
    isAppInForeground.mockReturnValue(true);
    const { supervisor, entry, turnId } = setupEntry();
    const finishEvent: SessionEvent = {
      seq: 2,
      id: "ev-2",
      session_id: "session-1",
      turn_id: turnId,
      event_type: "turn_finished",
      payload_json: {},
      created_at: baseIso,
    };

    asSupervisorInternals(supervisor).applyEventToTurns(entry, finishEvent);
    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });

  it("skips notifications when the setting is disabled", () => {
    getClientSettingsState.mockReturnValue({
      loaded: true,
      settings: {
        v: 2,
        desktopNotifications: {
          turnCompleted: false,
          turnFailed: true,
          badgeUnreadCount: true,
        },
      },
    });
    const { supervisor, entry, turnId } = setupEntry();
    const finishEvent: SessionEvent = {
      seq: 2,
      id: "ev-2",
      session_id: "session-1",
      turn_id: turnId,
      event_type: "turn_finished",
      payload_json: {},
      created_at: baseIso,
    };

    asSupervisorInternals(supervisor).applyEventToTurns(entry, finishEvent);
    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });

  it("uses the error event message when a failed turn has no assistant message", () => {
    const { supervisor, entry, turnId } = setupEntry();
    supervisor.setWorkspaceSnapshotState(makeWorkspaceSnapshot("Fix login race"));
    const errorEvent: SessionEvent = {
      seq: 2,
      id: "ev-2",
      session_id: "session-1",
      turn_id: turnId,
      event_type: "error",
      payload_json: {
        message: "OAuth token has expired. Please reconnect.",
      },
      created_at: baseIso,
    };
    const finishEvent: SessionEvent = {
      seq: 3,
      id: "ev-3",
      session_id: "session-1",
      turn_id: turnId,
      event_type: "turn_finished",
      payload_json: {
        status: "failed",
      },
      created_at: baseIso,
    };

    entry.events = [errorEvent];
    asSupervisorInternals(supervisor).applyEventToTurns(entry, errorEvent);
    asSupervisorInternals(supervisor).applyEventToTurns(entry, finishEvent);

    expect(sendDesktopNotification).toHaveBeenCalledWith({
      kind: "turn_failed",
      title: "Fix login race",
      body: "OAuth token has expired. Please reconnect.",
      workspaceId: "workspace-1",
      taskId: "task-1",
      sessionId: "session-1",
    });
  });
});
