import React, { useMemo } from "react";
import { SplitSquareHorizontal, SplitSquareVertical, X } from "lucide-react";

import { TerminalPanel } from "../../components/TerminalPanel";
import { TitleGenerationInstallBanner } from "../../components/TitleGenerationInstallBanner";
import { WorktreeBootstrapSnackbar } from "../../components/WorktreeBootstrapSnackbar";
import type { LayoutNode, PersistedWorkbenchWindowV1, SplitDirection, WorkbenchTemplateState } from "../../workbench/types";
import { getActiveTabFromLeaf } from "../../utils/workbenchStoreLayout";
import {
  formatAgentWorkSummaryChips,
  type AgentWorkTaskDetail,
  type WorkbenchTaskBoardProjection,
} from "./agentWorkProjection";
import { WorkbenchActiveTaskView } from "./WorkbenchActiveTaskView";
import { WorkbenchEmptyState } from "./WorkbenchEmptyState";
import { WorkbenchSidebar, WorkbenchTopbar } from "./WorkbenchShellChrome";
import { WorkbenchKanbanTemplate } from "./WorkbenchKanbanTemplate";
import { WorkbenchMultipaneTemplate } from "./WorkbenchMultipaneTemplate";
import { WorkbenchPageMenus } from "./WorkbenchPageMenus";
import { WorkbenchReviewTemplate } from "./WorkbenchReviewTemplate";
import type {
  WorkbenchKanbanLane,
  WorkbenchSplitNode,
  WorkbenchSplitPaneNode,
  WorkbenchSplitResize,
} from "./WorkbenchTemplateTypes";
import { WorkbenchProviderWarningBanner } from "./WorkbenchProviderWarningBanner";
import {
  getWorkbenchRootStyleVars,
  type WorkbenchRootStyleVars,
} from "./workbenchLayoutVars";

type RootStyle = React.CSSProperties & WorkbenchRootStyleVars;

const boardCardTone = (laneId: string): WorkbenchKanbanLane["cards"][number]["tone"] => {
  switch (laneId) {
    case "active":
      return "active";
    case "needs-review":
      return "warning";
    case "archived":
      return "success";
    default:
      return "default";
  }
};

const formatPullRequest = (detail: AgentWorkTaskDetail["linkedPullRequests"][number]): string => {
  const pr = detail.pullRequest;
  if (pr.owner && pr.repo && pr.number) return `${pr.owner}/${pr.repo}#${pr.number}`;
  if (pr.number) return `#${pr.number}`;
  return detail.key;
};

const taskTitleFromCard = (card: WorkbenchTaskBoardProjection["cardsByTaskId"][string]): string =>
  card.item.task.title?.trim() || "New Task";

const MULTIPANE_TERMINAL_SPLIT_ID = "ctx-multipane-terminal-split";

const buildKanbanLanes = (projection: WorkbenchTaskBoardProjection): WorkbenchKanbanLane[] =>
  projection.lanes.map((lane) => ({
    id: lane.id,
    title: lane.title,
    countLabel: String(lane.cards.length),
    emptyLabel: "No tasks",
    cards: lane.cards.map((card) => {
      const chips = formatAgentWorkSummaryChips(card.agentWorkSummary);
      const sessionCount = card.item.sessions.length;
      const meta = [
        ...chips,
        sessionCount ? `${sessionCount} session${sessionCount === 1 ? "" : "s"}` : null,
      ].filter(Boolean) as string[];
      return {
        id: card.taskId,
        title: taskTitleFromCard(card),
        subtitle: chips.length ? chips.join(" · ") : null,
        meta,
        tone: boardCardTone(lane.id),
      };
    }),
  }));

const buildTaskTitleLookup = (projection: WorkbenchTaskBoardProjection): Record<string, string> => {
  const titles: Record<string, string> = {};
  for (const [taskId, card] of Object.entries(projection.cardsByTaskId)) {
    titles[taskId] = taskTitleFromCard(card);
  }
  return titles;
};

function workbenchLeafTitle(
  leaf: Extract<LayoutNode, { kind: "leaf" }>,
  taskTitlesById: Record<string, string>,
): { title: string; subtitle: string | null; taskId: string | null } {
  const tab = getActiveTabFromLeaf(leaf);
  if (!tab) return { title: "Empty pane", subtitle: null, taskId: null };
  if (tab.kind === "new_task") return { title: "New task", subtitle: "Composer", taskId: null };
  const title = taskTitlesById[tab.ref.taskId] ?? tab.titleOverride ?? "Task";
  return {
    title,
    subtitle: tab.ref.sessionId ? `Session ${tab.ref.sessionId}` : "Task",
    taskId: tab.ref.taskId,
  };
}

