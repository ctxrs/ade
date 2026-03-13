import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { WorkbenchConversationMenu } from "./WorkbenchShellChrome";

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
