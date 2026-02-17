import { describe, expect, it } from "vitest";

import { isContainerizedEnvironment } from "./SettingsPage.utils";

describe("isContainerizedEnvironment", () => {
  it("returns true for container environments", () => {
    expect(isContainerizedEnvironment("container_host_mounted")).toBe(true);
    expect(isContainerizedEnvironment("container_disk_isolated")).toBe(true);
  });

  it("returns false for host and empty environments", () => {
    expect(isContainerizedEnvironment("host")).toBe(false);
    expect(isContainerizedEnvironment(null)).toBe(false);
    expect(isContainerizedEnvironment(undefined)).toBe(false);
  });
});
