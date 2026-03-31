import { describe, expect, it, vi } from "vitest";
import { loadProviderOnboardingBootstrap } from "../state/providerOnboardingCoordinator";
import {
  resolveWorkspaceBootstrapGateState,
  waitForWorkspaceBootstrapBeforeNavigation,
} from "./workspaceBootstrapGate";

vi.mock("../state/providerOnboardingCoordinator", () => ({
  loadProviderOnboardingBootstrap: vi.fn(),
}));

describe("workspaceBootstrapGate", () => {
  it("keeps the route gated until the workbench and provider bootstrap are both ready", () => {
    expect(
      resolveWorkspaceBootstrapGateState({
        workbenchHydrated: false,
        providerBootstrapState: "ready",
      }),
    ).toBe("loading");
    expect(
      resolveWorkspaceBootstrapGateState({
        workbenchHydrated: true,
        providerBootstrapState: "idle",
      }),
    ).toBe("loading");
    expect(
      resolveWorkspaceBootstrapGateState({
        workbenchHydrated: true,
        providerBootstrapState: "loading",
      }),
    ).toBe("loading");
    expect(
      resolveWorkspaceBootstrapGateState({
        workbenchHydrated: true,
        providerBootstrapState: "ready",
      }),
    ).toBe("ready");
  });

  it("surfaces an explicit error state once the provider bootstrap fails", () => {
    expect(
      resolveWorkspaceBootstrapGateState({
        workbenchHydrated: true,
        providerBootstrapState: "error",
      }),
    ).toBe("error");
  });

  it("waits for provider bootstrap before workbench navigation", async () => {
    vi.mocked(loadProviderOnboardingBootstrap).mockResolvedValue({} as never);

    await waitForWorkspaceBootstrapBeforeNavigation("ws-ready");

    expect(loadProviderOnboardingBootstrap).toHaveBeenCalledWith("ws-ready");
  });
});
