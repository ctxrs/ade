import type {
  MembershipRole,
  TeamEnterpriseAdminState,
  TeamEnterpriseCloudState,
  TeamEnterpriseFeatureGrant,
  TeamEnterpriseInvite,
  TeamEnterpriseOrg,
  TeamEnterprisePolicyDraft,
  TeamEnterpriseSubscription,
} from "./teamEnterpriseSettingsApi";

export type FunctionResponse = {
  data: unknown;
  error: unknown;
};

export function readErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    const message = record.message;
    if (typeof message === "string" && message.trim()) return message;
  }
  return fallback;
}

export function assertFunctionOk(response: FunctionResponse, fallback: string): unknown {
  if (response.error) {
    throw new Error(readErrorMessage(response.error, fallback));
  }
  return response.data;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function readString(row: Record<string, unknown> | null, key: string): string | null {
  const value = row?.[key];
  return typeof value === "string" && value.trim() ? value : null;
}

function readBoolean(row: Record<string, unknown> | null, key: string): boolean {
  return row?.[key] === true;
}

function readNumber(row: Record<string, unknown> | null, key: string): number | null {
  const value = row?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function parseRole(value: unknown): MembershipRole | null {
  return value === "owner" || value === "admin" || value === "member" ? value : null;
}

function parsePlanType(value: unknown): TeamEnterpriseSubscription["planType"] | null {
  return value === "free_local" || value === "pro" || value === "team" || value === "enterprise"
    ? value
    : null;
}

function parseFeatureState(value: unknown): TeamEnterpriseFeatureGrant["state"] | null {
  return value === "enabled" || value === "disabled" ? value : null;
}

function parseOrganizationSummaries(value: unknown): TeamEnterpriseOrg[] {
  return asArray(value).flatMap((item) => {
    const summary = asRecord(item);
    const organization = asRecord(summary?.organization);
    const membership = asRecord(summary?.membership);
    const billing = asRecord(summary?.billing);
    const seatState = asRecord(summary?.seatState);
    const id = readString(organization, "id");
    const name = readString(organization, "name");
    const role = parseRole(membership?.role);
    if (!id || !name || !role) return [];

    const planType = parsePlanType(billing?.planType) ?? "free_local";
    return [{
      id,
      name,
      slug: readString(organization, "slug"),
      role,
      status: readString(organization, "status") ?? "active",
      createdAt: readString(organization, "createdAt"),
      billingSubjectId: readString(summary, "billingSubjectId"),
      activeMemberCount: readNumber(seatState, "activeMembers") ?? 0,
      suspendedMemberCount: readNumber(seatState, "suspendedMembers") ?? 0,
      pendingInviteCount: readNumber(seatState, "pendingInvites") ?? 0,
      seatCount: readNumber(seatState, "seatCount") ?? 1,
      seatsAvailable: readNumber(seatState, "seatsAvailable") ?? 0,
      enforceSeatLimit: readBoolean(seatState, "enforceSeatLimit"),
      planType,
      subscriptionStatus: readString(billing, "status") ?? "none",
    }];
  });
}

function parseInviteRows(value: unknown): TeamEnterpriseInvite[] {
  return asArray(value).flatMap((item) => {
    const row = asRecord(item);
    const id = readString(row, "id");
    const email = readString(row, "email");
    const role = parseRole(row?.role);
    const status = readString(row, "status");
    if (!id || !email || !role || !status) return [];
    return [{
      id,
      email,
      role,
      status,
      expiresAt: readString(row, "expires_at") ?? readString(row, "expiresAt"),
      createdAt: readString(row, "created_at") ?? readString(row, "createdAt"),
      inviteToken: readString(row, "invite_token") ?? readString(row, "inviteToken"),
    }];
  });
}

function parseFeatureGrantRows(value: unknown): TeamEnterpriseFeatureGrant[] {
  return asArray(value).flatMap((item) => {
    const row = asRecord(item);
    const featureKey = readString(row, "feature_key") ?? readString(row, "featureKey");
    const state = parseFeatureState(row?.state ?? row?.feature_state);
    if (!featureKey || !state) return [];
    return [{
      featureKey,
      state,
      startsAt: readString(row, "starts_at") ?? readString(row, "startsAt"),
      expiresAt: readString(row, "expires_at") ?? readString(row, "expiresAt"),
    }];
  });
}

function parsePolicyDraft(value: unknown): TeamEnterprisePolicyDraft | null {
  const row = asRecord(value);
  if (!row) return null;
  const providers = typeof row.providers === "string" ? row.providers : null;
  const models = typeof row.models === "string" ? row.models : null;
  const allowPersonalRoutes =
    typeof row.allowPersonalRoutes === "boolean" ? row.allowPersonalRoutes : null;
  const sandboxProfile = row.sandboxProfile === "sandbox_required" || row.sandboxProfile === "sandbox_preferred"
    ? row.sandboxProfile
    : null;
  const networkProfile = row.networkProfile === "default" || row.networkProfile === "restricted" ||
      row.networkProfile === "offline"
    ? row.networkProfile
    : null;
  const archiveVisibility = row.archiveVisibility === "local_only" || row.archiveVisibility === "org_summary" ||
      row.archiveVisibility === "org_transcript" || row.archiveVisibility === "org_evidence"
    ? row.archiveVisibility
    : null;
  if (
    providers === null || models === null || allowPersonalRoutes === null ||
    !sandboxProfile || !networkProfile || !archiveVisibility
  ) {
    return null;
  }
  return {
    providers,
    models,
    allowPersonalRoutes,
    sandboxProfile,
    networkProfile,
    archiveVisibility,
  };
}

function parseAdminState(value: unknown): TeamEnterpriseAdminState | null {
  const row = asRecord(value);
  if (!row) return null;
  return {
    seatTarget: readNumber(row, "seat_target") ?? readNumber(row, "seatTarget"),
    policy: parsePolicyDraft(row.policy_json ?? row.policyJson),
    enterpriseSetupRequestedAt: readString(row, "enterprise_setup_requested_at") ??
      readString(row, "enterpriseSetupRequestedAt"),
    updatedAt: readString(row, "updated_at") ?? readString(row, "updatedAt"),
  };
}

function subscriptionFromOrg(org: TeamEnterpriseOrg | null): TeamEnterpriseSubscription[] {
  if (!org || org.subscriptionStatus === "none") return [];
  return [{
    id: org.billingSubjectId ?? org.id,
    planType: org.planType,
    status: org.subscriptionStatus,
    currentPeriodEnd: null,
    cancelAtPeriodEnd: false,
    seatCount: org.seatCount,
  }];
}

function selectActiveOrgId(
  orgs: TeamEnterpriseOrg[],
  requestedOrgId: string | null,
  serverOrgId: string | null,
): string | null {
  if (serverOrgId && orgs.some((org) => org.id === serverOrgId)) return serverOrgId;
  if (requestedOrgId && orgs.some((org) => org.id === requestedOrgId)) return requestedOrgId;
  return orgs[0]?.id ?? null;
}

export function readCheckoutUrl(data: unknown): string {
  const record = asRecord(data);
  const url = readString(record, "url");
  if (!url) throw new Error("Checkout URL missing.");
  return url;
}

export function parseTeamEnterpriseCloudState(
  data: unknown,
  requestedActiveOrgId: string | null,
): TeamEnterpriseCloudState {
  const payload = asRecord(data);
  const orgs = parseOrganizationSummaries(payload?.organizations);
  if (orgs.length === 0) {
    return {
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
  }

  const activeOrgId = selectActiveOrgId(
    orgs,
    requestedActiveOrgId,
    readString(payload, "active_org_id"),
  );
  const activeOrg = orgs.find((org) => org.id === activeOrgId) ?? null;
  const featureGrants = parseFeatureGrantRows(payload?.feature_grants ?? payload?.featureGrants);

  return {
    orgs,
    activeOrgId,
    activeOrg,
    billingSubjectId: activeOrg?.billingSubjectId ?? null,
    invites: parseInviteRows(payload?.active_invites ?? payload?.invites),
    subscriptions: subscriptionFromOrg(activeOrg),
    featureGrants,
    adminState: parseAdminState(payload?.active_admin_state ?? payload?.activeAdminState),
    memberDirectoryAvailable: false,
  };
}
