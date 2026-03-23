import React, { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { copyTextToClipboardMock } = vi.hoisted(() => ({
  copyTextToClipboardMock: vi.fn(async () => true),
}));

vi.mock("../../utils/clipboard", () => ({
  copyTextToClipboard: copyTextToClipboardMock,
}));

import { AssistantEntry, WorkbenchToolRow, WorkbenchTurnHeaderView } from "./SessionThreadItemViews";

function TestHeader({ plainText = "line 1\nline 2\nline 3\nline 4\nline 5" }: { plainText?: string }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div>
      <p data-testid="outside">outside selected text</p>
      <WorkbenchTurnHeaderView
        header={{
          id: "header-1",
          content: plainText,
          attachments: [],
          created_at: "2025-01-01T00:00:00.000Z",
        }}
        plainText={plainText}
        expanded={expanded}
        onToggle={() => setExpanded((value) => !value)}
      />
    </div>
  );
}

function selectNodeContents(node: Node) {
  const range = document.createRange();
  range.selectNodeContents(node);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}

function getHeader(): HTMLDivElement {
  const header = document.querySelector(".wb-turn-header");
  if (!(header instanceof HTMLDivElement)) {
    throw new Error("Expected .wb-turn-header to be rendered");
  }
  return header;
}

describe("WorkbenchTurnHeaderView", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    window.getSelection()?.removeAllRanges();
    copyTextToClipboardMock.mockClear();
  });

  it("expands when clicked even if unrelated text elsewhere is selected", () => {
    render(<TestHeader />);

    const outside = screen.getByTestId("outside");
    selectNodeContents(outside);
    expect(window.getSelection()?.toString()).toContain("outside selected text");

    const header = getHeader();
    expect(header).toHaveAttribute("aria-expanded", "false");

    fireEvent.mouseDown(header);
    fireEvent.click(header);

    expect(header).toHaveAttribute("aria-expanded", "true");
  });

  it("does not expand when the current interaction is selecting header text", () => {
    render(<TestHeader plainText="hello world" />);

    const header = getHeader();
    const headerText = screen.getByText("hello world");
    expect(header).toHaveAttribute("aria-expanded", "false");

    fireEvent.mouseDown(header);
    selectNodeContents(headerText);
    fireEvent.click(header);

    expect(window.getSelection()?.toString()).toContain("hello world");
    expect(header).toHaveAttribute("aria-expanded", "false");
  });

  it("preserves multiline header structure when collapsed", () => {
    const { container } = render(<TestHeader plainText={"line 1\nline 2\nline 3"} />);

    const content = container.querySelector(".wb-turn-header-content");
    expect(content?.querySelectorAll("br")).toHaveLength(2);
  });

  it("copies the message when the copy button is clicked without expanding the header", async () => {
    render(<TestHeader plainText="copy me" />);

    const header = getHeader();
    const copyButton = screen.getByRole("button", { name: "Copy message" });
    expect(header).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(copyButton);

    expect(copyTextToClipboardMock).toHaveBeenCalledWith("copy me");
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("does not expand when the copy button interaction starts with mouse down", async () => {
    render(<TestHeader plainText="copy me" />);

    const header = getHeader();
    const copyButton = screen.getByRole("button", { name: "Copy message" });
    expect(header).toHaveAttribute("aria-expanded", "false");

    fireEvent.mouseDown(copyButton);
    fireEvent.click(copyButton);

    expect(copyTextToClipboardMock).toHaveBeenCalledWith("copy me");
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("does not expand when the icon glyph itself is clicked", async () => {
    render(<TestHeader plainText="copy me" />);

    const header = getHeader();
    const copyButton = screen.getByRole("button", { name: "Copy message" });
    const icon = copyButton.querySelector("svg");
    if (!(icon instanceof SVGElement)) {
      throw new Error("Expected copy icon svg to be rendered");
    }
    expect(header).toHaveAttribute("aria-expanded", "false");

    fireEvent.mouseDown(icon);
    fireEvent.click(icon);

    expect(copyTextToClipboardMock).toHaveBeenCalledWith("copy me");
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  });
});

describe("WorkbenchToolRow", () => {
  it("keeps action-style tool summaries on the primary line", () => {
    const onToggle = vi.fn();

    const { container } = render(
      <WorkbenchToolRow
        item={{
          kind: "tool",
          id: "tool-1",
          tool_call_id: "tool-call-1",
          created_at: "2025-01-01T00:00:00.000Z",
          updated_at: "2025-01-01T00:00:01.000Z",
          tool_kind: "execute",
          provider_tool_name: "Bash",
          title: "Ran",
          subtitle: "/bin/zsh -lc pwd",
          status: "running",
          locations: [],
          input: { command: "pwd" },
          output_text: "",
          raw: null,
          updates_seen: 1,
          has_details: true,
        }}
        verbosity="default"
        expanded={false}
        onToggle={onToggle}
      />,
    );

    const mainline = container.querySelector(".wb-tool-mainline");
    expect(mainline?.textContent).toContain("Ran");
    expect(mainline?.textContent).toContain("/bin/zsh -lc pwd");
    expect(container.querySelector(".wb-tool-description")).toBeNull();
  });

  it("keeps the tool description inline on the first line", () => {
    const onToggle = vi.fn();

    const { container } = render(
      <WorkbenchToolRow
        item={{
          kind: "tool",
          id: "tool-1",
          tool_call_id: "tool-call-1",
          created_at: "2025-01-01T00:00:00.000Z",
          updated_at: "2025-01-01T00:00:01.000Z",
          tool_kind: "execute",
          provider_tool_name: "Bash",
          title: "Bash",
          subtitle: "Get current working directory",
          status: "running",
          locations: [],
          input: { command: "pwd" },
          output_text: "",
          raw: null,
          updates_seen: 1,
          has_details: true,
        }}
        verbosity="default"
        expanded={false}
        onToggle={onToggle}
      />,
    );

    const mainline = container.querySelector(".wb-tool-mainline");
    expect(mainline?.textContent).toContain("Bash");
    expect(mainline?.textContent).toContain("Get current working directory");
    expect(container.querySelector(".wb-tool-description")).toBeNull();
    expect(container.querySelector("button")).toBeNull();
  });

  it("does not render tool input or output details even when expanded", () => {
    const onToggle = vi.fn();

    const { container } = render(
      <WorkbenchToolRow
        item={{
          kind: "tool",
          id: "tool-1",
          tool_call_id: "tool-call-1",
          created_at: "2025-01-01T00:00:00.000Z",
          updated_at: "2025-01-01T00:00:01.000Z",
          tool_kind: "execute",
          provider_tool_name: "Bash",
          title: "Ran",
          subtitle: "pwd",
          status: "completed",
          locations: [],
          input: { command: "pwd" },
          output_text: "output",
          raw: null,
          updates_seen: 1,
          has_details: true,
        }}
        verbosity="verbose"
        expanded={true}
        onToggle={onToggle}
      />,
    );

    expect(container.textContent).not.toContain("Input");
    expect(container.textContent).not.toContain("Output");
    expect(container.querySelector(".wb-tool-details")).toBeNull();
  });
});

describe("AssistantEntry", () => {
  it("always renders long completed assistant content without a collapse toggle", () => {
    const content = Array.from({ length: 24 }, (_, index) => `assistant line ${index + 1}`).join("\n");

    const { container } = render(
      <AssistantEntry
        content={content}
        worktreeId={null}
        onFileOpenError={() => {}}
        modifierDown={false}
      />,
    );

    expect(container.textContent).toContain("assistant line 1");
    expect(container.textContent).toContain("assistant line 24");
    expect(screen.queryByRole("button", { name: "Show more" })).toBeNull();
  });
});
