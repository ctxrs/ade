import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, SessionEvent } from "../api/client";
import { SessionSupervisor, type SessionCacheEntry } from "./sessionSupervisorCore";

const sendDesktopNotification = vi.hoisted(() => vi.fn());
const isAppInForeground = vi.hoisted(() => vi.fn());
const getClientSettings = vi.hoisted(() => vi.fn());

vi.mock("../utils/desktopNotifications", () => ({
  sendDesktopNotification,
}));

vi.mock("../utils/windowFocus", () => ({
  isAppInForeground,
}));

vi.mock("./clientSettings", () => ({
  getClientSettings,
}));

const baseIso = "2024-01-01T00:00:00.000Z";

type SessionSupervisorInternals = {
  ensureEntry: (sessionId: string) => SessionCacheEntry;
  ensureTurnFromEvent: (entry: SessionCacheEntry, event: SessionEvent) => boolean;
  applyEventToTurns: (entry: SessionCacheEntry, event: SessionEvent) => boolean;
};

const asSupervisorInternals = (supervisor: SessionSupervisor): SessionSupervisorInternals =>
  supervisor as unknown as SessionSupervisorInternals;

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
    getClientSettings.mockReturnValue({
      v: 1,
      desktopNotifications: { turnCompleted: true },
    });
    isAppInForeground.mockReturnValue(false);
  });

  it("sends a notification when a turn completes while unfocused", () => {
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

    const changed = asSupervisorInternals(supervisor).applyEventToTurns(entry, finishEvent);
    expect(changed).toBe(true);
    expect(sendDesktopNotification).toHaveBeenCalledWith({
      title: "Turn completed",
      body: "Demo session",
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
    getClientSettings.mockReturnValue({
      v: 1,
      desktopNotifications: { turnCompleted: false },
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
});
