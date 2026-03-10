import React from "react";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { VirtuosoMockContext } from "react-virtuoso";
import { describe, it, vi, beforeAll, beforeEach, afterEach } from "vitest";
import type { WorkbenchTab } from "../workbench/types";
import type { SessionSupervisorSnapshot } from "../state/sessionSupervisorCore";
import WorkbenchPage from "./WorkbenchPage";

const workspaceId = "ws-1";
const taskId = "task-1";
const taskId2 = "task-2";
const sessionId = "session-1";
const sessionId2 = "session-2";
const worktreeId = "worktree-1";

const baseIso = "2024-01-01T00:00:00.000Z";

const buildSessionSnap = (): SessionSupervisorSnapshot => ({
  connection: "connected",
  sessions: {
    [sessionId]: {
      sessionId,
      session: {
        id: sessionId,
        task_id: taskId,
        workspace_id: workspaceId,
        provider_id: "codex",
        worktree_id: worktreeId,
        model_id: "gpt-5",
        title: "Starter session",
        agent_role: "assistant",
        status: "active",
        created_at: baseIso,
      },
      turns: [],
      turnToolsByTurnId: {},
      turnToolsLoading: [],
      toolSummaries: [],
      toolSummariesReady: true,
      hasMoreTurns: false,
      events: [],
      messages: [{
        id: "message-1",
        session_id: sessionId,
        task_id: taskId,
        role: "assistant",
        content: "Hello",
        delivery: "immediate",
        created_at: baseIso,
      }],
      artifacts: [],
      artifactsLoading: false,
      subagentInvocations: [],
      subagentInvocationsLoaded: true,
      subagentInvocationsLoading: false,
      stateLoaded: true,
      stateRev: 1,
      stateLoading: false,
      queue: [],
      loadState: "live",
      loading: false,
      subscribed: true,
      updatedAtMs: 0,
    },
  },
});

const buildWorkspaceSnapshotSnap = () => {
  const tasksById: Record<string, unknown> = {
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
  };
  return {
  workspaceId,
  initialized: true,
  connection: "connected",
  tasksById,
  activeIds: [taskId],
  archivedIds: [],
  totalActive: 1,
  totalArchived: 0,
  fetchState: { active: "idle", archived: "idle" },
  hasMoreActive: false,
  hasMoreArchived: false,
  archivedLoaded: true,
  };
};

let sessionSnap = buildSessionSnap();
let workspaceSnapshotSnap = buildWorkspaceSnapshotSnap();
let navToken = 0;
let activeTab: WorkbenchTab | null = null;
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
const sessionSupervisorMock = {
  bindWorkspaceActiveSnapshotStore: vi.fn(),
  setActiveTaskSessionIds: vi.fn(),
  setWarmSessionIds: vi.fn(),
  setSubscribedSessionIdsSink: vi.fn(),
  setWorkspaceSnapshotState: vi.fn(),
  setWorkspaceSessionHeads: vi.fn(),
  handleWorkspaceEvent: vi.fn(),
  setDiff: vi.fn(),
  loadSessionState: vi.fn(),
  loadArtifacts: vi.fn(),
  loadSubagentInvocations: vi.fn(),
};
const workspaceSnapshotStoreMock = {
  applyTaskUpdate: applyTaskUpdateSpy,
  ensureArchivedLoaded: vi.fn(),
  getWorktreeRoot: vi.fn(() => null),
  loadMoreActive: vi.fn(),
  loadMoreArchived: vi.fn(),
  subscribe: vi.fn(() => () => {}),
  subscribeEvents: vi.fn(() => () => {}),
  getSnapshot: vi.fn(() => workspaceSnapshotSnap),
  getSessionHeadSnapshot: vi.fn(() => null),
  getSessionHeadsSnapshot: vi.fn(() => ({})),
  setForegroundTaskId: vi.fn(),
  setSubscribedSessionIds: vi.fn(),
};
const { trackWorkbenchPanelToggledMock } = vi.hoisted(() => ({
  trackWorkbenchPanelToggledMock: vi.fn(),
}));
const getInstallMock = vi.hoisted(() =>
  vi.fn(async (_installId?: string): Promise<{ install_id?: string; last_event?: unknown }> => ({})),
);

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
  getTitleGenerationLocalStatus: vi.fn(async () => ({
    ready: true,
    runtime: { version: "1.0.0", installed: true, path: "/tmp/runtime" },
    model: { model_id: "model", file_name: "model.gguf", installed: true },
    install_id: null,
    install_running: false,
  })),
  checkUpdates: vi.fn(async () => ({
    channel: "stable",
    base_url: "https://example.test",
    current_version: "0.0.0",
    update_available: false,
  })),
  getInstall: getInstallMock,
  getInstallStatuses: vi.fn(async (installIds: string[]) => ({
    installs: await Promise.all(
      installIds.map(async (installId) => {
        const info = await getInstallMock(installId);
        return {
          install_id: installId,
          info: info && typeof info.install_id === "string" ? info : null,
        };
      }),
    ),
  })),
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
  listInstallEvents: vi.fn(async (installId: string) => {
    const info = await getInstallMock(installId);
    return info?.last_event ? [info.last_event] : [];
  }),
  listProviders: vi.fn(async () => []),
  listWorkspaces: vi.fn(async () => []),
  markTaskRead: vi.fn(async () => ({})),
  markTaskUnread: vi.fn(async () => ({})),
  postMessage: vi.fn(async () => ({})),
  unarchiveTask: vi.fn(async () => ({})),
  updateTaskTitle: vi.fn(async () => ({})),
  verifyProviderForWorkspace: vi.fn(async () => ({})),
}));

