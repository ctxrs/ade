import { fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import type { User } from "@supabase/supabase-js";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { MobileAccessSection } from "./MobileAccessSection";

const billingUser = {
  id: "user-1",
  email: "user@example.com",
} as User;

const renderSection = (
  overrides: Partial<ComponentProps<typeof MobileAccessSection>> = {},
) => {
  const props: ComponentProps<typeof MobileAccessSection> = {
    supabaseConfigured: true,
    billingUser,
    entitlementsBusy: false,
    proEnabled: true,
    mobileStatus: {
      enabled: false,
      tunnel_state: "idle",
    },
    mobileStatusBusy: false,
    mobileStatusError: null,
    mobileEnableBusy: false,
    mobileEnableError: null,
    mobileQr: null,
    qrFgColor: "#ffffff",
    onEnable: vi.fn(),
    onDisable: vi.fn(),
    ...overrides,
  };
  render(
    <MemoryRouter>
      <MobileAccessSection {...props} />
    </MemoryRouter>,
  );
  return props;
};

describe("MobileAccessSection", () => {
  it("renders production QR scan instructions without exposing tunnel internals", () => {
    renderSection({
      mobileStatus: {
        enabled: true,
        tunnel_state: "running",
        public_base_url: "https://tunnel.ctx.rs/t/tunnel-1",
        tunnel_id: "tunnel-1",
      },
      mobileQr: {
        status: {
          enabled: true,
          tunnel_state: "running",
        },
        qr_payload: {
          type: "context_mobile_e2ee",
          base_url: "https://tunnel.ctx.rs/t/tunnel-1",
        },
        pairing_expires_at: "2026-04-30T12:00:00Z",
      },
    });

    expect(screen.getByText("Scan From ctx Mobile")).toBeInTheDocument();
    expect(screen.getByText(/Open ctx mobile and tap Scan QR/)).toBeInTheDocument();
    expect(screen.getByText(/manual paste fallback/)).toBeInTheDocument();
    expect(screen.queryByText("Public URL")).not.toBeInTheDocument();
    expect(screen.queryByText("Tunnel ID")).not.toBeInTheDocument();
    expect(screen.queryByText("https://tunnel.ctx.rs/t/tunnel-1")).not.toBeInTheDocument();
  });

  it("disables enable until the user is signed in and entitled", () => {
    renderSection({
      billingUser: null,
      proEnabled: false,
    });

    expect(screen.getByRole("button", { name: "Enable" })).toBeDisabled();
    expect(screen.getByText("Remote mobile access is a Pro feature.")).toBeInTheDocument();
  });

  it("surfaces disable when mobile access is enabled", () => {
    const onDisable = vi.fn();
    renderSection({
      onDisable,
      mobileStatus: {
        enabled: true,
        tunnel_state: "running",
      },
    });

    fireEvent.click(screen.getByRole("button", { name: "Disable" }));
    expect(onDisable).toHaveBeenCalledTimes(1);
  });
});
