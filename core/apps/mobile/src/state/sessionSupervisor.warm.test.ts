/// <reference types="vitest" />
import { afterEach, describe, expect, it, vi } from "vitest";
import { waitForCondition } from "../test/utils/waitForCondition";

vi.mock("../api/client", () => ({
  getSessionSnapshot: vi.fn(),
  getSessionHistory: vi.fn(),
  listTurnTools: vi.fn(async () => []),
  getSessionDiff: vi.fn(async () => ({ diff: "" })),
  idToString: (value) => {
    if (typeof value === "string") return value;
    if (value && typeof value === "object" && "0" in value) {
      return String(value["0"]);
    }
    return value ? String(value) : "";
  },
}));

vi.mock("./uiStateStore", () => ({
  loadSessionHeadV1: vi.fn(async () => null),
  saveSessionHeadV1: vi.fn(async () => {}),
}));

import { getSessionSnapshot } from "../api/client";
import { SessionSupervisor } from "./sessionSupervisor";

const conn = { baseUrl: "https://example.com", token: "test-token" };

const mkHead = (sessionId: string) => ({
  session: {
    id: sessionId,
    task_id: "task-1",
    workspace_id: "ws-1",
    worktree_id: "wt-1",
    provider_id: "codex",
    model_id: "gpt-4",
    title: "New Task",
    agent_role: "assistant",
    status: "active",
  },
  turns: [],
  events: [],
  messages: [],
  last_event_seq: 0,
  has_more_turns: false,
  has_more_history: false,
  history_cursor: null,
});

describe("SessionSupervisor warm heads", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("warms at most the budgeted session heads", async () => {
    vi.mocked(getSessionSnapshot).mockImplementation(async (_conn, sessionId) => {
      const head = mkHead(String(sessionId));
      return {
        summary: {
          session: head.session,
          last_message_at: null,
          last_message_preview: null,
          last_event_seq: head.last_event_seq,
          activity: { is_working: false, last_turn_status: null },
          unread: false,
        },
        head,
      } as any;
    });

    const sup = new SessionSupervisor(conn);
    const ids = Array.from({ length: 25 }, (_, i) => `session-${i + 1}`);
    sup.setActiveTaskSessionIds(ids);

    await waitForCondition(() => vi.mocked(getSessionSnapshot).mock.calls.length > 0);

    // Budget is derived at module init; in tests (no localStorage) it should default to 12.
    await waitForCondition(() => vi.mocked(getSessionSnapshot).mock.calls.length === 12);

    const warmed = Object.values(sup.getSnapshot().sessions).filter((s) => s.subscribed);
    expect(warmed.length).toBe(12);
  });

  it("does not refetch an already hydrated head when opening the same session again", async () => {
    const head = mkHead("session-1");
    vi.mocked(getSessionSnapshot).mockResolvedValueOnce({
      summary: {
        session: head.session,
        last_message_at: null,
        last_message_preview: null,
        last_event_seq: head.last_event_seq,
        activity: { is_working: false, last_turn_status: null },
        unread: false,
      },
      head,
    } as any);

    const sup = new SessionSupervisor(conn);
    sup.openSession("session-1");

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions["session-1"];
      return Boolean(entry && !entry.loading);
    });

    vi.mocked(getSessionSnapshot).mockClear();
    sup.openSession("session-1");

    // Give the openSession call a tick to run ensureLoaded.
    await new Promise((r) => setTimeout(r, 0));
    expect(getSessionSnapshot).not.toHaveBeenCalled();
  });
});
