import { Link } from "react-router-dom";
import type { User } from "@supabase/supabase-js";
import { QRCodeSVG } from "qrcode.react";
import type { EnableMobileAccessResponse, MobileAccessStatus } from "../../../api/client";
import { Card, Row } from "../../SettingsPage.components";

export function MobileAccessSection({
  supabaseConfigured,
  billingUser,
  entitlementsBusy,
  proEnabled,
  mobileStatus,
  mobileStatusBusy,
  mobileStatusError,
  mobileEnableBusy,
  mobileEnableError,
  mobileQr,
  qrFgColor,
  onEnable,
  onDisable,
}: {
  supabaseConfigured: boolean;
  billingUser: User | null;
  entitlementsBusy: boolean;
  proEnabled: boolean;
  mobileStatus: MobileAccessStatus | null;
  mobileStatusBusy: boolean;
  mobileStatusError: string | null;
  mobileEnableBusy: boolean;
  mobileEnableError: string | null;
  mobileQr: EnableMobileAccessResponse | null;
  qrFgColor: string;
  onEnable: () => void | Promise<void>;
  onDisable: () => void | Promise<void>;
}) {
  if (!supabaseConfigured) {
    return (
      <div className="settings-empty">
        Mobile access requires Supabase config. Set <code>VITE_SUPABASE_URL</code> and <code>VITE_SUPABASE_ANON_KEY</code>.
      </div>
    );
  }

  const statusLabel = mobileStatusBusy ? "Loading" : mobileStatus?.enabled ? "Enabled" : "Disabled";
  const tunnelState = mobileStatus?.tunnel_state ?? "idle";

  return (
    <>
      <Card title="Remote Mobile Access">
        <Row
          title="Entitlement"
          description={entitlementsBusy ? "Loading…" : proEnabled ? "Pro enabled" : "Pro required"}
          control={<div className="settings-pill">{proEnabled ? "Enabled" : "Disabled"}</div>}
        />
        <Row
          title="Status"
          description="Mobile access tunnel status on this daemon."
          control={<div className="settings-pill">{statusLabel}</div>}
        />
        <Row
          title="Tunnel state"
          description={mobileStatus?.last_error ?? "Router tunnel lifecycle state."}
          control={<div className="settings-pill">{tunnelState}</div>}
        />
        {mobileStatus?.public_base_url ? (
          <Row title="Public URL" control={<span className="settings-pill wb-mono">{mobileStatus.public_base_url}</span>} />
        ) : null}
        {mobileStatus?.tunnel_id ? (
          <Row title="Tunnel ID" control={<span className="settings-pill wb-mono">{mobileStatus.tunnel_id}</span>} />
        ) : null}
        <Row
          title="Actions"
          description={!billingUser ? "Sign in to enable or revoke mobile access." : "Manage remote tunnel access."}
          control={
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap", justifyContent: "flex-end" }}>
              <button
                type="button"
                className="settings-btn settings-btn-secondary"
                onClick={() => void onEnable()}
                disabled={!billingUser || !proEnabled || mobileEnableBusy}
              >
                {mobileStatus?.enabled ? "Show QR" : "Enable"}
              </button>
              {mobileStatus?.enabled ? (
                <button
                  type="button"
                  className="settings-btn"
                  onClick={() => void onDisable()}
                  disabled={!billingUser || mobileEnableBusy}
                >
                  Disable
                </button>
              ) : null}
            </div>
          }
        />
        {!proEnabled ? (
          <Row
            title="Upgrade"
            description="Remote mobile access is a Pro feature."
            control={<Link to="#billing">Go to billing</Link>}
          />
        ) : null}
      </Card>

      {mobileStatusBusy ? <div className="settings-banner">Loading mobile access status…</div> : null}
      {mobileStatusError ? <div className="settings-banner settings-banner-error">{mobileStatusError}</div> : null}
      {mobileEnableError ? <div className="settings-banner settings-banner-error">{mobileEnableError}</div> : null}

      {mobileQr ? (
        <Card title="Pair a mobile device">
          <div className="settings-card-block" style={{ display: "flex", gap: 24, alignItems: "center", flexWrap: "wrap" }}>
            <QRCodeSVG value={JSON.stringify(mobileQr.qr_payload)} size={220} bgColor="transparent" fgColor={qrFgColor} />
            <div style={{ minWidth: 240 }}>
              <div className="settings-row-title" style={{ marginBottom: 6 }}>Scan with ctx mobile</div>
              <div className="settings-row-desc">
                This QR code pairs a device using end-to-end encryption. It expires at {new Date(mobileQr.pairing_expires_at).toLocaleString()}.
              </div>
            </div>
          </div>
        </Card>
      ) : null}
    </>
  );
}
