import { Card, Row } from "../SettingsPage.components";
import { TextInput } from "../../../components/ui/text-input";
import type { CtxBillingUser } from "../teamEnterpriseSettingsApi";

export function BillingSection({
  controlPlaneConfigured,
  checkoutStatus,
  billingUser,
  billingEmail,
  onBillingEmailChange,
  billingPassword,
  onBillingPasswordChange,
  billingBusy,
  billingError,
  entitlementsBusy,
  plan,
  proEnabled,
  onSignIn,
  onSignUp,
  onSignOut,
  onStartCheckout,
  onOpenPortal,
}: {
  controlPlaneConfigured: boolean;
  checkoutStatus: string | null;
  billingUser: CtxBillingUser | null;
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
  if (!controlPlaneConfigured) {
    return (
      <div className="settings-empty">
        Billing is not configured. Set <code>VITE_CTX_CONTROL_PLANE_URL</code> for the web app.
      </div>
    );
  }

  return (
    <>
      {checkoutStatus === "success" ? <div className="settings-banner">Checkout complete. Confirming subscription…</div> : null}
      {checkoutStatus === "cancel" ? <div className="settings-banner settings-banner-error">Checkout canceled.</div> : null}
      <Card title="Account">
        {billingUser ? (
          <Row
            title="Signed in"
            description={billingUser.email ?? "Signed in"}
            control={
              <button type="button" className="settings-btn settings-btn-secondary" onClick={() => void onSignOut()} disabled={billingBusy}>
                Sign out
              </button>
            }
          />
        ) : (
          <>
            <Row
              title="Email"
              control={
                <TextInput
                  className="settings-control settings-control-wide"
                  value={billingEmail}
                  onChange={(event) => onBillingEmailChange(event.target.value)}
                  placeholder="you@company.com"
                />
              }
            />
            <Row
              title="Password"
              control={
                <TextInput
                  className="settings-control settings-control-wide"
                  value={billingPassword}
                  onChange={(event) => onBillingPasswordChange(event.target.value)}
                  type="password"
                  placeholder="••••••••"
                />
              }
            />
            <Row
              title="Sign in / Create account"
              description="Subscriptions are purchased via Stripe on desktop; mobile devices inherit access when connected."
              control={
                <div style={{ display: "flex", gap: 8 }}>
                  <button type="button" className="settings-btn settings-btn-secondary" onClick={() => void onSignIn()} disabled={billingBusy}>
                    Sign in
                  </button>
                  <button type="button" className="settings-btn" onClick={() => void onSignUp()} disabled={billingBusy}>
                    Create account
                  </button>
                </div>
              }
            />
          </>
        )}
      </Card>

      <Card title="Subscription">
        <Row
          title="Plan"
          description={entitlementsBusy ? "Loading…" : proEnabled ? "Pro enabled" : "Free/Local"}
          control={<div className="settings-pill">{plan}</div>}
        />
        <Row
          title="Remote mobile access"
          description="Stable remote access + push notifications are Pro features. Purchase on desktop."
          control={<div className="settings-pill">{proEnabled ? "Enabled" : "Disabled"}</div>}
        />
        <Row
          title="Subscribe"
          description="USD only. CTX Pro is $20/month or $200/year."
          control={
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap", justifyContent: "flex-end" }}>
              <button type="button" className="settings-btn settings-btn-secondary" onClick={() => void onStartCheckout("month")} disabled={!billingUser || billingBusy}>
                $20 / month
              </button>
              <button type="button" className="settings-btn settings-btn-secondary" onClick={() => void onStartCheckout("year")} disabled={!billingUser || billingBusy}>
                $200 / year
              </button>
              <button type="button" className="settings-btn" onClick={() => void onOpenPortal()} disabled={!billingUser || billingBusy}>
                Manage
              </button>
            </div>
          }
        />
      </Card>

      {billingError ? <div className="settings-banner settings-banner-error">{billingError}</div> : null}
    </>
  );
}
