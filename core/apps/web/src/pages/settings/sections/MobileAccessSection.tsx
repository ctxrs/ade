import type { EnableMobileAccessResponse, MobileAccessStatus } from "../../../api/client";

export function MobileAccessSection(_props: {
  hostedServicesConfigured: boolean;
  billingUser: unknown;
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
  return (
    <div className="settings-empty">
      This settings surface is unavailable in the source build/local ADE.
    </div>
  );
}
