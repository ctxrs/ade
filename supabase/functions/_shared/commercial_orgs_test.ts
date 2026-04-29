import {
  assertEquals,
  assertRejects,
} from "https://deno.land/std@0.224.0/assert/mod.ts";
import type {
  AccountRecord,
  BillingSubjectRecord,
  CommerceSubscriptionRecord,
  CommercialOrgStore,
  InviteRecord,
  MembershipRecord,
  MembershipStatus,
  OrganizationRecord,
} from "./commercial_orgs.ts";
import {
  acceptOrganizationInvite,
  CommercialOrgError,
  hashInviteToken,
  inviteOrganizationMember,
  listOrganizationsForAccount,
  resolveTeamCheckoutContext,
} from "./commercial_orgs.ts";
import type { MembershipRole } from "./commercial.ts";

const AUTH_OWNER_ID = "550e8400-e29b-41d4-a716-446655440000";
const AUTH_MEMBER_ID = "550e8400-e29b-41d4-a716-446655440001";
const OWNER_ACCOUNT_ID = "550e8400-e29b-41d4-a716-446655440010";
const MEMBER_ACCOUNT_ID = "550e8400-e29b-41d4-a716-446655440011";
const ORG_ID = "550e8400-e29b-41d4-a716-446655440020";
const BILLING_SUBJECT_ID = "550e8400-e29b-41d4-a716-446655440030";

function rowId(index: number): string {
  return `550e8400-e29b-41d4-a716-${String(index).padStart(12, "0")}`;
}

class FakeCommercialOrgStore implements CommercialOrgStore {
  private readonly accountByAuthUserId = new Map<string, AccountRecord>([
    [
      AUTH_OWNER_ID,
      {
        id: OWNER_ACCOUNT_ID,
        primaryEmail: "owner@example.com",
        displayName: null,
        status: "active",
      },
    ],
    [
      AUTH_MEMBER_ID,
      {
        id: MEMBER_ACCOUNT_ID,
        primaryEmail: "member@example.com",
        displayName: null,
        status: "active",
      },
    ],
  ]);
  private readonly organizations = new Map<string, OrganizationRecord>([
    [
      ORG_ID,
      {
        id: ORG_ID,
        name: "Acme",
        slug: "acme",
        status: "active",
        createdAt: "2026-04-28T00:00:00.000Z",
      },
    ],
  ]);
  private readonly memberships = new Map<string, MembershipRecord>([
    [
      rowId(100),
      {
        id: rowId(100),
        organizationId: ORG_ID,
        accountId: OWNER_ACCOUNT_ID,
        role: "owner",
        status: "active",
        invitedByAccountId: null,
        createdAt: "2026-04-28T00:00:00.000Z",
        updatedAt: null,
      },
    ],
  ]);
  private readonly invites = new Map<string, InviteRecord>();
  private nextId = 200;
  subscription: CommerceSubscriptionRecord | null = {
    billingSubjectId: BILLING_SUBJECT_ID,
    provider: "stripe",
    planType: "team",
    status: "active",
    providerCustomerId: "cus_123",
    providerPriceId: "price_team",
    seatCount: 2,
    currentPeriodEnd: "2026-05-28T00:00:00.000Z",
    cancelAtPeriodEnd: false,
    updatedAt: "2026-04-28T00:00:00.000Z",
  };

  async getAccountByAuthUserId(
    authUserId: string,
  ): Promise<AccountRecord | null> {
    return this.accountByAuthUserId.get(authUserId) ?? null;
  }

  async getOrganization(
    organizationId: string,
  ): Promise<OrganizationRecord | null> {
    return this.organizations.get(organizationId) ?? null;
  }

  async createOrganization(input: {
    name: string;
    slug: string | null;
    createdByAccountId: string;
  }): Promise<OrganizationRecord> {
    const organization = {
      id: rowId(this.nextId++),
      name: input.name,
      slug: input.slug,
      status: "active" as const,
      createdAt: "2026-04-28T00:00:00.000Z",
    };
    this.organizations.set(organization.id, organization);
    return organization;
  }

