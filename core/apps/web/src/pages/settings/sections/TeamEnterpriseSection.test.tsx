import type { ComponentProps } from "react";
import type { User } from "@supabase/supabase-js";
import { fireEvent, render, screen } from "@testing-library/react";
import { vi } from "vitest";
import { TeamEnterpriseSection } from "./TeamEnterpriseSection";
import type {
  EntitlementsSnapshot,
  TeamEnterpriseCloudState,
  TeamEnterprisePolicyDraft,
} from "../teamEnterpriseSettingsApi";
import { DEFAULT_TEAM_ENTERPRISE_POLICY_DRAFT } from "../teamEnterpriseSettingsApi";

const emptyCloudState: TeamEnterpriseCloudState = {
  orgs: [],
  activeOrgId: null,
  activeOrg: null,
  billingSubjectId: null,
  invites: [],
  subscriptions: [],
  featureGrants: [],
  adminState: null,
  memberDirectoryAvailable: false,
};

function renderSection(overrides: Partial<ComponentProps<typeof TeamEnterpriseSection>> = {}) {
  const props: ComponentProps<typeof TeamEnterpriseSection> = {
    supabaseConfigured: true,
    billingUser: { email: "owner@example.com" } as User,
    entitlementsBusy: false,
    plan: "team",
    entitlements: {
      plan_type: "team",
      subject_type: "org",
      billing_subject: "org",
      account_id: "acct_123",
      org_id: "org_456",
      active_org_id: "org_456",
      membership_role: "owner",
      features: {
        org_admin: "enabled",
        org_policy: "enabled",
        org_run_history: "enabled",
        org_audit: "disabled",
        mobile_relay: "disabled",
        llm_token_relay: "disabled",
      },
    },
    cloudState: {
      orgs: [
        {
          id: "org_456",
          name: "Acme Engineering",
          slug: "acme-eng",
          role: "owner",
          status: "active",
          createdAt: "2026-04-01T00:00:00Z",
          billingSubjectId: "bs_org_456",
          activeMemberCount: 2,
          suspendedMemberCount: 0,
          pendingInviteCount: 1,
          seatCount: 5,
          seatsAvailable: 3,
          enforceSeatLimit: true,
          planType: "team",
          subscriptionStatus: "active",
        },
      ],
      activeOrgId: "org_456",
      activeOrg: {
        id: "org_456",
        name: "Acme Engineering",
        slug: "acme-eng",
        role: "owner",
        status: "active",
        createdAt: "2026-04-01T00:00:00Z",
        billingSubjectId: "bs_org_456",
        activeMemberCount: 2,
        suspendedMemberCount: 0,
        pendingInviteCount: 1,
        seatCount: 5,
        seatsAvailable: 3,
        enforceSeatLimit: true,
        planType: "team",
        subscriptionStatus: "active",
      },
      billingSubjectId: "bs_org_456",
      invites: [
        {
          id: "invite_1",
          email: "member@example.com",
          role: "member",
          status: "pending",
          expiresAt: "2026-05-01T00:00:00Z",
          createdAt: "2026-04-28T00:00:00Z",
        },
      ],
      subscriptions: [
        {
          id: "sub_1",
          planType: "team",
          status: "active",
          currentPeriodEnd: "2026-05-28T00:00:00Z",
          cancelAtPeriodEnd: false,
          seatCount: 5,
        },
      ],
      featureGrants: [{ featureKey: "org_policy", state: "enabled", startsAt: null, expiresAt: null }],
      adminState: null,
      memberDirectoryAvailable: false,
    },
    cloudBusy: false,
    cloudError: null,
    actionBusy: false,
    actionError: null,
    actionNotice: null,
    orgName: "",
    onOrgNameChange: vi.fn(),
    inviteEmail: "",
    onInviteEmailChange: vi.fn(),
    inviteRole: "member",
    onInviteRoleChange: vi.fn(),
    seatTarget: "",
    onSeatTargetChange: vi.fn(),
    policyDraft: DEFAULT_TEAM_ENTERPRISE_POLICY_DRAFT,
    onPolicyDraftChange: vi.fn(),
    onRefresh: vi.fn(),
    onSelectOrg: vi.fn(),
    onCreateOrg: vi.fn(),
    onInviteMember: vi.fn(),
    onAcceptInvite: vi.fn(),
    onUpdateSeats: vi.fn(),
    onSavePolicy: vi.fn(),
    onStartTeamCheckout: vi.fn(),
    onRequestEnterpriseSetup: vi.fn(),
    ...overrides,
  };
  render(<TeamEnterpriseSection {...props} />);
  return props;
}

