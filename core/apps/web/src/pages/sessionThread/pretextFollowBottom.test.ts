import { describe, expect, it } from "vitest";
import {
  resolveFollowBottomAfterScroll,
  shouldRestoreBottomOnViewportResize,
} from "./pretextFollowBottom";

describe("pretext follow-bottom helpers", () => {
  it("detaches when the user scrolls upward even if they remain within the bottom threshold", () => {
    expect(
      resolveFollowBottomAfterScroll({
        followBottom: true,
        previousScrollTop: 1100,
        currentScrollTop: 1090,
        bottomOffsetPx: 10,
        thresholdPx: 16,
        programmaticScroll: false,
      }),
    ).toBe(false);
  });

  it("re-enables follow-bottom when the user scrolls back down to the bottom threshold", () => {
    expect(
      resolveFollowBottomAfterScroll({
        followBottom: false,
        previousScrollTop: 1090,
        currentScrollTop: 1100,
        bottomOffsetPx: 0,
        thresholdPx: 16,
        programmaticScroll: false,
      }),
    ).toBe(true);
  });

  it("does not reattach just because a detached near-bottom scroll settles slightly downward within the threshold", () => {
    expect(
      resolveFollowBottomAfterScroll({
        followBottom: false,
        previousScrollTop: 1090,
        currentScrollTop: 1096,
        bottomOffsetPx: 10,
        thresholdPx: 16,
        programmaticScroll: false,
      }),
    ).toBe(false);
  });

  it("re-enables follow-bottom for programmatic bottom restores", () => {
    expect(
      resolveFollowBottomAfterScroll({
        followBottom: false,
        previousScrollTop: 1090,
        currentScrollTop: 1100,
        bottomOffsetPx: 8,
        thresholdPx: 16,
        programmaticScroll: true,
      }),
    ).toBe(true);
  });

  it("restores bottom on resize only while follow-bottom is still attached", () => {
    expect(shouldRestoreBottomOnViewportResize(true, false)).toBe(false);
    expect(shouldRestoreBottomOnViewportResize(true, true)).toBe(true);
  });
});
