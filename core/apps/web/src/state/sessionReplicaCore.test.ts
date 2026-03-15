import { describe, expect, it, vi } from "vitest";
import type {
  Message,
  Session,
  SessionActivityState,
  SessionEvent,
  SessionHeadSnapshot,
  SessionTurn,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
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

  it("refetches /head on session_gap", async () => {
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

    await waitForCondition(() => getSessionHead.mock.calls.length === 2);
    expect(getSessionHead).toHaveBeenCalledTimes(2);
    alertSpy.mockRestore();
  });

  it("recovers from session_gap via authoritative /head rehydrate and resumed deltas", async () => {
    const sessionId = "session-gap-recovery";
    const initialHead = mkHead(sessionId, "before-gap");
    const recoveredHead = {
      ...mkHead(sessionId, "recovered-from-head"),
      last_event_seq: 2,
      state_rev: 2,
    };
    const getSessionHead = vi
      .fn(async () => initialHead)
      .mockResolvedValueOnce(initialHead)
      .mockResolvedValueOnce(recoveredHead);
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

    await waitForCondition(() => getSessionHead.mock.calls.length === 2);

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
      const headPatch = patches.find(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op !== "evict" &&
          patch.data?.messages?.some((message) => message.content === "recovered-from-head") &&
          patch.data?.lastEventSeq === 2,
      );
      const deltaPatch = patches.find(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op === "append" &&
          patch.data?.messages?.some((message) => message.content === "recovered-from-delta") &&
          patch.data?.lastEventSeq === 3,
      );
      return Boolean(headPatch && deltaPatch);
    });

    expect(getSessionHead).toHaveBeenCalledTimes(2);
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

  it("preserves newer streamed state when an older /head hydrate resolves later", async () => {
    const sessionId = "session-stale-head";
    let resolveHead: (value: SessionHeadSnapshot | null) => void = () => {
      throw new Error("pending /head resolver was not initialized");
    };
    const getSessionHead = vi.fn(
      () =>
        new Promise<SessionHeadSnapshot | null>((resolve) => {
          resolveHead = resolve;
        }),
    );
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({ type: "open_session", sessionId });
    core.handleCommand({ type: "hydrate_session_head", sessionId });

    await waitForCondition(() => getSessionHead.mock.calls.length === 1);

    const now = new Date().toISOString();
    const deltaMessage: Message = {
      id: "m-live-delta",
      session_id: sessionId,
      task_id: "task-1",
      turn_id: "turn-live",
      role: "assistant",
      content: "from-live-delta",
      delivery: "immediate",
      created_at: now,
    };
    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          last_event_seq: 2,
          state_rev: 2,
          message: deltaMessage,
        },
      },
    });

    resolveHead({
      ...mkHead(sessionId, "from-stale-head"),
      last_event_seq: 1,
      state_rev: 1,
    });

    await waitForCondition(() =>
      patches.some(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op === "replace" &&
          patch.data?.freshness === "authoritative",
      ),
    );

    const authoritativePatch = [...patches].reverse().find(
      (patch: SessionReplicaPatch) =>
        patch.sessionId === sessionId &&
        patch.op === "replace" &&
        patch.data.freshness === "authoritative",
    );
    if (!authoritativePatch || authoritativePatch.op === "evict") {
      throw new Error("expected authoritative replace patch");
    }

    expect(authoritativePatch.data.lastEventSeq).toBe(2);
    expect(authoritativePatch.data.stateRev).toBe(2);
    expect(authoritativePatch.data.freshness).toBe("authoritative");
    expect(authoritativePatch.data.messages?.map((message: Message) => message.content)).toEqual(
      expect.arrayContaining(["from-stale-head", "from-live-delta"]),
    );
  });

  it("does not overwrite newer summary activity on later session_head_deltas", async () => {
    const sessionId = "session-activity";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    const completedActivity: SessionActivityState = {
      is_working: false,
      last_turn_status: "completed",
    };

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_summary_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          task_id: "task-1",
          activity: completedActivity,
          last_event_seq: 2,
          state_rev: 2,
        },
      },
    });

    await waitForCondition(() =>
      patches.some(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op === "append" &&
          patch.data?.activity?.last_turn_status === "completed",
      ),
    );

    const now = new Date().toISOString();
    const deltaMessage: Message = {
      id: "m-post-summary",
      session_id: sessionId,
      task_id: "task-1",
      turn_id: "turn-post-summary",
      role: "assistant",
      content: "post-summary-delta",
      delivery: "immediate",
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
          message: deltaMessage,
        },
      },
    });

    const latest = [...patches].reverse().find(
      (patch: SessionReplicaPatch) =>
        patch.sessionId === sessionId &&
        patch.op === "append" &&
        Array.isArray(patch.data.messages),
    );
    if (!latest || latest.op === "evict" || !latest.data.messages) {
      throw new Error("expected appended head-delta patch");
    }
    expect(latest.data.messages[0]?.content).toBe("post-summary-delta");
    expect(latest.data.activity).toBeUndefined();
  });
});
