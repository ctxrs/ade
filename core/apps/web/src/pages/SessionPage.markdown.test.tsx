import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as desktop from "../utils/desktop";
import { MemoMarkdown } from "./SessionPage.markdown";

vi.mock("../utils/desktop", () => ({
  isDesktopApp: vi.fn(() => false),
  openExternalLink: vi.fn(async () => true),
  desktopOpenFile: vi.fn(async () => true),
  desktopOpenPath: vi.fn(async () => true),
}));

describe("MemoMarkdown", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(desktop.isDesktopApp).mockReturnValue(false);
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
});
