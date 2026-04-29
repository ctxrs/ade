import type { SupabaseClient } from "npm:@supabase/supabase-js@2.49.1";
import {
  isActiveSubscriptionStatus,
  isMembershipRole,
  type MembershipRole,
  normalizePlanType,
  type PlanType,
} from "./commercial.ts";

export const MEMBERSHIP_STATUSES = ["active", "suspended"] as const;
export type MembershipStatus = (typeof MEMBERSHIP_STATUSES)[number];

export type AccountRecord = {
  id: string;
  primaryEmail: string | null;
  displayName: string | null;
  status: "active" | "disabled";
};

export type OrganizationRecord = {
  id: string;
  name: string;
  slug: string | null;
  status: "active" | "disabled";
  createdAt: string | null;
};

export type MembershipRecord = {
  id: string;
  organizationId: string;
  accountId: string;
  role: MembershipRole;
  status: MembershipStatus;
  invitedByAccountId: string | null;
  createdAt: string | null;
  updatedAt: string | null;
};

export type InviteRecord = {
  id: string;
  organizationId: string;
  email: string;
  role: MembershipRole;
  status: "pending" | "accepted" | "revoked" | "expired";
  invitedByAccountId: string | null;
  acceptedByAccountId: string | null;
  membershipId: string | null;
  tokenHash: string;
  expiresAt: string;
  respondedAt: string | null;
  createdAt: string | null;
  updatedAt: string | null;
};

export type BillingSubjectRecord = {
  id: string;
  subjectType: "account" | "org";
  accountId: string | null;
  organizationId: string | null;
};

export type CommerceSubscriptionRecord = {
  billingSubjectId: string;
  provider: "stripe";
  planType: PlanType;
  status: string;
  providerCustomerId: string | null;
  providerPriceId: string | null;
  seatCount: number;
  currentPeriodEnd: string | null;
  cancelAtPeriodEnd: boolean;
  updatedAt: string | null;
};

export type OrganizationSummary = {
  organization: OrganizationRecord;
  membership: MembershipRecord;
  billingSubjectId: string;
  seatState: SeatState;
  billing:
    | {
      planType: PlanType;
      status: string;
      seatCount: number;
      currentPeriodEnd: string | null;
      cancelAtPeriodEnd: boolean;
    }
    | null;
};

export type SeatState = {
  activeMembers: number;
  suspendedMembers: number;
  pendingInvites: number;
  seatCount: number;
  seatsAvailable: number;
  enforceSeatLimit: boolean;
};

export type CreateOrganizationInput = {
  name: unknown;
  slug?: unknown;
};

export type InviteMemberInput = {
  organizationId: unknown;
  email: unknown;
  role: unknown;
};

export type AcceptInviteInput = {
  token: unknown;
};

export type UpdateMemberInput = {
  organizationId: unknown;
  membershipId: unknown;
  role?: unknown;
  status?: unknown;
};

export type TeamCheckoutInput = {
  organizationId: unknown;
  interval: unknown;
  seatCount?: unknown;
};

export type TeamCheckoutContext = {
  account: AccountRecord;
  organization: OrganizationRecord;
  actorMembership: MembershipRecord;
  billingSubjectId: string;
  stripeCustomerId: string | null;
  seatCount: number;
  activeMembers: number;
  pendingInvites: number;
};

export interface CommercialOrgStore {
  getAccountByAuthUserId(authUserId: string): Promise<AccountRecord | null>;
  getOrganization(organizationId: string): Promise<OrganizationRecord | null>;
  createOrganization(input: {
    name: string;
    slug: string | null;
    createdByAccountId: string;
  }): Promise<OrganizationRecord>;
  getMembership(
    organizationId: string,
    accountId: string,
  ): Promise<MembershipRecord | null>;
  getMembershipById(membershipId: string): Promise<MembershipRecord | null>;
  listMembershipsByAccount(accountId: string): Promise<MembershipRecord[]>;
  listMembershipsByOrganization(
    organizationId: string,
  ): Promise<MembershipRecord[]>;
  createMembership(input: {
    organizationId: string;
    accountId: string;
    role: MembershipRole;
    status: MembershipStatus;
    invitedByAccountId: string | null;
  }): Promise<MembershipRecord>;
  updateMembership(input: {
    membershipId: string;
    role: MembershipRole;
    status: MembershipStatus;
  }): Promise<MembershipRecord>;
  getOrgBillingSubject(
    organizationId: string,
  ): Promise<BillingSubjectRecord | null>;
  ensureOrgBillingSubject(
    organizationId: string,
  ): Promise<BillingSubjectRecord>;
  getCommerceSubscription(
    billingSubjectId: string,
  ): Promise<CommerceSubscriptionRecord | null>;
  upsertCheckoutPendingSubscription(input: {
    billingSubjectId: string;
    stripeCustomerId: string;
    planType: "team";
    providerPriceId: string;
    seatCount: number;
  }): Promise<void>;
  listPendingInvitesByOrganization(organizationId: string): Promise<
    InviteRecord[]
  >;
  getPendingInviteByEmail(
    organizationId: string,
    email: string,
  ): Promise<InviteRecord | null>;
  getInviteByTokenHash(tokenHash: string): Promise<InviteRecord | null>;
  createInvite(input: {
    organizationId: string;
    email: string;
    role: MembershipRole;
    invitedByAccountId: string;
    tokenHash: string;
    expiresAt: string;
  }): Promise<InviteRecord>;
  markInviteAccepted(input: {
    inviteId: string;
    acceptedByAccountId: string;
    membershipId: string;
    respondedAt: string;
  }): Promise<InviteRecord>;
}

