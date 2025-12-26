/// <reference types="vitest" />
import { afterEach, describe, expect, it, vi } from "vitest";

import { buildDummyWorkspaceSnapshot } from "../test/fixtures/dummyWorkspace";

vi.mock("../api/client", async () => {
  const actual = /** @type {Record<string, unknown>} */ (await vi.importActual("../api/client"));
  return {
    ...actual,
    getWorkspaceCatchup: vi.fn(),
  };
});

import { getWorkspaceCatchup, idToString } from "../api/client";
import { WorkspaceCatchupStoreImpl } from "./workspaceCatchupStore";

const conn = { baseUrl: "https://example.com", token: "test-token" };

async function waitForCondition(cond, timeoutMs = 1000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (cond()) return;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("Timed out waiting for condition");
}

describe("WorkspaceCatchupStore", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it("hydrates snapshot data from catchup", async () => {
    const snapshot = buildDummyWorkspaceSnapshot({ taskCount: 2, tracksPerTask: 1, sessionsPerTrack: 2 });
    vi.mocked(getWorkspaceCatchup).mockResolvedValue(snapshot);

    const workspaceId = idToString(snapshot.workspace_id);
    const store = new WorkspaceCatchupStoreImpl(workspaceId, conn, { streamEnabled: false });
    store.init();

    await waitForCondition(() => store.getSnapshot().initialized);

    const state = store.getSnapshot();
    expect(state.activeIds.length).toBe(snapshot.active.tasks.length);
    expect(getWorkspaceCatchup).toHaveBeenCalledWith(
      conn,
      workspaceId,
      expect.objectContaining({ includeArchived: false }),
    );

    store.destroy();
  });
});
