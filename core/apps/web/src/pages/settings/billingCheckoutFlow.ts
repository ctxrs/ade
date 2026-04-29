export type BillingInterval = "month" | "year";

export type BillingCheckoutInvokeResult = {
  error: unknown;
  data: unknown;
};

export type BillingCheckoutInvoke = (args: {
  interval: BillingInterval;
  returnPath: string;
  planType?: "pro" | "team";
  organizationId?: string;
}) => Promise<BillingCheckoutInvokeResult>;

type BillingCheckoutFlowOptions = {
  interval: BillingInterval;
  returnPath: string;
  planType?: "pro" | "team";
  organizationId?: string;
  invokeCheckout: BillingCheckoutInvoke;
  trackSubscribeCtaClicked: (interval: BillingInterval) => void;
  trackCheckoutStarted: (interval: BillingInterval) => void;
};

const readErrorMessage = (error: unknown): string => {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  if (error && typeof error === "object") {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.trim()) return message;
  }
  return String(error ?? "Unknown checkout error");
};

const readCheckoutUrl = (data: unknown): string => {
  if (!data || typeof data !== "object") return "";
  const url = (data as { url?: unknown }).url;
  return typeof url === "string" ? url.trim() : "";
};

export const runBillingCheckoutFlow = async (
  options: BillingCheckoutFlowOptions,
): Promise<string> => {
  options.trackSubscribeCtaClicked(options.interval);
  const result = await options.invokeCheckout({
    interval: options.interval,
    returnPath: options.returnPath,
    planType: options.planType,
    organizationId: options.organizationId,
  });
  if (result.error) {
    throw new Error(readErrorMessage(result.error));
  }
  const url = readCheckoutUrl(result.data);
  if (!url) {
    throw new Error("Checkout URL missing.");
  }
  options.trackCheckoutStarted(options.interval);
  return url;
};