export class CommercialOrgError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(code: string, status: number, message: string) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

export class SupabaseCommercialOrgStore implements CommercialOrgStore {
  constructor(private readonly supabase: SupabaseClient) {}

  async getAccountByAuthUserId(
    authUserId: string,
  ): Promise<AccountRecord | null> {
    const { data: link, error: linkError } = await this.supabase
      .from("external_identity_link")
      .select("account_id")
      .eq("provider", "supabase_auth")
      .eq("external_subject", authUserId)
      .maybeSingle();
    if (linkError) throw queryError("account_identity_load_failed", linkError);

    const accountId = readNonEmptyString(readField(link, "account_id"));
    if (!accountId) return null;

    const { data, error } = await this.supabase
      .from("ctx_account")
      .select("id, primary_email, display_name, status")
      .eq("id", accountId)
      .maybeSingle();
    if (error) throw queryError("account_load_failed", error);
    return parseAccount(data);
  }

  async getOrganization(
    organizationId: string,
  ): Promise<OrganizationRecord | null> {
    const { data, error } = await this.supabase
      .from("organization")
      .select("id, name, slug, status, created_at")
      .eq("id", organizationId)
      .maybeSingle();
    if (error) throw queryError("organization_load_failed", error);
    return parseOrganization(data);
  }

  async createOrganization(input: {
    name: string;
    slug: string | null;
    createdByAccountId: string;
  }): Promise<OrganizationRecord> {
    const { data, error } = await this.supabase
      .from("organization")
      .insert({
        name: input.name,
        slug: input.slug,
        created_by_account_id: input.createdByAccountId,
      })
      .select("id, name, slug, status, created_at")
      .single();
    if (error) throw queryError("organization_create_failed", error);
    const parsed = parseOrganization(data);
    if (!parsed) {
      throw new CommercialOrgError(
        "organization_create_failed",
        500,
        "Organization creation returned an invalid row.",
      );
    }
    return parsed;
  }

  async getMembership(
    organizationId: string,
    accountId: string,
  ): Promise<MembershipRecord | null> {
    const { data, error } = await this.supabase
      .from("organization_membership")
      .select(
        "id, organization_id, account_id, role, status, invited_by_account_id, created_at, updated_at",
      )
      .eq("organization_id", organizationId)
      .eq("account_id", accountId)
      .maybeSingle();
    if (error) throw queryError("membership_load_failed", error);
    return parseMembership(data);
  }

  async getMembershipById(
    membershipId: string,
  ): Promise<MembershipRecord | null> {
    const { data, error } = await this.supabase
      .from("organization_membership")
      .select(
        "id, organization_id, account_id, role, status, invited_by_account_id, created_at, updated_at",
      )
      .eq("id", membershipId)
      .maybeSingle();
    if (error) throw queryError("membership_load_failed", error);
    return parseMembership(data);
  }

  async listMembershipsByAccount(
    accountId: string,
  ): Promise<MembershipRecord[]> {
    const { data, error } = await this.supabase
      .from("organization_membership")
      .select(
        "id, organization_id, account_id, role, status, invited_by_account_id, created_at, updated_at",
      )
      .eq("account_id", accountId);
    if (error) throw queryError("membership_list_failed", error);
    return parseRowList(data, parseMembership);
  }

  async listMembershipsByOrganization(
    organizationId: string,
  ): Promise<MembershipRecord[]> {
    const { data, error } = await this.supabase
      .from("organization_membership")
      .select(
        "id, organization_id, account_id, role, status, invited_by_account_id, created_at, updated_at",
      )
      .eq("organization_id", organizationId);
    if (error) throw queryError("membership_list_failed", error);
    return parseRowList(data, parseMembership);
  }

