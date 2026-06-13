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

const HOSTED_BILLING_UNAVAILABLE =
  "This settings action is unavailable in the source build/local ADE.";

export const runBillingCheckoutFlow = async (
  options: BillingCheckoutFlowOptions,
): Promise<string> => {
  void options;
  throw new Error(HOSTED_BILLING_UNAVAILABLE);
};
