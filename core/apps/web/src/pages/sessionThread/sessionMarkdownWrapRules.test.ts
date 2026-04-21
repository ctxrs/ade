import { describe, expect, it } from "vitest";
import { getPretextWrapRuleById } from "../../testdata/pretextWrapRuleCatalog";
import { measureSessionMarkdownDocument } from "./sessionMarkdownMeasurement";
import { BODY_LINE_HEIGHT_PX } from "./sessionMarkdownMeasurementCore";

describe("sessionMarkdownWrapRules", () => {
  it("collapses ordinary spaces in body prose like the browser's normal white-space mode", () => {
    const rule = getPretextWrapRuleById("ws-collapse-normal");

    const collapsed = measureSessionMarkdownDocument("alpha beta", 220);
    const repeated = measureSessionMarkdownDocument(rule.markdown, 220);

    expect(repeated).toBe(collapsed);
  });

  it("treats markdown hard breaks as forced line breaks in inline content", () => {
    const rule = getPretextWrapRuleById("hard-break-forced-line");

    const height = measureSessionMarkdownDocument(rule.markdown, 420);

    expect(height).toBe(BODY_LINE_HEIGHT_PX * 2);
  });
});
