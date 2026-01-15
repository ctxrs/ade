import React from "react";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { VirtuosoMockContext } from "react-virtuoso";
import { describe, it, vi, beforeAll, afterEach } from "vitest";
import WorkbenchPage from "./WorkbenchPage";

const workspaceId = "ws-1";
const taskId = "task-1";
const sessionId = "session-1";

const baseIso = "2024-01-01T00:00:00.000Z";

let sessionSnap = {
  connection: "connected",
  sessions: {
    [sessionId]: {
      sessionId,
      session: {
        id: sessionId,
        task_id: taskId,
        provider_id: "codex",
        status: "active",
        created_at: baseIso,
      },
      turns: [],
      turnToolsByTurnId: {},
      turnToolsLoading: [],
      toolSummariesReady: true,
      hasMoreTurns: false,
      events: [],
      messages: [{ role: "assistant", created_at: baseIso }],
      queue: [],
      loading: false,
      subscribed: true,
      updatedAtMs: 0,
    },
  },
};

let workspaceSnapshotSnap = {
  workspaceId,
  initialized: true,
  connection: "connected",
  tasksById: {
    [taskId]: {
      id: taskId,
      sortAtMs: Date.parse(baseIso),
      task: {
        id: taskId,
        title: "Starter task",
        created_at: baseIso,
        updated_at: baseIso,
        last_activity_at: baseIso,
        archived_at: null,
        assistant_seen_at: null,
        last_assistant_message_at: null,
      },
      sessions: [
        {
          session: {
            id: sessionId,
            task_id: taskId,
            provider_id: "codex",
            status: "active",
            created_at: baseIso,
          },
          last_message_at: null,
          last_event_seq: null,
          activity: { is_working: false, last_turn_status: null },
          unread: false,
        },
      ],
    },
  },
  activeIds: [taskId],
  archivedIds: [],
  totalActive: 1,
  totalArchived: 0,
  fetchState: { active: "idle", archived: "idle" },
  hasMoreActive: false,
  hasMoreArchived: false,
  archivedLoaded: true,
};

vi.mock("../api/client", () => ({
  archiveTask: vi.fn(async () => ({})),
  createSession: vi.fn(async () => ({})),
  createTask: vi.fn(async () => ({})),
  deleteTask: vi.fn(async () => ({})),
  getDaemonBaseUrl: vi.fn(() => ""),
  resolveDaemonWsBaseUrl: vi.fn(() => "ws://localhost:4399"),
  getInstall: vi.fn(async () => ({})),
  getProviderOptions: vi.fn(async () => ({})),
  getSettings: vi.fn(async () => ({ dictation: { enabled: false } })),
  getWorktree: vi.fn(async () => ({})),
  getWorkspace: vi.fn(async () => ({ id: workspaceId, name: "Mock Workspace", root_path: "/tmp/mock" })),
  idToString: (id: any) => (typeof id === "string" ? id : id?.["0"]),
  installAllProviders: vi.fn(async () => ({})),
  installProvider: vi.fn(async () => ({ install_id: "install-1" })),
  listProviders: vi.fn(async () => []),
  markTaskRead: vi.fn(async () => ({})),
  markTaskUnread: vi.fn(async () => ({})),
  postMessage: vi.fn(async () => ({})),
  unarchiveTask: vi.fn(async () => ({})),
  updateTaskTitle: vi.fn(async () => ({})),
  verifyProviderForWorkspace: vi.fn(async () => ({})),
}));

vi.mock("../state/sessionSupervisor", () => ({
  useSessionSupervisor: () => ({
    bindWorkspaceActiveSnapshotStore: vi.fn(),
    setActiveTaskSessionIds: vi.fn(),
    setWarmSessionIds: vi.fn(),
  }),
  useSessionCacheSnapshot: () => sessionSnap,
  useSessionEntry: () => null,
  useOpenSession: () => {},
}));

