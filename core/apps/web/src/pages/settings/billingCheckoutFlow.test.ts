import { describe, expect, it, vi } from "vitest";
import { runBillingCheckoutFlow } from "./billingCheckoutFlow";

describe("runBillingCheckoutFlow", () => {
  it("fails explicitly in the source build without invoking checkout", async () => {
    const invokeCheckout = vi.fn();
    const trackSubscribeCtaClicked = vi.fn();
    const trackCheckoutStarted = vi.fn();

    await expect(
      runBillingCheckoutFlow({
        interval: "month",
        returnPath: "/settings#general",
        invokeCheckout,
        trackSubscribeCtaClicked,
        trackCheckoutStarted,
      }),
    ).rejects.toThrow("This settings action is unavailable in the source build/local ADE.");

    expect(invokeCheckout).not.toHaveBeenCalled();
    expect(trackSubscribeCtaClicked).not.toHaveBeenCalled();
    expect(trackCheckoutStarted).not.toHaveBeenCalled();
  });
});
