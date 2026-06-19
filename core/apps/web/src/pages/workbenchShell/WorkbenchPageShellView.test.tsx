// @vitest-environment jsdom

import React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

import type { AgentWorkTaskDetail, WorkbenchTaskBoardProjection } from "./agentWorkProjection";
import { WorkbenchPageShellView } from "./WorkbenchPageShellView";
import type { PersistedWorkbenchWindowV1, WorkbenchTemplateState } from "../../workbench/types";

vi.mock("../../components/TitleGenerationInstallBanner", () => ({
  TitleGenerationInstallBanner: () => null,
}));

vi.mock("../../components/WorktreeBootstrapSnackbar", () => ({
  WorktreeBootstrapSnackbar: () => null,
}));

vi.mock("../../components/TerminalPanel", () => ({
  TerminalPanel: () => <div data-testid="terminal-panel" />,
}));

vi.mock("./WorkbenchProviderWarningBanner", () => ({
  WorkbenchProviderWarningBanner: () => null,
}));

vi.mock("./WorkbenchPageMenus", () => ({
  WorkbenchPageMenus: () => null,
}));

vi.mock("./WorkbenchEmptyState", () => ({
  WorkbenchEmptyState: () => <div data-testid="empty-state">Empty state</div>,
}));

vi.mock("./WorkbenchActiveTaskView", () => ({
  WorkbenchActiveTaskView: () => <div data-testid="active-task-view">Active task view</div>,
}));

type Props = React.ComponentProps<typeof WorkbenchPageShellView>;

const baseWindow: PersistedWorkbenchWindowV1 = {
  v: 1,
  layout: {
    kind: "leaf",
    id: "leaf-1",
    tabs: [{ id: "tab-1", kind: "new_task" }],
    activeTabId: "tab-1",
  },
  focusedLeafId: "leaf-1",
};

const splitWindow: PersistedWorkbenchWindowV1 = {
  v: 1,
  layout: {
    kind: "split",
    id: "split-1",
    direction: "horizontal",
    ratio: 0.55,
    first: {
      kind: "leaf",
      id: "leaf-1",
      tabs: [{ id: "tab-1", kind: "task", ref: { taskId: "task-1", sessionId: "session-1" } }],
      activeTabId: "tab-1",
    },
    second: {
      kind: "leaf",
      id: "leaf-2",
      tabs: [{ id: "tab-2", kind: "new_task" }],
      activeTabId: "tab-2",
    },
  },
  focusedLeafId: "leaf-1",
};

const taskCard = {
  taskId: "task-1",
  item: {
    id: "task-1",
    task: {
      id: "task-1",
      title: "Fix onboarding bug",
    },
    sessions: [],
    primarySessionId: "session-1",
    sortAtMs: 10,
  },
  agentWorkSummary: {
    taskId: "task-1",
    changeSetCount: 1,
    contributionCount: 2,
    linkedPullRequestCount: 1,
    latestUpdateTimestamp: "2026-01-01T00:00:00Z",
  },
  laneId: "active",
  sortAtMs: 10,
} as unknown as WorkbenchTaskBoardProjection["lanes"][number]["cards"][number];

const taskBoardProjection: WorkbenchTaskBoardProjection = {
  lanes: [
    { id: "active", title: "Active", cards: [taskCard] },
    { id: "needs-review", title: "Needs review", cards: [] },
    { id: "archived", title: "Archived", cards: [] },
    { id: "other", title: "Other", cards: [] },
  ],
  cardsByTaskId: {
    "task-1": taskCard,
  },
};

const activeAgentWorkDetail: AgentWorkTaskDetail = {
  ...taskCard.agentWorkSummary,
  counts: {
    changeSets: 1,
    contributions: 2,
    linkedPullRequests: 1,
  },
  changeSetIds: ["change-set-1"],
  contributionIds: ["contribution-1", "contribution-2"],
  linkedPullRequestKeys: ["github:ctxrs/ctx:42"],
  changeSets: [
    {
      id: "change-set-1",
      workspace_id: "workspace-1",
      title: "Onboarding fix",
    },
  ],
  contributions: [],
  linkedPullRequests: [
    {
      key: "github:ctxrs/ctx:42",
      pullRequest: {
        provider: "github",
        owner: "ctxrs",
        repo: "ctx",
        number: 42,
      },
      links: [],
      changeSetIds: ["change-set-1"],
      contributionIds: [],
      latestUpdateTimestamp: "2026-01-01T00:00:00Z",
    },
  ],
};

