import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MemoMarkdown } from "./SessionPage.markdown";

vi.mock("../utils/desktop", () => ({
  isDesktopApp: vi.fn(() => false),
  openExternalLink: vi.fn(async () => true),
  desktopOpenFile: vi.fn(async () => true),
  desktopOpenPath: vi.fn(async () => true),
}));

describe("MemoMarkdown", () => {
  it("applies the shared markdown link class to external links", () => {
    render(<MemoMarkdown content="[docs](https://example.com/docs)" />);

    const link = screen.getByRole("link", { name: "docs" });
    expect(link.className).toContain("ctx-markdown-link");
  });

  it("works with the modifier wrapper used by assistant messages", () => {
    const { container } = render(
      <div className="markdown-modifier">
        <MemoMarkdown content="[docs](https://example.com/docs)" />
      </div>,
    );

    expect(container.querySelector(".markdown-modifier .ctx-markdown-link")).not.toBeNull();
  });

  it("applies the shared markdown link class to ctx file links", () => {
    render(<MemoMarkdown content="[file](ctx://open?path=/tmp/demo.txt)" />);

    const link = screen.getByRole("link", { name: "file" });
    expect(link.className).toContain("ctx-markdown-link");
    expect(link.className).toContain("ctx-file-link");
  });
});