describe("TeamEnterpriseSection", () => {
  it("renders explicit unavailable states when org wiring is missing", () => {
    renderSection({
      supabaseConfigured: false,
      billingUser: null,
      plan: "free_local",
      entitlements: null,
      cloudState: emptyCloudState,
    });

    expect(screen.getByText(/Billing\/auth is not configured/i)).toBeInTheDocument();
    expect(screen.getByText("Account & Plans")).toBeInTheDocument();
    expect(screen.getByText("Organization")).toBeInTheDocument();
    expect(screen.getByText("Members & Seats")).toBeInTheDocument();
    expect(screen.getByText("Policy")).toBeInTheDocument();
    expect(screen.getByText("Enterprise setup")).toBeInTheDocument();
    expect(screen.getAllByText("Unavailable").length).toBeGreaterThan(0);
  });

  it("shows org state, billing subject checkout controls, and managed-service hooks", () => {
    renderSection();

    expect(screen.getByText("Team")).toBeInTheDocument();
    expect(screen.getByText("owner@example.com")).toBeInTheDocument();
    expect(screen.getAllByText("Acme Engineering").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Owner").length).toBeGreaterThan(0);
    expect(screen.getByText(/member@example.com/i)).toBeInTheDocument();
    expect(screen.getByText("Mobile relay")).toBeInTheDocument();
    expect(screen.getByText("LLM token relay")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Monthly" })).toBeEnabled();
  });

  it("wires form controls to the controller callbacks", () => {
    const policyDraft: TeamEnterprisePolicyDraft = {
      ...DEFAULT_TEAM_ENTERPRISE_POLICY_DRAFT,
      providers: "openai",
    };
    const props = renderSection({
      orgName: "Platform",
      inviteEmail: "new@example.com",
      seatTarget: "8",
      policyDraft,
    });

    fireEvent.change(screen.getByLabelText("Organization name"), { target: { value: "Infra" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    fireEvent.change(screen.getByLabelText("Invite email"), { target: { value: "dev@example.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Invite" }));
    fireEvent.change(screen.getByLabelText("Invite token"), { target: { value: "tok_123" } });
    fireEvent.click(screen.getByRole("button", { name: "Accept" }));
    fireEvent.change(screen.getByLabelText("Seat count"), { target: { value: "12" } });
    fireEvent.click(screen.getByRole("button", { name: "Update" }));
    fireEvent.change(screen.getByLabelText("Provider allowlist"), { target: { value: "openai, anthropic" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    fireEvent.click(screen.getByRole("button", { name: "Monthly" }));

    expect(props.onOrgNameChange).toHaveBeenCalledWith("Infra");
    expect(props.onCreateOrg).toHaveBeenCalledTimes(1);
    expect(props.onInviteEmailChange).toHaveBeenCalledWith("dev@example.com");
    expect(props.onInviteMember).toHaveBeenCalledTimes(1);
    expect(props.onAcceptInvite).toHaveBeenCalledWith("tok_123");
    expect(props.onSeatTargetChange).toHaveBeenCalledWith("12");
    expect(props.onUpdateSeats).toHaveBeenCalledTimes(1);
    expect(props.onPolicyDraftChange).toHaveBeenCalledWith({ ...policyDraft, providers: "openai, anthropic" });
    expect(props.onSavePolicy).toHaveBeenCalledTimes(1);
    expect(props.onStartTeamCheckout).toHaveBeenCalledWith("month");
  });

  it("keeps admin mutations disabled for signed-in non-admin members", () => {
    const entitlements: EntitlementsSnapshot = {
      plan_type: "team",
      subject_type: "org",
      billing_subject: "org",
      membership_role: "member",
      features: {
        org_admin: "disabled",
        org_policy: "disabled",
      },
    };
    renderSection({
      entitlements,
      cloudState: {
        ...emptyCloudState,
        orgs: [{
          id: "org_456",
          name: "Member Org",
          slug: null,
          role: "member",
          status: "active",
          createdAt: null,
          billingSubjectId: "bs_org_456",
          activeMemberCount: 1,
          suspendedMemberCount: 0,
          pendingInviteCount: 0,
          seatCount: 1,
          seatsAvailable: 0,
          enforceSeatLimit: false,
          planType: "free_local",
          subscriptionStatus: "none",
        }],
        activeOrgId: "org_456",
        activeOrg: {
          id: "org_456",
          name: "Member Org",
          slug: null,
          role: "member",
          status: "active",
          createdAt: null,
          billingSubjectId: "bs_org_456",
          activeMemberCount: 1,
          suspendedMemberCount: 0,
          pendingInviteCount: 0,
          seatCount: 1,
          seatsAvailable: 0,
          enforceSeatLimit: false,
          planType: "free_local",
          subscriptionStatus: "none",
        },
        billingSubjectId: "bs_org_456",
      },
    });

    expect(screen.getByRole("button", { name: "Invite" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Monthly" })).toBeDisabled();
  });
});
