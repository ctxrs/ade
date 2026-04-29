import type { SupabaseClient } from "@supabase/supabase-js";
import type { BillingInterval } from "./billingCheckoutFlow";
import {
  assertFunctionOk,
  parseTeamEnterpriseCloudState,
  readCheckoutUrl,
  readErrorMessage,
} from "./teamEnterpriseSettingsApi.parsers";

export { readCheckoutUrl } from "./teamEnterpriseSettingsApi.parsers";

export type EntitlementFeatureState = "enabled" | "disabled";
export type EntitlementSubjectType = "install" | "account" | "org";
export type MembershipRole = "owner" | "admin" | "member";

export type EntitlementsSnapshot = {
  plan_type: "free_local" | "pro" | "team" | "enterprise";
  subject_type?: EntitlementSubjectType | null;
  account_id?: string | null;
  org_id?: string | null;
  active_org_id?: string | null;
  membership_role?: MembershipRole | null;
  billing_subject?: EntitlementSubjectType | null;
  features: Record<string, EntitlementFeatureState>;
  expires_at?: string | null;
  grace_expires_at?: string | null;
};

export const TEAM_ENTERPRISE_ACTIVE_ORG_STORAGE_KEY = "ctx.settings.teamEnterprise.activeOrgId";

export type TeamEnterpriseOrg = {
  id: string;
  name: string;
  slug: string | null;
  role: MembershipRole;
  status: string;
  createdAt: string | null;
  billingSubjectId: string | null;
  activeMemberCount: number;
  suspendedMemberCount: number;
  pendingInviteCount: number;
  seatCount: number;
  seatsAvailable: number;
  enforceSeatLimit: boolean;
  planType: "free_local" | "pro" | "team" | "enterprise";
  subscriptionStatus: string;
};

export type TeamEnterpriseInvite = {
  id: string;
  email: string;
  role: MembershipRole;
  status: string;
  expiresAt: string | null;
  createdAt: string | null;
  inviteToken?: string | null;
};

export type TeamEnterpriseSubscription = {
  id: string;
  planType: "free_local" | "pro" | "team" | "enterprise";
  status: string;
  currentPeriodEnd: string | null;
  cancelAtPeriodEnd: boolean;
  seatCount: number;
};

export type TeamEnterpriseFeatureGrant = {
  featureKey: string;
  state: "enabled" | "disabled";
  startsAt: string | null;
  expiresAt: string | null;
};

export type TeamEnterpriseCloudState = {
  orgs: TeamEnterpriseOrg[];
  activeOrgId: string | null;
  activeOrg: TeamEnterpriseOrg | null;
  billingSubjectId: string | null;
  invites: TeamEnterpriseInvite[];
  subscriptions: TeamEnterpriseSubscription[];
  featureGrants: TeamEnterpriseFeatureGrant[];
  adminState: TeamEnterpriseAdminState | null;
  memberDirectoryAvailable: boolean;
};

export type TeamEnterprisePolicyDraft = {
  providers: string;
  models: string;
  allowPersonalRoutes: boolean;
  sandboxProfile: "sandbox_required" | "sandbox_preferred";
  networkProfile: "default" | "restricted" | "offline";
  archiveVisibility: "local_only" | "org_summary" | "org_transcript" | "org_evidence";
};

export const DEFAULT_TEAM_ENTERPRISE_POLICY_DRAFT: TeamEnterprisePolicyDraft = {
  providers: "openai, anthropic",
  models: "",
  allowPersonalRoutes: false,
  sandboxProfile: "sandbox_required",
  networkProfile: "default",
  archiveVisibility: "org_summary",
};

export type TeamEnterpriseAdminState = {
  seatTarget: number | null;
  policy: TeamEnterprisePolicyDraft | null;
  enterpriseSetupRequestedAt: string | null;
  updatedAt: string | null;
};

export type TeamEnterpriseAdminAction =
  | { action: "create_organization"; name: string }
  | { action: "invite_member"; organization_id: string; email: string; role: MembershipRole }
  | { action: "accept_invite"; token: string }
  | {
    action: "update_member_role";
    organization_id: string;
    membership_id: string;
    role: MembershipRole;
  }
  | { action: "update_seats"; organization_id: string; seats: number }
  | { action: "update_policy"; organization_id: string; policy: TeamEnterprisePolicyDraft }
  | { action: "request_enterprise_setup"; organization_id: string };

export async function invokeTeamEnterpriseAdminAction(
  client: SupabaseClient,
  action: TeamEnterpriseAdminAction,
): Promise<void> {
  const response = await client.functions.invoke("team-admin", {
    method: "POST",
    body: action,
  });
  if (response.error) {
    throw new Error(readErrorMessage(response.error, "Team/Enterprise admin API is unavailable."));
  }
}

export async function startTeamBillingCheckout(options: {
  client: SupabaseClient;
  organizationId: string;
  billingSubjectId: string;
  interval: BillingInterval;
  returnPath: string;
  seatCount?: number | null;
}): Promise<string> {
  const response = await options.client.functions.invoke("billing-checkout", {
    method: "POST",
    body: {
      interval: options.interval,
      return_path: options.returnPath,
      plan_type: "team",
      organization_id: options.organizationId,
      seat_count: options.seatCount ?? undefined,
    },
  });
  if (response.error) {
    throw new Error(readErrorMessage(response.error, "Team checkout API is unavailable."));
  }
  return readCheckoutUrl(response.data);
}

export async function fetchEntitlementsSnapshot(options: {
  client: SupabaseClient;
  activeOrgId: string | null;
}): Promise<EntitlementsSnapshot | null> {
  const headers = options.activeOrgId ? { "x-ctx-active-org-id": options.activeOrgId } : undefined;
  const response = await options.client.functions.invoke("entitlements", {
    method: "GET",
    headers,
  });
  if (response.error) {
    throw new Error(readErrorMessage(response.error, "Failed to refresh entitlements."));
  }
  return (response.data ?? null) as EntitlementsSnapshot | null;
}

export async function fetchTeamEnterpriseCloudState(options: {
  client: SupabaseClient;
  requestedActiveOrgId: string | null;
}): Promise<TeamEnterpriseCloudState> {
  const response = await options.client.functions.invoke("team-admin", {
    method: "GET",
    headers: options.requestedActiveOrgId
      ? { "x-ctx-active-org-id": options.requestedActiveOrgId }
      : undefined,
  });
  const payload = assertFunctionOk(
    { data: response.data, error: response.error },
    "Failed to load Team/Enterprise organization state.",
  );
  return parseTeamEnterpriseCloudState(payload, options.requestedActiveOrgId);
}