vi.mock("../utils/analytics", async () => {
  const actual = await vi.importActual<typeof import("../utils/analytics")>("../utils/analytics");
  return {
    ...actual,
    trackWorkbenchPanelToggled: trackWorkbenchPanelToggledMock,
  };
});

vi.mock("../state/sessionSupervisor", () => ({
  useSessionSupervisor: () => sessionSupervisorMock,
  useSessionCacheSnapshot: () => sessionSnap,
  useSessionEntry: (id: string) => sessionSnap.sessions[id] ?? null,
  useOpenSession: () => {},
}));

vi.mock("../state/workspaceActiveSnapshotStore", () => ({
  WorkspaceActiveSnapshotProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useWorkspaceActiveSnapshotEvents: () => {},
  useWorkspaceActiveSnapshotSnapshot: () => workspaceSnapshotSnap,
  useWorkspaceActiveSnapshotStore: () => workspaceSnapshotStoreMock,
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
  const globalWithMocks = globalThis as typeof globalThis & {
    localStorage?: Storage;
    ResizeObserver?: typeof ResizeObserver;
  };
  if (typeof globalWithMocks.localStorage?.getItem !== "function") {
    const store = new Map<string, string>();
    globalWithMocks.localStorage = {
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
      key: (index: number) => Array.from(store.keys())[index] ?? null,
      get length() {
        return store.size;
      },
    };
  }
  if (!("ResizeObserver" in globalThis)) {
    class ResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    globalWithMocks.ResizeObserver = ResizeObserver;
  }
});

beforeEach(() => {
  navToken = 0;
  activeTab = { id: `tab-${taskId}`, kind: "task", ref: { taskId, sessionId } };
  activeTaskId = taskId;
  activeSessionId = sessionId;
  sessionSnap = buildSessionSnap();
  workspaceSnapshotSnap = buildWorkspaceSnapshotSnap();
  trackWorkbenchPanelToggledMock.mockReset();
  sessionSupervisorMock.bindWorkspaceActiveSnapshotStore.mockReset();
  sessionSupervisorMock.setActiveTaskSessionIds.mockReset();
  sessionSupervisorMock.setWarmSessionIds.mockReset();
  sessionSupervisorMock.setSubscribedSessionIdsSink.mockReset();
  sessionSupervisorMock.setWorkspaceSnapshotState.mockReset();
  sessionSupervisorMock.setWorkspaceSessionHeads.mockReset();
  sessionSupervisorMock.handleWorkspaceEvent.mockReset();
  sessionSupervisorMock.setDiff.mockReset();
  sessionSupervisorMock.loadSessionState.mockReset();
  sessionSupervisorMock.loadArtifacts.mockReset();
  sessionSupervisorMock.loadSubagentInvocations.mockReset();
  workspaceSnapshotStoreMock.ensureArchivedLoaded.mockReset();
  workspaceSnapshotStoreMock.getWorktreeRoot.mockReset();
  workspaceSnapshotStoreMock.getWorktreeRoot.mockReturnValue(null);
  workspaceSnapshotStoreMock.loadMoreActive.mockReset();
  workspaceSnapshotStoreMock.loadMoreArchived.mockReset();
  workspaceSnapshotStoreMock.subscribe.mockReset();
  workspaceSnapshotStoreMock.subscribe.mockReturnValue(() => {});
  workspaceSnapshotStoreMock.subscribeEvents.mockReset();
  workspaceSnapshotStoreMock.subscribeEvents.mockReturnValue(() => {});
  workspaceSnapshotStoreMock.getSnapshot.mockReset();
  workspaceSnapshotStoreMock.getSnapshot.mockImplementation(() => workspaceSnapshotSnap);
  workspaceSnapshotStoreMock.getSessionHeadSnapshot.mockReset();
  workspaceSnapshotStoreMock.getSessionHeadSnapshot.mockReturnValue(null);
  workspaceSnapshotStoreMock.getSessionHeadsSnapshot.mockReset();
  workspaceSnapshotStoreMock.getSessionHeadsSnapshot.mockReturnValue({});
  workspaceSnapshotStoreMock.setForegroundTaskId.mockReset();
  workspaceSnapshotStoreMock.setSubscribedSessionIds.mockReset();
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
            {
              id: "message-2",
              session_id: sessionId,
              task_id: taskId,
              role: "assistant",
              content: "Follow-up",
              delivery: "immediate",
              created_at: "2024-01-01T00:00:01.000Z",
            },
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
    const archiveDeferred = createDeferred<Awaited<ReturnType<typeof archiveTask>>>();
    vi.mocked(archiveTask).mockReturnValueOnce(archiveDeferred.promise);

    const ui = (
      <VirtuosoMockContext.Provider value={{ itemHeight: 40, viewportHeight: 400 }}>
        <MemoryRouter initialEntries={[`/workspaces/${workspaceId}`]}>
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

    archiveDeferred.resolve({
      id: taskId,
      workspace_id: workspaceId,
      title: "Starter task",
      status: "completed",
      created_at: baseIso,
      updated_at: baseIso,
      archived_at: baseIso,
    });

    await waitFor(() => expect(applyTaskUpdateSpy).toHaveBeenCalledTimes(1));
    expect(focusNewTaskSpy).not.toHaveBeenCalled();
  });
});

