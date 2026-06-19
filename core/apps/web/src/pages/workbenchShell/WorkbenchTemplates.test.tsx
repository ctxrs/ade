// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { WorkbenchKanbanTemplate } from "./WorkbenchKanbanTemplate";
import { WorkbenchMultipaneTemplate } from "./WorkbenchMultipaneTemplate";
import { WorkbenchReviewTemplate } from "./WorkbenchReviewTemplate";
import { WorkbenchTemplateSwitcher } from "./WorkbenchTemplateSwitcher";
import {
  WORKBENCH_TEMPLATE_LIST,
  isWorkbenchTemplateId,
  type WorkbenchSplitNode,
} from "./WorkbenchTemplateTypes";

describe("workbench template registry", () => {
  it("defines the built-in templates in switcher order", () => {
    expect(WORKBENCH_TEMPLATE_LIST.map((template) => template.id)).toEqual([
      "classic",
      "kanban",
      "multipane",
      "review",
    ]);
    expect(isWorkbenchTemplateId("review")).toBe(true);
    expect(isWorkbenchTemplateId("missing")).toBe(false);
  });
});

describe("WorkbenchTemplateSwitcher", () => {
  it("selects a template from the compact control", () => {
    const onSelectTemplate = vi.fn();

    render(
      <WorkbenchTemplateSwitcher
        activeTemplateId="classic"
        onSelectTemplate={onSelectTemplate}
      />,
    );

    fireEvent.click(screen.getByRole("radio", { name: "Kanban" }));

    expect(onSelectTemplate).toHaveBeenCalledWith("kanban");
    expect(screen.getByRole("radio", { name: "Classic" })).toHaveAttribute("aria-checked", "true");
  });
});

describe("WorkbenchKanbanTemplate", () => {
  it("renders lanes and calls onSelectTask with card context", () => {
    const onSelectTask = vi.fn();

    render(
      <WorkbenchKanbanTemplate
        selectedTaskId="task-2"
        lanes={[
          {
            id: "todo",
            title: "Todo",
            cards: [
              { id: "task-1", title: "Wire switcher", meta: ["UI"] },
              { id: "task-2", title: "Review diff", tone: "active" },
            ],
          },
        ]}
        onSelectTask={onSelectTask}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Wire switcher/ }));

    expect(screen.getByRole("heading", { name: "Todo" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Review diff/ })).toHaveAttribute("aria-current", "true");
    expect(onSelectTask).toHaveBeenCalledWith(
      "task-1",
      expect.objectContaining({ id: "task-1" }),
      expect.objectContaining({ id: "todo" }),
    );
  });
});

describe("WorkbenchMultipaneTemplate", () => {
  const splitTree: WorkbenchSplitNode = {
    id: "root",
    kind: "split",
    direction: "horizontal",
    splitPercent: 40,
    first: {
      id: "conversation",
      kind: "pane",
      preview: { title: "Conversation" },
    },
    second: {
      id: "diff",
      kind: "pane",
      preview: { title: "Diff" },
    },
  };

  it("reports focused panes and controlled split resize intents", () => {
    const onFocusPane = vi.fn();
    const onResizeSplit = vi.fn();

    render(
      <WorkbenchMultipaneTemplate
        splitTree={splitTree}
        activePaneId="diff"
        onFocusPane={onFocusPane}
        onResizeSplit={onResizeSplit}
      />,
    );

    fireEvent.pointerDown(screen.getByText("Conversation").closest(".wb-split-pane") as Element);
    fireEvent.keyDown(screen.getByRole("separator", { name: "Resize panes" }), { key: "ArrowRight" });

    expect(onFocusPane).toHaveBeenCalledWith("conversation", expect.objectContaining({ id: "conversation" }));
    expect(onResizeSplit).toHaveBeenCalledWith({
      splitId: "root",
      direction: "horizontal",
      percent: 45,
      source: "keyboard",
    });
  });
});

describe("WorkbenchReviewTemplate", () => {
  it("renders summary metadata and supplied content slots", () => {
    render(
      <WorkbenchReviewTemplate
        title="Task review"
        subtitle="Workbench shell"
        statusLabel="Ready"
        metrics={[{ label: "Files", value: 3 }]}
        details={[{ label: "Owner", value: "agent" }]}
        activeTaskSlot={<div>Active task content</div>}
        diffSlot={<div>Diff content</div>}
        sidebarSlot={<div>Notes</div>}
      />,
    );

    expect(screen.getByRole("heading", { name: "Task review" })).toBeInTheDocument();
    expect(screen.getByText("Files")).toBeInTheDocument();
    expect(screen.getByText("Active task content")).toBeInTheDocument();
    expect(screen.getByText("Diff content")).toBeInTheDocument();
    expect(screen.getByText("Notes")).toBeInTheDocument();
  });
});
