import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MobileAccessSection } from "./MobileAccessSection";

describe("MobileAccessSection", () => {
  it("renders the source-build unavailable state", () => {
    render(
      <MobileAccessSection
        hostedServicesConfigured={false}
        billingUser={null}
        entitlementsBusy={false}
        proEnabled={false}
        mobileStatus={null}
        mobileStatusBusy={false}
        mobileStatusError={null}
        mobileEnableBusy={false}
        mobileEnableError={null}
        mobileQr={null}
        qrFgColor="#000"
        onEnable={vi.fn()}
        onDisable={vi.fn()}
      />,
    );

    expect(screen.getByText(/This settings surface is unavailable in the source build\/local ADE/i)).toBeInTheDocument();
  });
});
