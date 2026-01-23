import { afterEach, describe, expect, it, vi } from "vitest";
import { waitForCondition } from "../testUtils/waitForCondition";

vi.mock("../api/client", () => {
  const idToString = (id: any): string => (typeof id === "string" ? id : id?.["0"]);
  return {
    idToString,
    getDaemonBaseUrl: vi.fn(() => ""),
    getHealth: vi.fn(async () => ({ daemon_url: "" })),
    getSessionHead: vi.fn(async () => null),
    getWorkspaceActiveHeads: vi.fn(async () => []),
    getWorkspaceActiveSnapshot: vi.fn(),
    listWorkspaceArchivedTaskSummaries: vi.fn(async () => ({
      workspace_id: "ws-1",
      archived_rev: 0,
      tasks: [],
      total_archived: 0,
      next_cursor: null,
    })),
  };
});

vi.mock("./uiStateStore", () => ({
  loadWorkspaceActiveSnapshotV1: vi.fn(async () => null),
  saveWorkspaceActiveSnapshotV1: vi.fn(async () => {}),
}));

describe("WorkspaceActiveSnapshotStore", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("refreshes active snapshot on reset_required", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStore");
    const { getWorkspaceActiveSnapshot } = await import("../api/client");

    (getWorkspaceActiveSnapshot as any).mockResolvedValue({
      workspace_id: "ws-1",
      snapshot_rev: 5,
      archived_rev: 0,
      active: { total_count: 0, tasks: [] },
    });

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
    await (store as any).handleStreamMessage(JSON.stringify({ type: "reset_required", latest_rev: 5 }));

    await waitForCondition(() => (getWorkspaceActiveSnapshot as any).mock.calls.length === 1);

    expect(getWorkspaceActiveSnapshot).toHaveBeenCalledWith("ws-1", { limit: 50 });
    const snapshot = store.getSnapshot();
    expect(snapshot.initialized).toBe(true);
    expect(snapshot.fetchState.active).toBe("idle");
  });

  it("refreshes session head on session_gap", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStore");
    const { getSessionHead } = await import("../api/client");

    (getSessionHead as any).mockResolvedValue(null);

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
    await (store as any).handleStreamMessage(
      JSON.stringify({
        type: "session_gap",
        workspace_id: "ws-1",
        snapshot_rev: 1,
        session_id: "session-1",
        after_seq: 5,
      }),
    );

    await waitForCondition(() => (getSessionHead as any).mock.calls.length === 1);
    expect(getSessionHead).toHaveBeenCalledWith("session-1", undefined, true);
  });

  it("reloads active snapshot on snapshot rev gap", async () => {
    const { WorkspaceActiveSnapshotStoreImpl } = await import("./workspaceActiveSnapshotStore");
    const { getWorkspaceActiveSnapshot } = await import("../api/client");

    (getWorkspaceActiveSnapshot as any).mockResolvedValue({
      workspace_id: "ws-1",
      snapshot_rev: 1,
      archived_rev: 0,
      active: { total_count: 0, tasks: [] },
    });

    const store = new WorkspaceActiveSnapshotStoreImpl("ws-1");
    await (store as any).handleStreamMessage(
      JSON.stringify({ type: "ready", workspace_id: "ws-1", snapshot_rev: 1, archived_rev: 0 }),
    );
    await waitForCondition(() => (getWorkspaceActiveSnapshot as any).mock.calls.length === 1);

    (getWorkspaceActiveSnapshot as any).mockClear();
    await (store as any).handleStreamMessage(
      JSON.stringify({ type: "ready", workspace_id: "ws-1", snapshot_rev: 3, archived_rev: 0 }),
    );
    await waitForCondition(() => (getWorkspaceActiveSnapshot as any).mock.calls.length === 1);
  });
});
