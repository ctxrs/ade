import { afterEach, describe, expect, it, vi } from "vitest";
import type { WorktreeVcsSnapshot } from "@ctx/types";
import { WorkspaceVcsStore } from "./workspaceVcsStore";

vi.mock("../api/client", () => ({
  getDaemonClientConfig: vi.fn(() => ({
    baseUrl: "http://localhost:4399",
    wsBaseUrl: "ws://localhost:4399",
    authToken: null,
    runId: null,
  })),
  idToString: (id: string | null | undefined): string => (typeof id === "string" ? id : ""),
  recordClientCounterMetric: vi.fn(),
  recordClientHistogramMetric: vi.fn(),
  subscribeDaemonConfig: vi.fn(() => () => {}),
}));

class MockWebSocket {
  static OPEN = 1;
  static CONNECTING = 0;
  static CLOSED = 3;
  readyState = MockWebSocket.CONNECTING;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;

  constructor(readonly url: string) {
    mockSockets.push(this);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.();
  }

  open() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.();
  }

  emit(data: unknown) {
    this.onmessage?.({ data: JSON.stringify(data) } as MessageEvent);
  }
}

const mockSockets: MockWebSocket[] = [];
const originalWebSocket = globalThis.WebSocket;

const makeSnapshot = (worktreeId: string, rev: number): WorktreeVcsSnapshot => ({
  worktree_id: worktreeId,
  rev,
  emitted_at_ms: Date.now(),
  base_commit_sha: "base",
  head_commit_sha: "head",
  base_resolution: { kind: "merge_base" },
  compute_state: "ready",
  summary: {
    file_count: rev,
    line_additions: rev,
    line_deletions: 0,
    line_count: rev,
  },
  git_status: {
    branch: "main",
    upstream: "origin/main",
    ahead: 0,
    behind: 0,
    detached: false,
    staged: 0,
    unstaged: rev,
    untracked: 0,
    entries: [],
  },
  touched_files: {
    total_count: rev,
    truncated: false,
    items: [],
  },
  touched_files_state: "ready",
  freshness: "fresh",
  available: true,
  unavailable_reason: null,
  schema_version: 2,
});

describe("WorkspaceVcsStore", () => {
  afterEach(() => {
    globalThis.WebSocket = originalWebSocket;
    mockSockets.length = 0;
    vi.useRealTimers();
  });

  it("subscribes to the dedicated VCS stream and ignores stale snapshot revisions", async () => {
    globalThis.WebSocket = MockWebSocket as unknown as typeof WebSocket;
    const store = new WorkspaceVcsStore("workspace-1");
    store.init();
    await Promise.resolve();
    await Promise.resolve();
    const socket = mockSockets[0];
    expect(socket?.url).toBe("ws://localhost:4399/api/workspaces/workspace-1/vcs/stream");
    socket.open();

    store.setDemand({
      summaryWorktreeIds: ["worktree-2", "worktree-1", "worktree-1"],
      detailWorktreeIds: ["worktree-1"],
    });

    expect(JSON.parse(socket.sent.at(-1) ?? "{}")).toEqual({
      type: "replace_subscription",
      summary_worktree_ids: ["worktree-1", "worktree-2"],
      detail_worktree_ids: ["worktree-1"],
    });

    socket.emit({
      type: "subscribed",
      workspace_id: "workspace-1",
      demand_generation: 1,
      summary_worktree_ids: ["worktree-1", "worktree-2"],
      detail_worktree_ids: ["worktree-1"],
    });
    await Promise.resolve();
    socket.emit({
      type: "summary_snapshot",
      workspace_id: "workspace-1",
      worktree_id: "worktree-1",
      demand_generation: 1,
      snapshot: makeSnapshot("worktree-1", 2),
    });
    await Promise.resolve();
    expect(store.getWorktreeVcsSnapshot("worktree-1")).toBeNull();
    socket.emit({
      type: "summary_snapshot",
      workspace_id: "workspace-1",
      worktree_id: "worktree-1",
      demand_generation: 1,
      snapshot: makeSnapshot("worktree-1", 1),
    });
    await Promise.resolve();
    socket.emit({
      type: "details_snapshot",
      workspace_id: "workspace-1",
      worktree_id: "worktree-1",
      demand_generation: 1,
      snapshot: makeSnapshot("worktree-1", 3),
    });
    await Promise.resolve();

    expect(store.getWorktreeVcsSnapshot("worktree-1")?.rev).toBe(3);
    store.destroy();
  });

  it("ignores snapshots from stale demand generations and no-longer-demanded tiers", async () => {
    globalThis.WebSocket = MockWebSocket as unknown as typeof WebSocket;
    const store = new WorkspaceVcsStore("workspace-1");
    store.init();
    await Promise.resolve();
    await Promise.resolve();
    const socket = mockSockets[0];
    socket.open();

    store.setDemand({ summaryWorktreeIds: ["worktree-1"] });
    socket.emit({
      type: "subscribed",
      workspace_id: "workspace-1",
      demand_generation: 1,
      summary_worktree_ids: ["worktree-1"],
      detail_worktree_ids: [],
    });
    await Promise.resolve();
    socket.emit({
      type: "summary_snapshot",
      workspace_id: "workspace-1",
      worktree_id: "worktree-1",
      demand_generation: 1,
      snapshot: makeSnapshot("worktree-1", 2),
    });
    await Promise.resolve();
    expect(store.getWorktreeVcsSnapshot("worktree-1")?.rev).toBe(2);

    store.setDemand({ summaryWorktreeIds: ["worktree-2"] });
    socket.emit({
      type: "summary_snapshot",
      workspace_id: "workspace-1",
      worktree_id: "worktree-2",
      demand_generation: 1,
      snapshot: makeSnapshot("worktree-2", 99),
    });
    await Promise.resolve();
    expect(store.getWorktreeVcsSnapshot("worktree-2")).toBeNull();

    socket.emit({
      type: "subscribed",
      workspace_id: "workspace-1",
      demand_generation: 2,
      summary_worktree_ids: ["worktree-2"],
      detail_worktree_ids: [],
    });
    await Promise.resolve();
    socket.emit({
      type: "summary_snapshot",
      workspace_id: "workspace-1",
      worktree_id: "worktree-1",
      demand_generation: 1,
      snapshot: makeSnapshot("worktree-1", 99),
    });
    await Promise.resolve();
    expect(store.getWorktreeVcsSnapshot("worktree-1")?.rev).toBe(2);

    store.setDemand({ summaryWorktreeIds: ["worktree-2"], detailWorktreeIds: ["worktree-2"] });
    socket.emit({
      type: "subscribed",
      workspace_id: "workspace-1",
      demand_generation: 3,
      summary_worktree_ids: ["worktree-2"],
      detail_worktree_ids: ["worktree-2"],
    });
    await Promise.resolve();
    socket.emit({
      type: "summary_snapshot",
      workspace_id: "workspace-1",
      worktree_id: "worktree-2",
      demand_generation: 3,
      snapshot: makeSnapshot("worktree-2", 5),
    });
    await Promise.resolve();
    expect(store.getWorktreeVcsSnapshot("worktree-2")).toBeNull();

    socket.emit({
      type: "details_snapshot",
      workspace_id: "workspace-1",
      worktree_id: "worktree-2",
      demand_generation: 3,
      snapshot: makeSnapshot("worktree-2", 6),
    });
    await Promise.resolve();
    expect(store.getWorktreeVcsSnapshot("worktree-2")?.rev).toBe(6);
    store.destroy();
  });
});
