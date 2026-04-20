import { describe, expect, it } from "vitest";
import {
  parseProbeWidthList,
  parseProbeWidthRange,
  resolveProbeWidths,
  selectProbeDebugWidth,
  shouldCleanupProbeScratchWorkspace,
  summarizeProbeMeasurements,
} from "./pretext-parity-probe-shared.mjs";

describe("pretext parity probe shared helpers", () => {
  it("parses deduped width lists", () => {
    expect(parseProbeWidthList("472, 540,472, 620.5")).toEqual([472, 540, 620.5]);
  });

  it("parses ascending width ranges", () => {
    expect(parseProbeWidthRange("472:476:2")).toEqual([472, 474, 476]);
  });

  it("resolves explicit width lists ahead of single width defaults", () => {
    expect(resolveProbeWidths({ width: 788, widths: "472,540", widthRange: "" })).toEqual([472, 540]);
  });

  it("summarizes drift counts and worst width", () => {
    expect(
      summarizeProbeMeasurements([
        { width: 472, planned: 42, actual: 42, delta: 0 },
        { width: 540, planned: 42, actual: 63, delta: -21 },
        { width: 620, planned: 63, actual: 42, delta: 21 },
      ]),
    ).toEqual({
      driftThreshold: 1,
      count: 3,
      driftCount: 2,
      allWithinThreshold: false,
      firstDriftWidth: 540,
      worstWidth: 540,
      worstAbsDelta: 21,
      worstDelta: -21,
      worstPlanned: 42,
      worstActual: 63,
    });
  });

  it("chooses the worst width for debug by default", () => {
    expect(
      selectProbeDebugWidth([
        { width: 472, delta: 0 },
        { width: 540, delta: -21 },
        { width: 620, delta: 7 },
      ]),
    ).toBe(540);
  });

  it("accepts an explicit debug width override", () => {
    expect(selectProbeDebugWidth([{ width: 472, delta: 0 }], "620")).toBe(620);
  });

  it("cleans up scratch workspaces by default", () => {
    expect(shouldCleanupProbeScratchWorkspace({ keepWorkspace: false }, "scratch-ws")).toBe(true);
    expect(shouldCleanupProbeScratchWorkspace({ keepWorkspace: true }, "scratch-ws")).toBe(false);
    expect(shouldCleanupProbeScratchWorkspace({ keepWorkspace: false }, "")).toBe(false);
  });
});
