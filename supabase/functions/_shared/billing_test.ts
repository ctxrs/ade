import {
  assertEquals,
  assertRejects,
} from "https://deno.land/std@0.224.0/assert/mod.ts";
import {
  BillingSyncError,
  resolveBillingSubjectIdForStripeSync,
} from "./billing.ts";

type Row = Record<string, unknown>;

class FakeQuery {
  private readonly filters: Array<{ column: string; value: string }> = [];

  constructor(
    private readonly rows: Row[],
  ) {}

  select(_columns: string): FakeQuery {
    return this;
  }

  eq(column: string, value: string): FakeQuery {
    this.filters.push({ column, value });
    return this;
  }

  maybeSingle(): Promise<{ data: Row | null }> {
    const row = this.rows.find((candidate) =>
      this.filters.every(({ column, value }) => candidate[column] === value)
    );
    return Promise.resolve({ data: row ?? null });
  }

  upsert(): Promise<Record<string, never>> {
    return Promise.resolve({});
  }
}

class FakeSupabase {
  constructor(
    private readonly tables: Record<string, Row[]>,
  ) {}

  from(table: string): FakeQuery {
    return new FakeQuery(this.tables[table] ?? []);
  }
}

Deno.test("resolveBillingSubjectIdForStripeSync rejects org plans without explicit org subject metadata", async () => {
  const supabase = new FakeSupabase({});

  await assertRejects(
    () =>
      resolveBillingSubjectIdForStripeSync(
        supabase,
        "user_123",
        "team",
      ),
    BillingSyncError,
    "organization billing subject id",
  );
});

Deno.test("resolveBillingSubjectIdForStripeSync resolves Team plans only to org billing subjects", async () => {
  const supabase = new FakeSupabase({
    billing_subject: [
      { id: "subject_account_123", subject_type: "account" },
      { id: "subject_org_456", subject_type: "org" },
    ],
  });

  assertEquals(
    await resolveBillingSubjectIdForStripeSync(
      supabase,
      "user_123",
      "team",
      "subject_org_456",
    ),
    "subject_org_456",
  );

  await assertRejects(
    () =>
      resolveBillingSubjectIdForStripeSync(
        supabase,
        "user_123",
        "enterprise",
        "subject_account_123",
      ),
    BillingSyncError,
    "organization billing subject",
  );
});

Deno.test("resolveBillingSubjectIdForStripeSync fails closed when account canonical subject is missing", async () => {
  const supabase = new FakeSupabase({
    external_identity_link: [
      {
        provider: "supabase_auth",
        external_subject: "user_123",
        account_id: "acct_123",
      },
    ],
    billing_subject: [],
  });

  await assertRejects(
    () =>
      resolveBillingSubjectIdForStripeSync(
        supabase,
        "user_123",
        "pro",
      ),
    BillingSyncError,
    "No canonical account billing subject",
  );
});

Deno.test("resolveBillingSubjectIdForStripeSync resolves account plans to account billing subjects", async () => {
  const supabase = new FakeSupabase({
    external_identity_link: [
      {
        provider: "supabase_auth",
        external_subject: "user_123",
        account_id: "acct_123",
      },
    ],
    billing_subject: [
      {
        id: "subject_account_123",
        subject_type: "account",
        account_id: "acct_123",
      },
    ],
  });

  assertEquals(
    await resolveBillingSubjectIdForStripeSync(
      supabase,
      "user_123",
      "pro",
    ),
    "subject_account_123",
  );
});