  async createMembership(input: {
    organizationId: string;
    accountId: string;
    role: MembershipRole;
    status: MembershipStatus;
    invitedByAccountId: string | null;
  }): Promise<MembershipRecord> {
    const { data, error } = await this.supabase
      .from("organization_membership")
      .insert({
        organization_id: input.organizationId,
        account_id: input.accountId,
        role: input.role,
        status: input.status,
        invited_by_account_id: input.invitedByAccountId,
      })
      .select(
        "id, organization_id, account_id, role, status, invited_by_account_id, created_at, updated_at",
      )
      .single();
    if (error) throw queryError("membership_create_failed", error);
    const parsed = parseMembership(data);
    if (!parsed) {
      throw new CommercialOrgError(
        "membership_create_failed",
        500,
        "Membership creation returned an invalid row.",
      );
    }
    return parsed;
  }

  async updateMembership(input: {
    membershipId: string;
    role: MembershipRole;
    status: MembershipStatus;
  }): Promise<MembershipRecord> {
    const { data, error } = await this.supabase
      .from("organization_membership")
      .update({
        role: input.role,
        status: input.status,
        updated_at: new Date().toISOString(),
      })
      .eq("id", input.membershipId)
      .select(
        "id, organization_id, account_id, role, status, invited_by_account_id, created_at, updated_at",
      )
      .single();
    if (error) throw queryError("membership_update_failed", error);
    const parsed = parseMembership(data);
    if (!parsed) {
      throw new CommercialOrgError(
        "membership_update_failed",
        500,
        "Membership update returned an invalid row.",
      );
    }
    return parsed;
  }

  async getOrgBillingSubject(
    organizationId: string,
  ): Promise<BillingSubjectRecord | null> {
    const { data, error } = await this.supabase
      .from("billing_subject")
      .select("id, subject_type, account_id, organization_id")
      .eq("subject_type", "org")
      .eq("organization_id", organizationId)
      .maybeSingle();
    if (error) throw queryError("billing_subject_load_failed", error);
    return parseBillingSubject(data);
  }

  async ensureOrgBillingSubject(
    organizationId: string,
  ): Promise<BillingSubjectRecord> {
    await this.supabase
      .from("billing_subject")
      .upsert(
        { subject_type: "org", organization_id: organizationId },
        { onConflict: "organization_id" },
      );
    const subject = await this.getOrgBillingSubject(organizationId);
    if (!subject) {
      throw new CommercialOrgError(
        "organization_billing_subject_missing",
        500,
        "No billing subject exists for this organization.",
      );
    }
    return subject;
  }

  async getCommerceSubscription(
    billingSubjectId: string,
  ): Promise<CommerceSubscriptionRecord | null> {
    const { data, error } = await this.supabase
      .from("commerce_subscription")
      .select(
        "billing_subject_id, provider, plan_type, status, provider_customer_id, provider_price_id, seat_count, current_period_end, cancel_at_period_end, updated_at",
      )
      .eq("billing_subject_id", billingSubjectId)
      .eq("provider", "stripe")
      .maybeSingle();
    if (error) throw queryError("commerce_subscription_load_failed", error);
    return parseCommerceSubscription(data);
  }

  async upsertCheckoutPendingSubscription(input: {
    billingSubjectId: string;
    stripeCustomerId: string;
    planType: "team";
    providerPriceId: string;
    seatCount: number;
  }): Promise<void> {
    const { error } = await this.supabase
      .from("commerce_subscription")
      .upsert(
        {
          billing_subject_id: input.billingSubjectId,
          provider: "stripe",
          plan_type: input.planType,
          status: "checkout_pending",
          provider_customer_id: input.stripeCustomerId,
          provider_price_id: input.providerPriceId,
          seat_count: input.seatCount,
          updated_at: new Date().toISOString(),
        },
        { onConflict: "billing_subject_id,provider" },
      );
    if (error) {
      throw queryError("commerce_subscription_update_failed", error);
    }
  }

  async listPendingInvitesByOrganization(
    organizationId: string,
  ): Promise<InviteRecord[]> {
    const { data, error } = await this.supabase
      .from("organization_invite")
      .select(inviteColumns)
      .eq("organization_id", organizationId)
      .eq("status", "pending");
    if (error) throw queryError("invite_list_failed", error);
    return parseRowList(data, parseInvite);
  }

  async getPendingInviteByEmail(
    organizationId: string,
    email: string,
  ): Promise<InviteRecord | null> {
    const { data, error } = await this.supabase
      .from("organization_invite")
      .select(inviteColumns)
      .eq("organization_id", organizationId)
      .eq("email", email)
      .eq("status", "pending")
      .maybeSingle();
    if (error) throw queryError("invite_load_failed", error);
    return parseInvite(data);
  }

  async getInviteByTokenHash(tokenHash: string): Promise<InviteRecord | null> {
    const { data, error } = await this.supabase
      .from("organization_invite")
      .select(inviteColumns)
      .eq("token_hash", tokenHash)
      .maybeSingle();
    if (error) throw queryError("invite_load_failed", error);
    return parseInvite(data);
  }