  async getMembership(
    organizationId: string,
    accountId: string,
  ): Promise<MembershipRecord | null> {
    return this.findMembership((membership) =>
      membership.organizationId === organizationId &&
      membership.accountId === accountId
    );
  }

  async getMembershipById(
    membershipId: string,
  ): Promise<MembershipRecord | null> {
    return this.memberships.get(membershipId) ?? null;
  }

  async listMembershipsByAccount(
    accountId: string,
  ): Promise<MembershipRecord[]> {
    return Array.from(this.memberships.values()).filter((membership) =>
      membership.accountId === accountId
    );
  }

  async listMembershipsByOrganization(
    organizationId: string,
  ): Promise<MembershipRecord[]> {
    return Array.from(this.memberships.values()).filter((membership) =>
      membership.organizationId === organizationId
    );
  }

  async createMembership(input: {
    organizationId: string;
    accountId: string;
    role: MembershipRole;
    status: MembershipStatus;
    invitedByAccountId: string | null;
  }): Promise<MembershipRecord> {
    const membership = {
      id: rowId(this.nextId++),
      organizationId: input.organizationId,
      accountId: input.accountId,
      role: input.role,
      status: input.status,
      invitedByAccountId: input.invitedByAccountId,
      createdAt: "2026-04-28T00:00:00.000Z",
      updatedAt: null,
    };
    this.memberships.set(membership.id, membership);
    return membership;
  }

  async updateMembership(input: {
    membershipId: string;
    role: MembershipRole;
    status: MembershipStatus;
  }): Promise<MembershipRecord> {
    const membership = this.memberships.get(input.membershipId);
    if (!membership) throw new Error("missing membership");
    const updated = {
      ...membership,
      role: input.role,
      status: input.status,
      updatedAt: "2026-04-28T00:00:00.000Z",
    };
    this.memberships.set(updated.id, updated);
    return updated;
  }

  async getOrgBillingSubject(
    organizationId: string,
  ): Promise<BillingSubjectRecord | null> {
    return organizationId === ORG_ID
      ? {
        id: BILLING_SUBJECT_ID,
        subjectType: "org",
        accountId: null,
        organizationId,
      }
      : null;
  }

  async ensureOrgBillingSubject(
    organizationId: string,
  ): Promise<BillingSubjectRecord> {
    return {
      id: BILLING_SUBJECT_ID,
      subjectType: "org",
      accountId: null,
      organizationId,
    };
  }

  async getCommerceSubscription(
    billingSubjectId: string,
  ): Promise<CommerceSubscriptionRecord | null> {
    return billingSubjectId === BILLING_SUBJECT_ID ? this.subscription : null;
  }

  async upsertCheckoutPendingSubscription(input: {
    billingSubjectId: string;
    stripeCustomerId: string;
    planType: "team";
    providerPriceId: string;
    seatCount: number;
  }): Promise<void> {
    this.subscription = {
      billingSubjectId: input.billingSubjectId,
      provider: "stripe",
      planType: input.planType,
      status: "checkout_pending",
      providerCustomerId: input.stripeCustomerId,
      providerPriceId: input.providerPriceId,
      seatCount: input.seatCount,
      currentPeriodEnd: null,
      cancelAtPeriodEnd: false,
      updatedAt: "2026-04-28T00:00:00.000Z",
    };
  }

  async listPendingInvitesByOrganization(
    organizationId: string,
  ): Promise<InviteRecord[]> {
    return Array.from(this.invites.values()).filter((invite) =>
      invite.organizationId === organizationId && invite.status === "pending"
    );
  }

  async getPendingInviteByEmail(
    organizationId: string,
    email: string,
  ): Promise<InviteRecord | null> {
    return this.findInvite((invite) =>
      invite.organizationId === organizationId && invite.email === email &&
      invite.status === "pending"
    );
  }

  async getInviteByTokenHash(tokenHash: string): Promise<InviteRecord | null> {
    return this.findInvite((invite) => invite.tokenHash === tokenHash);
  }

