import { describe, expect, it } from "vitest";
import {
  SESSION_MARKDOWN_MEASUREMENT_CONTRACT,
  SESSION_THREAD_MEASUREMENT_GEOMETRY_REVISION,
  SESSION_THREAD_ROW_MEASUREMENT_CONTRACT,
} from "./sessionThreadMeasurementContract";
import { resolveSessionMarkdownListMarkerColumnWidthPx } from "./sessionThreadLayoutTokens";

describe("sessionThreadMeasurementContract", () => {
  it("keeps inline code chrome derived from the edge geometry", () => {
    expect(SESSION_MARKDOWN_MEASUREMENT_CONTRACT.inlineCode.fragmentChromeWidthPx).toBe(
      SESSION_MARKDOWN_MEASUREMENT_CONTRACT.inlineCode.edgeInlinePx * 2,
    );
    expect(SESSION_MARKDOWN_MEASUREMENT_CONTRACT.inlineCode.fragmentChromeHeightPx).toBe(
      SESSION_MARKDOWN_MEASUREMENT_CONTRACT.inlineCode.edgeBlockPx * 2,
    );
  });

  it("keeps list marker widths above the minimum gutter and expands for wider ordered markers", () => {
    const bulletWidth = resolveSessionMarkdownListMarkerColumnWidthPx(["•"]);
    const orderedWidth = resolveSessionMarkdownListMarkerColumnWidthPx(["1.", "10.", "100."]);

    expect(bulletWidth).toBeGreaterThanOrEqual(
      SESSION_MARKDOWN_MEASUREMENT_CONTRACT.list.markerMinWidthPx,
    );
    expect(orderedWidth).toBeGreaterThan(bulletWidth);
  });

  it("keeps both measurement contracts on the same shared geometry revision", () => {
    expect(SESSION_MARKDOWN_MEASUREMENT_CONTRACT.geometryRevision).toBe(
      SESSION_THREAD_MEASUREMENT_GEOMETRY_REVISION,
    );
    expect(SESSION_THREAD_ROW_MEASUREMENT_CONTRACT.geometryRevision).toBe(
      SESSION_THREAD_MEASUREMENT_GEOMETRY_REVISION,
    );
  });

  it("captures markdown block-gap overrides and checkbox gutter in the shared contract", () => {
    expect(SESSION_MARKDOWN_MEASUREMENT_CONTRACT.list.checkboxGutterPx).toBeGreaterThan(
      SESSION_MARKDOWN_MEASUREMENT_CONTRACT.list.markerGapPx,
    );
    expect(SESSION_MARKDOWN_MEASUREMENT_CONTRACT.blockSpacing.entryGapPxByContext.root.heading).toBe(16);
    expect(SESSION_MARKDOWN_MEASUREMENT_CONTRACT.blockSpacing.exitGapPxByContext.listItem.paragraph).toBe(0);
  });
});