  async createInvite(input: {
    organizationId: string;
    email: string;
    role: MembershipRole;
    invitedByAccountId: string;
    tokenHash: string;
    expiresAt: string;
  }): Promise<InviteRecord> {
    const { data, error } = await this.supabase
      .from("organization_invite")
      .insert({
        organization_id: input.organizationId,
        email: input.email,
        role: input.role,
        invited_by_account_id: input.invitedByAccountId,
        token_hash: input.tokenHash,
        expires_at: input.expiresAt,
      })
      .select(inviteColumns)
      .single();
    if (error) throw queryError("invite_create_failed", error);
    const parsed = parseInvite(data);
    if (!parsed) {
      throw new CommercialOrgError(
        "invite_create_failed",
        500,
        "Invite creation returned an invalid row.",
      );
    }
    return parsed;
  }

  async markInviteAccepted(input: {
    inviteId: string;
    acceptedByAccountId: string;
    membershipId: string;
    respondedAt: string;
  }): Promise<InviteRecord> {
    const { data, error } = await this.supabase
      .from("organization_invite")
      .update({
        status: "accepted",
        accepted_by_account_id: input.acceptedByAccountId,
        membership_id: input.membershipId,
        responded_at: input.respondedAt,
        updated_at: input.respondedAt,
      })
      .eq("id", input.inviteId)
      .select(inviteColumns)
      .single();
    if (error) throw queryError("invite_update_failed", error);
    const parsed = parseInvite(data);
    if (!parsed) {
      throw new CommercialOrgError(
        "invite_update_failed",
        500,
        "Invite update returned an invalid row.",
      );
    }
    return parsed;
  }
}

const inviteColumns =
  "id, organization_id, email, role, status, invited_by_account_id, accepted_by_account_id, membership_id, token_hash, expires_at, responded_at, created_at, updated_at";

export async function createOrganizationForAccount(
  store: CommercialOrgStore,
  authUserId: string,
  input: CreateOrganizationInput,
): Promise<OrganizationSummary> {
  const account = await requireActiveAccount(store, authUserId);
  const name = normalizeOrganizationName(input.name);
  const slug = normalizeOptionalSlug(input.slug);
  const organization = await store.createOrganization({
    name,
    slug,
    createdByAccountId: account.id,
  });
  const membership = await store.createMembership({
    organizationId: organization.id,
    accountId: account.id,
    role: "owner",
    status: "active",
    invitedByAccountId: null,
  });
  const subject = await store.ensureOrgBillingSubject(organization.id);
  return await buildOrganizationSummary(
    store,
    organization,
    membership,
    subject.id,
  );
}

export async function listOrganizationsForAccount(
  store: CommercialOrgStore,
  authUserId: string,
): Promise<{ account: AccountRecord; organizations: OrganizationSummary[] }> {
  const account = await requireActiveAccount(store, authUserId);
  const memberships = await store.listMembershipsByAccount(account.id);
  const organizations: OrganizationSummary[] = [];
  for (const membership of memberships) {
    if (membership.status !== "active") continue;
    const organization = await store.getOrganization(membership.organizationId);
    if (!organization || organization.status !== "active") continue;
    const subject = await store.ensureOrgBillingSubject(organization.id);
    organizations.push(
      await buildOrganizationSummary(
        store,
        organization,
        membership,
        subject.id,
      ),
    );
  }
  return { account, organizations };
}

export async function inviteOrganizationMember(
  store: CommercialOrgStore,
  authUserId: string,
  input: InviteMemberInput,
  options?: { now?: Date; token?: string },
): Promise<{ invite: InviteRecord; inviteToken: string }> {
  const account = await requireActiveAccount(store, authUserId);
  const organizationId = normalizeUuid(input.organizationId, "organization_id");
  const email = normalizeEmail(input.email);
  const role = normalizeMembershipRole(input.role);
  const actor = await requireAdminMembership(store, organizationId, account.id);
  if (role === "owner" && actor.role !== "owner") {
    throw new CommercialOrgError(
      "owner_role_required",
      403,
      "Only owners can invite another owner.",
    );
  }

  const existingMembership = await store.getMembership(
    organizationId,
    account.id,
  );
  if (!existingMembership || existingMembership.status !== "active") {
    throw new CommercialOrgError(
      "active_membership_required",
      403,
      "The authenticated account is not an active organization member.",
    );
  }

  const duplicateInvite = await store.getPendingInviteByEmail(
    organizationId,
    email,
  );
  if (duplicateInvite) {
    throw new CommercialOrgError(
      "pending_invite_exists",
      409,
      "A pending invite already exists for this email address.",
    );
  }

  const token = options?.token ?? generateInviteToken();
  const tokenHash = await hashInviteToken(token);
  const now = options?.now ?? new Date();
  const invite = await store.createInvite({
    organizationId,
    email,
    role,
    invitedByAccountId: account.id,
    tokenHash,
    expiresAt: new Date(now.getTime() + 14 * 24 * 60 * 60 * 1000)
      .toISOString(),
  });
  return { invite, inviteToken: token };
}

