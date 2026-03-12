import type React from "react";
import { Virtuoso, type ListRange } from "react-virtuoso";
import { Link } from "react-router-dom";
import { ChevronsLeft, ChevronsRight, Settings, SquarePen } from "lucide-react";

import { TASK_LIST_COMPONENTS } from "../WorkbenchPage.taskList";
import type { AnchorRect, TaskListContext, TaskListItem } from "../WorkbenchPage.types";

type WorkbenchTopbarProps = {
  workspaceId: string;
  workspaceTitle: string;
  showDebugIds: boolean;
  debugIdLabel: string;
  onCopyDebugIds: () => void;
};

export function WorkbenchTopbar({
  workspaceId,
  workspaceTitle,
  showDebugIds,
  debugIdLabel,
  onCopyDebugIds,
}: WorkbenchTopbarProps) {
  return (
    <div className="wb-topbar">
      <div className="wb-topbar-left" />
      <div className="wb-topbar-center">
        {workspaceTitle ? <div className="wb-topbar-title">{workspaceTitle}</div> : null}
      </div>
      <div className="wb-topbar-right" data-tauri-drag-region={false}>
        {showDebugIds ? (
          <button
            type="button"
            className="wb-topbar-ids"
            title="Click to copy workspace/task/session IDs"
            onClick={onCopyDebugIds}
            data-tauri-drag-region={false}
          >
            {debugIdLabel}
          </button>
        ) : null}
        <Link
          className="wb-topbar-icon"
          to={`/settings?ws=${encodeURIComponent(String(workspaceId))}`}
          title="Settings"
          aria-label="Settings"
          data-tauri-drag-region={false}
        >
          <Settings size={14} />
        </Link>
      </div>
    </div>
  );
}

type WorkbenchSidebarProps = {
  collapsed: boolean;
  taskSearchRef: React.RefObject<HTMLInputElement | null>;
  taskQuery: string;
  onTaskQueryChange: (value: string) => void;
  onNewTask: () => void;
  taskListVirtuosoKey: string;
  taskListItems: TaskListItem[];
  initialTaskListItemCount: number | undefined;
  computeTaskListItemKey: (_: number, item: TaskListItem) => string;
  renderTaskListItem: (item: TaskListItem) => React.ReactNode;
  taskListContext: TaskListContext;
  onTaskListRangeChanged: (range: ListRange) => void;
  onExpandSidebar: () => void;
  onCollapseSidebar: () => void;
  onSidebarResizerMouseDown: (event: React.MouseEvent<HTMLDivElement>) => void;
  onResetSidebarWidth: () => void;
};

export function WorkbenchSidebar({
  collapsed,
  taskSearchRef,
  taskQuery,
  onTaskQueryChange,
  onNewTask,
  taskListVirtuosoKey,
  taskListItems,
  initialTaskListItemCount,
  computeTaskListItemKey,
  renderTaskListItem,
  taskListContext,
  onTaskListRangeChanged,
  onExpandSidebar,
  onCollapseSidebar,
  onSidebarResizerMouseDown,
  onResetSidebarWidth,
}: WorkbenchSidebarProps) {
  return (
    <>
      {collapsed ? (
        <button
          type="button"
          className="wb-sidebar-tab wb-sidebar-tab-collapsed"
          aria-label="Show sidebar"
          title="Show sidebar"
          onClick={onExpandSidebar}
        >
          <ChevronsRight size={16} />
        </button>
      ) : (
        <button
          type="button"
          className="wb-sidebar-tab wb-sidebar-tab-open"
          aria-label="Collapse sidebar"
          title="Collapse"
          onClick={onCollapseSidebar}
        >
          <ChevronsLeft size={16} />
        </button>
      )}

      <div className="wb-sidebar" aria-hidden={collapsed}>
        <div className="wb-sidebar-top">
          <div className="wb-sidebar-header">
            <input
              ref={taskSearchRef}
              className="wb-search"
              data-testid="workbench-task-search"
              placeholder="Search Tasks"
              value={taskQuery}
              onChange={(event) => onTaskQueryChange(event.target.value)}
            />
            <button
              type="button"
              className="wb-sidebar-action"
              aria-label="New task"
              title="New Task"
              onClick={onNewTask}
            >
              <SquarePen size={16} />
            </button>
          </div>
        </div>

        <div className="wb-sidebar-section wb-sidebar-grow" style={{ minHeight: 0, display: "flex" }}>
          <Virtuoso
            key={taskListVirtuosoKey}
            style={{ height: "100%" }}
            data={taskListItems}
            initialItemCount={initialTaskListItemCount}
            overscan={8}
            computeItemKey={computeTaskListItemKey}
            itemContent={(_, item) => renderTaskListItem(item)}
            context={taskListContext}
            rangeChanged={onTaskListRangeChanged}
            components={TASK_LIST_COMPONENTS}
          />
        </div>
      </div>

      {!collapsed ? (
        <div
          className="wb-sidebar-resizer"
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize sidebar"
          onMouseDown={onSidebarResizerMouseDown}
          onDoubleClick={onResetSidebarWidth}
        />
      ) : null}
    </>
  );
}

