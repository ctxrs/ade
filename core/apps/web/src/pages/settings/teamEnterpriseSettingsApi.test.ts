import { afterEach, describe, expect, it, vi } from "vitest";
import {
  fetchTeamEnterpriseCloudState,
  invokeTeamEnterpriseAdminAction,
  readCheckoutUrl,
  startTeamBillingCheckout,
} from "./teamEnterpriseSettingsApi";

function jsonResponse(body: unknown, init?: ResponseInit): Response {
  return new Response(JSON.stringify(body), {
    status: init?.status ?? 200,
    headers: { "content-type": "application/json" },
  });
}

describe("teamEnterpriseSettingsApi", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("loads active org state from the ctx control-plane", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({
      active_org_id: "org_1",
      organizations: [
        {
          organization: {
            id: "org_1",
            name: "Acme",
            slug: "acme",
            status: "active",
            createdAt: "2026-04-01T00:00:00Z",
          },
          membership: { role: "owner" },
          billingSubjectId: "bs_1",
          seatState: {
            activeMembers: 3,
            suspendedMembers: 1,
            pendingInvites: 2,
            seatCount: 5,
            seatsAvailable: 2,
            enforceSeatLimit: true,
          },
          billing: {
            planType: "team",
            status: "active",
            seatCount: 5,
            currentPeriodEnd: "2026-05-28T00:00:00Z",
            cancelAtPeriodEnd: false,
          },
        },
      ],
      active_invites: [
        {
          id: "invite_1",
          email: "member@example.com",
          role: "member",
          status: "pending",
          expires_at: "2026-05-12T00:00:00Z",
        },
      ],
      active_admin_state: {
        seat_target: 6,
        policy_json: {
          providers: "openai",
          models: "gpt-5.4",
          allowPersonalRoutes: false,
          sandboxProfile: "sandbox_required",
          networkProfile: "restricted",
          archiveVisibility: "org_summary",
        },
        enterprise_setup_requested_at: null,
        updated_at: "2026-04-28T00:00:00Z",
      },
    }));
    vi.stubGlobal("fetch", fetchMock);

    const state = await fetchTeamEnterpriseCloudState({ requestedActiveOrgId: "org_1" });

    expect(state.activeOrg?.name).toBe("Acme");
    expect(state.activeOrg?.role).toBe("owner");
    expect(state.billingSubjectId).toBe("bs_1");
    expect(state.invites).toHaveLength(1);
    expect(state.subscriptions[0]?.planType).toBe("team");
    expect(state.activeOrg?.seatCount).toBe(5);
    expect(state.adminState?.seatTarget).toBe(6);
    expect(state.adminState?.policy?.providers).toBe("openai");
    expect(fetchMock).toHaveBeenCalledWith("https://api.ctx.rs/v1/team/state", expect.objectContaining({
      credentials: "include",
      headers: expect.objectContaining({ "x-ctx-active-org-id": "org_1" }),
      method: "GET",
    }));
  });

  it("falls back to the first visible organization when the server returns no active org", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => jsonResponse({
      active_org_id: null,
      organizations: [
        {
          organization: { id: "org_1", name: "Fallback", slug: null, status: "active", createdAt: null },
          membership: { role: "admin" },
          billingSubjectId: "bs_1",
          seatState: {},
          billing: null,
        },
      ],
    })));

    const state = await fetchTeamEnterpriseCloudState({ requestedActiveOrgId: "org_missing" });

    expect(state.activeOrgId).toBe("org_1");
    expect(state.activeOrg?.role).toBe("admin");
  });

  it("starts Team checkout through the ctx control-plane with a team plan body", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ url: "https://checkout.example/team" }));
    vi.stubGlobal("fetch", fetchMock);

    const url = await startTeamBillingCheckout({
      organizationId: "org_1",
      billingSubjectId: "bs_1",
      interval: "month",
      returnPath: "/settings#team_enterprise",
      seatCount: 4,
    });

    expect(url).toBe("https://checkout.example/team");
    expect(fetchMock).toHaveBeenCalledWith("https://api.ctx.rs/v1/billing/checkout", expect.objectContaining({
      body: JSON.stringify({
        interval: "month",
        return_path: "/settings#team_enterprise",
        plan_type: "team",
        organization_id: "org_1",
        billing_subject_id: "bs_1",
        seat_count: 4,
      }),
      method: "POST",
    }));
  });

  it("posts admin actions to the ctx control-plane", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);

    await invokeTeamEnterpriseAdminAction({ action: "create_organization", name: "Acme" });

    expect(fetchMock).toHaveBeenCalledWith("https://api.ctx.rs/v1/team/admin", expect.objectContaining({
      body: JSON.stringify({ action: "create_organization", name: "Acme" }),
      method: "POST",
    }));
  });

  it("surfaces ctx control-plane error messages", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => jsonResponse({ message: "Team is prelaunch." }, { status: 409 })));

    await expect(invokeTeamEnterpriseAdminAction({ action: "create_organization", name: "Acme" })).rejects.toThrow("Team is prelaunch.");
  });

  it("rejects checkout responses without a URL", () => {
    expect(() => readCheckoutUrl({})).toThrow("Checkout URL missing.");
  });
});