export async function acceptOrganizationInvite(
  store: CommercialOrgStore,
  authUserId: string,
  input: AcceptInviteInput,
  options?: { now?: Date },
): Promise<{ invite: InviteRecord; membership: MembershipRecord }> {
  const account = await requireActiveAccount(store, authUserId);
  const token = readNonEmptyString(input.token);
  if (!token) {
    throw new CommercialOrgError(
      "invalid_invite_token",
      400,
      "Invite token is required.",
    );
  }

  const invite = await store.getInviteByTokenHash(await hashInviteToken(token));
  if (!invite || invite.status !== "pending") {
    throw new CommercialOrgError(
      "invite_not_found",
      404,
      "No pending invite exists for this token.",
    );
  }

  const now = options?.now ?? new Date();
  if (Date.parse(invite.expiresAt) <= now.getTime()) {
    throw new CommercialOrgError(
      "invite_expired",
      410,
      "This invite has expired.",
    );
  }

  if (
    !account.primaryEmail ||
    normalizeEmail(account.primaryEmail) !== invite.email
  ) {
    throw new CommercialOrgError(
      "invite_email_mismatch",
      403,
      "The authenticated account email does not match this invite.",
    );
  }

  const existingMembership = await store.getMembership(
    invite.organizationId,
    account.id,
  );
  if (existingMembership) {
    throw new CommercialOrgError(
      "membership_already_exists",
      409,
      "This account already has a membership for the organization.",
    );
  }

  await assertSeatAvailableForActivation(store, invite.organizationId, null);
  const membership = await store.createMembership({
    organizationId: invite.organizationId,
    accountId: account.id,
    role: invite.role,
    status: "active",
    invitedByAccountId: invite.invitedByAccountId,
  });
  const accepted = await store.markInviteAccepted({
    inviteId: invite.id,
    acceptedByAccountId: account.id,
    membershipId: membership.id,
    respondedAt: now.toISOString(),
  });
  return { invite: accepted, membership };
}

export async function updateOrganizationMember(
  store: CommercialOrgStore,
  authUserId: string,
  input: UpdateMemberInput,
): Promise<MembershipRecord> {
  const account = await requireActiveAccount(store, authUserId);
  const organizationId = normalizeUuid(input.organizationId, "organization_id");
  const membershipId = normalizeUuid(input.membershipId, "membership_id");
  const actor = await requireAdminMembership(store, organizationId, account.id);
  const target = await store.getMembershipById(membershipId);
  if (!target || target.organizationId !== organizationId) {
    throw new CommercialOrgError(
      "membership_not_found",
      404,
      "No membership exists for this organization and membership id.",
    );
  }

  const nextRole = input.role === undefined
    ? target.role
    : normalizeMembershipRole(input.role);
  const nextStatus = input.status === undefined
    ? target.status
    : normalizeMembershipStatus(input.status);

  assertCanUpdateMembership(actor, target, nextRole, nextStatus);
  if (target.status !== "active" && nextStatus === "active") {
    await assertSeatAvailableForActivation(store, organizationId, target.id);
  }

  await assertOwnerContinuity(store, target, nextRole, nextStatus);
  return await store.updateMembership({
    membershipId,
    role: nextRole,
    status: nextStatus,
  });
}

export async function resolveTeamCheckoutContext(
  store: CommercialOrgStore,
  authUserId: string,
  input: TeamCheckoutInput,
): Promise<TeamCheckoutContext> {
  const account = await requireActiveAccount(store, authUserId);
  const organizationId = normalizeUuid(input.organizationId, "organization_id");
  const actor = await requireAdminMembership(store, organizationId, account.id);
  const organization = await store.getOrganization(organizationId);
  if (!organization || organization.status !== "active") {
    throw new CommercialOrgError(
      "organization_not_found",
      404,
      "No active organization exists for this id.",
    );
  }

  const subject = await store.ensureOrgBillingSubject(organizationId);
  const [memberships, pendingInvites, subscription] = await Promise.all([
    store.listMembershipsByOrganization(organizationId),
    store.listPendingInvitesByOrganization(organizationId),
    store.getCommerceSubscription(subject.id),
  ]);
  const activeMembers =
    memberships.filter((membership) => membership.status === "active").length;
  const defaultSeatCount = Math.max(1, activeMembers + pendingInvites.length);
  const requestedSeatCount = normalizeOptionalSeatCount(input.seatCount) ??
    defaultSeatCount;
  if (requestedSeatCount < activeMembers) {
    throw new CommercialOrgError(
      "seat_count_below_active_members",
      400,
      "Team checkout seat count cannot be lower than active member count.",
    );
  }

  return {
    account,
    organization,
    actorMembership: actor,
    billingSubjectId: subject.id,
    stripeCustomerId: subscription?.providerCustomerId ?? null,
    seatCount: requestedSeatCount,
    activeMembers,
    pendingInvites: pendingInvites.length,
  };
}

