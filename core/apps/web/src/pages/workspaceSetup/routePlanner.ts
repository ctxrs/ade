import type { WizardRoutePlan } from "./wizardFlow";
import type {
  EnsureOnboardingAfterDaemonConnectResult,
  RoutePlanInsertionStep,
  WorkspaceSetupProvisioningSnapshot,
} from "./workflowTypes";

type ResolveRoutePlanInsertionOptions = {
  allowTitlingInsertion?: boolean;
};

export const buildWizardRoutePlan = (
  snapshot: WorkspaceSetupProvisioningSnapshot,
): WizardRoutePlan => ({
  targetKey: `${snapshot.targetKey}|${snapshot.containerSelection}`,
  containerSelection: snapshot.containerSelection,
  includeHarnessDownloads: snapshot.missingHarnessCount > 0,
  includeAuthImport: snapshot.authImportCandidateCount > 0,
  includeTitling: snapshot.titlingMode !== "skip" && snapshot.titlingRequired,
});

export const resolveRoutePlanInsertionStep = (
  routePlan: WizardRoutePlan,
  previousPlan: WizardRoutePlan | null,
  options?: ResolveRoutePlanInsertionOptions,
): RoutePlanInsertionStep | null => {
  const allowTitlingInsertion = options?.allowTitlingInsertion ?? true;
  const shouldInsertHarnessDownloads =
    routePlan.includeHarnessDownloads && previousPlan?.includeHarnessDownloads !== true;
  if (shouldInsertHarnessDownloads) {
    return "harness-downloads";
  }

  const shouldInsertAuthImport =
    routePlan.includeAuthImport && previousPlan?.includeAuthImport !== true;
  if (shouldInsertAuthImport) {
    return "auth-import";
  }

  const shouldInsertTitling =
    allowTitlingInsertion
    && routePlan.includeTitling
    && previousPlan?.includeTitling !== true;
  if (shouldInsertTitling) {
    return "session-titling";
  }

  return null;
};

export const buildOnboardingAfterConnectResult = (
  snapshot: WorkspaceSetupProvisioningSnapshot,
  previousPlan: WizardRoutePlan | null,
  options?: ResolveRoutePlanInsertionOptions,
): EnsureOnboardingAfterDaemonConnectResult => {
  const routePlan = buildWizardRoutePlan(snapshot);
  return {
    routePlan,
    insertionStep: resolveRoutePlanInsertionStep(routePlan, previousPlan, options),
  };
};

