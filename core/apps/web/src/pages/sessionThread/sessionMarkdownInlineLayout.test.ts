import { describe, expect, it } from "vitest";
import { createSessionMarkdownDocument, type SessionMarkdownBlock } from "./sessionMarkdownContract";
import { prepareInlineLayoutItems, type PreparedInlineLayoutItem } from "./sessionMarkdownInlineLayout";
import { shouldDropLeadingCollapsedSpaceAtWrap } from "./sessionMarkdownInlineMeasurementContext";
import { BODY_TYPOGRAPHY } from "./sessionMarkdownMeasurementCore";

function findParagraphBlock(markdown: string, text: string): Extract<SessionMarkdownBlock, { kind: "paragraph" }> {
  const document = createSessionMarkdownDocument(markdown);
  const paragraph = document.blocks.find(
    (block): block is Extract<SessionMarkdownBlock, { kind: "paragraph" }> =>
      block.kind === "paragraph" && block.text.plainText.includes(text),
  );
  if (paragraph == null) {
    throw new Error(`Paragraph containing ${JSON.stringify(text)} not found`);
  }
  return paragraph;
}

function prepareParagraphItems(markdown: string, text: string): PreparedInlineLayoutItem[] {
  const paragraph = findParagraphBlock(markdown, text);
  return prepareInlineLayoutItems({
    runs: paragraph.text.runs,
    typography: BODY_TYPOGRAPHY,
    cacheKeyPrefix: "session-markdown-inline-layout-test",
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
});