function buildSplitTree(
  node: LayoutNode,
  opts: {
    focusedLeafId: string;
    activeTaskId: string | null;
    taskTitlesById: Record<string, string>;
    renderActiveTaskView: () => React.ReactNode;
    emptyState: React.ReactNode;
    workDetailSlot: React.ReactNode;
    taskBoardProjection: WorkbenchTaskBoardProjection;
  },
): WorkbenchSplitNode {
  if (node.kind === "split") {
    return {
      id: node.id,
      kind: "split",
      direction: node.direction,
      splitPercent: node.ratio * 100,
      first: buildSplitTree(node.first, opts),
      second: buildSplitTree(node.second, opts),
    };
  }

  const leafInfo = workbenchLeafTitle(node, opts.taskTitlesById);
  const active = node.id === opts.focusedLeafId;
  const card = leafInfo.taskId ? opts.taskBoardProjection.cardsByTaskId[leafInfo.taskId] : null;
  const chips = card ? formatAgentWorkSummaryChips(card.agentWorkSummary) : [];
  const tab = getActiveTabFromLeaf(node);
  let content: React.ReactNode = opts.emptyState;
  if (tab?.kind === "task") {
    content = tab.ref.taskId === opts.activeTaskId ? (
      opts.renderActiveTaskView() ?? opts.emptyState
    ) : (
      <div className="wb-multipane-unloaded-task">
        <div className="wb-multipane-unloaded-title">{leafInfo.title}</div>
        <div className="wb-multipane-unloaded-subtitle">Focus this pane to load its active session.</div>
      </div>
    );
  } else if (opts.activeTaskId && opts.workDetailSlot) {
    content = opts.workDetailSlot;
  }

  return {
    id: node.id,
    kind: "pane",
    content,
    preview: {
      title: leafInfo.title,
      subtitle: leafInfo.subtitle,
      meta: chips,
      active,
      muted: !active,
      badge: card?.laneId ? card.laneId.replace("-", " ") : null,
    },
  };
}

type WorkbenchPageShellViewProps = {
  workspaceId: string;
  activeTaskId: string | null;
  activeSessionId: string | null;
  sidebarCollapsed: boolean;
  sidebarResizing: boolean;
  sidebarWidth: number;
  mobileShell: boolean;
  desktopUi: boolean;
  useHtmlTopbar: boolean;
  desktopStorageNoticeReason: string | null;
  onDismissDesktopStorageNotice: () => void;
  composerHarnessAuthModal: React.ReactNode;
  workspaceBootstrapGateState: "loading" | "error" | "ready";
  providerBootstrapError: string | null;
  onRefreshBootstrap: () => void;
  workbenchWarnings: string[];
  workbenchWindow: PersistedWorkbenchWindowV1;
  templateState: WorkbenchTemplateState;
  taskBoardProjection: WorkbenchTaskBoardProjection;
  activeAgentWorkDetail: AgentWorkTaskDetail | null;
  activeTaskTitle: string | null;
  onSelectTaskFromTemplate: (taskId: string, sessionId?: string | null) => void;
  onFocusWorkbenchLeaf: (leafId: string) => boolean;
  onSplitWorkbenchLeaf: (direction: SplitDirection) => boolean;
  onResizeWorkbenchSplit: (splitId: string, ratio: number) => boolean;
  activeTaskController: React.ComponentProps<typeof WorkbenchPageMenus>["activeTaskController"];
  taskListController: React.ComponentProps<typeof WorkbenchPageMenus>["taskListController"];
  topbarProps: React.ComponentProps<typeof WorkbenchTopbar>;
  providerWarningProps: React.ComponentProps<typeof WorkbenchProviderWarningBanner>;
  sidebarProps: React.ComponentProps<typeof WorkbenchSidebar>;
  emptyStateProps: React.ComponentProps<typeof WorkbenchEmptyState>;
  activeTaskViewProps: React.ComponentProps<typeof WorkbenchActiveTaskView> | null;
};

