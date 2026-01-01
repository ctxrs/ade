/// <reference types="vitest" />
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../api/client", () => ({
  getSessionHead: vi.fn(),
  getSessionHistory: vi.fn(),
  listTurnTools: vi.fn(async () => []),
  fetchTrackDiff: vi.fn(async () => ({ diff: "" })),
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

import { getSessionHead } from "../api/client";
import { SessionSupervisor } from "./sessionSupervisor";

const conn = { baseUrl: "https://example.com", token: "test-token" };

const mkHead = (sessionId: string) => ({
  session: {
    id: sessionId,
    track_id: `track-${sessionId}`,
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
});

async function waitForCondition(cond: () => boolean, timeoutMs = 1000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (cond()) return;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("Timed out waiting for condition");
}

describe("SessionSupervisor warm heads", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("warms at most the budgeted session heads", async () => {
    vi.mocked(getSessionHead).mockImplementation(async (_conn, sessionId) => mkHead(String(sessionId)) as any);

    const sup = new SessionSupervisor(conn);
    const ids = Array.from({ length: 25 }, (_, i) => `session-${i + 1}`);
    sup.setActiveTaskSessionIds(ids);

    await waitForCondition(() => vi.mocked(getSessionHead).mock.calls.length > 0);

    // Budget is derived at module init; in tests (no localStorage) it should default to 12.
    await waitForCondition(() => vi.mocked(getSessionHead).mock.calls.length === 12);

    const warmed = Object.values(sup.getSnapshot().sessions).filter((s) => s.subscribed);
    expect(warmed.length).toBe(12);
  });

  it("does not refetch an already hydrated head when opening the same session again", async () => {
    vi.mocked(getSessionHead).mockResolvedValueOnce(mkHead("session-1") as any);

    const sup = new SessionSupervisor(conn);
    sup.openSession("session-1");

    await waitForCondition(() => {
      const entry = sup.getSnapshot().sessions["session-1"];
      return Boolean(entry && !entry.loading);
    });

    vi.mocked(getSessionHead).mockClear();
    sup.openSession("session-1");

    // Give the openSession call a tick to run ensureLoaded.
    await new Promise((r) => setTimeout(r, 0));
    expect(getSessionHead).not.toHaveBeenCalled();
  });
});