  async createInvite(input: {
    organizationId: string;
    email: string;
    role: MembershipRole;
    invitedByAccountId: string;
    tokenHash: string;
    expiresAt: string;
  }): Promise<InviteRecord> {
    const invite = {
      id: rowId(this.nextId++),
      organizationId: input.organizationId,
      email: input.email,
      role: input.role,
      status: "pending" as const,
      invitedByAccountId: input.invitedByAccountId,
      acceptedByAccountId: null,
      membershipId: null,
      tokenHash: input.tokenHash,
      expiresAt: input.expiresAt,
      respondedAt: null,
      createdAt: "2026-04-28T00:00:00.000Z",
      updatedAt: null,
    };
    this.invites.set(invite.id, invite);
    return invite;
  }

  async markInviteAccepted(input: {
    inviteId: string;
    acceptedByAccountId: string;
    membershipId: string;
    respondedAt: string;
  }): Promise<InviteRecord> {
    const invite = this.invites.get(input.inviteId);
    if (!invite) throw new Error("missing invite");
    const accepted = {
      ...invite,
      status: "accepted" as const,
      acceptedByAccountId: input.acceptedByAccountId,
      membershipId: input.membershipId,
      respondedAt: input.respondedAt,
      updatedAt: input.respondedAt,
    };
    this.invites.set(accepted.id, accepted);
    return accepted;
  }

  private findMembership(
    predicate: (membership: MembershipRecord) => boolean,
  ): MembershipRecord | null {
    return Array.from(this.memberships.values()).find(predicate) ?? null;
  }

  private findInvite(
    predicate: (invite: InviteRecord) => boolean,
  ): InviteRecord | null {
    return Array.from(this.invites.values()).find(predicate) ?? null;
  }
}

Deno.test("inviteOrganizationMember creates a tokenized pending invite", async () => {
  const store = new FakeCommercialOrgStore();
  const result = await inviteOrganizationMember(
    store,
    AUTH_OWNER_ID,
    {
      organizationId: ORG_ID,
      email: "Member@Example.com",
      role: "member",
    },
    {
      now: new Date("2026-04-28T00:00:00.000Z"),
      token: "tok_member",
    },
  );

  assertEquals(result.invite.email, "member@example.com");
  assertEquals(result.inviteToken, "tok_member");
  assertEquals(result.invite.tokenHash, await hashInviteToken("tok_member"));
});

Deno.test("acceptOrganizationInvite activates the matching account and closes the invite", async () => {
  const store = new FakeCommercialOrgStore();
  await inviteOrganizationMember(
    store,
    AUTH_OWNER_ID,
    {
      organizationId: ORG_ID,
      email: "member@example.com",
      role: "member",
    },
    {
      now: new Date("2026-04-28T00:00:00.000Z"),
      token: "tok_member",
    },
  );

  const result = await acceptOrganizationInvite(
    store,
    AUTH_MEMBER_ID,
    { token: "tok_member" },
    { now: new Date("2026-04-29T00:00:00.000Z") },
  );

  assertEquals(result.invite.status, "accepted");
  assertEquals(result.membership.accountId, MEMBER_ACCOUNT_ID);
  assertEquals(result.membership.role, "member");
});

Deno.test("listOrganizationsForAccount omits suspended memberships", async () => {
  const store = new FakeCommercialOrgStore();
  await store.updateMembership({
    membershipId: rowId(100),
    role: "owner",
    status: "suspended",
  });

  const result = await listOrganizationsForAccount(store, AUTH_OWNER_ID);

  assertEquals(result.organizations, []);
});

Deno.test("resolveTeamCheckoutContext rejects a seat target below active members", async () => {
  const store = new FakeCommercialOrgStore();
  await store.createMembership({
    organizationId: ORG_ID,
    accountId: MEMBER_ACCOUNT_ID,
    role: "member",
    status: "active",
    invitedByAccountId: OWNER_ACCOUNT_ID,
  });

  await assertRejects(
    () =>
      resolveTeamCheckoutContext(
        store,
        AUTH_OWNER_ID,
        {
          organizationId: ORG_ID,
          interval: "month",
          seatCount: 1,
        },
      ),
    CommercialOrgError,
    "Team checkout seat count cannot be lower than active member count.",
  );
});
