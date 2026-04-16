import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { WorkbenchConversationMenu, WorkbenchSidebar } from "./WorkbenchShellChrome";

describe("WorkbenchConversationMenu", () => {
  it("renders Copy Task ID immediately after Copy Worktree Location", () => {
    render(
      <WorkbenchConversationMenu
        convoMenu={{ style: {} }}
        convoMenuRef={{ current: null }}
        activeSessionId="session-1"
        activeTaskId="task-1"
        canCopyTaskId
        copyTranscriptBusy={false}
        transcriptSpinnerDelayMs={0}
        canCopyWorktree
        archiveConversationDisabled={false}
        onExportTranscript={vi.fn()}
        onCopyTranscript={vi.fn()}
        onExportSessionLog={vi.fn()}
        onCopySessionLog={vi.fn()}
        onCopyWorktreeLocation={vi.fn()}
        onCopyTaskId={vi.fn()}
        onArchiveConversation={vi.fn()}
      />,
    );

    const menuItems = screen.getAllByRole("menuitem");
    const labels = menuItems.map((item) => item.textContent?.trim() ?? "");
    const worktreeIndex = labels.indexOf("Copy Worktree Location");
    expect(worktreeIndex).toBeGreaterThanOrEqual(0);
    expect(labels[worktreeIndex + 1]).toBe("Copy Task ID");
  });
});

describe("WorkbenchSidebar", () => {
  it("disables browser text assistance on task search", () => {
    render(
      <WorkbenchSidebar
        collapsed={false}
        taskSearchRef={{ current: null }}
        taskQuery=""
        onTaskQueryChange={vi.fn()}
        onNewTask={vi.fn()}
        taskListVirtuosoKey="tasks"
        taskListItems={[]}
        initialTaskListItemCount={undefined}
        computeTaskListItemKey={() => "task"}
        renderTaskListItem={() => null}
        taskListContext={{
          archivedCollapsed: false,
          archivedFetchState: "idle",
          hasMoreArchived: false,
          onLoadMoreArchived: vi.fn(),
        }}
        onTaskListRangeChanged={vi.fn()}
        onExpandSidebar={vi.fn()}
        onCollapseSidebar={vi.fn()}
        onSidebarResizerMouseDown={vi.fn()}
        onResetSidebarWidth={vi.fn()}
      />,
    );

    const input = screen.getByTestId("workbench-task-search");
    expect(input).toHaveAttribute("autocomplete", "off");
    expect(input).toHaveAttribute("autocorrect", "off");
    expect(input).toHaveAttribute("autocapitalize", "none");
    expect(input).toHaveAttribute("spellcheck", "false");
  });
});
