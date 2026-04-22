import { describe, expect, it } from "vitest";
import { createSessionMarkdownDocument, type SessionMarkdownBlock } from "./sessionMarkdownContract";
import { prepareInlineLayoutItems, type PreparedInlineLayoutItem } from "./sessionMarkdownInlineLayout";
import { shouldDropLeadingCollapsedSpaceAtWrap } from "./sessionMarkdownInlineMeasurementContext";
import { BODY_TYPOGRAPHY, segmentImplicitWordBreaks } from "./sessionMarkdownMeasurementCore";

function findParagraphBlock(markdown: string, text: string): Extract<SessionMarkdownBlock, { kind: "paragraph" }> {
  const document = createSessionMarkdownDocument(markdown);
  const visitBlocks = (
    blocks: readonly SessionMarkdownBlock[],
  ): Extract<SessionMarkdownBlock, { kind: "paragraph" }> | null => {
    for (const block of blocks) {
      if (block.kind === "paragraph" && block.text.plainText.includes(text)) {
        return block;
      }
      if (block.kind === "blockquote") {
        const nested = visitBlocks(block.blocks);
        if (nested != null) {
          return nested;
        }
      }
      if (block.kind === "list") {
        for (const item of block.items) {
          const nested = visitBlocks(item.blocks);
          if (nested != null) {
            return nested;
          }
        }
      }
      if (block.kind === "table") {
        for (const row of block.rows) {
          for (const cell of row.cells) {
            if (cell == null) {
              continue;
            }
            const nested = visitBlocks(cell.blocks);
            if (nested != null) {
              return nested;
            }
          }
        }
      }
    }
    return null;
  };
  const paragraph = visitBlocks(document.blocks);
  if (paragraph == null) {
    throw new Error(`Paragraph containing ${JSON.stringify(text)} not found`);
  }
  return paragraph;
}

function prepareParagraphItems(
  markdown: string,
  text: string,
  wrapMode?: "normal" | "break-word",
): PreparedInlineLayoutItem[] {
  const paragraph = findParagraphBlock(markdown, text);
  return prepareInlineLayoutItems({
    runs: paragraph.text.runs,
    typography: BODY_TYPOGRAPHY,
    cacheKeyPrefix: "session-markdown-inline-layout-test",
    wrapMode,
  });
}

