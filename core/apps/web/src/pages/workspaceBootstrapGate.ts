import {
  loadProviderOnboardingBootstrap,
  type ProviderOnboardingBootstrapState,
} from "../state/providerOnboardingCoordinator";
import { withProviderBootstrapTimeout } from "../utils/providerBootstrapTimeout";

export type WorkspaceBootstrapGateState = "loading" | "ready" | "error";

export function resolveWorkspaceBootstrapGateState({
  workbenchHydrated,
  providerBootstrapState,
}: {
  workbenchHydrated: boolean;
  providerBootstrapState: ProviderOnboardingBootstrapState;
}): WorkspaceBootstrapGateState {
  if (!workbenchHydrated) return "loading";
  if (providerBootstrapState === "error") return "error";
  if (providerBootstrapState === "idle" || providerBootstrapState === "loading") {
    return "loading";
  }
  return "ready";
}

export async function waitForWorkspaceBootstrapBeforeNavigation(workspaceId: string): Promise<void> {
  await withProviderBootstrapTimeout(loadProviderOnboardingBootstrap(workspaceId));
}
