import type { SupabaseClient } from "npm:@supabase/supabase-js@2.49.1";

export const PLAN_TYPES = ["free_local", "pro", "team", "enterprise"] as const;
export type PlanType = (typeof PLAN_TYPES)[number];

export const BILLING_SUBJECT_TYPES = ["install", "account", "org"] as const;
export type BillingSubjectType = (typeof BILLING_SUBJECT_TYPES)[number];

export const MEMBERSHIP_ROLES = ["owner", "admin", "member"] as const;
export type MembershipRole = (typeof MEMBERSHIP_ROLES)[number];

export const FEATURE_STATES = ["enabled", "disabled"] as const;
export type FeatureState = (typeof FEATURE_STATES)[number];

export const COMMERCIAL_FEATURE_KEYS = [
  "account_cloud_settings",
  "org_admin",
  "org_policy",
  "org_run_history",
  "org_audit",
  "mobile_relay",
  "llm_token_relay",
] as const;
export type CommercialFeatureKey = (typeof COMMERCIAL_FEATURE_KEYS)[number];

export type FeatureMap = Record<CommercialFeatureKey, FeatureState>;

export type EntitlementsSnapshot = {
  plan_type: PlanType;
  subject_type: BillingSubjectType;
  account_id?: string;
  org_id?: string;
  features: FeatureMap;
  expires_at?: string | null;
  grace_expires_at?: string | null;
};

export type SerializedEntitlementsSnapshot =
  & Omit<EntitlementsSnapshot, "features">
  & {
    features: Record<string, FeatureState>;
  };

export type AccessContext = {
  install_id?: string | null;
  account_id?: string;
  active_org_id?: string;
  membership_role?: MembershipRole;
  plan_type: PlanType;
  billing_subject: BillingSubjectType;
  entitlements: EntitlementsSnapshot;
};

type IdentityLinkRow = {
  accountId: string;
};

type MembershipRow = {
  organizationId: string;
  role: MembershipRole;
};

type BillingSubjectRow = {
  id: string;
  subjectType: Extract<BillingSubjectType, "account" | "org">;
};

type CommerceSubscriptionRow = {
  billingSubjectId: string;
  provider: string;
  planType: PlanType;
  status: string;
  currentPeriodEnd: string | null;
  cancelAtPeriodEnd: boolean;
  updatedAt: string | null;
};

type EntitlementGrantRow = {
  featureKey: CommercialFeatureKey;
  featureState: FeatureState;
  startsAt: string | null;
  expiresAt: string | null;
  graceExpiresAt: string | null;
  source: string;
};

export type CommercialResolverOptions = {
  supabase: SupabaseClient;
  authUserId: string;
  activeOrgId?: string | null;
  installId?: string | null;
  now?: Date;
};

export type SubjectSelection = {
  subjectType: Extract<BillingSubjectType, "account" | "org">;
  subjectId: string;
  activeOrgId?: string;
  membershipRole?: MembershipRole;
};

export type BuildEntitlementsSnapshotOptions = {
  subjectType: BillingSubjectType;
  accountId?: string;
  orgId?: string;
  membershipRole?: MembershipRole;
  subscription: CommerceSubscriptionRow | null;
  grants: ReadonlyArray<EntitlementGrantRow>;
  now?: Date;
};

export class CommercialResolverError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(code: string, status: number, message: string) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

const OFFLINE_GRACE_WINDOW_MS = 7 * 24 * 60 * 60 * 1000;

export function freeLocalSnapshot(): EntitlementsSnapshot {
  return {
    plan_type: "free_local",
    subject_type: "install",
    features: emptyFeatureMap(),
    expires_at: null,
    grace_expires_at: null,
  };
}

export function serializeEntitlementsSnapshot(
  snapshot: EntitlementsSnapshot,
): SerializedEntitlementsSnapshot {
  return {
    ...snapshot,
    features: {
      ...snapshot.features,
      remote_mobile_access: snapshot.features.mobile_relay,
      push_notifications: snapshot.features.mobile_relay,
    },
  };
}