describe("WorkbenchPage title generation install banner", () => {
  it("shows progress while local title model install is running", async () => {
    const { getSettings, getTitleGenerationLocalStatus, getInstall } = await import("../api/client");
    vi.mocked(getSettings).mockResolvedValue({
      title_generation: {
        mode: "local",
        remote: {
          base_url: "https://openrouter.ai/api/v1",
          api_key: "",
          model: "google/gemini-3-flash-preview",
          use_json: true,
        },
        local: {
          model_id: "ggml-org/Qwen3-1.7B-GGUF",
          use_json: true,
        },
      },
    } as never);
    vi.mocked(getTitleGenerationLocalStatus).mockResolvedValue({
      ready: false,
      runtime: { version: "0.0.0-test", installed: true, path: "/tmp/runtime" },
      model: {
        model_id: "ggml-org/Qwen3-1.7B-GGUF",
        file_name: "Qwen3-1.7B-GGUF.gguf",
        installed: false,
      },
      install_id: "install-1",
      install_running: true,
    } as never);
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install-1",
      provider_id: "title_generation_local",
      state: "running",
      started_at: "2026-02-20T00:00:00Z",
      last_event: {
        install_id: "install-1",
        provider_id: "title_generation_local",
        at: "2026-02-20T00:00:01Z",
        stage: "download_model",
        message: "Downloading model file",
        level: "info",
        bytes: 5,
        total_bytes: 10,
      },
    } as never);

    const ui = (
      <VirtuosoMockContext.Provider value={{ itemHeight: 40, viewportHeight: 400 }}>
        <MemoryRouter initialEntries={[`/workspaces/${workspaceId}`]}>
          <Routes>
            <Route path="/workspaces/:id" element={<WorkbenchPage />} />
          </Routes>
        </MemoryRouter>
      </VirtuosoMockContext.Provider>
    );

    render(ui);

    expect(await screen.findByText("Session titling model download in progress.")).toBeInTheDocument();
    expect(await screen.findByText("Downloading… 38%")).toBeInTheDocument();
  });
});

