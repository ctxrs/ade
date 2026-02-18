import { describe, expect, it, vi } from "vitest";
import { runBillingCheckoutFlow } from "./billingCheckoutFlow";

describe("runBillingCheckoutFlow", () => {
  it("tracks checkout_started only after checkout url is present", async () => {
    const invokeCheckout = vi.fn().mockResolvedValue({
      error: null,
      data: { url: " https://checkout.example/session " },
    });
    const trackSubscribeCtaClicked = vi.fn();
    const trackCheckoutStarted = vi.fn();

    const url = await runBillingCheckoutFlow({
      interval: "month",
      returnPath: "/settings#billing",
      invokeCheckout,
      trackSubscribeCtaClicked,
      trackCheckoutStarted,
    });

    expect(url).toBe("https://checkout.example/session");
    expect(trackSubscribeCtaClicked).toHaveBeenCalledTimes(1);
    expect(trackCheckoutStarted).toHaveBeenCalledTimes(1);
  });

  it("does not track checkout_started when checkout invocation fails", async () => {
    const invokeCheckout = vi.fn().mockResolvedValue({
      error: { message: "network" },
      data: null,
    });
    const trackSubscribeCtaClicked = vi.fn();
    const trackCheckoutStarted = vi.fn();

    await expect(
      runBillingCheckoutFlow({
        interval: "year",
        returnPath: "/settings#billing",
        invokeCheckout,
        trackSubscribeCtaClicked,
        trackCheckoutStarted,
      }),
    ).rejects.toThrow("network");
    expect(trackSubscribeCtaClicked).toHaveBeenCalledTimes(1);
    expect(trackCheckoutStarted).not.toHaveBeenCalled();
  });

  it("does not track checkout_started when url is missing", async () => {
    const invokeCheckout = vi.fn().mockResolvedValue({
      error: null,
      data: {},
    });
    const trackSubscribeCtaClicked = vi.fn();
    const trackCheckoutStarted = vi.fn();

    await expect(
      runBillingCheckoutFlow({
        interval: "month",
        returnPath: "/settings#billing",
        invokeCheckout,
        trackSubscribeCtaClicked,
        trackCheckoutStarted,
      }),
    ).rejects.toThrow("Checkout URL missing.");
    expect(trackSubscribeCtaClicked).toHaveBeenCalledTimes(1);
    expect(trackCheckoutStarted).not.toHaveBeenCalled();
  });
});