export function emptyFeatureMap(): FeatureMap {
  return {
    account_cloud_settings: "disabled",
    org_admin: "disabled",
    org_policy: "disabled",
    org_run_history: "disabled",
    org_audit: "disabled",
    mobile_relay: "disabled",
    llm_token_relay: "disabled",
  };
}

export function isPlanType(value: string): value is PlanType {
  return PLAN_TYPES.includes(value as PlanType);
}

export function isMembershipRole(value: string): value is MembershipRole {
  return MEMBERSHIP_ROLES.includes(value as MembershipRole);
}

export function isFeatureState(value: string): value is FeatureState {
  return FEATURE_STATES.includes(value as FeatureState);
}

export function isCommercialFeatureKey(
  value: string,
): value is CommercialFeatureKey {
  return COMMERCIAL_FEATURE_KEYS.includes(value as CommercialFeatureKey);
}

export function normalizePlanType(value: unknown): PlanType {
  const parsed = readNonEmptyString(value);
  return parsed && isPlanType(parsed) ? parsed : "free_local";
}

export function isActiveSubscriptionStatus(status: string): boolean {
  return status === "active" || status === "trialing";
}

export function defaultFeaturesForContext(
  subjectType: BillingSubjectType,
  planType: PlanType,
  membershipRole?: MembershipRole,
  hasActiveSubscription = false,
): FeatureMap {
  const features = emptyFeatureMap();

  if (subjectType === "account") {
    features.account_cloud_settings = "enabled";
  }

  if (hasActiveSubscription && planType !== "free_local") {
    features.mobile_relay = "enabled";
  }

  if (
    subjectType === "org" && hasActiveSubscription &&
    (planType === "team" || planType === "enterprise")
  ) {
    features.org_run_history = "enabled";

    if (membershipRole === "owner" || membershipRole === "admin") {
      features.org_admin = "enabled";
      features.org_policy = "enabled";
      if (planType === "enterprise") {
        features.org_audit = "enabled";
      }
    }
  }

  return features;
}

export function selectCommercialSubject(
  accountSubjectId: string,
  activeOrgId?: string | null,
  orgSubjectId?: string | null,
  membershipRole?: MembershipRole,
): SubjectSelection {
  if (activeOrgId) {
    if (!orgSubjectId || !membershipRole) {
      throw new CommercialResolverError(
        "active_org_access_denied",
        403,
        "The requested active organization is not accessible for this account.",
      );
    }
    return {
      subjectType: "org",
      subjectId: orgSubjectId,
      activeOrgId,
      membershipRole,
    };
  }

  return {
    subjectType: "account",
    subjectId: accountSubjectId,
  };
}

export function buildEntitlementsSnapshot(
  options: BuildEntitlementsSnapshotOptions,
): EntitlementsSnapshot {
  const now = options.now ?? new Date();
  const planType = options.subscription?.planType ?? "free_local";
  const subscriptionIsActive = options.subscription
    ? isActiveSubscriptionStatus(options.subscription.status)
    : false;
  const features = defaultFeaturesForContext(
    options.subjectType,
    planType,
    options.membershipRole,
    subscriptionIsActive,
  );

  let expiresAt: string | null = null;
  let graceExpiresAt: string | null = null;

  if (options.subscription?.currentPeriodEnd) {
    expiresAt = options.subscription.currentPeriodEnd;
    graceExpiresAt = addGraceWindow(options.subscription.currentPeriodEnd);
  }

  for (const grant of options.grants) {
    if (!isGrantActive(grant, now)) {
      continue;
    }

    features[grant.featureKey] = grant.featureState;
    expiresAt = pickEarlierIso(expiresAt, grant.expiresAt);
    graceExpiresAt = pickEarlierIso(graceExpiresAt, grant.graceExpiresAt);
  }

  return {
    plan_type: planType,
    subject_type: options.subjectType,
    account_id: options.accountId,
    org_id: options.orgId,
    features,
    expires_at: expiresAt,
    grace_expires_at: graceExpiresAt,
  };
}

