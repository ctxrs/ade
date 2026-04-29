import {
  assertEquals,
  assertThrows,
} from "https://deno.land/std@0.224.0/assert/mod.ts";
import {
  buildAccessContext,
  buildEntitlementsSnapshot,
  type EntitlementsSnapshot,
  freeLocalSnapshot,
  selectCommercialSubject,
} from "./commercial.ts";

Deno.test("freeLocalSnapshot returns the install-scoped baseline", () => {
  assertEquals(freeLocalSnapshot(), {
    plan_type: "free_local",
    subject_type: "install",
    features: {
      account_cloud_settings: "disabled",
      org_admin: "disabled",
      org_policy: "disabled",
      org_run_history: "disabled",
      org_audit: "disabled",
      mobile_relay: "disabled",
      llm_token_relay: "disabled",
    },
    expires_at: null,
    grace_expires_at: null,
  });
});

Deno.test("buildEntitlementsSnapshot enables account settings for authenticated account context", () => {
  const snapshot = buildEntitlementsSnapshot({
    subjectType: "account",
    accountId: "acct_123",
    subscription: null,
    grants: [],
    now: new Date("2026-04-28T12:00:00.000Z"),
  });

  assertEquals(snapshot, {
    plan_type: "free_local",
    subject_type: "account",
    account_id: "acct_123",
    org_id: undefined,
    features: {
      account_cloud_settings: "enabled",
      org_admin: "disabled",
      org_policy: "disabled",
      org_run_history: "disabled",
      org_audit: "disabled",
      mobile_relay: "disabled",
      llm_token_relay: "disabled",
    },
    expires_at: null,
    grace_expires_at: null,
  });
});

Deno.test("buildEntitlementsSnapshot maps active team admin subscription to org features", () => {
  const snapshot = buildEntitlementsSnapshot({
    subjectType: "org",
    accountId: "acct_123",
    orgId: "org_456",
    membershipRole: "admin",
    subscription: {
      billingSubjectId: "subject_org_456",
      provider: "stripe",
      planType: "team",
      status: "active",
      currentPeriodEnd: "2026-05-28T12:00:00.000Z",
      cancelAtPeriodEnd: false,
      updatedAt: "2026-04-28T12:00:00.000Z",
    },
    grants: [],
    now: new Date("2026-04-28T12:00:00.000Z"),
  });

  assertEquals(snapshot, {
    plan_type: "team",
    subject_type: "org",
    account_id: "acct_123",
    org_id: "org_456",
    features: {
      account_cloud_settings: "disabled",
      org_admin: "enabled",
      org_policy: "enabled",
      org_run_history: "enabled",
      org_audit: "disabled",
      mobile_relay: "enabled",
      llm_token_relay: "disabled",
    },
    expires_at: "2026-05-28T12:00:00.000Z",
    grace_expires_at: "2026-06-04T12:00:00.000Z",
  });
});

Deno.test("buildEntitlementsSnapshot applies active grants on top of subscription defaults", () => {
  const snapshot = buildEntitlementsSnapshot({
    subjectType: "account",
    accountId: "acct_123",
    subscription: {
      billingSubjectId: "subject_acct_123",
      provider: "stripe",
      planType: "pro",
      status: "active",
      currentPeriodEnd: "2026-05-28T12:00:00.000Z",
      cancelAtPeriodEnd: false,
      updatedAt: "2026-04-28T12:00:00.000Z",
    },
    grants: [
      {
        featureKey: "llm_token_relay",
        featureState: "enabled",
        startsAt: "2026-04-28T00:00:00.000Z",
        expiresAt: "2026-05-05T12:00:00.000Z",
        graceExpiresAt: "2026-05-12T12:00:00.000Z",
        source: "manual",
      },
      {
        featureKey: "mobile_relay",
        featureState: "disabled",
        startsAt: "2026-04-28T00:00:00.000Z",
        expiresAt: null,
        graceExpiresAt: null,
        source: "manual",
      },
    ],
    now: new Date("2026-04-28T12:00:00.000Z"),
  });

  assertEquals(snapshot, {
    plan_type: "pro",
    subject_type: "account",
    account_id: "acct_123",
    org_id: undefined,
    features: {
      account_cloud_settings: "enabled",
      org_admin: "disabled",
      org_policy: "disabled",
      org_run_history: "disabled",
      org_audit: "disabled",
      mobile_relay: "disabled",
      llm_token_relay: "enabled",
    },
    expires_at: "2026-05-05T12:00:00.000Z",
    grace_expires_at: "2026-05-12T12:00:00.000Z",
  });
});

Deno.test("selectCommercialSubject prefers the explicit org context when available", () => {
  assertEquals(
    selectCommercialSubject(
      "subject_account_123",
      "550e8400-e29b-41d4-a716-446655440000",
      "subject_org_456",
      "owner",
    ),
    {
      subjectType: "org",
      subjectId: "subject_org_456",
      activeOrgId: "550e8400-e29b-41d4-a716-446655440000",
      membershipRole: "owner",
    },
  );
});

Deno.test("selectCommercialSubject rejects an org context without access", () => {
  assertThrows(
    () =>
      selectCommercialSubject(
        "subject_account_123",
        "550e8400-e29b-41d4-a716-446655440000",
      ),
    Error,
    "not accessible",
  );
});

Deno.test("buildAccessContext mirrors the resolved snapshot into the public contract", () => {
  const snapshot: EntitlementsSnapshot = {
    plan_type: "enterprise",
    subject_type: "org",
    account_id: "acct_123",
    org_id: "org_456",
    features: {
      account_cloud_settings: "disabled",
      org_admin: "enabled",
      org_policy: "enabled",
      org_run_history: "enabled",
      org_audit: "enabled",
      mobile_relay: "enabled",
      llm_token_relay: "disabled",
    },
    expires_at: "2026-05-28T12:00:00.000Z",
    grace_expires_at: "2026-06-04T12:00:00.000Z",
  };

  assertEquals(
    buildAccessContext(snapshot, "install_789", "admin"),
    {
      install_id: "install_789",
      account_id: "acct_123",
      active_org_id: "org_456",
      membership_role: "admin",
      plan_type: "enterprise",
      billing_subject: "org",
      entitlements: snapshot,
    },
  );
});
