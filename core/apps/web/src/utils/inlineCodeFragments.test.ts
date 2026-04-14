import { describe, expect, it } from "vitest";
import { splitInlineCodeFragments } from "./inlineCodeFragments";

describe("splitInlineCodeFragments", () => {
  it("coalesces short slash stems into stable path fragments", () => {
    expect(splitInlineCodeFragments("table/pages/inline-code/blockquote/sessionMarkdownMeasurement.ts")).toEqual([
      "table/pages/",
      "inline-code/blockquote/",
      "sessionMarkdownMeasurement.",
      "ts",
    ]);
  });

  it("coalesces short hyphen fragments with following path clusters", () => {
    expect(
      splitInlineCodeFragments(
        "sessionThread/sessionThreadDomMeasurement.tsx/inline-code/pages/pretextVirtualizerRowLayout.ts/web",
      ),
    ).toEqual([
      "sessionThread/",
      "sessionThreadDomMeasurement.",
      "tsx/inline-code/pages/",
      "pretextVirtualizerRowLayout.",
      "ts/web",
    ]);
  });

  it("coalesces hyphenated prefixes with short path clusters but preserves longer tails", () => {
    expect(
      splitInlineCodeFragments(
        "turn-header/fixtures/sessionMarkdownMeasurement.ts/src/blockquote/pretextVirtualizerRowLayout.ts/core",
      ),
    ).toEqual([
      "turn-header/fixtures/",
      "sessionMarkdownMeasurement.",
      "ts/src/blockquote/",
      "pretextVirtualizerRowLayout.",
      "ts/core",
    ]);
    expect(splitInlineCodeFragments("pages/apps/inline-code/blockquote/turn-header/fixtures/web")).toEqual([
      "pages/apps/",
      "inline-code/blockquote/",
      "turn-header/fixtures/",
      "web",
    ]);
  });
});
