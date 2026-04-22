import { describe, expect, it } from "vitest";
import { shouldBreakBeforePunctuationOnlyContinuationTail } from "./sessionMarkdownInlineMeasurementTextPlacement";

describe("sessionMarkdownInlineMeasurementTextPlacement", () => {
  it("breaks before a punctuation-only tail slice after a continued path-like code fragment", () => {
    expect(
      shouldBreakBeforePunctuationOnlyContinuationTail({
        codeGroupId: null,
        allowsBreakWord: true,
        startsAfterPathLikeInlineCodeSeam: true,
        lineHasContent: true,
        atItemStart: true,
        lineStartedWithContinuedCode: true,
        lineEndsAtItemEnd: false,
        lineSegmentText: ": ",
      }),
    ).toBe(true);
  });

  it("does not break once the tail slice already carries following prose", () => {
    expect(
      shouldBreakBeforePunctuationOnlyContinuationTail({
        codeGroupId: null,
        allowsBreakWord: true,
        startsAfterPathLikeInlineCodeSeam: true,
        lineHasContent: true,
        atItemStart: true,
        lineStartedWithContinuedCode: true,
        lineEndsAtItemEnd: false,
        lineSegmentText: ": thread delta ",
      }),
    ).toBe(false);
  });

  it("does not break for ordinary non-path inline-code seams", () => {
    expect(
      shouldBreakBeforePunctuationOnlyContinuationTail({
        codeGroupId: null,
        allowsBreakWord: true,
        startsAfterPathLikeInlineCodeSeam: false,
        lineHasContent: true,
        atItemStart: true,
        lineStartedWithContinuedCode: true,
        lineEndsAtItemEnd: false,
        lineSegmentText: ": ",
      }),
    ).toBe(false);
  });
});