export function normalizeCheckoutInterval(value: unknown): "month" | "year" {
  return value === "year" ? "year" : "month";
}

export async function recordTeamCheckoutPending(
  store: CommercialOrgStore,
  input: {
    billingSubjectId: string;
    stripeCustomerId: string;
    providerPriceId: string;
    seatCount: number;
  },
): Promise<void> {
  await store.upsertCheckoutPendingSubscription({
    billingSubjectId: input.billingSubjectId,
    stripeCustomerId: input.stripeCustomerId,
    providerPriceId: input.providerPriceId,
    seatCount: input.seatCount,
    planType: "team",
  });
}

async function buildOrganizationSummary(
  store: CommercialOrgStore,
  organization: OrganizationRecord,
  membership: MembershipRecord,
  billingSubjectId: string,
): Promise<OrganizationSummary> {
  const seatState = await resolveSeatState(
    store,
    organization.id,
    billingSubjectId,
  );
  const subscription = await store.getCommerceSubscription(billingSubjectId);
  const canSeeBilling = membership.role === "owner" ||
    membership.role === "admin";
  return {
    organization,
    membership,
    billingSubjectId,
    seatState,
    billing: canSeeBilling && subscription
      ? {
        planType: subscription.planType,
        status: subscription.status,
        seatCount: subscription.seatCount,
        currentPeriodEnd: subscription.currentPeriodEnd,
        cancelAtPeriodEnd: subscription.cancelAtPeriodEnd,
      }
      : null,
  };
}

async function requireActiveAccount(
  store: CommercialOrgStore,
  authUserId: string,
): Promise<AccountRecord> {
  const parsedAuthUserId = normalizeUuid(authUserId, "auth_user_id");
  const account = await store.getAccountByAuthUserId(parsedAuthUserId);
  if (!account || account.status !== "active") {
    throw new CommercialOrgError(
      "account_not_provisioned",
      500,
      "No active ctx account is provisioned for the authenticated user.",
    );
  }
  return account;
}

async function requireAdminMembership(
  store: CommercialOrgStore,
  organizationId: string,
  accountId: string,
): Promise<MembershipRecord> {
  const membership = await store.getMembership(organizationId, accountId);
  if (
    !membership || membership.status !== "active" ||
    (membership.role !== "owner" && membership.role !== "admin")
  ) {
    throw new CommercialOrgError(
      "org_admin_required",
      403,
      "An active owner or admin membership is required for this action.",
    );
  }
  return membership;
}

async function resolveSeatState(
  store: CommercialOrgStore,
  organizationId: string,
  billingSubjectId: string,
): Promise<SeatState> {
  const [memberships, pendingInvites, subscription] = await Promise.all([
    store.listMembershipsByOrganization(organizationId),
    store.listPendingInvitesByOrganization(organizationId),
    store.getCommerceSubscription(billingSubjectId),
  ]);
  const activeMembers =
    memberships.filter((membership) => membership.status === "active").length;
  const suspendedMembers =
    memberships.filter((membership) => membership.status === "suspended")
      .length;
  const enforceSeatLimit = Boolean(
    subscription &&
      (subscription.planType === "team" ||
        subscription.planType === "enterprise") &&
      isActiveSubscriptionStatus(subscription.status),
  );
  const seatCount = enforceSeatLimit
    ? subscription?.seatCount ?? 1
    : Math.max(1, activeMembers);
  return {
    activeMembers,
    suspendedMembers,
    pendingInvites: pendingInvites.length,
    seatCount,
    seatsAvailable: Math.max(0, seatCount - activeMembers),
    enforceSeatLimit,
  };
}

async function assertSeatAvailableForActivation(
  store: CommercialOrgStore,
  organizationId: string,
  activatingMembershipId: string | null,
): Promise<void> {
  const subject = await store.ensureOrgBillingSubject(organizationId);
  const seatState = await resolveSeatState(store, organizationId, subject.id);
  if (!seatState.enforceSeatLimit) return;

  if (activatingMembershipId) {
    const target = await store.getMembershipById(activatingMembershipId);
    if (target?.status === "active") return;
  }

  if (seatState.activeMembers >= seatState.seatCount) {
    throw new CommercialOrgError(
      "no_seat_available",
      409,
      "No active paid seat is available for this organization.",
    );
  }
}

