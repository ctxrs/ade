import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Message,
  Session,
  SessionActivityState,
  SessionEvent,
  SessionHead,
  SessionHeadSnapshot,
  SessionTurn,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import { waitForCondition } from "../testUtils/waitForCondition";
import { SessionReplicaCore } from "./sessionReplicaCore";
import type { SessionReplicaPatch } from "./sessionReplicaProtocol";
import { loadSessionHeadV1 } from "./uiStateStore";

vi.mock("./uiStateStore", () => ({
  clearSessionHeadV1: vi.fn(async () => {}),
  clearSessionHistoryPagesV1: vi.fn(async () => {}),
  loadSessionHeadV1: vi.fn(async () => null),
  saveSessionHeadV1: vi.fn(async () => {}),
}));

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

const loadSessionHeadV1Mock = vi.mocked(loadSessionHeadV1);

describe("SessionReplicaCore", () => {
  beforeEach(() => {
    loadSessionHeadV1Mock.mockReset();
    loadSessionHeadV1Mock.mockResolvedValue(null);
  });

  it("skips bounded cached bootstrap heads when requested during active open", async () => {
    const sessionId = "session-bounded-bootstrap-skip";
    const cachedHead = {
      session: mkSession(sessionId),
      turns: [] as SessionTurn[],
      events: [] as SessionEvent[],
      messages: [
        {
          id: `m-${sessionId}-cached`,
          session_id: sessionId,
          task_id: "task-1",
          role: "assistant",
          content: "cached-bootstrap",
          delivery: "immediate",
          created_at: new Date().toISOString(),
        },
      ],
      last_event_seq: 1,
      has_more_turns: true,
      head_window: {
        turn_limit: 40,
        message_limit: 120,
        event_limit: 120,
        byte_limit: 1024 * 1024,
        turn_count: 40,
        message_count: 40,
        event_count: 40,
        bytes: 4096,
      },
    } satisfies SessionHead;
    const authoritativeHead = {
      ...mkHead(sessionId, "authoritative-head"),
      last_event_seq: 2,
      state_rev: 2,
      head_window: {
        turn_limit: 40,
        message_limit: 120,
        event_limit: 120,
        byte_limit: 1024 * 1024,
        turn_count: 40,
        message_count: 40,
        event_count: 40,
        bytes: 4096,
      },
    } satisfies SessionHeadSnapshot;
    loadSessionHeadV1Mock.mockResolvedValueOnce({
      v: 1,
      sessionId,
      updatedAtMs: Date.now(),
      head: cachedHead,
    });
    const getSessionHead = vi.fn(async () => authoritativeHead);
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "open_session",
      sessionId,
      hydrateIfNeeded: true,
      skipBoundedBootstrapCache: true,
    });

    await waitForCondition(() =>
      patches.some(
        (patch) =>
          patch.op !== "evict" &&
          patch.sessionId === sessionId &&
          patch.data.messages?.some((message) => message.content === "authoritative-head"),
      ),
    );

    expect(getSessionHead).toHaveBeenCalledTimes(1);
    expect(
      patches.some(
        (patch) =>
          patch.op !== "evict" &&
          patch.sessionId === sessionId &&
          patch.data.freshness === "bootstrap" &&
          patch.data.messages?.some((message) => message.content === "cached-bootstrap"),
      ),
    ).toBe(false);
  });

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

  it("emits explicit lifecycle replace modes for bootstrap seeds and authoritative repairs", () => {
    const sessionId = "session-replace-modes";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: mkHead(sessionId, "bootstrap-head"),
      mode: "bootstrap_seed",
    });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        ...mkHead(sessionId, "repair-head"),
        last_event_seq: 2,
        projection_rev: 2,
        activity: { is_working: false, last_turn_status: "completed" },
      },
      mode: "repair_replace",
    });

    const replacePatches = patches.filter(
      (patch): patch is Exclude<SessionReplicaPatch, { op: "evict" }> =>
        patch.sessionId === sessionId && patch.op === "replace",
    );
    expect(replacePatches).toHaveLength(2);
    expect(replacePatches[0]?.data.replaceMode).toBe("bootstrap_seed");
    expect(replacePatches[0]?.data.freshness).toBe("bootstrap");
    expect(replacePatches[1]?.data.replaceMode).toBe("repair_replace");
    expect(replacePatches[1]?.data.freshness).toBe("authoritative");
  });

  it("accepts repair_replace heads when last_event_seq advances even if projection_rev trails", () => {
    const sessionId = "session-repair-seq-dominates";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    const createdAt = "2026-03-09T00:00:01.000Z";

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "running",
            start_seq: 1,
            end_seq: null,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: "",
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [] as SessionEvent[],
        messages: [] as Message[],
        activity: { is_working: true, last_turn_status: "running" },
        last_event_seq: 4,
        projection_rev: 7,
        state_rev: 7,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "completed",
            start_seq: 1,
            end_seq: 5,
            started_at: createdAt,
            updated_at: "2026-03-09T00:00:05.000Z",
            assistant_partial: null,
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [] as SessionEvent[],
        messages: [] as Message[],
        activity: { is_working: false, last_turn_status: "completed" },
        last_event_seq: 5,
        projection_rev: 6,
        state_rev: 7,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "repair_replace",
    });

    const latest = [...patches].reverse().find(
      (patch) => patch.sessionId === sessionId && patch.op === "replace",
    );
    if (!latest || latest.op === "evict") {
      throw new Error("expected repair replace patch");
    }

    expect(latest.data.replaceMode).toBe("repair_replace");
    expect(latest.data.lastEventSeq).toBe(5);
    expect(latest.data.activity).toEqual({ is_working: false, last_turn_status: "completed" });
    expect(latest.data.turns?.[0]?.status).toBe("completed");
    expect(latest.data.turns?.[0]?.end_seq).toBe(5);
  });

  it("keeps terminal turns when merging an older cached bootstrap head", async () => {
    const sessionId = "session-bootstrap-cache-merge";
    const patches: SessionReplicaPatch[] = [];
    const getSessionHead = vi.fn();
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });
    const createdAt = "2026-03-09T00:00:01.000Z";

    loadSessionHeadV1Mock.mockResolvedValueOnce({
      v: 1,
      sessionId,
      updatedAtMs: Date.now(),
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "running",
            start_seq: 1,
            end_seq: null,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: "",
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [] as SessionEvent[],
        messages: [] as Message[],
        activity: { is_working: true, last_turn_status: "running" },
        last_event_seq: 4,
        projection_rev: 6,
        has_more_turns: false,
        head_window: undefined,
        summary_checkpoint: null,
      } satisfies SessionHead,
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "completed",
            start_seq: 1,
            end_seq: 5,
            started_at: createdAt,
            updated_at: "2026-03-09T00:00:05.000Z",
            assistant_partial: null,
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [] as SessionEvent[],
        messages: [] as Message[],
        activity: { is_working: false, last_turn_status: "completed" },
        last_event_seq: 5,
        projection_rev: 7,
        state_rev: 7,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "repair_replace",
    });
    core.handleCommand({ type: "open_session", sessionId, force: true });

    await waitForCondition(() =>
      patches.some(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op === "replace" &&
          patch.data.freshness === "bootstrap",
      ),
    );

    const latest = [...patches].reverse().find(
      (patch) => patch.sessionId === sessionId && patch.op === "replace",
    );
    if (!latest || latest.op === "evict") {
      throw new Error("expected bootstrap replace patch");
    }

    expect(getSessionHead).not.toHaveBeenCalled();
    expect(latest.data.activity).toEqual({ is_working: false, last_turn_status: "completed" });
    expect(latest.data.turns?.[0]?.status).toBe("completed");
    expect(latest.data.turns?.[0]?.end_seq).toBe(5);
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

  it("refetches /head on explicit refresh even after authoritative hydration", async () => {
    const sessionId = "session-refresh";
    const initialHead = mkHead(sessionId, "initial");
    const refreshedHead = {
      ...mkHead(sessionId, "refreshed"),
      last_event_seq: 2,
      state_rev: 2,
    };
    const getSessionHead = vi
      .fn(async () => initialHead)
      .mockResolvedValueOnce(initialHead)
      .mockResolvedValueOnce(refreshedHead);
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({ type: "hydrate_session_head", sessionId });
    await waitForCondition(() => getSessionHead.mock.calls.length === 1);

    core.handleCommand({ type: "refresh_session", sessionId });
    await waitForCondition(() => getSessionHead.mock.calls.length === 2);

    const latest = [...patches].reverse().find(
      (patch: SessionReplicaPatch) =>
        patch.sessionId === sessionId
        && patch.op !== "evict"
        && patch.data.messages?.some((message) => message.content === "refreshed"),
    );
    if (!latest || latest.op === "evict") {
      throw new Error("expected refreshed authoritative patch");
    }

    expect(getSessionHead).toHaveBeenCalledTimes(2);
    expect(latest.data.freshness).toBe("authoritative");
    expect(latest.data.lastEventSeq).toBe(2);
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

  it("emits repair_replace for bounded session_head_seed events", async () => {
    const getSessionHead = vi.fn();
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead },
      emit: (next) => patches.push(...next),
    });
    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });

    const seedHead = {
      ...mkHead("session-seed-bounded", "from-bounded-seed"),
      head_window: {
        turn_limit: 0,
        message_limit: 50,
        event_limit: 800,
        byte_limit: 200_000,
        turn_count: 1,
        message_count: 1,
        event_count: 0,
        bytes: 256,
        truncated: true,
      },
    };
    const seedEvent: WorkspaceActiveSnapshotEvent = {
      type: "session_head_seed",
      workspace_id: "ws-1",
      snapshot_rev: 1,
      head: seedHead,
    };
    core.handleCommand({ type: "workspace_event", event: seedEvent });

    await waitForCondition(() => patches.some((patch) => patch.sessionId === "session-seed-bounded"));
    const lastPatch = patches.filter((patch) => patch.sessionId === "session-seed-bounded").slice(-1)[0];
    if (lastPatch?.op === "evict") {
      throw new Error("expected append/replace patch");
    }
    expect(lastPatch?.data?.replaceMode).toBe("repair_replace");
    expect(lastPatch?.data?.messages?.[0]?.content).toBe("from-bounded-seed");
  });

  it("repairs narrower unbounded session_head_seed events instead of dropping loaded history", async () => {
    const sessionId = "session-seed-covered-history";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });

    const fullHead: SessionHeadSnapshot = {
      session: mkSession(sessionId),
      turns: [
        {
          turn_id: "turn-1",
          session_id: sessionId,
          status: "completed",
          start_seq: 1,
          started_at: "2026-03-09T00:00:01.000Z",
          updated_at: "2026-03-09T00:00:01.000Z",
          tool_total: 0,
          tool_pending: 0,
          tool_running: 0,
          tool_completed: 0,
          tool_failed: 0,
        },
        {
          turn_id: "turn-2",
          session_id: sessionId,
          status: "completed",
          start_seq: 3,
          started_at: "2026-03-09T00:00:02.000Z",
          updated_at: "2026-03-09T00:00:02.000Z",
          tool_total: 0,
          tool_pending: 0,
          tool_running: 0,
          tool_completed: 0,
          tool_failed: 0,
        },
      ],
      events: [] as SessionEvent[],
      messages: [
        {
          id: "m-1",
          session_id: sessionId,
          task_id: "task-1",
          turn_id: "turn-1",
          role: "assistant",
          content: "older",
          delivery: "immediate",
          created_at: "2026-03-09T00:00:01.000Z",
        },
        {
          id: "m-2",
          session_id: sessionId,
          task_id: "task-1",
          turn_id: "turn-2",
          role: "assistant",
          content: "newer",
          delivery: "immediate",
          created_at: "2026-03-09T00:00:02.000Z",
        },
      ],
      last_event_seq: 10,
      projection_rev: 7,
      state_rev: 7,
      has_more_turns: false,
      has_more_history: false,
      history_cursor: null,
      head_window: {
        turn_limit: 0,
        message_limit: 0,
        event_limit: 0,
        byte_limit: 0,
        turn_count: 2,
        message_count: 2,
        event_count: 0,
        bytes: 512,
        truncated: false,
      },
    };
    core.handleCommand({ type: "seed_head", sessionId, head: fullHead, mode: "repair_replace" });

    const compactHead: SessionHeadSnapshot = {
      ...fullHead,
      turns: [fullHead.turns[1]!],
      messages: [fullHead.messages[1]!],
      last_event_seq: 20,
      projection_rev: 9,
    };
    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_seed",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        head: compactHead,
      },
    });

    await waitForCondition(() =>
      patches.some(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op === "replace" &&
          patch.data.lastEventSeq === 20,
      ),
    );

    const lastPatch = [...patches]
      .reverse()
      .find((patch) => patch.sessionId === sessionId && patch.op === "replace");
    if (!lastPatch || lastPatch.op === "evict") {
      throw new Error("expected replace patch");
    }
    expect(lastPatch.data.replaceMode).toBe("repair_replace");
    expect(lastPatch.data.messages?.map((message) => message.id)).toEqual(["m-1", "m-2"]);
    expect(lastPatch.data.turns?.map((turn) => turn.turn_id)).toEqual(["turn-1", "turn-2"]);
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

  it("ignores summary activity deltas and advances canonical activity from session_head_deltas only", () => {
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

    expect(
      patches.some(
        (patch) =>
          patch.sessionId === sessionId &&
          patch.op === "append" &&
          patch.data?.activity?.last_turn_status === "completed",
      ),
    ).toBe(false);

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
          projection_rev: 3,
          state_rev: 3,
          activity: { is_working: true, last_turn_status: "running" },
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
    expect(latest.data.activity).toEqual({ is_working: true, last_turn_status: "running" });
  });

  it("keeps canonical activity interrupted when later deltas report completed or failed", () => {
    const sessionId = "session-interrupt-activity-stable";
    const turnId = "turn-1";
    const createdAt = new Date().toISOString();
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: turnId,
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "running",
            start_seq: 1,
            end_seq: null,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: "",
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [],
        messages: [],
        last_event_seq: 1,
        projection_rev: 1,
        state_rev: 1,
        activity: { is_working: true, last_turn_status: "running" },
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          last_event_seq: 2,
          projection_rev: 2,
          state_rev: 2,
          activity: { is_working: false, last_turn_status: "interrupted" },
          event: {
            seq: 2,
            id: "event-2",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: turnId,
            event_type: "turn_interrupted",
            payload_json: { status: "interrupted" },
            transient: false,
            created_at: createdAt,
          },
        },
      },
    });
    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 3,
        delta: {
          session_id: sessionId,
          last_event_seq: 3,
          projection_rev: 3,
          state_rev: 3,
          activity: { is_working: false, last_turn_status: "completed" },
          event: {
            seq: 3,
            id: "event-3",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: turnId,
            event_type: "turn_finished",
            payload_json: { status: "completed" },
            transient: false,
            created_at: createdAt,
          },
        },
      },
    });
    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 4,
        delta: {
          session_id: sessionId,
          last_event_seq: 4,
          projection_rev: 4,
          state_rev: 4,
          activity: { is_working: false, last_turn_status: "failed" },
          event: {
            seq: 4,
            id: "event-4",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: turnId,
            event_type: "error",
            payload_json: { message: "cancelled" },
            transient: false,
            created_at: createdAt,
          },
        },
      },
    });

    const latest = [...patches].reverse().find(
      (patch) =>
        patch.sessionId === sessionId &&
        patch.op === "append" &&
        patch.data.activity?.last_turn_status === "interrupted",
    );
    if (!latest || latest.op === "evict") {
      throw new Error("expected interrupted canonical patch");
    }
    expect(latest.data.activity).toEqual({ is_working: false, last_turn_status: "interrupted" });
    expect(latest.data.turns?.at(-1)?.status).toBe("interrupted");
  });

  it("does not clear interrupted activity when later head deltas omit activity", () => {
    const sessionId = "session-interrupt-activity-null-delta";
    const turnId = "turn-1";
    const createdAt = new Date().toISOString();
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: turnId,
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "interrupted",
            start_seq: 1,
            end_seq: 2,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: "",
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [],
        messages: [],
        last_event_seq: 2,
        projection_rev: 2,
        state_rev: 2,
        activity: { is_working: false, last_turn_status: "interrupted" },
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 3,
        delta: {
          session_id: sessionId,
          last_event_seq: 3,
          projection_rev: 3,
          state_rev: 3,
          activity: null,
          event: {
            seq: 3,
            id: "event-3",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: turnId,
            event_type: "assistant_message_inserted",
            payload_json: { message_id: "m-2", content: "still interrupted" },
            transient: false,
            created_at: createdAt,
          },
          message: {
            id: "m-2",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: turnId,
            role: "assistant",
            content: "still interrupted",
            delivery: "immediate",
            created_at: createdAt,
          },
        },
      },
    });

    const latest = [...patches].reverse().find(
      (patch) =>
        patch.sessionId === sessionId &&
        patch.op === "append" &&
        patch.data.messages?.some((message) => message.id === "m-2"),
    );
    if (!latest || latest.op === "evict") {
      throw new Error("expected appended message patch");
    }
    expect(latest.data.activity).toEqual({ is_working: false, last_turn_status: "interrupted" });
    expect(latest.data.turns?.at(-1)?.status).toBe("interrupted");
  });

  it("emits canonical merged transcript state for streamed event-only deltas", () => {
    const sessionId = "session-canonical-stream-delta";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    const createdAt = new Date().toISOString();

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "running",
            start_seq: 1,
            end_seq: null,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: "",
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [],
        messages: [
          {
            id: "message-1",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: "turn-1",
            role: "user",
            content: "queued later",
            delivery: "immediate",
            created_at: createdAt,
          },
        ],
        last_event_seq: 1,
        state_rev: 1,
        activity: { is_working: true, last_turn_status: "running" },
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          last_event_seq: 2,
          projection_rev: 2,
          state_rev: 2,
          event: {
            seq: 2,
            id: "event-done",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: "turn-1",
            event_type: "done",
            payload_json: {},
            created_at: createdAt,
          },
        },
      },
    });

    const latest = [...patches].reverse().find(
      (patch) => patch.sessionId === sessionId && patch.op === "append" && Array.isArray(patch.data.turns),
    );
    if (!latest || latest.op === "evict" || !latest.data.turns || !latest.data.messages || !latest.data.events) {
      throw new Error("expected canonical append patch");
    }

    expect(latest.data.turnsRev).toBeTypeOf("number");
    expect(latest.data.messagesRev).toBeTypeOf("number");
    expect(latest.data.eventsRev).toBeTypeOf("number");
    expect(latest.data.turns[0]?.status).toBe("completed");
    expect(latest.data.messages[0]?.delivery).toBe("immediate");
    expect(latest.data.events.map((event) => event.id)).toContain("event-done");
    expect(latest.data.lastEventSeq).toBe(2);
  });

  it("clears stale assistant streaming overlay when the assistant message delta arrives before the inserted event", () => {
    const sessionId = "session-assistant-stream-cleared-by-message";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    const createdAt = new Date().toISOString();

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "running",
            start_seq: 1,
            end_seq: null,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: null,
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [],
        messages: [
          {
            id: "message-1",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: "turn-1",
            role: "user",
            content: "hi",
            delivery: "immediate",
            created_at: createdAt,
          },
        ],
        last_event_seq: 1,
        state_rev: 1,
        activity: { is_working: true, last_turn_status: "running" },
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          last_event_seq: 2,
          projection_rev: 2,
          state_rev: 2,
          event: {
            seq: 2,
            id: "event-assistant-complete",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: "turn-1",
            event_type: "assistant_complete",
            payload_json: {
              full_content: "pong",
              message_id: "provider-msg-1",
              order_seq: 2,
            },
            created_at: createdAt,
          },
        },
      },
    });

    const streamingPatch = [...patches].reverse().find(
      (patch) =>
        patch.sessionId === sessionId &&
        patch.op === "append" &&
        patch.data.assistantStreamingByTurnId?.["turn-1"]?.content === "pong",
    );
    expect(streamingPatch).toBeTruthy();

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 3,
        delta: {
          session_id: sessionId,
          last_event_seq: 3,
          projection_rev: 3,
          state_rev: 3,
          message: {
            id: "assistant-msg-1",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: "turn-1",
            role: "assistant",
            content: "pong",
            delivery: "immediate",
            created_at: createdAt,
            order_seq: 2,
          },
        },
      },
    });

    const latest = [...patches].reverse().find(
      (patch) => patch.sessionId === sessionId && patch.op === "append" && Array.isArray(patch.data.messages),
    );
    if (!latest || latest.op === "evict" || !latest.data.messages) {
      throw new Error("expected canonical append patch with assistant message");
    }

    expect(latest.data.messages.some((message) => message.role === "assistant")).toBe(true);
    expect(latest.data.assistantStreamingByTurnId ?? {}).toEqual({});
  });

  it("preserves assistant streaming continuity across close_session background deltas", () => {
    const sessionId = "session-assistant-stream-preserved-on-close";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    const createdAt = new Date().toISOString();

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "running",
            start_seq: 1,
            end_seq: null,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: null,
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [],
        messages: [
          {
            id: "message-1",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: "turn-1",
            role: "user",
            content: "hi",
            delivery: "immediate",
            created_at: createdAt,
          },
        ],
        last_event_seq: 1,
        state_rev: 1,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          last_event_seq: 2,
          projection_rev: 2,
          state_rev: 2,
          event: {
            seq: 2,
            id: "event-assistant-chunk-1",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: "turn-1",
            event_type: "assistant_chunk",
            payload_json: {
              content_fragment: "Hello ",
            },
            created_at: createdAt,
          },
        },
      },
    });

    core.handleCommand({ type: "close_session", sessionId });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 3,
        delta: {
          session_id: sessionId,
          last_event_seq: 3,
          projection_rev: 3,
          state_rev: 3,
          event: {
            seq: 3,
            id: "event-assistant-chunk-2",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: "turn-1",
            event_type: "assistant_chunk",
            payload_json: {
              content_fragment: "world",
            },
            created_at: createdAt,
          },
        },
      },
    });

    const latest = [...patches].reverse().find(
      (patch) =>
        patch.sessionId === sessionId &&
        patch.op === "append" &&
        patch.data.assistantStreamingByTurnId?.["turn-1"]?.content != null,
    );
    if (!latest || latest.op === "evict") {
      throw new Error("expected append patch with assistant streaming state");
    }

    expect(latest.data.assistantStreamingByTurnId?.["turn-1"]?.content).toBe("Hello world");
  });

  it("drops assistant streaming continuity only on explicit drop_session", () => {
    const sessionId = "session-assistant-stream-dropped-explicitly";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    const createdAt = new Date().toISOString();

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "running",
            start_seq: 1,
            end_seq: null,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: null,
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [],
        messages: [
          {
            id: "message-1",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: "turn-1",
            role: "user",
            content: "hi",
            delivery: "immediate",
            created_at: createdAt,
          },
        ],
        last_event_seq: 1,
        state_rev: 1,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          last_event_seq: 2,
          projection_rev: 2,
          state_rev: 2,
          event: {
            seq: 2,
            id: "event-assistant-chunk-1",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: "turn-1",
            event_type: "assistant_chunk",
            payload_json: {
              content_fragment: "Hello ",
            },
            created_at: createdAt,
          },
        },
      },
    });

    core.handleCommand({ type: "drop_session", sessionId });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 3,
        delta: {
          session_id: sessionId,
          last_event_seq: 3,
          projection_rev: 3,
          state_rev: 3,
          event: {
            seq: 3,
            id: "event-assistant-chunk-2",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: "turn-1",
            event_type: "assistant_chunk",
            payload_json: {
              content_fragment: "world",
            },
            created_at: createdAt,
          },
        },
      },
    });

    const latest = [...patches].reverse().find(
      (patch) =>
        patch.sessionId === sessionId &&
        patch.op === "append" &&
        patch.data.assistantStreamingByTurnId?.["turn-1"]?.content != null,
    );
    if (!latest || latest.op === "evict") {
      throw new Error("expected append patch with assistant streaming state");
    }

    expect(latest.data.assistantStreamingByTurnId?.["turn-1"]?.content).toBe("world");
  });

  it("clears stale assistant streaming overlay on repair replace", () => {
    const sessionId = "session-assistant-stream-cleared-by-repair";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    const createdAt = new Date().toISOString();

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "running",
            start_seq: 1,
            end_seq: null,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: null,
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [],
        messages: [
          {
            id: "message-1",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: "turn-1",
            role: "user",
            content: "hi",
            delivery: "immediate",
            created_at: createdAt,
          },
        ],
        last_event_seq: 1,
        state_rev: 1,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          last_event_seq: 2,
          projection_rev: 2,
          state_rev: 2,
          event: {
            seq: 2,
            id: "event-assistant-complete",
            session_id: sessionId,
            run_id: "run-1",
            turn_id: "turn-1",
            event_type: "assistant_complete",
            payload_json: {
              full_content: "pong",
              message_id: "provider-msg-1",
              order_seq: 2,
            },
            created_at: createdAt,
          },
        },
      },
    });

    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [
          {
            turn_id: "turn-1",
            session_id: sessionId,
            run_id: "run-1",
            user_message_id: "message-1",
            status: "completed",
            start_seq: 1,
            end_seq: 3,
            started_at: createdAt,
            updated_at: createdAt,
            assistant_partial: null,
            thought_partial: "",
            metrics_json: null,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
          },
        ],
        events: [],
        messages: [
          {
            id: "message-1",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: "turn-1",
            role: "user",
            content: "hi",
            delivery: "immediate",
            created_at: createdAt,
          },
          {
            id: "assistant-msg-1",
            session_id: sessionId,
            task_id: "task-1",
            turn_id: "turn-1",
            role: "assistant",
            content: "pong",
            delivery: "immediate",
            created_at: createdAt,
            order_seq: 2,
          },
        ],
        last_event_seq: 3,
        state_rev: 3,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "repair_replace",
    });

    const latest = [...patches].reverse().find(
      (patch) => patch.sessionId === sessionId && patch.op === "replace",
    );
    if (!latest || latest.op === "evict") {
      throw new Error("expected repair replace patch");
    }

    expect(latest.data.replaceMode).toBe("repair_replace");
    expect(latest.data.assistantStreamingByTurnId ?? {}).toEqual({});
  });

  it("applies queue events into canonical message state before emitting append patches", () => {
    const sessionId = "session-canonical-queue-delta";
    const patches: SessionReplicaPatch[] = [];
    const core = new SessionReplicaCore({
      api: { getSessionHead: vi.fn() },
      emit: (next) => patches.push(...next),
    });
    const createdAt = new Date().toISOString();

    core.handleCommand({ type: "init", config: { eventBufferLimit: 100, headLimit: 50 } });
    core.handleCommand({
      type: "seed_head",
      sessionId,
      head: {
        session: mkSession(sessionId),
        turns: [] as SessionTurn[],
        events: [] as SessionEvent[],
        messages: [
          {
            id: "message-queue",
            session_id: sessionId,
            task_id: "task-1",
            role: "user",
            content: "queue me",
            delivery: "immediate",
            created_at: createdAt,
          },
        ],
        last_event_seq: 1,
        state_rev: 1,
        has_more_turns: false,
        has_more_history: false,
        history_cursor: null,
      },
      mode: "bootstrap_seed",
    });

    core.handleCommand({
      type: "workspace_event",
      event: {
        type: "session_head_delta",
        workspace_id: "ws-1",
        snapshot_rev: 2,
        delta: {
          session_id: sessionId,
          last_event_seq: 2,
          projection_rev: 2,
          state_rev: 2,
          event: {
            seq: 2,
            id: "event-queue-added",
            session_id: sessionId,
            run_id: "run-1",
            event_type: "message_queue_added",
            payload_json: { message_id: "message-queue" },
            created_at: createdAt,
          },
        },
      },
    });

    const latest = [...patches].reverse().find(
      (patch) => patch.sessionId === sessionId && patch.op === "append" && Array.isArray(patch.data.messages),
    );
    if (!latest || latest.op === "evict" || !latest.data.messages) {
      throw new Error("expected canonical queue append patch");
    }

    expect(latest.data.messages[0]?.delivery).toBe("queued");
    expect(latest.data.messagesRev).toBeTypeOf("number");
    expect(latest.data.lastEventSeq).toBe(2);
  });
});
