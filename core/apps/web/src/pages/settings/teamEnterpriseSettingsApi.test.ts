import type { SupabaseClient } from "@supabase/supabase-js";
import { describe, expect, it, vi } from "vitest";
import {
  fetchTeamEnterpriseCloudState,
  invokeTeamEnterpriseAdminAction,
  readCheckoutUrl,
  startTeamBillingCheckout,
} from "./teamEnterpriseSettingsApi";

function makeClient(invoke: ReturnType<typeof vi.fn>) {
  return {
    functions: { invoke },
  } as unknown as SupabaseClient;
}

describe("teamEnterpriseSettingsApi", () => {
  it("loads active org state from the team-admin function", async () => {
    const invoke = vi.fn(async () => ({
      error: null,
      data: {
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
      },
    }));

    const state = await fetchTeamEnterpriseCloudState({
      client: makeClient(invoke),
      requestedActiveOrgId: "org_1",
    });

    expect(state.activeOrg?.name).toBe("Acme");
    expect(state.activeOrg?.role).toBe("owner");
    expect(state.billingSubjectId).toBe("bs_1");
    expect(state.invites).toHaveLength(1);
    expect(state.subscriptions[0]?.planType).toBe("team");
    expect(state.activeOrg?.seatCount).toBe(5);
    expect(state.adminState?.seatTarget).toBe(6);
    expect(state.adminState?.policy?.providers).toBe("openai");
    expect(invoke).toHaveBeenCalledWith("team-admin", {
      method: "GET",
      headers: { "x-ctx-active-org-id": "org_1" },
    });
  });

  it("falls back to the first visible organization when the server returns no active org", async () => {
    const invoke = vi.fn(async () => ({
      error: null,
      data: {
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
      },
    }));

    const state = await fetchTeamEnterpriseCloudState({
      client: makeClient(invoke),
      requestedActiveOrgId: "org_missing",
    });

    expect(state.activeOrgId).toBe("org_1");
    expect(state.activeOrg?.role).toBe("admin");
  });

  it("starts Team checkout through billing-checkout with a team plan body", async () => {
    const invoke = vi.fn(async () => ({ data: { url: "https://checkout.example/team" }, error: null }));
    const client = makeClient(invoke);
    const url = await startTeamBillingCheckout({
      client,
      organizationId: "org_1",
      billingSubjectId: "bs_1",
      interval: "month",
      returnPath: "/settings#team_enterprise",
      seatCount: 4,
    });

    expect(url).toBe("https://checkout.example/team");
    expect(client.functions.invoke).toHaveBeenCalledWith("billing-checkout", {
      method: "POST",
      body: {
        interval: "month",
        return_path: "/settings#team_enterprise",
        plan_type: "team",
        organization_id: "org_1",
        seat_count: 4,
      },
    });
  });

  it("posts admin actions to team-admin", async () => {
    const invoke = vi.fn(async () => ({ data: {}, error: null }));
    await invokeTeamEnterpriseAdminAction(makeClient(invoke), {
      action: "create_organization",
      name: "Acme",
    });

    expect(invoke).toHaveBeenCalledWith("team-admin", {
      method: "POST",
      body: { action: "create_organization", name: "Acme" },
    });
  });

  it("rejects checkout responses without a URL", () => {
    expect(() => readCheckoutUrl({})).toThrow("Checkout URL missing.");
  });
});