vi.mock("../state/workspaceActiveSnapshotStore", () => ({
  WorkspaceActiveSnapshotProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useWorkspaceActiveSnapshotEvents: () => {},
  useWorkspaceActiveSnapshotSnapshot: () => workspaceSnapshotSnap,
  useWorkspaceActiveSnapshotStore: () => ({
    applyTaskUpdate: vi.fn(),
    ensureArchivedLoaded: vi.fn(),
    loadMoreActive: vi.fn(),
    loadMoreArchived: vi.fn(),
  }),
}));

vi.mock("../workbench/store", () => ({
  WorkbenchStoreProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  NEW_TASK_DRAFT_KEY: "new_task",
  scrollKey: () => "scroll-key",
  sessionDraftKey: () => "draft-key",
  useWorkbenchStore: () => ({
    focusNewTask: vi.fn(),
    focusTask: vi.fn(),
    setActiveSessionForActiveTask: vi.fn(),
    setScrollState: vi.fn(),
    flushDraft: vi.fn(),
    getActiveTab: vi.fn(() => null),
  }),
  useWorkbenchShellSnapshot: () => ({
    workspaceId,
    windowId: "window-1",
    hydrated: true,
    warnings: [],
    window: { scrollByKey: {} },
  }),
  useActiveWorkbenchTab: () => null,
  useActiveWorkbenchIds: () => ({ taskId: null, sessionId: null }),
  useNewTaskDraft: () => ({ value: { text: "", modeId: "default" }, setValue: vi.fn() }),
  useWorkbenchDraft: () => ({ value: { text: "", modeId: "default" }, updatedAtMs: 0, setValue: vi.fn() }),
}));

vi.mock("../components/WorkbenchComposer", () => ({
  WorkbenchComposer: () => null,
}));

vi.mock("../components/DiffReviewPane", () => ({
  DiffReviewPane: () => null,
}));

vi.mock("./SessionPage", () => ({
  SessionView: () => null,
  buildWorkbenchThreadViewModel: () => ({ groups: [] }),
}));

beforeAll(() => {
  if (typeof (globalThis as any).localStorage?.getItem !== "function") {
    const store = new Map<string, string>();
    (globalThis as any).localStorage = {
      getItem: (key: string) => (store.has(key) ? store.get(key) ?? null : null),
      setItem: (key: string, value: string) => {
        store.set(key, String(value));
      },
      removeItem: (key: string) => {
        store.delete(key);
      },
      clear: () => {
        store.clear();
      },
    };
  }
  if (!("ResizeObserver" in globalThis)) {
    class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    (globalThis as any).ResizeObserver = ResizeObserver;
  }
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("WorkbenchPage task rename selection", () => {
  it("keeps rename selection stable on session updates", async () => {
    const selectSpy = vi.spyOn(HTMLInputElement.prototype, "select");
    const ui = (
      <VirtuosoMockContext.Provider value={{ itemHeight: 40, viewportHeight: 400 }}>
        <MemoryRouter initialEntries={[`/workspaces/${workspaceId}`]}>
          <Routes>
            <Route path="/workspaces/:id" element={<WorkbenchPage />} />
          </Routes>
        </MemoryRouter>
      </VirtuosoMockContext.Provider>
    );

    const { rerender } = render(ui);

    const menuButton = await screen.findByRole("button", { name: "More actions" });
    fireEvent.click(menuButton);
    const renameItem = await screen.findByRole("menuitem", { name: "Rename Task" });
    fireEvent.click(renameItem);

    await waitFor(() => expect(selectSpy).toHaveBeenCalledTimes(1));
    expect(await screen.findByLabelText("Rename task")).toBeTruthy();

    sessionSnap = {
      ...sessionSnap,
      sessions: {
        ...sessionSnap.sessions,
        [sessionId]: {
          ...sessionSnap.sessions[sessionId],
          messages: [
            ...sessionSnap.sessions[sessionId].messages,
            { role: "assistant", created_at: "2024-01-01T00:00:01.000Z" },
          ],
          updatedAtMs: sessionSnap.sessions[sessionId].updatedAtMs + 1,
        },
      },
    };

    rerender(ui);
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    expect(selectSpy).toHaveBeenCalledTimes(1);
  });
});