type WorkbenchTaskMenuProps = {
  taskMenu: { taskId: string; style: React.CSSProperties } | null;
  taskMenuRef: React.RefObject<HTMLDivElement | null>;
  archiveDisabled: boolean;
  archiveLabel: string;
  markReadDisabled: boolean;
  markReadLabel: string;
  onRename: () => void;
  onToggleArchive: (event: React.MouseEvent<HTMLButtonElement>) => void;
  onToggleRead: () => void;
  onDelete: () => void;
};

export function WorkbenchTaskMenu({
  taskMenu,
  taskMenuRef,
  archiveDisabled,
  archiveLabel,
  markReadDisabled,
  markReadLabel,
  onRename,
  onToggleArchive,
  onToggleRead,
  onDelete,
}: WorkbenchTaskMenuProps) {
  if (!taskMenu) {
    return null;
  }

  return (
    <div className="wb-menu wb-task-menu" role="menu" ref={taskMenuRef} style={taskMenu.style}>
      <button type="button" className="wb-menu-item" onClick={onRename} role="menuitem">
        Rename Task
      </button>
      <button
        type="button"
        className="wb-menu-item wb-archive-confirm-trigger"
        disabled={archiveDisabled}
        onClick={onToggleArchive}
        role="menuitem"
      >
        {archiveLabel}
      </button>
      <button
        type="button"
        className="wb-menu-item"
        disabled={markReadDisabled}
        onClick={onToggleRead}
        role="menuitem"
      >
        {markReadLabel}
      </button>
      <button
        type="button"
        className="wb-menu-item wb-menu-item-danger"
        onClick={onDelete}
        role="menuitem"
      >
        Delete Task
      </button>
    </div>
  );
}

type WorkbenchArchiveConfirmProps = {
  archiveConfirm: { taskId: string; anchor: AnchorRect } | null;
  archiveConfirmStyle: React.CSSProperties | null;
  archiveConfirmRef: React.RefObject<HTMLDivElement | null>;
  archiveConfirmDontRemind: boolean;
  onArchiveConfirmDontRemindChange: (checked: boolean) => void;
  onCancel: () => void;
  onConfirm: () => void;
};