function assertCanUpdateMembership(
  actor: MembershipRecord,
  target: MembershipRecord,
  nextRole: MembershipRole,
  _nextStatus: MembershipStatus,
): void {
  if (
    actor.role === "admin" && (target.role === "owner" || nextRole === "owner")
  ) {
    throw new CommercialOrgError(
      "owner_role_required",
      403,
      "Admins cannot create, update, or suspend owner memberships.",
    );
  }
}

async function assertOwnerContinuity(
  store: CommercialOrgStore,
  target: MembershipRecord,
  nextRole: MembershipRole,
  nextStatus: MembershipStatus,
): Promise<void> {
  if (target.role !== "owner") return;
  if (nextRole === "owner" && nextStatus === "active") return;

  const memberships = await store.listMembershipsByOrganization(
    target.organizationId,
  );
  const activeOwnerCount =
    memberships.filter((membership) =>
      membership.status === "active" && membership.role === "owner"
    ).length;
  if (activeOwnerCount <= 1) {
    throw new CommercialOrgError(
      "last_owner_required",
      409,
      "An organization must retain at least one active owner.",
    );
  }
}

function normalizeOrganizationName(value: unknown): string {
  const parsed = readNonEmptyString(value);
  if (!parsed || parsed.length > 120) {
    throw new CommercialOrgError(
      "invalid_organization_name",
      400,
      "Organization name must be between 1 and 120 characters.",
    );
  }
  return parsed;
}

function normalizeOptionalSlug(value: unknown): string | null {
  const parsed = readNonEmptyString(value);
  if (!parsed) return null;
  const slug = parsed.toLowerCase();
  if (!/^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$/.test(slug)) {
    throw new CommercialOrgError(
      "invalid_organization_slug",
      400,
      "Organization slug must be 3-64 lowercase letters, numbers, or hyphens.",
    );
  }
  return slug;
}

function normalizeEmail(value: unknown): string {
  const parsed = readNonEmptyString(value)?.toLowerCase();
  if (
    !parsed || parsed.length > 320 || !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(parsed)
  ) {
    throw new CommercialOrgError(
      "invalid_email",
      400,
      "A valid email address is required.",
    );
  }
  return parsed;
}

function normalizeMembershipRole(value: unknown): MembershipRole {
  const parsed = readNonEmptyString(value);
  if (!parsed || !isMembershipRole(parsed)) {
    throw new CommercialOrgError(
      "invalid_membership_role",
      400,
      "Membership role must be owner, admin, or member.",
    );
  }
  return parsed;
}

function normalizeMembershipStatus(value: unknown): MembershipStatus {
  const parsed = readNonEmptyString(value);
  if (!parsed || !MEMBERSHIP_STATUSES.includes(parsed as MembershipStatus)) {
    throw new CommercialOrgError(
      "invalid_membership_status",
      400,
      "Membership status must be active or suspended.",
    );
  }
  return parsed as MembershipStatus;
}

function normalizeOptionalSeatCount(value: unknown): number | null {
  if (value === undefined || value === null) return null;
  if (
    typeof value !== "number" || !Number.isInteger(value) || value < 1 ||
    value > 999
  ) {
    throw new CommercialOrgError(
      "invalid_seat_count",
      400,
      "Seat count must be an integer between 1 and 999.",
    );
  }
  return value;
}

function normalizeUuid(value: unknown, fieldName: string): string {
  const parsed = readNonEmptyString(value);
  const uuidPattern =
    /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  if (!parsed || !uuidPattern.test(parsed)) {
    throw new CommercialOrgError(
      `invalid_${fieldName}`,
      400,
      `${fieldName} must be a valid UUID.`,
    );
  }
  return parsed;
}

function generateInviteToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  let raw = "";
  for (const byte of bytes) raw += String.fromCharCode(byte);
  return btoa(raw).replaceAll("+", "-").replaceAll("/", "_").replaceAll(
    "=",
    "",
  );
}

export async function hashInviteToken(token: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(token),
  );
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function parseAccount(value: unknown): AccountRecord | null {
  const id = readNonEmptyString(readField(value, "id"));
  const status = readNonEmptyString(readField(value, "status"));
  if (!id || (status !== "active" && status !== "disabled")) return null;
  return {
    id,
    primaryEmail: readNonEmptyString(readField(value, "primary_email")),
    displayName: readNonEmptyString(readField(value, "display_name")),
    status,
  };
}

function parseOrganization(value: unknown): OrganizationRecord | null {
  const id = readNonEmptyString(readField(value, "id"));
  const name = readNonEmptyString(readField(value, "name"));
  const status = readNonEmptyString(readField(value, "status"));
  if (!id || !name || (status !== "active" && status !== "disabled")) {
    return null;
  }
  return {
    id,
    name,
    slug: readNonEmptyString(readField(value, "slug")),
    status,
    createdAt: readIsoTimestamp(readField(value, "created_at")),
  };
}

