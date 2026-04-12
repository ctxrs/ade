import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as desktop from "../../utils/desktop";
import { MemoMarkdown } from "./SessionPage.markdown";

vi.mock("../../utils/desktop", () => ({
  isDesktopApp: vi.fn(() => false),
  openExternalLink: vi.fn(async () => true),
  desktopOpenFile: vi.fn(async () => true),
  desktopOpenPath: vi.fn(async () => true),
}));

function selectNodeContents(node: Node) {
  const range = document.createRange();
  range.selectNodeContents(node);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}

describe("MemoMarkdown", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(desktop.isDesktopApp).mockReturnValue(false);
    window.getSelection()?.removeAllRanges();
  });

  it("applies the shared markdown link class to external links", () => {
    render(<MemoMarkdown content="[docs](https://example.com/docs)" />);

    const link = screen.getByRole("link", { name: "docs" });
    expect(link.className).toContain("ctx-markdown-link");
  });

  it("applies the shared markdown link class to ctx file links", () => {
    render(<MemoMarkdown content="[file](ctx://open?path=/tmp/demo.txt)" />);

    const link = screen.getByRole("link", { name: "file" });
    expect(link.className).toContain("ctx-markdown-link");
    expect(link.className).toContain("ctx-file-link");
  });

  it("tokenizes assistant file paths as neutral code tokens before modifier hover", () => {
    render(<MemoMarkdown content="`.ctx/ctx-pack/agent-basics`" linkifyFiles worktreeId="wt_123" />);

    const token = screen.getByText(".ctx/ctx-pack/agent-basics");
    expect(token.tagName).toBe("SPAN");
    expect(token.className).toContain("code-token-path");
    expect(token.className).not.toContain("ctx-modifier-hover");
  });

  it("requires a modifier click before opening desktop external links", () => {
    vi.mocked(desktop.isDesktopApp).mockReturnValue(true);

    render(<MemoMarkdown content="[docs](https://example.com/docs)" />);

    const link = screen.getByRole("link", { name: "docs" });
    fireEvent.click(link);
    expect(desktop.openExternalLink).not.toHaveBeenCalled();

    fireEvent.click(link, { metaKey: true });
    expect(desktop.openExternalLink).toHaveBeenCalledWith("https://example.com/docs");
  });

  it("activates modifier hover styling when the modifier key changes while hovered", () => {
    render(<MemoMarkdown content="[docs](https://example.com/docs)" />);

    const link = screen.getByRole("link", { name: "docs" });
    expect(link.className).not.toContain("ctx-modifier-hover");

    fireEvent.mouseEnter(link);
    expect(link.className).not.toContain("ctx-modifier-hover");

    fireEvent.keyDown(window, { key: "Meta", metaKey: true });
    expect(link.className).toContain("ctx-modifier-hover");

    fireEvent.keyUp(window, { key: "Meta", metaKey: false });
    expect(link.className).not.toContain("ctx-modifier-hover");
  });

  it("renders markdown tables with transcript-owned structure classes", () => {
    render(<MemoMarkdown content={"| Day | Count |\n|---|---:|\n| 2026-04-06 | 8 |"} />);

    const wrapper = document.querySelector(".wb-md-table-scroll");
    const table = wrapper?.querySelector("table.wb-md-table");
    const headerCells = wrapper?.querySelectorAll("th.wb-md-table-cell-head");
    const bodyCells = wrapper?.querySelectorAll("td.wb-md-table-cell");

    expect(wrapper).not.toBeNull();
    expect(table).not.toBeNull();
    expect(headerCells?.length).toBe(2);
    expect(bodyCells?.length).toBe(2);
  });

  it("renders blockquotes with transcript-owned structure classes", () => {
    render(<MemoMarkdown content={"> Quoted transcript guidance"} />);

    const blockquote = document.querySelector("blockquote.wb-md-blockquote");
    expect(blockquote).not.toBeNull();
    expect(blockquote?.textContent).toContain("Quoted transcript guidance");
  });

  it("renders explicit list-marker columns and preserves markers in selection text", () => {
    const { container } = render(<MemoMarkdown content={"- first bullet\n- second bullet"} />);

    const markers = Array.from(container.querySelectorAll(".wb-md-list-item-marker")).map((node) => node.textContent);
    expect(markers).toEqual(["•", "•"]);

    const root = container.querySelector(".wb-markdown-root");
    if (!(root instanceof HTMLElement)) {
      throw new Error("Expected markdown root");
    }
    selectNodeContents(root);

    expect(window.getSelection()?.toString()).toContain("•");
    expect(window.getSelection()?.toString()).toContain("first bullet");
    expect(container.querySelector("ul.wb-md-unordered-list")).not.toBeNull();
  });

  it("renders ordered-list marker text as explicit DOM content", () => {
    const { container } = render(<MemoMarkdown content={"1. alpha\n2. beta\n10. gamma"} />);

    const markers = Array.from(container.querySelectorAll(".wb-md-list-item-marker")).map((node) => node.textContent);
    expect(markers).toEqual(["1.", "2.", "3."]);
    expect(container.querySelector("ol.wb-md-ordered-list")).not.toBeNull();
  });

  it("renders list structure for partial-looking assistant markdown content", () => {
    const { container } = render(<MemoMarkdown content={"Before\n\n- partial item"} />);

    expect(container.querySelector("ul.wb-md-unordered-list")).not.toBeNull();
    expect(container.querySelector(".wb-md-list-item-marker")?.textContent).toBe("•");
    expect(container.textContent).toContain("Before");
    expect(container.textContent).toContain("partial item");
  });
});