export function WorkbenchArchiveConfirm({
  archiveConfirm,
  archiveConfirmStyle,
  archiveConfirmRef,
  archiveConfirmDontRemind,
  onArchiveConfirmDontRemindChange,
  onCancel,
  onConfirm,
}: WorkbenchArchiveConfirmProps) {
  if (!archiveConfirm || !archiveConfirmStyle) {
    return null;
  }

  return (
    <div
      className="wb-archive-confirm wb-menu-tooltip"
      data-open="true"
      role="dialog"
      aria-label="Archive confirmation"
      ref={archiveConfirmRef}
      style={archiveConfirmStyle}
    >
      <div className="wb-archive-confirm-title">Archive conversation?</div>
      <div className="wb-archive-confirm-body">
        Archiving deletes the ctx-managed worktrees and branches associated with this task, including its
        subagents. Later, you can unarchive to recreate them, but unmerged changes will be lost.
        <br />
        <br />
        If you want to keep changes made here, consider instructing the primary agent to use the Merge Queue
        to bring the changes into your main branch. Otherwise, tell it to stash the changes into another
        local or remote branch for later use.
        <br />
        <br />
        In general, we recommend aggressively archiving tasks as you complete work for performance and
        organization. You can always unarchive any task later, which will restore all conversation history,
        including subagents.
      </div>
      <label className="wb-archive-confirm-toggle">
        <input
          type="checkbox"
          checked={archiveConfirmDontRemind}
          onChange={(event) => onArchiveConfirmDontRemindChange(event.target.checked)}
        />
        Don&apos;t ask me again
      </label>
      <div className="wb-archive-confirm-actions">
        <button type="button" className="wb-snackbar-btn wb-snackbar-btn-secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="wb-snackbar-btn wb-archive-confirm-danger" onClick={onConfirm}>
          Archive
        </button>
      </div>
    </div>
  );
}

type WorkbenchConversationMenuProps = {
  convoMenu: { style: React.CSSProperties } | null;
  convoMenuRef: React.RefObject<HTMLDivElement | null>;
  activeSessionId: string | null;
  copyTranscriptBusy: boolean;
  transcriptSpinnerDelayMs: number;
  canCopyWorktree: boolean;
  archiveConversationDisabled: boolean;
  onExportTranscript: () => void;
  onCopyTranscript: () => void;
  onExportSessionLog: () => void;
  onCopySessionLog: () => void;
  onCopyWorktreeLocation: () => void;
  onArchiveConversation: (event: React.MouseEvent<HTMLButtonElement>) => void;
};

export function WorkbenchConversationMenu({
  convoMenu,
  convoMenuRef,
  activeSessionId,
  copyTranscriptBusy,
  transcriptSpinnerDelayMs,
  canCopyWorktree,
  archiveConversationDisabled,
  onExportTranscript,
  onCopyTranscript,
  onExportSessionLog,
  onCopySessionLog,
  onCopyWorktreeLocation,
  onArchiveConversation,
}: WorkbenchConversationMenuProps) {
  if (!convoMenu) {
    return null;
  }

  return (
    <div className="wb-menu wb-convo-menu" role="menu" ref={convoMenuRef} style={convoMenu.style}>
      <button
        type="button"
        className="wb-menu-item"
        disabled={!activeSessionId}
        onClick={onExportTranscript}
        role="menuitem"
      >
        Export Transcript
      </button>
      <button
        type="button"
        className="wb-menu-item"
        disabled={!activeSessionId || copyTranscriptBusy}
        onClick={onCopyTranscript}
        role="menuitem"
      >
        <span className="wb-menu-item-row">
          <span>Copy Transcript</span>
          {copyTranscriptBusy ? (
            <span
              className="wb-task-spinner wb-menu-item-spinner"
              style={{ animationDelay: `${transcriptSpinnerDelayMs}ms` }}
              aria-hidden="true"
            />
          ) : null}
        </span>
      </button>
      <button
        type="button"
        className="wb-menu-item"
        disabled={!activeSessionId}
        onClick={onExportSessionLog}
        role="menuitem"
      >
        Export Session Log
      </button>
      <button
        type="button"
        className="wb-menu-item"
        disabled={!activeSessionId}
        onClick={onCopySessionLog}
        role="menuitem"
      >
        Copy Session Log
      </button>
      <button
        type="button"
        className="wb-menu-item"
        disabled={!canCopyWorktree}
        onClick={onCopyWorktreeLocation}
        role="menuitem"
      >
        Copy Worktree Location
      </button>
      <button
        type="button"
        className="wb-menu-item wb-archive-confirm-trigger"
        disabled={archiveConversationDisabled}
        onClick={onArchiveConversation}
        role="menuitem"
      >
        Archive Conversation
      </button>
    </div>
  );
}