function parseMembership(value: unknown): MembershipRecord | null {
  const id = readNonEmptyString(readField(value, "id"));
  const organizationId = readNonEmptyString(
    readField(value, "organization_id"),
  );
  const accountId = readNonEmptyString(readField(value, "account_id"));
  const role = readNonEmptyString(readField(value, "role"));
  const status = readNonEmptyString(readField(value, "status"));
  if (
    !id || !organizationId || !accountId || !role || !isMembershipRole(role) ||
    !status || !MEMBERSHIP_STATUSES.includes(status as MembershipStatus)
  ) {
    return null;
  }
  return {
    id,
    organizationId,
    accountId,
    role,
    status: status as MembershipStatus,
    invitedByAccountId: readNonEmptyString(
      readField(value, "invited_by_account_id"),
    ),
    createdAt: readIsoTimestamp(readField(value, "created_at")),
    updatedAt: readIsoTimestamp(readField(value, "updated_at")),
  };
}

function parseInvite(value: unknown): InviteRecord | null {
  const id = readNonEmptyString(readField(value, "id"));
  const organizationId = readNonEmptyString(
    readField(value, "organization_id"),
  );
  const email = readNonEmptyString(readField(value, "email"));
  const role = readNonEmptyString(readField(value, "role"));
  const status = readNonEmptyString(readField(value, "status"));
  const tokenHash = readNonEmptyString(readField(value, "token_hash"));
  const expiresAt = readIsoTimestamp(readField(value, "expires_at"));
  if (
    !id || !organizationId || !email || !role || !isMembershipRole(role) ||
    !status || !isInviteStatus(status) || !tokenHash || !expiresAt
  ) {
    return null;
  }
  return {
    id,
    organizationId,
    email,
    role,
    status,
    invitedByAccountId: readNonEmptyString(
      readField(value, "invited_by_account_id"),
    ),
    acceptedByAccountId: readNonEmptyString(
      readField(value, "accepted_by_account_id"),
    ),
    membershipId: readNonEmptyString(readField(value, "membership_id")),
    tokenHash,
    expiresAt,
    respondedAt: readIsoTimestamp(readField(value, "responded_at")),
    createdAt: readIsoTimestamp(readField(value, "created_at")),
    updatedAt: readIsoTimestamp(readField(value, "updated_at")),
  };
}

function parseBillingSubject(value: unknown): BillingSubjectRecord | null {
  const id = readNonEmptyString(readField(value, "id"));
  const subjectType = readNonEmptyString(readField(value, "subject_type"));
  if (!id || (subjectType !== "account" && subjectType !== "org")) return null;
  return {
    id,
    subjectType,
    accountId: readNonEmptyString(readField(value, "account_id")),
    organizationId: readNonEmptyString(readField(value, "organization_id")),
  };
}

function parseCommerceSubscription(
  value: unknown,
): CommerceSubscriptionRecord | null {
  const billingSubjectId = readNonEmptyString(
    readField(value, "billing_subject_id"),
  );
  const provider = readNonEmptyString(readField(value, "provider"));
  const status = readNonEmptyString(readField(value, "status"));
  if (!billingSubjectId || provider !== "stripe" || !status) return null;
  return {
    billingSubjectId,
    provider,
    planType: normalizePlanType(readField(value, "plan_type")),
    status,
    providerCustomerId: readNonEmptyString(
      readField(value, "provider_customer_id"),
    ),
    providerPriceId: readNonEmptyString(readField(value, "provider_price_id")),
    seatCount: readPositiveInteger(readField(value, "seat_count")) ?? 1,
    currentPeriodEnd: readIsoTimestamp(readField(value, "current_period_end")),
    cancelAtPeriodEnd: readField(value, "cancel_at_period_end") === true,
    updatedAt: readIsoTimestamp(readField(value, "updated_at")),
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
    if (parsed) rows.push(parsed);
  }
  return rows;
}

function isInviteStatus(
  value: string,
): value is InviteRecord["status"] {
  return value === "pending" || value === "accepted" || value === "revoked" ||
    value === "expired";
}

function readField(value: unknown, key: string): unknown {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return undefined;
  }
  return (value as Record<string, unknown>)[key];
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
  return Number.isFinite(parsed) ? new Date(parsed).toISOString() : null;
}

function readPositiveInteger(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 1) {
    return null;
  }
  return value;
}

function queryError(
  code: string,
  error: { message?: string; code?: string },
): CommercialOrgError {
  const status = error.code === "23505" ? 409 : 500;
  return new CommercialOrgError(
    code,
    status,
    error.message ?? "Commercial organization query failed.",
  );
}