const makeProps = (overrides: Partial<Props> = {}): Props => ({
  workspaceId: "workspace-1",
  activeTaskId: null,
  activeSessionId: null,
  sidebarCollapsed: true,
  sidebarResizing: false,
  sidebarWidth: 260,
  mobileShell: false,
  desktopUi: false,
  useHtmlTopbar: true,
  desktopStorageNoticeReason: null,
  onDismissDesktopStorageNotice: vi.fn(),
  composerHarnessAuthModal: null,
  workspaceBootstrapGateState: "ready",
  providerBootstrapError: null,
  onRefreshBootstrap: vi.fn(),
  workbenchWarnings: [],
  workbenchWindow: baseWindow,
  templateState: { id: "classic", version: 1, layout: {} } as WorkbenchTemplateState,
  taskBoardProjection,
  activeAgentWorkDetail: null,
  activeTaskTitle: null,
  onSelectTaskFromTemplate: vi.fn(),
  onFocusWorkbenchLeaf: vi.fn(),
  onSplitWorkbenchLeaf: vi.fn(),
  onResizeWorkbenchSplit: vi.fn(),
  activeTaskController: {
    terminalHeight: 240,
    terminalOpen: false,
    diffResizing: false,
    terminalResizing: false,
    transcriptNotice: null,
    dismissTranscriptNotice: vi.fn(),
  } as unknown as Props["activeTaskController"],
  taskListController: {
    archiveCleanupNotice: false,
    dismissArchiveCleanupNotice: vi.fn(),
  } as unknown as Props["taskListController"],
  topbarProps: {
    workspaceId: "workspace-1",
    workspaceTitle: "Workspace",
    showDebugIds: false,
    debugIdLabel: "",
    onCopyDebugIds: vi.fn(),
  },
  providerWarningProps: {} as Props["providerWarningProps"],
  sidebarProps: {
    collapsed: true,
    taskSearchRef: { current: null },
    taskQuery: "",
    onTaskQueryChange: vi.fn(),
    onNewTask: vi.fn(),
    taskListVirtuosoKey: "tasks",
    taskListItems: [],
    initialTaskListItemCount: undefined,
    computeTaskListItemKey: () => "task",
    renderTaskListItem: () => null,
    taskListContext: {
      archivedCollapsed: true,
      archivedFetchState: "idle",
      hasMoreArchived: false,
      onLoadMoreArchived: vi.fn(),
    },
    onTaskListRangeChanged: vi.fn(),
    onExpandSidebar: vi.fn(),
    onCollapseSidebar: vi.fn(),
    onSidebarResizerMouseDown: vi.fn(),
    onResetSidebarWidth: vi.fn(),
  },
  emptyStateProps: {} as Props["emptyStateProps"],
  activeTaskViewProps: null,
  ...overrides,
});

const renderShell = (props: Props) =>
  render(
    <MemoryRouter>
      <WorkbenchPageShellView {...props} />
    </MemoryRouter>,
  );

describe("WorkbenchPageShellView templates", () => {
  it("renders the classic template as the default workbench body", () => {
    renderShell(makeProps());

    expect(screen.getByTestId("empty-state")).toBeInTheDocument();
  });

  it("renders the kanban template from projected task work", () => {
    const onSelectTaskFromTemplate = vi.fn();
    renderShell(
      makeProps({
        templateState: { id: "kanban", version: 1, layout: {} } as WorkbenchTemplateState,
        onSelectTaskFromTemplate,
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: /Fix onboarding bug/ }));

    expect(screen.getByRole("heading", { name: "Active" })).toBeInTheDocument();
    expect(onSelectTaskFromTemplate).toHaveBeenCalledWith("task-1", "session-1");
  });

  it("renders selected task content beside the kanban board", () => {
    renderShell(
      makeProps({
        activeTaskId: "task-1",
        activeSessionId: "session-1",
        templateState: { id: "kanban", version: 1, layout: {} } as WorkbenchTemplateState,
        activeTaskViewProps: {} as Props["activeTaskViewProps"],
      }),
    );

    expect(screen.getByRole("heading", { name: "Active" })).toBeInTheDocument();
    expect(screen.getByLabelText("Selected task")).toContainElement(screen.getByTestId("active-task-view"));
  });

  it("renders the multipane template and dispatches pane layout actions", () => {
    const onSplitWorkbenchLeaf = vi.fn();
    const onFocusWorkbenchLeaf = vi.fn();
    renderShell(
      makeProps({
        activeTaskId: "task-1",
        activeSessionId: "session-1",
        workbenchWindow: splitWindow,
        templateState: { id: "multipane", version: 1, layout: {} } as WorkbenchTemplateState,
        activeTaskViewProps: {} as Props["activeTaskViewProps"],
        onSplitWorkbenchLeaf,
        onFocusWorkbenchLeaf,
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Split right" }));
    fireEvent.pointerDown(screen.getByTestId("empty-state").closest(".wb-split-pane") as Element);

    expect(screen.getByTestId("active-task-view")).toBeInTheDocument();
    expect(screen.getByTestId("empty-state")).toBeInTheDocument();
    expect(onSplitWorkbenchLeaf).toHaveBeenCalledWith("horizontal");
    expect(onFocusWorkbenchLeaf).toHaveBeenCalledWith("leaf-2");
  });

  it("renders an open terminal inside multipane instead of the global terminal shell", () => {
    const { container } = renderShell(
      makeProps({
        activeTaskId: "task-1",
        activeSessionId: "session-1",
        workbenchWindow: splitWindow,
        templateState: { id: "multipane", version: 1, layout: {} } as WorkbenchTemplateState,
        activeTaskViewProps: {} as Props["activeTaskViewProps"],
        activeTaskController: {
          terminalHeight: 240,
          terminalOpen: true,
          diffResizing: false,
          terminalResizing: false,
          transcriptNotice: null,
          dismissTranscriptNotice: vi.fn(),
        } as unknown as Props["activeTaskController"],
      }),
    );

    expect(screen.getByTestId("terminal-panel")).toBeInTheDocument();
    expect(container.querySelector(".wb-multipane-terminal-pane")).toBeInTheDocument();
    expect(container.querySelector(".wb-terminal-shell")).not.toBeInTheDocument();
  });

  it("renders the review template with Work detail and active task content", () => {
    renderShell(
      makeProps({
        activeTaskId: "task-1",
        activeSessionId: "session-1",
        templateState: { id: "review", version: 1, layout: {} } as WorkbenchTemplateState,
        activeAgentWorkDetail,
        activeTaskTitle: "Fix onboarding bug",
        activeTaskViewProps: {} as Props["activeTaskViewProps"],
      }),
    );

    expect(screen.getByRole("heading", { name: "Fix onboarding bug" })).toBeInTheDocument();
    expect(screen.getByText("Onboarding fix")).toBeInTheDocument();
    expect(screen.getByTestId("active-task-view")).toBeInTheDocument();
  });
});
