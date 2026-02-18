import { describe, expect, it } from "vitest";
import { computeAnalyticsCaptureEnabled } from "./runtimePolicy";

describe("computeAnalyticsCaptureEnabled", () => {
  it("defaults to disabled until settings are loaded", () => {
    expect(
      computeAnalyticsCaptureEnabled({
        settingsLoaded: false,
        telemetryEnabled: true,
        isDev: false,
        devCaptureFlag: undefined,
      }),
    ).toBe(false);
  });

  it("respects telemetry opt-out once loaded", () => {
    expect(
      computeAnalyticsCaptureEnabled({
        settingsLoaded: true,
        telemetryEnabled: false,
        isDev: false,
        devCaptureFlag: undefined,
      }),
    ).toBe(false);
  });

  it("requires explicit dev override when running in dev", () => {
    expect(
      computeAnalyticsCaptureEnabled({
        settingsLoaded: true,
        telemetryEnabled: true,
        isDev: true,
        devCaptureFlag: undefined,
      }),
    ).toBe(false);
    expect(
      computeAnalyticsCaptureEnabled({
        settingsLoaded: true,
        telemetryEnabled: true,
        isDev: true,
        devCaptureFlag: "1",
      }),
    ).toBe(true);
  });
});
