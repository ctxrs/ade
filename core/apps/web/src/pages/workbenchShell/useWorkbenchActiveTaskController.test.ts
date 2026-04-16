import { describe, expect, it } from "vitest";

import { resolveMeasuredSessionSwitchId } from "./useWorkbenchSessionSwitchMetrics";

describe("resolveMeasuredSessionSwitchId", () => {
  it("drops optimistic placeholder session ids from switch timing", () => {
    expect(resolveMeasuredSessionSwitchId("optimistic-session-1", true)).toBeNull();
  });

  it("keeps non-optimistic session ids for switch timing", () => {
    expect(resolveMeasuredSessionSwitchId("session-1", false)).toBe("session-1");
  });

  it("normalizes empty session ids to null", () => {
    expect(resolveMeasuredSessionSwitchId("   ", false)).toBeNull();
    expect(resolveMeasuredSessionSwitchId(null, false)).toBeNull();
  });
});
