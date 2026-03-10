import { describe, expect, it, vi } from "vitest";
import type { Message, Session, SessionEvent, SessionHeadSnapshot, SessionTurn, WorkspaceActiveSnapshotEvent } from "@ctx/types";
import { waitForCondition } from "../testUtils/waitForCondition";
import { SessionReplicaCore } from "./sessionReplicaCore";
import type { SessionReplicaPatch } from "./sessionReplicaProtocol";

const mkSession = (sessionId: string): Session => ({
  id: sessionId,
  task_id: "task-1",
  workspace_id: "ws-1",
  worktree_id: "wt-1",
  provider_id: "fake",
  model_id: "fake-model",
  title: "Session",
  agent_role: "assistant",
  status: "active",
});

const mkHead = (sessionId: string, messageText = "hello"): SessionHeadSnapshot => {
  const now = new Date().toISOString();
  const message: Message = {
    id: `m-${sessionId}`,
    session_id: sessionId,
    task_id: "task-1",
    role: "assistant",
    content: messageText,
    delivery: "immediate",
    created_at: now,
  };
  return {
    session: mkSession(sessionId),
    turns: [] as SessionTurn[],
    events: [] as SessionEvent[],
    messages: [message],
    last_event_seq: 1,
    state_rev: 1,
    has_more_turns: false,
    has_more_history: false,
    history_cursor: null,
  };
};

describe("SessionReplicaCore", () => {
  it("keeps open_session stream-only by default", async () => {
    const getSessionHead = vi.fn();
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({ type: "open_session", sessionId: "session-stream-only" });

    await waitForCondition(() => patches.length > 0);
    expect(getSessionHead).not.toHaveBeenCalled();
    const latest = patches[patches.length - 1];
    if (latest?.op === "evict") {
      throw new Error("expected append/replace patch");
    }
    expect(latest?.data?.error).toBeFalsy();
  });

  it("does not refetch /head on session_gap", async () => {
    const head = mkHead("session-gap");
    const getSessionHead = vi.fn(async () => head);
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: () => {},
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({ type: "open_session", sessionId: "session-gap" });
    core.handleCommand({ type: "hydrate_session_head", sessionId: "session-gap" });

    await waitForCondition(() => getSessionHead.mock.calls.length === 1);
    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});

    const gapEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_gap",
      workspace_id: "ws-1",
      snapshot_rev: 2,
      session_id: "session-gap",
      after_seq: 5,
    };
    core.handleCommand({ type: "workspace_event", event: gapEvent });

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(getSessionHead).toHaveBeenCalledTimes(1);
    alertSpy.mockRestore();
  });

  it("recovers from session_gap via stream seed and delta without extra /head fetches", async () => {
    const sessionId = "session-gap-recovery";
    const head = mkHead(sessionId, "before-gap");
    const getSessionHead = vi.fn(async () => head);
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({ type: "open_session", sessionId });
    core.handleCommand({ type: "hydrate_session_head", sessionId });

    await waitForCondition(() => getSessionHead.mock.calls.length === 1);

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_gap",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        session_id: sessionId,
        after_seq: 5,
      },
    });

    const recoveredHead = {
      ...mkHead(sessionId, "recovered-from-seed"),
      last_event_seq: 2,
      state_rev: 2,
    };
    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_seed",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        head: recoveredHead,
      },
    });

    const now = new Date().toISOString();
    const deltaMessage: Message = {
      id: "m-delta",
      session_id: sessionId,
      task_id: "task-1",
      turn_id: "turn-delta",
      role: "assistant",
      content: "recovered-from-delta",
      delivery: "immediate",
      created_at: now,
    };
    const deltaEvent: SessionEvent = {
      seq: 3,
      id: "e-delta",
      session_id: sessionId,
      turn_id: "turn-delta",
      event_type: "assistant_message_inserted",
      payload_json: {
        message_id: deltaMessage.id,
        content: deltaMessage.content,
        delivery: deltaMessage.delivery,
      },
      created_at: now,
    };
    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 3,
        delta: {
          session_id: sessionId,
          last_event_seq: 3,
          state_rev: 3,
          event: deltaEvent,
          message: deltaMessage,
        },
      },
    });

    await waitForCondition(() => {
      const seedPatch = patches.find(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op !== "evict" &&
          patch.data?.messages?.some((message) => message.content === "recovered-from-seed"),
      );
      const deltaPatch = patches.find(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op === "append" &&
          patch.data?.messages?.some((message) => message.content === "recovered-from-delta") &&
          patch.data?.lastEventSeq === 3,
      );
      return Boolean(seedPatch && deltaPatch);
    });

    expect(getSessionHead).toHaveBeenCalledTimes(1);
  });

  it("hydrates /head only when explicitly requested", async () => {
    const head = mkHead("session-archived");
    const getSessionHead = vi.fn(async () => head);
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });
    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });

    core.handleCommand({ type: "open_session", sessionId: "session-archived" });
    await waitForCondition(() => patches.length > 0);
    expect(getSessionHead).not.toHaveBeenCalled();

    core.handleCommand({ type: "hydrate_session_head", sessionId: "session-archived" });
    await waitForCondition(() => getSessionHead.mock.calls.length === 1);
    expect(getSessionHead).toHaveBeenCalledTimes(1);
  });

  it("applies session_head_seed events", async () => {
    const getSessionHead = vi.fn();
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });
    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });

    const seedHead = mkHead("session-seed", "from-seed");
    const seedEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_seed",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      head: seedHead,
    };
    core.handleCommand({ type: "workspace_event", event: seedEvent });

    await waitForCondition(() => patches.some((patch) => patch.sessionId === "session-seed"));
    const lastPatch = patches.filter((patch) => patch.sessionId === "session-seed").slice(-1)[0];
    if (lastPatch?.op === "evict") {
      throw new Error("expected append/replace patch");
    }
    expect(lastPatch?.data?.messages?.[0]?.content).toBe("from-seed");
  });
});
