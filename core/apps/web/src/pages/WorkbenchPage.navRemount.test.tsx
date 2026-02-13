import React from "react";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { VirtuosoMockContext } from "react-virtuoso";
import { describe, it, vi, beforeAll, beforeEach, afterEach } from "vitest";
import WorkbenchPage from "./WorkbenchPage";

const workspaceId = "ws-1";
const taskId = "task-1";
const taskId2 = "task-2";
const sessionId = "session-1";
const sessionId2 = "session-2";

const baseIso = "2024-01-01T00:00:00.000Z";

const buildSessionSnap = () => ({
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
});

const buildWorkspaceSnapshotSnap = (): any => ({
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
});

let sessionSnap = buildSessionSnap();
let workspaceSnapshotSnap = buildWorkspaceSnapshotSnap();
let navToken = 0;
let activeTab: any = null;
let activeTaskId = taskId;
let activeSessionId: string | null = sessionId;
const focusNewTaskSpy = vi.fn();
const focusTaskSpy = vi.fn(
  (nextTaskId: string, nextSessionId?: string | null, opts?: { source?: string }) => {
    if (opts?.source !== "system") {
      navToken += 1;
    }
    activeTab = {
      id: `tab-${nextTaskId}`,
      kind: "task",
      ref: { taskId: nextTaskId, sessionId: nextSessionId ?? null },
    };
    activeTaskId = nextTaskId;
    activeSessionId = nextSessionId ?? null;
  },
);
const applyTaskUpdateSpy = vi.fn();

const createDeferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
};

vi.mock("../api/client", () => ({
  archiveTask: vi.fn(async () => ({})),
  createSession: vi.fn(async () => ({})),
  createTask: vi.fn(async () => ({})),
  deleteTask: vi.fn(async () => ({})),
  getHealth: vi.fn(async () => ({
    version: "0.0.0",
    daemon_version: "0.0.0",
    pid: 1,
    data_root: "/tmp/ctx",
    daemon_url: "",
    auth_required: false,
    compatibility: {
      desktop_exact_version: "0.0.0",
      mobile_api_min: 1,
      mobile_api_max: 1,
    },
  })),
  checkUpdates: vi.fn(async () => ({
    channel: "stable",
    base_url: "https://example.test",
    current_version: "0.0.0",
    update_available: false,
  })),
  getInstall: vi.fn(async () => ({})),
  getProviderOptions: vi.fn(async () => ({})),
  getSessionGitStatusSummary: vi.fn(async () => null),
  getSettings: vi.fn(async () => ({ dictation: { enabled: false } })),
  getWorktree: vi.fn(async () => ({})),
  getWorkspace: vi.fn(async () => ({ id: workspaceId, name: "Mock Workspace", root_path: "/tmp/mock" })),
  idToString: (id: string | null | undefined) => {
    if (id === null || id === undefined) return "";
    if (typeof id !== "string") {
      throw new Error("Expected id to be a string");
    }
    return id;
  },
  installAllProviders: vi.fn(async () => ({})),
  installProvider: vi.fn(async () => ({ install_id: "install-1" })),
  listProviders: vi.fn(async () => []),
  listWorkspaces: vi.fn(async () => []),
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
    applyTaskUpdate: applyTaskUpdateSpy,
    ensureArchivedLoaded: vi.fn(),
    getWorktreeRoot: vi.fn(() => null),
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
    focusNewTask: focusNewTaskSpy,
    focusTask: focusTaskSpy,
    setActiveSessionForActiveTask: vi.fn(),
    setScrollState: vi.fn(),
    flushDraft: vi.fn(),
    getActiveTab: () => activeTab,
    getNavToken: () => navToken,
  }),
  useWorkbenchShellSnapshot: () => ({
    workspaceId,
    windowId: "window-1",
    hydrated: true,
    warnings: [],
    window: { scrollByKey: {} },
  }),
  useActiveWorkbenchTab: () => null,
  useActiveWorkbenchIds: () => ({ taskId: activeTaskId, sessionId: activeSessionId }),
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

beforeEach(() => {
  navToken = 0;
  activeTab = { id: `tab-${taskId}`, kind: "task", ref: { taskId, sessionId } };
  activeTaskId = taskId;
  activeSessionId = sessionId;
  sessionSnap = buildSessionSnap();
  workspaceSnapshotSnap = buildWorkspaceSnapshotSnap();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("WorkbenchPage task rename selection", () => {
  it("keeps rename selection stable on session updates", async () => {
    const selectSpy = vi.spyOn(HTMLInputElement.prototype, "select");
    const ui = (
      <VirtuosoMockContext.Provider value={{ itemHeight: 40, viewportHeight: 400 }}>
        <MemoryRouter
          initialEntries={[`/workspaces/${workspaceId}`]}
          future={{ v7_startTransition: true, v7_relativeSplatPath: true }}
        >
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

describe("WorkbenchPage archive navigation", () => {
  it("does not refocus new task after navigation during archive", async () => {
    const getTaskRow = (title: string) => {
      const row = screen
        .getAllByText(title)
        .map((node) => node.closest(".wb-task-row"))
        .find((node): node is HTMLElement => Boolean(node));
      if (!row) throw new Error(`Missing task row for ${title}`);
      return row;
    };

    workspaceSnapshotSnap = {
      ...workspaceSnapshotSnap,
      tasksById: {
        ...workspaceSnapshotSnap.tasksById,
        [taskId2]: {
          id: taskId2,
          sortAtMs: Date.parse("2024-01-01T00:00:02.000Z"),
          task: {
            id: taskId2,
            title: "Second task",
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
                id: sessionId2,
                task_id: taskId2,
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
      activeIds: [taskId, taskId2],
      totalActive: 2,
    };

    const { archiveTask } = await import("../api/client");
    const archiveDeferred = createDeferred<any>();
    vi.mocked(archiveTask).mockReturnValueOnce(archiveDeferred.promise);

    const ui = (
      <VirtuosoMockContext.Provider value={{ itemHeight: 40, viewportHeight: 400 }}>
        <MemoryRouter
          initialEntries={[`/workspaces/${workspaceId}`]}
          future={{ v7_startTransition: true, v7_relativeSplatPath: true }}
        >
          <Routes>
            <Route path="/workspaces/:id" element={<WorkbenchPage />} />
          </Routes>
        </MemoryRouter>
      </VirtuosoMockContext.Provider>
    );

    render(ui);

    const starterRow = getTaskRow("Starter task");
    const archiveButton = within(starterRow).getByRole("button", { name: "Archive" });
    fireEvent.click(archiveButton);

    const dialog = await screen.findByRole("dialog", { name: "Archive confirmation" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(archiveTask).toHaveBeenCalledTimes(1));

    fireEvent.click(within(getTaskRow("Second task")).getByText("Second task"));
    fireEvent.click(within(getTaskRow("Starter task")).getByText("Starter task"));

    archiveDeferred.resolve({ id: taskId, archived_at: baseIso });

    await waitFor(() => expect(applyTaskUpdateSpy).toHaveBeenCalledTimes(1));
    expect(focusNewTaskSpy).not.toHaveBeenCalled();
  });
});
