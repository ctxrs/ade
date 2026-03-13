import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session } from "../../api/client";
import type { SessionCacheEntry, SessionSupervisorSnapshot } from "../../state/sessionSupervisor";
import { useWorkbenchSessionActions } from "./useWorkbenchSessionActions";

const clipboardSpy = vi.hoisted(() => vi.fn());

vi.mock("../SessionPage", () => ({
  buildWorkbenchThreadViewModel: () => ({ groups: [] }),
}));

vi.mock("../../utils/clipboard", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../utils/clipboard")>();
  return {
    ...actual,
    tryCopyTextToClipboard: (...args: Parameters<typeof actual.tryCopyTextToClipboard>) => clipboardSpy(...args),
  };
});

const baseSession: Session = {
  id: "session-1",
  task_id: "task-1",
  workspace_id: "workspace-1",
  worktree_id: "worktree-1",
  provider_id: "codex",
  model_id: "gpt-5",
  title: "Conversation",
  agent_role: "primary",
  status: "idle",
  created_at: "2026-03-13T00:00:00.000Z",
  updated_at: "2026-03-13T00:00:00.000Z",
};

const buildEntry = (overrides?: Partial<SessionCacheEntry>): SessionCacheEntry => ({
  sessionId: "session-1",
  loadState: "live",
  session: baseSession,
  turns: [],
  turnToolsByTurnId: {},
  turnToolsLoading: [],
  toolSummaries: [],
  toolSummariesReady: true,
  hasMoreTurns: false,
  events: [],
  messages: [],
  artifacts: [],
  artifactsLoading: false,
  subagentInvocations: [],
  subagentInvocationsLoading: false,
  stateLoaded: true,
  stateLoading: false,
  queue: [],
  loading: false,
  subscribed: true,
  fetching: {
    head: false,
    history: false,
  },
  updatedAtMs: 0,
  ...overrides,
});

const buildSnapshot = (entry: SessionCacheEntry): SessionSupervisorSnapshot => ({
  connection: "connected",
  sessions: {
    [entry.sessionId]: entry,
  },
});

describe("useWorkbenchSessionActions", () => {
  beforeEach(() => {
    clipboardSpy.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("prefers the history request error over a clipboard-blocked notice", async () => {
    const entry = buildEntry({ hasMoreTurns: true, oldestTurnSeq: 10 });
    const snapshot = buildSnapshot(entry);
    clipboardSpy.mockResolvedValue({ ok: false, reason: "blocked" });
    const loadMoreTurns = vi.fn(async () => {
      throw new Error("History request failed.");
    });

    const { result } = renderHook(() =>
      useWorkbenchSessionActions({
        activeEntry: entry,
        activeSessionId: entry.sessionId,
        activeTaskId: "task-1",
        activeWorktreeId: "worktree-1",
        singleSessionTitle: "Conversation",
        worktreePath: "/tmp/worktree",
        canCopyWorktree: true,
        canOpenTerminal: true,
        terminalPanelRef: { current: null },
        setTerminalOpen: vi.fn(),
        getSupervisorSnapshot: () => snapshot,
        loadMoreTurns,
      }),
    );

    await act(async () => {
      await result.current.copyTranscript();
    });

    expect(loadMoreTurns).toHaveBeenCalledWith("session-1");
    expect(result.current.transcriptNotice).toBe("History request failed.");
  });

  it("shows a blocked clipboard notice when the clipboard error is explicitly blocked", async () => {
    const entry = buildEntry();
    clipboardSpy.mockResolvedValue({ ok: false, reason: "blocked" });

    const { result } = renderHook(() =>
      useWorkbenchSessionActions({
        activeEntry: entry,
        activeSessionId: entry.sessionId,
        activeTaskId: "task-1",
        activeWorktreeId: "worktree-1",
        singleSessionTitle: "Conversation",
        worktreePath: "/tmp/worktree",
        canCopyWorktree: true,
        canOpenTerminal: true,
        terminalPanelRef: { current: null },
        setTerminalOpen: vi.fn(),
        getSupervisorSnapshot: () => buildSnapshot(entry),
        loadMoreTurns: vi.fn(),
      }),
    );

    await act(async () => {
      await result.current.copyTranscript();
    });

    expect(result.current.transcriptNotice).toBe("Clipboard access is blocked. Use HTTPS or copy manually.");
  });

  it("keeps the partial-history notice after a successful copy", async () => {
    const entry = buildEntry({ hasMoreTurns: true, oldestTurnSeq: 10 });
    const snapshot = buildSnapshot(entry);
    clipboardSpy.mockResolvedValue({ ok: true });
    const loadMoreTurns = vi.fn(async () => {
      throw new Error("History request failed.");
    });

    const { result } = renderHook(() =>
      useWorkbenchSessionActions({
        activeEntry: entry,
        activeSessionId: entry.sessionId,
        activeTaskId: "task-1",
        activeWorktreeId: "worktree-1",
        singleSessionTitle: "Conversation",
        worktreePath: "/tmp/worktree",
        canCopyWorktree: true,
        canOpenTerminal: true,
        terminalPanelRef: { current: null },
        setTerminalOpen: vi.fn(),
        getSupervisorSnapshot: () => snapshot,
        loadMoreTurns,
      }),
    );

    await act(async () => {
      await result.current.copyTranscript();
    });

    expect(result.current.transcriptNotice).toBe("Couldn't load full history. Copied what's already loaded.");
  });
});
