import type { BillingCheckoutInvokeResult, BillingInterval } from "./billingCheckoutFlow";
import {
  parseTeamEnterpriseCloudState,
  readCheckoutUrl,
  readErrorMessage,
} from "./teamEnterpriseSettingsApi.parsers";

export { readCheckoutUrl } from "./teamEnterpriseSettingsApi.parsers";

export type CtxBillingUser = {
  id: string;
  email: string | null;
  displayName?: string | null;
};

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

type ControlPlaneRequestOptions = {
  body?: unknown;
  headers?: Record<string, string>;
  method?: "GET" | "POST";
};

const DEFAULT_CONTROL_PLANE_BASE_URL = "https://api.ctx.rs";

export function getCtxControlPlaneBaseUrl(): string {
  const configured = String(import.meta.env.VITE_CTX_CONTROL_PLANE_URL ?? DEFAULT_CONTROL_PLANE_BASE_URL).trim();
  return configured.replace(/\/+$/, "");
}

export function isCtxControlPlaneConfigured(): boolean {
  return getCtxControlPlaneBaseUrl().length > 0;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

async function requestControlPlane(path: string, fallback: string, options: ControlPlaneRequestOptions = {}): Promise<unknown> {
  const headers: Record<string, string> = {
    accept: "application/json",
    ...(options.headers ?? {}),
  };
  const init: RequestInit = {
    credentials: "include",
    method: options.method ?? "GET",
    headers,
  };
  if (options.body !== undefined) {
    headers["content-type"] = "application/json";
    init.body = JSON.stringify(options.body);
  }
  const response = await fetch(`${getCtxControlPlaneBaseUrl()}${path}`, init);
  const text = await response.text();
  let data: unknown = null;
  if (text.trim()) {
    try {
      data = JSON.parse(text);
    } catch {
      throw new Error(`${fallback} (${response.status})`);
    }
  }
  if (!response.ok) {
    throw new Error(readErrorMessage(asRecord(data), fallback));
  }
  return data;
}

function parseSessionUser(data: unknown): CtxBillingUser | null {
  const user = asRecord(asRecord(data)?.user);
  const id = typeof user?.id === "string" && user.id.trim() ? user.id : null;
  if (!id) return null;
  const email = typeof user?.email === "string" && user.email.trim() ? user.email : null;
  const displayName = typeof user?.displayName === "string" && user.displayName.trim()
    ? user.displayName
    : null;
  return { id, email, displayName };
}

export async function fetchAccountSession(): Promise<CtxBillingUser | null> {
  return parseSessionUser(await requestControlPlane("/v1/session", "Failed to load account session."));
}

export async function startAccountAuth(email: string, password: string, mode: "sign_in" | "sign_up"): Promise<void> {
  await requestControlPlane("/v1/auth/start", "ctx account auth is unavailable in this deployment.", {
    method: "POST",
    body: { email, password, mode },
  });
}

export async function signOutAccount(): Promise<void> {
  await requestControlPlane("/v1/auth/logout", "Failed to sign out.", { method: "POST" });
}

export async function syncBillingCheckout(checkoutSessionId: string | null): Promise<void> {
  await requestControlPlane("/v1/billing/sync", "Failed to sync billing checkout.", {
    method: "POST",
    body: checkoutSessionId ? { checkout_session_id: checkoutSessionId } : {},
  });
}

export async function invokePersonalBillingCheckout(args: {
  interval: BillingInterval;
  returnPath: string;
}): Promise<BillingCheckoutInvokeResult> {
  try {
    const data = await requestControlPlane("/v1/billing/checkout", "Billing checkout is unavailable in this deployment.", {
      method: "POST",
      body: {
        interval: args.interval,
        return_path: args.returnPath,
        plan_type: "pro",
      },
    });
    return { data, error: null };
  } catch (error: unknown) {
    return { data: null, error };
  }
}

export async function openBillingPortal(returnPath: string): Promise<string> {
  const data = await requestControlPlane("/v1/billing/portal", "Billing portal is unavailable in this deployment.", {
    method: "POST",
    body: { return_path: returnPath },
  });
  return readCheckoutUrl(data);
}

export async function invokeTeamEnterpriseAdminAction(action: TeamEnterpriseAdminAction): Promise<void> {
  await requestControlPlane("/v1/team/admin", "Team/Enterprise admin API is unavailable.", {
    method: "POST",
    body: action,
  });
}

export async function startTeamBillingCheckout(options: {
  organizationId: string;
  billingSubjectId: string;
  interval: BillingInterval;
  returnPath: string;
  seatCount?: number | null;
}): Promise<string> {
  const data = await requestControlPlane("/v1/billing/checkout", "Team checkout API is unavailable.", {
    method: "POST",
    body: {
      interval: options.interval,
      return_path: options.returnPath,
      plan_type: "team",
      organization_id: options.organizationId,
      billing_subject_id: options.billingSubjectId,
      seat_count: options.seatCount ?? undefined,
    },
  });
  return readCheckoutUrl(data);
}

export async function fetchEntitlementsSnapshot(options: {
  activeOrgId: string | null;
}): Promise<EntitlementsSnapshot | null> {
  const headers = options.activeOrgId ? { "x-ctx-active-org-id": options.activeOrgId } : undefined;
  return await requestControlPlane("/v1/entitlements", "Failed to refresh entitlements.", { headers }) as EntitlementsSnapshot | null;
}

export async function requestManagedTunnelGrant(): Promise<string> {
  const data = await requestControlPlane("/v1/mobile/tunnel-grant", "Managed mobile tunnel grants are not enabled yet.", {
    method: "POST",
  });
  const grant = asRecord(data)?.managed_tunnel_grant;
  if (typeof grant !== "string" || !grant.trim().startsWith("ctmt_")) {
    throw new Error("Control plane returned an invalid managed mobile tunnel grant.");
  }
  return grant.trim();
}

export async function fetchTeamEnterpriseCloudState(options: {
  requestedActiveOrgId: string | null;
}): Promise<TeamEnterpriseCloudState> {
  const headers = options.requestedActiveOrgId
    ? { "x-ctx-active-org-id": options.requestedActiveOrgId }
    : undefined;
  const payload = await requestControlPlane("/v1/team/state", "Failed to load Team/Enterprise organization state.", { headers });
  return parseTeamEnterpriseCloudState(payload, options.requestedActiveOrgId);
}
