import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, SessionEvent } from "../api/client";
import { SessionSupervisor, type SessionCacheEntry } from "./sessionSupervisorCore";

const trackProviderRunCompleted = vi.hoisted(() => vi.fn());
const trackFirstTurnCompleted = vi.hoisted(() => vi.fn());

vi.mock("../utils/analytics", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../utils/analytics")>();
  return {
    ...actual,
    trackProviderRunCompleted,
    trackFirstTurnCompleted,
  };
});

const baseIso = "2024-01-01T00:00:00.000Z";

type SessionSupervisorInternals = {
  ensureEntry: (sessionId: string) => SessionCacheEntry;
  ensureTurnFromEvent: (entry: SessionCacheEntry, event: SessionEvent) => boolean;
  applyEventToTurns: (
    entry: SessionCacheEntry,
    event: SessionEvent,
    opts?: { notify?: boolean },
  ) => boolean;
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

describe("SessionSupervisor analytics tracking", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("does not emit terminal analytics during replay (notify=false)", () => {
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

    const changed = asSupervisorInternals(supervisor).applyEventToTurns(entry, finishEvent, {
      notify: false,
    });
    expect(changed).toBe(true);
    expect(trackProviderRunCompleted).not.toHaveBeenCalled();
    expect(trackFirstTurnCompleted).not.toHaveBeenCalled();
  });

  it("emits terminal analytics for live transitions (notify=true)", () => {
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

    const changed = asSupervisorInternals(supervisor).applyEventToTurns(entry, finishEvent, {
      notify: true,
    });
    expect(changed).toBe(true);
    expect(trackProviderRunCompleted).toHaveBeenCalledTimes(1);
    expect(trackFirstTurnCompleted).toHaveBeenCalledTimes(1);
  });
});