describe("WorkbenchPage session support load issues", () => {
  it("loads active session support data and retries from the banner", async () => {
    sessionSnap = {
      ...sessionSnap,
      sessions: {
        ...sessionSnap.sessions,
        [sessionId]: {
          ...sessionSnap.sessions[sessionId],
          loadErrors: {
            state: "Failed to load session state: daemon offline",
            subagentInvocations: "Failed to load subagent invocations: query failed",
          },
        },
      },
    };

    const ui = (
      <VirtuosoMockContext.Provider value={{ itemHeight: 40, viewportHeight: 400 }}>
        <MemoryRouter initialEntries={[`/workspaces/${workspaceId}`]}>
          <Routes>
            <Route path="/workspaces/:id" element={<WorkbenchPage />} />
          </Routes>
        </MemoryRouter>
      </VirtuosoMockContext.Provider>
    );

    render(ui);

    await waitFor(() => {
      expect(sessionSupervisorMock.loadSessionState).toHaveBeenCalledWith(sessionId);
      expect(sessionSupervisorMock.loadSubagentInvocations).toHaveBeenCalledWith(sessionId);
    });

    expect(await screen.findByText("Some session details failed to load.")).toBeInTheDocument();
    expect(screen.getByText("Failed to load session state: daemon offline")).toBeInTheDocument();
    expect(screen.getByText("Failed to load subagent invocations: query failed")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));

    expect(sessionSupervisorMock.loadSessionState).toHaveBeenCalledWith(sessionId, { force: true });
    expect(sessionSupervisorMock.loadSubagentInvocations).toHaveBeenCalledWith(sessionId, { force: true });
  });

  it("reloads support data when the active session state revision changes", async () => {
    const renderWorkbench = () => (
      <VirtuosoMockContext.Provider value={{ itemHeight: 40, viewportHeight: 400 }}>
        <MemoryRouter initialEntries={[`/workspaces/${workspaceId}`]}>
          <Routes>
            <Route path="/workspaces/:id" element={<WorkbenchPage />} />
          </Routes>
        </MemoryRouter>
      </VirtuosoMockContext.Provider>
    );

    const rendered = render(renderWorkbench());

    await screen.findAllByText("Starter task");
    sessionSupervisorMock.loadSessionState.mockClear();
    sessionSupervisorMock.loadSubagentInvocations.mockClear();

    sessionSnap = {
      ...sessionSnap,
      sessions: {
        ...sessionSnap.sessions,
        [sessionId]: {
          ...sessionSnap.sessions[sessionId],
          stateRev: 2,
          updatedAtMs: 1,
        },
      },
    };

    rendered.rerender(renderWorkbench());

    await waitFor(() => {
      expect(sessionSupervisorMock.loadSessionState).toHaveBeenCalledWith(sessionId);
      expect(sessionSupervisorMock.loadSubagentInvocations).toHaveBeenCalledWith(sessionId);
    });
  });
});

describe("WorkbenchPage panel analytics", () => {
  it("tracks terminal panel toggles from header button", async () => {
    const ui = (
      <VirtuosoMockContext.Provider value={{ itemHeight: 40, viewportHeight: 400 }}>
        <MemoryRouter initialEntries={[`/workspaces/${workspaceId}`]}>
          <Routes>
            <Route path="/workspaces/:id" element={<WorkbenchPage />} />
          </Routes>
        </MemoryRouter>
      </VirtuosoMockContext.Provider>
    );

    render(ui);

    fireEvent.click(await screen.findByRole("button", { name: "Toggle terminal panel" }));

    expect(trackWorkbenchPanelToggledMock).toHaveBeenCalledWith({
      panelKey: "terminal",
      open: true,
      source: "header_button",
    });
  });
});
