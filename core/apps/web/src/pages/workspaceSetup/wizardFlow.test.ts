import { describe, expect, it } from "vitest";
import {
  buildWizardStepPath,
  nextAfterAuthImport,
  nextAfterHarnessDownloads,
  nextBoundaryStep,
  resolveWizardCurrentStepKey,
  stepKeyOffset,
  type WizardRoutePlan,
} from "./wizardFlow";

const routePlan = (overrides?: Partial<WizardRoutePlan>): WizardRoutePlan => ({
  targetKey: "local",
  containerSelection: "disk-isolated",
  includeHarnessDownloads: false,
  includeAuthImport: false,
  includeTitling: false,
  ...overrides,
});

describe("wizardFlow", () => {
  it("builds the stable local host path", () => {
    expect(
      buildWizardStepPath({
        containerSelection: "no-container",
        routePlan: routePlan({ containerSelection: "no-container" }),
      }),
    ).toEqual([
      "location",
      "container",
      "source",
      "setup",
      "merge-queue",
      "confirm",
    ]);
  });

  it("includes optional post-container steps from the frozen route plan", () => {
    expect(
      buildWizardStepPath({
        containerSelection: "disk-isolated",
        routePlan: routePlan({
          includeHarnessDownloads: true,
          includeAuthImport: true,
          includeTitling: true,
        }),
      }),
    ).toEqual([
      "location",
      "container",
      "harness-downloads",
      "auth-import",
      "session-titling",
      "source",
      "network",
      "setup",
      "merge-queue",
      "confirm",
    ]);
  });

  it("preserves the current optional step even if the latest route plan no longer includes it", () => {
    expect(
      buildWizardStepPath({
        containerSelection: "disk-isolated",
        routePlan: routePlan({ includeHarnessDownloads: false }),
        currentStepKey: "harness-downloads",
      }),
    ).toContain("harness-downloads");
  });

  it("resolves the current step without falling back to location when a later optional step disappears", () => {
    const keys = buildWizardStepPath({
      containerSelection: "disk-isolated",
      routePlan: routePlan({ includeAuthImport: true }),
    });
    expect(resolveWizardCurrentStepKey(keys, "harness-downloads", 2)).toBe("auth-import");
  });

  it("uses the frozen route plan to determine explicit forward routing", () => {
    const plan = routePlan({
      includeHarnessDownloads: true,
      includeAuthImport: true,
      includeTitling: true,
    });
    expect(nextBoundaryStep(plan)).toBe("harness-downloads");
    expect(nextAfterHarnessDownloads(plan)).toBe("auth-import");
    expect(nextAfterAuthImport(plan)).toBe("session-titling");
  });

  it("walks backward and forward over the explicit path", () => {
    const keys = buildWizardStepPath({
      containerSelection: "disk-isolated",
      routePlan: routePlan({
        includeHarnessDownloads: true,
        includeAuthImport: true,
      }),
    });
    expect(stepKeyOffset(keys, "container", 1)).toBe("harness-downloads");
    expect(stepKeyOffset(keys, "source", -1)).toBe("auth-import");
  });
});
