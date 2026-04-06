import { describe, expect, it, vi } from "vitest";
import { loadProviderOnboardingBootstrap } from "../state/providerOnboardingCoordinator";
import {
  getProviderBootstrapTimeoutMessage,
  PROVIDER_BOOTSTRAP_TIMEOUT_MS,
} from "../utils/providerBootstrapTimeout";
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

  it("fails workspace navigation with an explicit timeout when provider bootstrap stalls", async () => {
    vi.useFakeTimers();
    try {
      vi.mocked(loadProviderOnboardingBootstrap).mockImplementation(
        () => new Promise(() => {}) as Promise<never>,
      );

      const waitPromise = waitForWorkspaceBootstrapBeforeNavigation("ws-hung");
      await vi.advanceTimersByTimeAsync(PROVIDER_BOOTSTRAP_TIMEOUT_MS);

      await expect(waitPromise).rejects.toThrow(getProviderBootstrapTimeoutMessage());
      expect(loadProviderOnboardingBootstrap).toHaveBeenCalledWith("ws-hung");
    } finally {
      vi.useRealTimers();
    }
  });
});
