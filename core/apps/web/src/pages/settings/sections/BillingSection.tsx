export function BillingSection(_props: {
  hostedServicesConfigured: boolean;
  checkoutStatus: string | null;
  billingUser: unknown;
  billingEmail: string;
  onBillingEmailChange: (value: string) => void;
  billingPassword: string;
  onBillingPasswordChange: (value: string) => void;
  billingBusy: boolean;
  billingError: string | null;
  entitlementsBusy: boolean;
  plan: "free_local" | "pro" | "team" | "enterprise";
  proEnabled: boolean;
  onSignIn: () => void | Promise<void>;
  onSignUp: () => void | Promise<void>;
  onSignOut: () => void | Promise<void>;
  onStartCheckout: (interval: "month" | "year") => void | Promise<void>;
  onOpenPortal: () => void | Promise<void>;
}) {
  return (
    <div className="settings-empty">
      This settings surface is unavailable in the source build/local ADE.
    </div>
  );
}
