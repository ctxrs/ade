/// <reference types="vitest" />
import { afterEach, describe, expect, it, vi } from "vitest";

import { buildDummyWorkspaceSnapshot } from "../test/fixtures/dummyWorkspace";

vi.mock("../utils/e2ee", () => ({
  decryptPayload: vi.fn(),
}));

vi.mock("../api/client", () => ({
  getWorkspaceActiveSnapshot: vi.fn(),
  idToString: (value) => {
    if (typeof value === "string") return value;
    if (value && typeof value === "object" && "0" in value) {
      return String(value["0"]);
    }
    return value ? String(value) : "";
  },
}));

import { getWorkspaceActiveSnapshot, idToString } from "../api/client";
import { WorkspaceActiveSnapshotStoreImpl } from "./workspaceActiveSnapshotStore";

const conn = { baseUrl: "https://example.com", token: "test-token" };

async function waitForCondition(cond, timeoutMs = 1000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (cond()) return;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("Timed out waiting for condition");
}

describe("WorkspaceActiveSnapshotStore", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it("hydrates snapshot data from active snapshot", async () => {
    const snapshot = buildDummyWorkspaceSnapshot({ taskCount: 2, sessionsPerTask: 2 });
    vi.mocked(getWorkspaceActiveSnapshot).mockResolvedValue(snapshot);

    const workspaceId = idToString(snapshot.workspace_id);
    const store = new WorkspaceActiveSnapshotStoreImpl(workspaceId, conn, { streamEnabled: false });
    store.init();

    await waitForCondition(() => store.getSnapshot().initialized);

    const state = store.getSnapshot();
    expect(state.activeIds.length).toBe(snapshot.active.tasks.length);
    expect(getWorkspaceActiveSnapshot).toHaveBeenCalledWith(
      conn,
      workspaceId,
      expect.objectContaining({ limit: 50 }),
    );

    store.destroy();
  });
});