export function WorkbenchPageShellView({
  workspaceId,
  activeTaskId,
  activeSessionId,
  sidebarCollapsed,
  sidebarResizing,
  sidebarWidth,
  mobileShell,
  desktopUi,
  useHtmlTopbar,
  desktopStorageNoticeReason,
  onDismissDesktopStorageNotice,
  composerHarnessAuthModal,
  workspaceBootstrapGateState,
  providerBootstrapError,
  onRefreshBootstrap,
  workbenchWarnings,
  workbenchWindow,
  templateState,
  taskBoardProjection,
  activeAgentWorkDetail,
  activeTaskTitle,
  onSelectTaskFromTemplate,
  onFocusWorkbenchLeaf,
  onSplitWorkbenchLeaf,
  onResizeWorkbenchSplit,
  activeTaskController,
  taskListController,
  topbarProps,
  providerWarningProps,
  sidebarProps,
  emptyStateProps,
  activeTaskViewProps,
}: WorkbenchPageShellViewProps) {
  const effectiveTemplateId = mobileShell ? "classic" : templateState.id;
  const terminalInGlobalShell = !mobileShell && effectiveTemplateId !== "multipane";
  const rootStyle = useMemo<RootStyle>(() => {
    return getWorkbenchRootStyleVars({
      mobileShell,
      sidebarWidth,
      terminalHeight: activeTaskController.terminalHeight,
      terminalOpen: terminalInGlobalShell && activeTaskController.terminalOpen,
      useHtmlTopbar,
      viewportWidth: window.innerWidth,
    });
  }, [
    activeTaskController.terminalHeight,
    activeTaskController.terminalOpen,
    mobileShell,
    sidebarWidth,
    terminalInGlobalShell,
    useHtmlTopbar,
  ]);

  const archiveCleanupSnackbar = taskListController.archiveCleanupNotice ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">Archived task, but some cleanup failed.</div>
        <div className="wb-snackbar-subtitle">
          Some worktree files were likely root-owned and could not be removed. Fix permissions and delete them manually if
          needed.
        </div>
      </div>
      <button
        type="button"
        className="wb-snackbar-close"
        onClick={taskListController.dismissArchiveCleanupNotice}
        aria-label="Dismiss"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

  const transcriptNoticeSnackbar = activeTaskController.transcriptNotice ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">{activeTaskController.transcriptNotice}</div>
      </div>
      <button
        type="button"
        className="wb-snackbar-close"
        onClick={activeTaskController.dismissTranscriptNotice}
        aria-label="Dismiss"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

  const desktopStorageNoticeSubtitle =
    desktopStorageNoticeReason === "schema_mismatch"
      ? "Desktop detected an outdated local UI state format and reset local UI state."
      : "Desktop detected invalid local UI state data and reset local UI state.";
  const desktopStorageNoticeSnackbar = desktopStorageNoticeReason ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">Local UI state was reset.</div>
        <div className="wb-snackbar-subtitle">{desktopStorageNoticeSubtitle}</div>
      </div>
      <button
        type="button"
        className="wb-snackbar-close"
        onClick={onDismissDesktopStorageNotice}
        aria-label="Dismiss"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

  const topbar = useHtmlTopbar ? (
    <div className="wb-topbar-host" data-tauri-drag-region={desktopUi ? true : undefined}>
      <WorkbenchTopbar {...topbarProps} />
    </div>
  ) : null;

  const kanbanLanes = useMemo(() => buildKanbanLanes(taskBoardProjection), [taskBoardProjection]);
  const taskTitlesById = useMemo(() => buildTaskTitleLookup(taskBoardProjection), [taskBoardProjection]);
  const emptyStateSlot = <WorkbenchEmptyState {...emptyStateProps} />;
  const renderActiveTaskSlot = () =>
    activeTaskId && activeTaskViewProps ? <WorkbenchActiveTaskView {...activeTaskViewProps} /> : null;
  const activeTaskSlot = renderActiveTaskSlot();
  const classicMainContent = (
    <>
      {!activeTaskId ? emptyStateSlot : null}
      {activeTaskSlot}
    </>
  );
  const activeWorkChips = formatAgentWorkSummaryChips(activeAgentWorkDetail);
  const reviewMetrics = activeAgentWorkDetail
    ? [
        { label: "Changes", value: activeAgentWorkDetail.changeSetCount },
        { label: "Links", value: activeAgentWorkDetail.contributionCount },
        { label: "PRs", value: activeAgentWorkDetail.linkedPullRequestCount },
      ]
    : [];
  const reviewDetails = activeAgentWorkDetail?.linkedPullRequests.slice(0, 4).map((pullRequest) => ({
    id: pullRequest.key,
    label: "Pull request",
    value: formatPullRequest(pullRequest),
  })) ?? [];
  const workDetailSlot = activeAgentWorkDetail ? (
    <div className="wb-work-detail-list" aria-label="Work evidence">
      {activeAgentWorkDetail.changeSets.slice(0, 8).map((changeSet) => (
        <div key={changeSet.id} className="wb-work-detail-row">
          <div className="wb-work-detail-title">{changeSet.title ?? changeSet.summary ?? changeSet.id}</div>
          <div className="wb-work-detail-meta">
            {changeSet.source ?? "work"} · {changeSet.id}
          </div>
        </div>
      ))}
      {activeAgentWorkDetail.changeSets.length === 0 ? (
        <div className="wb-template-empty">No linked change sets.</div>
      ) : null}
    </div>
  ) : null;
  const terminalPanelSlot = (
    <TerminalPanel
      ref={activeTaskController.terminalPanelRef}
      workspaceId={workspaceId}
      activeTaskId={activeTaskId}
      activeSessionId={activeSessionId}
      open={activeTaskController.terminalOpen}
      height={activeTaskController.terminalHeight}
      onRequestClose={activeTaskController.closeTerminalPanel}
    />
  );
  const splitTree = useMemo(
    () =>
      buildSplitTree(workbenchWindow.layout, {
        focusedLeafId: workbenchWindow.focusedLeafId,
        activeTaskId,
        taskTitlesById,
        renderActiveTaskView: renderActiveTaskSlot,
        emptyState: emptyStateSlot,
        workDetailSlot,
        taskBoardProjection,
      }),
    [
      activeTaskId,
      emptyStateSlot,
      renderActiveTaskSlot,
      taskBoardProjection,
      taskTitlesById,
      workDetailSlot,
      workbenchWindow.focusedLeafId,
      workbenchWindow.layout,
    ],
  );
  const multipaneSplitTree = useMemo<WorkbenchSplitNode>(() => {
    if (!activeTaskController.terminalOpen) return splitTree;
    return {
      id: MULTIPANE_TERMINAL_SPLIT_ID,
      kind: "split",
      direction: "vertical",
      splitPercent: 72,
      minPercent: 45,
      maxPercent: 88,
      handleLabel: "Resize terminal pane",
      first: splitTree,
      second: {
        id: "ctx-terminal-pane",
        kind: "pane",
        content: <div className="wb-multipane-terminal-pane">{terminalPanelSlot}</div>,
        preview: {
          title: "Terminal",
          subtitle: activeTaskId ? "Task terminal" : "Workspace terminal",
          active: false,
        },
      },
    };
  }, [activeTaskController.terminalOpen, activeTaskId, splitTree, terminalPanelSlot]);
  const templateMainContent = (() => {
    switch (effectiveTemplateId) {
      case "kanban":
        return (
          <div className="wb-kanban-workspace-template">
            <WorkbenchKanbanTemplate
              lanes={kanbanLanes}
              selectedTaskId={activeTaskId}
              onSelectTask={(taskId) => {
                const card = taskBoardProjection.cardsByTaskId[taskId];
                onSelectTaskFromTemplate(taskId, card?.item.primarySessionId ?? null);
              }}
            />
            <section className="wb-kanban-detail-panel" aria-label="Selected task">
              {activeTaskSlot ?? <div className="wb-template-empty">Select a task to open its active session.</div>}
            </section>
          </div>
        );
      case "multipane":
        return (
          <WorkbenchMultipaneTemplate
            splitTree={multipaneSplitTree}
            activePaneId={workbenchWindow.focusedLeafId}
            header={
              <div className="wb-multipane-actions">
                <button
                  type="button"
                  className="wb-template-action"
                  onClick={() => onSplitWorkbenchLeaf("horizontal")}
                  title="Split right"
                  aria-label="Split right"
                >
                  <SplitSquareHorizontal size={14} aria-hidden="true" />
                  <span>Split right</span>
                </button>
                <button
                  type="button"
                  className="wb-template-action"
                  onClick={() => onSplitWorkbenchLeaf("vertical")}
                  title="Split down"
                  aria-label="Split down"
                >
                  <SplitSquareVertical size={14} aria-hidden="true" />
                  <span>Split down</span>
                </button>
              </div>
            }
            onFocusPane={(paneId: string, _pane: WorkbenchSplitPaneNode) => {
              if (paneId === "ctx-terminal-pane") return;
              onFocusWorkbenchLeaf(paneId);
            }}
            onResizeSplit={(resize: WorkbenchSplitResize) => {
              if (resize.splitId === MULTIPANE_TERMINAL_SPLIT_ID) return;
              onResizeWorkbenchSplit(resize.splitId, resize.percent / 100);
            }}
          />
        );
      case "review":
        return (
          <WorkbenchReviewTemplate
            title={activeTaskTitle ?? "Work review"}
            subtitle={activeWorkChips.length ? activeWorkChips.join(" · ") : "No linked Work records"}
            statusLabel={activeTaskId ? "Task" : null}
            metrics={reviewMetrics}
            details={reviewDetails}
            activeTaskSlot={activeTaskSlot ?? undefined}
            diffSlot={workDetailSlot ?? undefined}
          />
        );
      case "classic":
      default:
        return classicMainContent;
    }
  })();

  const rootClassName = `wb-root ${mobileShell ? "wb-root-mobile" : ""} ${sidebarCollapsed ? "wb-root-collapsed" : ""} ${sidebarResizing ? "wb-root-resizing" : ""} ${activeTaskController.diffResizing ? "wb-root-diff-resizing" : ""} ${activeTaskController.terminalResizing ? "wb-root-terminal-resizing" : ""} ${!useHtmlTopbar ? "wb-root-native-titlebar" : ""}`;
  const sharedChrome = (
    <>
      <WorktreeBootstrapSnackbar />
      {composerHarnessAuthModal}
      {archiveCleanupSnackbar}
      {transcriptNoticeSnackbar}
      {desktopStorageNoticeSnackbar}
      {topbar}
    </>
  );

  if (workspaceBootstrapGateState === "loading") {
    return (
      <div className={rootClassName} style={rootStyle}>
        {sharedChrome}
        <div className="wb-main">
          <div className="wb-center">
            <div className="wb-muted" style={{ padding: 16 }}>
              Loading workspace...
            </div>
          </div>
        </div>
      </div>
    );
  }

  if (workspaceBootstrapGateState === "error") {
    return (
      <div className={rootClassName} style={rootStyle}>
        {sharedChrome}
        <div className="wb-main">
          <div className="wb-center">
            <div style={{ maxWidth: 480, padding: 16 }}>
              <div>Failed to load workspace.</div>
              {providerBootstrapError ? (
                <div className="wb-muted" style={{ paddingTop: 8 }}>
                  {providerBootstrapError}
                </div>
              ) : null}
              <button
                style={{ marginTop: 12 }}
                onClick={onRefreshBootstrap}
                type="button"
              >
                Retry workspace load
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className={rootClassName} style={rootStyle}>
      <WorktreeBootstrapSnackbar />
      <TitleGenerationInstallBanner />
      {archiveCleanupSnackbar}
      {transcriptNoticeSnackbar}
      {desktopStorageNoticeSnackbar}
      {topbar}
      {composerHarnessAuthModal}

      {workbenchWarnings.length > 0 && (
        <div className="banner" style={{ margin: "8px 12px 0" }}>
          {workbenchWarnings[0]}
        </div>
      )}
      <WorkbenchProviderWarningBanner {...providerWarningProps} />
      {mobileShell && !sidebarCollapsed ? (
        <button
          type="button"
          className="wb-sidebar-backdrop"
          aria-label="Hide task list"
          onClick={sidebarProps.onCollapseSidebar}
        />
      ) : null}
      <WorkbenchSidebar {...sidebarProps} />

      <div className={`wb-main wb-main-template-${effectiveTemplateId}`}>
        {templateMainContent}
      </div>

      {terminalInGlobalShell ? (
        <div className="wb-terminal-shell" aria-hidden={!activeTaskController.terminalOpen}>
          {activeTaskController.terminalOpen && (
            <div className="wb-terminal-resizer" onMouseDown={activeTaskController.onTerminalResizerMouseDown} />
          )}
          <div
            className="wb-terminal-panel"
            style={{
              height: activeTaskController.terminalOpen ? activeTaskController.terminalHeight : 0,
              pointerEvents: activeTaskController.terminalOpen ? "auto" : "none",
            }}
            aria-hidden={!activeTaskController.terminalOpen}
          >
            {terminalPanelSlot}
          </div>
        </div>
      ) : null}

      <WorkbenchPageMenus
        activeTaskController={activeTaskController}
        taskListController={taskListController}
        activeTaskId={activeTaskId}
        activeSessionId={activeSessionId}
      />
    </div>
  );
}