export function buildAccessContext(
  snapshot: EntitlementsSnapshot,
  installId?: string | null,
  membershipRole?: MembershipRole,
): AccessContext {
  return {
    install_id: installId ?? null,
    account_id: snapshot.account_id,
    active_org_id: snapshot.org_id,
    membership_role: membershipRole,
    plan_type: snapshot.plan_type,
    billing_subject: snapshot.subject_type,
    entitlements: snapshot,
  };
}

export async function resolveAccessContext(
  options: CommercialResolverOptions,
): Promise<AccessContext> {
  const authUserId = readNonEmptyString(options.authUserId);
  if (!authUserId) {
    throw new CommercialResolverError(
      "invalid_auth_user",
      400,
      "Missing authenticated user id.",
    );
  }

  const activeOrgId = normalizeOptionalUuid(options.activeOrgId);
  const link = await fetchIdentityLink(options.supabase, authUserId);
  const accountSubject = await fetchBillingSubjectByAccountId(
    options.supabase,
    link.accountId,
  );

  let membership: MembershipRow | null = null;
  let orgSubject: BillingSubjectRow | null = null;
  if (activeOrgId) {
    membership = await fetchActiveMembership(
      options.supabase,
      link.accountId,
      activeOrgId,
    );
    if (!membership) {
      throw new CommercialResolverError(
        "active_org_access_denied",
        403,
        "The requested active organization is not accessible for this account.",
      );
    }
    orgSubject = await fetchBillingSubjectByOrgId(
      options.supabase,
      activeOrgId,
    );
  }

  const selection = selectCommercialSubject(
    accountSubject.id,
    activeOrgId,
    orgSubject?.id ?? null,
    membership?.role,
  );

  const [subscriptions, grants] = await Promise.all([
    fetchCommerceSubscriptions(options.supabase, selection.subjectId),
    fetchEntitlementGrants(options.supabase, selection.subjectId),
  ]);

  const preferredSubscription = pickPreferredSubscription(subscriptions);
  const snapshot = buildEntitlementsSnapshot({
    subjectType: selection.subjectType,
    accountId: link.accountId,
    orgId: selection.activeOrgId,
    membershipRole: selection.membershipRole,
    subscription: preferredSubscription,
    grants,
    now: options.now,
  });

  return buildAccessContext(
    snapshot,
    options.installId,
    selection.membershipRole,
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function readIsoTimestamp(value: unknown): string | null {
  const raw = readNonEmptyString(value);
  if (!raw) return null;
  const parsed = Date.parse(raw);
  if (!Number.isFinite(parsed)) return null;
  return new Date(parsed).toISOString();
}

function readBoolean(value: unknown): boolean {
  return value === true;
}

function normalizeOptionalUuid(value: unknown): string | null {
  const parsed = readNonEmptyString(value);
  if (!parsed) return null;
  const uuidPattern =
    /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  if (!uuidPattern.test(parsed)) {
    throw new CommercialResolverError(
      "invalid_active_org_id",
      400,
      "The supplied active organization id is not a valid UUID.",
    );
  }
  return parsed;
}

function parseIdentityLinkRow(value: unknown): IdentityLinkRow | null {
  if (!isRecord(value)) return null;
  const accountId = readNonEmptyString(value.account_id);
  if (!accountId) return null;
  return { accountId };
}

function parseMembershipRow(value: unknown): MembershipRow | null {
  if (!isRecord(value)) return null;
  const organizationId = readNonEmptyString(value.organization_id);
  const role = readNonEmptyString(value.role);
  if (!organizationId || !role || !isMembershipRole(role)) return null;
  return { organizationId, role };
}

function parseBillingSubjectRow(value: unknown): BillingSubjectRow | null {
  if (!isRecord(value)) return null;
  const id = readNonEmptyString(value.id);
  const subjectType = readNonEmptyString(value.subject_type);
  if (!id || !subjectType) return null;
  if (subjectType !== "account" && subjectType !== "org") {
    return null;
  }
  return { id, subjectType };
}

function parseCommerceSubscriptionRow(
  value: unknown,
): CommerceSubscriptionRow | null {
  if (!isRecord(value)) return null;
  const billingSubjectId = readNonEmptyString(value.billing_subject_id);
  const provider = readNonEmptyString(value.provider);
  const status = readNonEmptyString(value.status);
  if (!billingSubjectId || !provider || !status) return null;
  return {
    billingSubjectId,
    provider,
    planType: normalizePlanType(value.plan_type),
    status,
    currentPeriodEnd: readIsoTimestamp(value.current_period_end),
    cancelAtPeriodEnd: readBoolean(value.cancel_at_period_end),
    updatedAt: readIsoTimestamp(value.updated_at),
  };
}

function parseEntitlementGrantRow(value: unknown): EntitlementGrantRow | null {
  if (!isRecord(value)) return null;
  const featureKey = readNonEmptyString(value.feature_key);
  const featureState = readNonEmptyString(value.feature_state);
  const source = readNonEmptyString(value.source) ?? "manual";
  if (
    !featureKey || !isCommercialFeatureKey(featureKey) || !featureState ||
    !isFeatureState(featureState)
  ) {
    return null;
  }
  return {
    featureKey,
    featureState,
    startsAt: readIsoTimestamp(value.starts_at),
    expiresAt: readIsoTimestamp(value.expires_at),
    graceExpiresAt: readIsoTimestamp(value.grace_expires_at),
    source,
  };
}

function parseRowList<T>(
  value: unknown,
  parser: (row: unknown) => T | null,
): T[] {
  if (!Array.isArray(value)) return [];
  const rows: T[] = [];
  for (const row of value) {
    const parsed = parser(row);
    if (parsed) {
      rows.push(parsed);
    }
  }
  return rows;
}

function isGrantActive(grant: EntitlementGrantRow, now: Date): boolean {
  const nowMs = now.getTime();
  const startsAtMs = grant.startsAt
    ? Date.parse(grant.startsAt)
    : Number.NEGATIVE_INFINITY;
  const expiresAtMs = grant.expiresAt
    ? Date.parse(grant.expiresAt)
    : Number.POSITIVE_INFINITY;
  return startsAtMs <= nowMs && nowMs < expiresAtMs;
}

function addGraceWindow(expiresAt: string): string | null {
  const expiresAtMs = Date.parse(expiresAt);
  if (!Number.isFinite(expiresAtMs)) return null;
  return new Date(expiresAtMs + OFFLINE_GRACE_WINDOW_MS).toISOString();
}

function pickEarlierIso(
  current: string | null,
  candidate: string | null,
): string | null {
  if (!candidate) return current;
  if (!current) return candidate;
  return Date.parse(candidate) < Date.parse(current) ? candidate : current;
}

function pickPreferredSubscription(
  subscriptions: ReadonlyArray<CommerceSubscriptionRow>,
): CommerceSubscriptionRow | null {
  if (subscriptions.length === 0) return null;

  const active = subscriptions.find((subscription) =>
    subscription.status === "active"
  );
  if (active) return active;

  const trialing = subscriptions.find((subscription) =>
    subscription.status === "trialing"
  );
  if (trialing) return trialing;

  return subscriptions.reduce<CommerceSubscriptionRow | null>(
    (latest, subscription) => {
      if (!latest) return subscription;
      const latestUpdatedAt = latest.updatedAt
        ? Date.parse(latest.updatedAt)
        : Number.NEGATIVE_INFINITY;
      const candidateUpdatedAt = subscription.updatedAt
        ? Date.parse(subscription.updatedAt)
        : Number.NEGATIVE_INFINITY;
      return candidateUpdatedAt > latestUpdatedAt ? subscription : latest;
    },
    null,
  );
}

async function fetchIdentityLink(
  supabase: SupabaseClient,
  authUserId: string,
): Promise<IdentityLinkRow> {
  const { data, error } = await supabase
    .from("external_identity_link")
    .select("account_id")
    .eq("provider", "supabase_auth")
    .eq("external_subject", authUserId)
    .maybeSingle();

  if (error) {
    throw new CommercialResolverError(
      "commercial_query_failed",
      500,
      `Failed to load the account identity link: ${error.message}`,
    );
  }

  const parsed = parseIdentityLinkRow(data);
  if (!parsed) {
    throw new CommercialResolverError(
      "account_not_provisioned",
      500,
      "No ctx account is provisioned for the authenticated user.",
    );
  }
  return parsed;
}

async function fetchActiveMembership(
  supabase: SupabaseClient,
  accountId: string,
  organizationId: string,
): Promise<MembershipRow | null> {
  const { data, error } = await supabase
    .from("organization_membership")
    .select("organization_id, role")
    .eq("account_id", accountId)
    .eq("organization_id", organizationId)
    .eq("status", "active")
    .maybeSingle();

  if (error) {
    throw new CommercialResolverError(
      "commercial_query_failed",
      500,
      `Failed to load the active organization membership: ${error.message}`,
    );
  }

  return parseMembershipRow(data);
}

async function fetchBillingSubjectByAccountId(
  supabase: SupabaseClient,
  accountId: string,
): Promise<BillingSubjectRow> {
  const { data, error } = await supabase
    .from("billing_subject")
    .select("id, subject_type")
    .eq("subject_type", "account")
    .eq("account_id", accountId)
    .maybeSingle();

  if (error) {
    throw new CommercialResolverError(
      "commercial_query_failed",
      500,
      `Failed to load the account billing subject: ${error.message}`,
    );
  }

  const parsed = parseBillingSubjectRow(data);
  if (!parsed) {
    throw new CommercialResolverError(
      "account_billing_subject_missing",
      500,
      "No account billing subject is provisioned for the authenticated account.",
    );
  }
  return parsed;
}

async function fetchBillingSubjectByOrgId(
  supabase: SupabaseClient,
  organizationId: string,
): Promise<BillingSubjectRow> {
  const { data, error } = await supabase
    .from("billing_subject")
    .select("id, subject_type")
    .eq("subject_type", "org")
    .eq("organization_id", organizationId)
    .maybeSingle();

  if (error) {
    throw new CommercialResolverError(
      "commercial_query_failed",
      500,
      `Failed to load the organization billing subject: ${error.message}`,
    );
  }

  const parsed = parseBillingSubjectRow(data);
  if (!parsed) {
    throw new CommercialResolverError(
      "organization_billing_subject_missing",
      500,
      "No organization billing subject is provisioned for the requested organization.",
    );
  }
  return parsed;
}

async function fetchCommerceSubscriptions(
  supabase: SupabaseClient,
  billingSubjectId: string,
): Promise<CommerceSubscriptionRow[]> {
  const { data, error } = await supabase
    .from("commerce_subscription")
    .select(
      "billing_subject_id, provider, plan_type, status, current_period_end, cancel_at_period_end, updated_at",
    )
    .eq("billing_subject_id", billingSubjectId);

  if (error) {
    throw new CommercialResolverError(
      "commercial_query_failed",
      500,
      `Failed to load commerce subscriptions: ${error.message}`,
    );
  }

  return parseRowList(data, parseCommerceSubscriptionRow);
}

async function fetchEntitlementGrants(
  supabase: SupabaseClient,
  billingSubjectId: string,
): Promise<EntitlementGrantRow[]> {
  const { data, error } = await supabase
    .from("entitlement_grant")
    .select(
      "feature_key, feature_state, starts_at, expires_at, grace_expires_at, source",
    )
    .eq("billing_subject_id", billingSubjectId);

  if (error) {
    throw new CommercialResolverError(
      "commercial_query_failed",
      500,
      `Failed to load entitlement grants: ${error.message}`,
    );
  }

  return parseRowList(data, parseEntitlementGrantRow);
}
