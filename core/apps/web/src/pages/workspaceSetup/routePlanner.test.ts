import { describe, expect, it } from "vitest";
import {
  buildOnboardingAfterConnectResult,
  buildWizardRoutePlan,
  resolveRoutePlanInsertionStep,
} from "./routePlanner";
import type { WorkspaceSetupProvisioningSnapshot } from "./workflowTypes";

const snapshot = (
  overrides?: Partial<WorkspaceSetupProvisioningSnapshot>,
): WorkspaceSetupProvisioningSnapshot => ({
  targetKey: "local",
  containerSelection: "disk-isolated",
  authImportCandidateCount: 0,
  missingHarnessCount: 0,
  titlingRequired: false,
  titlingMode: "unset",
  ...overrides,
});

describe("routePlanner", () => {
  it("builds a route plan from explicit provisioning snapshot state", () => {
    expect(buildWizardRoutePlan(snapshot({
      authImportCandidateCount: 2,
      missingHarnessCount: 1,
      titlingRequired: true,
      titlingMode: "remote",
    }))).toEqual({
      targetKey: "local|disk-isolated",
      containerSelection: "disk-isolated",
      includeHarnessDownloads: true,
      includeAuthImport: true,
      includeTitling: true,
    });
  });

  it("suppresses titling insertion when the user already chose skip", () => {
    expect(buildWizardRoutePlan(snapshot({
      titlingRequired: true,
      titlingMode: "skip",
    })).includeTitling).toBe(false);
  });

  it("prefers harness downloads before auth import and titling for new insertions", () => {
    const plan = buildWizardRoutePlan(snapshot({
      authImportCandidateCount: 1,
      missingHarnessCount: 1,
      titlingRequired: true,
      titlingMode: "remote",
    }));
    expect(resolveRoutePlanInsertionStep(plan, null)).toBe("harness-downloads");
  });

  it("can suppress titling insertion during create-time rechecks", () => {
    const result = buildOnboardingAfterConnectResult(snapshot({
      titlingRequired: true,
      titlingMode: "remote",
    }), null, { allowTitlingInsertion: false });
    expect(result.insertionStep).toBeNull();
    expect(result.routePlan.includeTitling).toBe(true);
  });
});