describe("sessionMarkdownInlineLayout", () => {
  it("keeps slash-delimited prose tokens as standalone text segments", () => {
    const items = prepareParagraphItems(
      "A second boundary sample keeps `inline-token 9` beside ordinary prose in a containerized/sandboxed path. The browser should keep that slash-delimited token whole while the surrounding sentence wraps naturally.",
      "containerized/sandboxed",
    );

    const textSegments = items
      .filter((item): item is Extract<PreparedInlineLayoutItem, { kind: "segment" }> => item.kind === "segment")
      .filter((item) => item.codeGroupId == null)
      .map((item) => item.text);

    expect(textSegments).toContain("containerized/sandboxed");
    expect(textSegments.some((text) => text.endsWith("containerized/"))).toBe(false);
  });

  it("prevents styled-seam prose from dropping its leading collapsed wrap space", () => {
    const items = prepareParagraphItems(
      [
        "**Important caveat**",
        "Keep the highlighted sample beside ordinary prose unless the layout rule requires a fresh line:",
      ].join("\n"),
      "Keep the highlighted sample beside ordinary prose",
    );

    const target = items.find(
      (item): item is Extract<PreparedInlineLayoutItem, { kind: "segment" }> =>
        item.kind === "segment" &&
        item.codeGroupId == null &&
        item.text.startsWith("Keep the highlighted sample beside ordinary prose"),
    );

    expect(target).toBeDefined();
    expect(target?.startsAfterStyledTextSeam).toBe(true);
    expect(
      shouldDropLeadingCollapsedSpaceAtWrap({
        item: target!,
        codeGroupId: null,
        lineHasContent: true,
        cursor: null,
        pendingSpaceWidth: 8,
      }),
    ).toBe(false);
    expect(
      shouldDropLeadingCollapsedSpaceAtWrap({
        item: {
          ...target!,
          startsAfterInlineCodeSeam: false,
          startsAfterStyledTextSeam: false,
          startsStyledTextAfterInlineCodeSeam: false,
          startsStyledTextAfterBodySeam: false,
        },
        codeGroupId: null,
        lineHasContent: true,
        cursor: null,
        pendingSpaceWidth: 8,
      }),
    ).toBe(true);
  });

  it("uses implicit Thai word boundaries for styled-seam min-start width", () => {
    const items = prepareParagraphItems(
      "Lead [parity](https://example.com) *token* ทดสอบการตัดคำ fragment session one host/workspace.",
      "ทดสอบการตัดคำ fragment session one",
    );

    const target = items.find(
      (item): item is Extract<PreparedInlineLayoutItem, { kind: "segment" }> =>
        item.kind === "segment" &&
        item.codeGroupId == null &&
        item.text.startsWith("ทดสอบการตัดคำ fragment session one"),
    );

    expect(target).toBeDefined();
    expect(target?.startsAfterStyledTextSeam).toBe(true);
    const implicitSegments = segmentImplicitWordBreaks("ทดสอบการตัดคำ");

    expect(implicitSegments.length).toBeGreaterThan(1);
    expect(implicitSegments.join("")).toBe("ทดสอบการตัดคำ");
    expect(target!.text.startsWith(implicitSegments[0] ?? "")).toBe(true);
  });

  it("disables prose min-start guards for break-word table-cell text", () => {
    const items = prepareParagraphItems(
      "| Kind | Note |\n|---|---|\n| summary | browser context containerd/BuildKit/nerdctl padding stream. |",
      "browser context containerd/BuildKit/nerdctl padding stream.",
      "break-word",
    );

    const target = items.find(
      (item): item is Extract<PreparedInlineLayoutItem, { kind: "segment" }> =>
        item.kind === "segment" &&
        item.codeGroupId == null &&
        item.text.includes("containerd/BuildKit/nerdctl"),
    );

    expect(target).toBeDefined();
    expect(target?.allowsBreakWord).toBe(true);
    expect(target?.minStartTextWidth).toBe(0);
  });

  it("keeps surrounding prose attached to slash tokens in break-word table-cell text", () => {
    const items = prepareParagraphItems(
      "| Kind | Note |\n|---|---|\n| summary | browser context containerd/BuildKit/nerdctl padding stream. |",
      "browser context containerd/BuildKit/nerdctl padding stream.",
      "break-word",
    );

    const textSegments = items
      .filter((item): item is Extract<PreparedInlineLayoutItem, { kind: "segment" }> => item.kind === "segment")
      .filter((item) => item.codeGroupId == null)
      .map((item) => item.text);

    expect(
      textSegments.some(
        (text) =>
          text.startsWith("browser context ") &&
          text.includes("containerd/BuildKit/nerdctl") &&
          text.endsWith(" padding stream."),
      ),
    ).toBe(true);
    expect(textSegments).not.toContain("containerd/BuildKit/nerdctl");
  });

  it("splits a trailing plain hyphenated path tail into deterministic code fragments", () => {
    const items = prepareParagraphItems(
      "Probe `pages/apps/turn-header`: trailing prose keeps wrapping.",
      "Probe",
    );

    const codeSegments = items
      .filter((item): item is Extract<PreparedInlineLayoutItem, { kind: "segment" }> => item.kind === "segment")
      .filter((item) => item.codeGroupId != null)
      .map((item) => item.text);

    expect(codeSegments).toContain("turn-");
    expect(codeSegments).toContain("header");
    expect(codeSegments).not.toContain("turn-header");
  });
});
