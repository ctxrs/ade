import type Stripe from "https://esm.sh/stripe@17.5.0?target=deno";

type BillingProfileRow = {
  user_id?: string;
  stripe_customer_id?: string;
};

type BillingSubscriptionRow = {
  stripe_last_event_created?: string;
};

type ExternalIdentityLinkRow = {
  account_id?: string;
};

type BillingSubjectRow = {
  id?: string;
  subject_type?: string;
};

type Awaitable<T> = PromiseLike<T>;

type FilterQuery<T> = {
  eq(column: string, value: string): FilterQuery<T>;
  maybeSingle(): Awaitable<{ data: T | null }>;
};

type UpsertQuery = {
  upsert(
    values: unknown,
    options?: { onConflict?: string },
  ): Awaitable<unknown>;
};

type TableQuery<T> = UpsertQuery & {
  select(columns: string): FilterQuery<T>;
};

type BillingSupabaseClient = {
  from(table: "billing_profile"): TableQuery<BillingProfileRow>;
  from(table: "billing_subscription"): TableQuery<BillingSubscriptionRow>;
  from(table: "external_identity_link"): TableQuery<ExternalIdentityLinkRow>;
  from(table: "billing_subject"): TableQuery<BillingSubjectRow>;
  from(table: "commerce_subscription"): TableQuery<Record<string, never>>;
};

function asBillingSupabaseClient(value: unknown): BillingSupabaseClient {
  return value as BillingSupabaseClient;
}

export type EventMeta = {
  id: string;
  created: number;
};

type PlanType = "pro" | "team" | "enterprise" | "free_local";

export class BillingSyncError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export function planTypeForPriceId(
  priceId: string,
): PlanType {
  const monthly = Deno.env.get("STRIPE_PRICE_ID_MONTHLY") ?? "";
  const yearly = Deno.env.get("STRIPE_PRICE_ID_YEARLY") ?? "";
  const teamMonthly = Deno.env.get("STRIPE_TEAM_PRICE_ID_MONTHLY") ?? "";
  const teamYearly = Deno.env.get("STRIPE_TEAM_PRICE_ID_YEARLY") ?? "";
  const enterprise = Deno.env.get("STRIPE_ENTERPRISE_PRICE_ID") ?? "";
  if (!priceId) return "free_local";
  if ((monthly && priceId === monthly) || (yearly && priceId === yearly)) {
    return "pro";
  }
  if (
    (teamMonthly && priceId === teamMonthly) ||
    (teamYearly && priceId === teamYearly)
  ) {
    return "team";
  }
  if (enterprise && priceId === enterprise) return "enterprise";
  throw new Error(`Unmapped Stripe price id: ${priceId}`);
}

export function stripeId(obj: unknown): string {
  if (!obj) return "";
  if (typeof obj === "string") return obj;
  if (typeof obj === "object" && (obj as { id?: string }).id) {
    return String((obj as { id?: string }).id);
  }
  return "";
}

export async function ensureBillingProfile(
  supabase: unknown,
  userId: string,
  stripeCustomerId: string,
) {
  if (!userId || !stripeCustomerId) return;
  const client = asBillingSupabaseClient(supabase);
  await client
    .from("billing_profile")
    .upsert(
      {
        user_id: userId,
        stripe_customer_id: stripeCustomerId,
        updated_at: new Date().toISOString(),
      },
      { onConflict: "user_id" },
    );
}

export async function resolveUserIdForCustomer(
  supabase: unknown,
  stripeCustomerId: string,
  fallbackUserId?: string,
): Promise<string> {
  if (!stripeCustomerId) return "";
  const client = asBillingSupabaseClient(supabase);
  const { data: profile } = await client
    .from("billing_profile")
    .select("user_id")
    .eq("stripe_customer_id", stripeCustomerId)
    .maybeSingle();

  const userId = profile?.user_id ? String(profile.user_id) : "";
  if (userId) return userId;
  if (fallbackUserId) {
    await ensureBillingProfile(supabase, fallbackUserId, stripeCustomerId);
    return fallbackUserId;
  }
  return "";
}

export async function shouldUpdateEventMeta(
  supabase: unknown,
  stripeSubscriptionId: string,
  eventCreatedMs: number,
): Promise<boolean> {
  const client = asBillingSupabaseClient(supabase);
  const { data } = await client
    .from("billing_subscription")
    .select("stripe_last_event_created")
    .eq("stripe_subscription_id", stripeSubscriptionId)
    .maybeSingle();
  const last = data?.stripe_last_event_created
    ? Date.parse(String(data.stripe_last_event_created))
    : NaN;
  if (!Number.isFinite(last)) return true;
  return eventCreatedMs >= last;
}

export async function syncSubscriptionFromStripe(
  supabase: unknown,
  stripe: Stripe,
  stripeSubscriptionId: string,
  eventMeta?: EventMeta,
  fallbackUserId?: string,
) {
  if (!stripeSubscriptionId) return;

  let sub: Stripe.Subscription;
  try {
    sub = await stripe.subscriptions.retrieve(stripeSubscriptionId);
  } catch (err) {
    console.error("stripe subscription retrieve failed", err);
    return;
  }

  const stripeCustomerId = stripeId(sub.customer);
  if (!stripeCustomerId) return;

  const metadataUserId = String(sub?.metadata?.supabase_user_id ?? "").trim();
  const priceId = String(sub.items?.data?.[0]?.price?.id ?? "").trim() || null;
  const planType = priceId ? planTypeForPriceId(priceId) : "free_local";
  const userId = planType === "team" || planType === "enterprise"
    ? metadataUserId || fallbackUserId || ""
    : await resolveUserIdForCustomer(
      supabase,
      stripeCustomerId,
      metadataUserId || fallbackUserId,
    );
  if (!userId) return;
  const metadataBillingSubjectId = String(
    sub?.metadata?.ctx_billing_subject_id ??
      sub?.metadata?.billing_subject_id ??
      "",
  ).trim();
  const status = String(sub.status ?? "unknown");
  const cancelAtPeriodEnd = Boolean(sub.cancel_at_period_end ?? false);
  const seatCount = normalizeSeatCount(sub.items?.data?.[0]?.quantity);
  const currentPeriodEnd = typeof sub.current_period_end === "number"
    ? new Date(sub.current_period_end * 1000).toISOString()
    : null;
  const billingSubjectId = await resolveBillingSubjectIdForStripeSync(
    supabase,
    userId,
    planType,
    metadataBillingSubjectId || undefined,
  );

  const update: Record<string, unknown> = {
    user_id: userId,
    plan_type: planType,
    status,
    stripe_subscription_id: stripeSubscriptionId,
    stripe_price_id: priceId,
    current_period_end: currentPeriodEnd,
    cancel_at_period_end: cancelAtPeriodEnd,
    updated_at: new Date().toISOString(),
  };

  if (eventMeta?.id && Number.isFinite(eventMeta.created)) {
    const eventCreatedMs = eventMeta.created * 1000;
    if (
      await shouldUpdateEventMeta(
        supabase,
        stripeSubscriptionId,
        eventCreatedMs,
      )
    ) {
      update.stripe_last_event_id = eventMeta.id;
      update.stripe_last_event_created = new Date(eventCreatedMs).toISOString();
    }
  }

  const client = asBillingSupabaseClient(supabase);
  await client.from("billing_subscription").upsert(update, {
    onConflict: "user_id",
  });
  await syncCommerceSubscriptionForBillingSubject(supabase, billingSubjectId, {
    providerCustomerId: stripeCustomerId,
    providerSubscriptionId: stripeSubscriptionId,
    providerPriceId: priceId,
    seatCount,
    planType,
    status,
    currentPeriodEnd,
    cancelAtPeriodEnd,
    eventMeta,
  });
}

export async function setSubscriptionFreeLocal(
  supabase: unknown,
  userId: string,
) {
  if (!userId) return;
  const billingSubjectId = await resolveBillingSubjectIdForStripeSync(
    supabase,
    userId,
    "free_local",
  );
  const client = asBillingSupabaseClient(supabase);
  await client
    .from("billing_subscription")
    .upsert(
      {
        user_id: userId,
        plan_type: "free_local",
        status: "none",
        stripe_subscription_id: null,
        stripe_price_id: null,
        current_period_end: null,
        cancel_at_period_end: false,
        updated_at: new Date().toISOString(),
      },
      { onConflict: "user_id" },
    );
  await syncCommerceSubscriptionForBillingSubject(supabase, billingSubjectId, {
    providerCustomerId: null,
    providerSubscriptionId: null,
    providerPriceId: null,
    seatCount: 1,
    planType: "free_local",
    status: "none",
    currentPeriodEnd: null,
    cancelAtPeriodEnd: false,
  });
}

export function pickPreferredSubscription(
  subs: Stripe.Subscription[],
): Stripe.Subscription | null {
  if (!Array.isArray(subs) || subs.length === 0) return null;
  const active = subs.find((sub) => sub?.status === "active");
  if (active) return active;
  const trialing = subs.find((sub) => sub?.status === "trialing");
  if (trialing) return trialing;
  return subs.reduce<Stripe.Subscription | null>((latest, sub) => {
    if (!latest) return sub;
    const latestCreated = typeof latest.created === "number"
      ? latest.created
      : 0;
    const subCreated = typeof sub?.created === "number" ? sub.created : 0;
    return subCreated > latestCreated ? sub : latest;
  }, null);
}

type CommerceSubscriptionSync = {
  providerCustomerId: string | null;
  providerSubscriptionId: string | null;
  providerPriceId: string | null;
  seatCount: number;
  planType: PlanType;
  status: string;
  currentPeriodEnd: string | null;
  cancelAtPeriodEnd: boolean;
  eventMeta?: EventMeta;
};

export async function resolveBillingSubjectIdForStripeSync(
  supabase: unknown,
  userId: string,
  planType: PlanType,
  metadataBillingSubjectId?: string,
): Promise<string> {
  if (planType === "team" || planType === "enterprise") {
    if (!metadataBillingSubjectId) {
      throw new BillingSyncError(
        "org_billing_subject_required",
        "Team and Enterprise Stripe subscriptions must include an organization billing subject id.",
      );
    }
    return await resolveOrgBillingSubjectId(
      supabase,
      metadataBillingSubjectId,
    );
  }

  return await resolveAccountBillingSubjectIdForUser(supabase, userId);
}

async function resolveAccountBillingSubjectIdForUser(
  supabase: unknown,
  userId: string,
): Promise<string> {
  const client = asBillingSupabaseClient(supabase);
  const { data: link } = await client
    .from("external_identity_link")
    .select("account_id")
    .eq("provider", "supabase_auth")
    .eq("external_subject", userId)
    .maybeSingle();
  const accountId = link?.account_id ? String(link.account_id) : "";
  if (!accountId) {
    throw new BillingSyncError(
      "account_identity_link_missing",
      "No canonical ctx account identity link exists for this Supabase user.",
    );
  }

  const { data: subject } = await client
    .from("billing_subject")
    .select("id")
    .eq("subject_type", "account")
    .eq("account_id", accountId)
    .maybeSingle();

  const billingSubjectId = subject?.id ? String(subject.id) : "";
  if (!billingSubjectId) {
    throw new BillingSyncError(
      "account_billing_subject_missing",
      "No canonical account billing subject exists for this Supabase user.",
    );
  }
  return billingSubjectId;
}

async function resolveOrgBillingSubjectId(
  supabase: unknown,
  billingSubjectId: string,
): Promise<string> {
  const client = asBillingSupabaseClient(supabase);
  const { data: subject } = await client
    .from("billing_subject")
    .select("id, subject_type")
    .eq("id", billingSubjectId)
    .maybeSingle();

  if (!subject?.id || subject.subject_type !== "org") {
    throw new BillingSyncError(
      "org_billing_subject_missing",
      "Team and Enterprise Stripe subscriptions must reference an organization billing subject.",
    );
  }
  return String(subject.id);
}

async function syncCommerceSubscriptionForBillingSubject(
  supabase: unknown,
  billingSubjectId: string,
  sync: CommerceSubscriptionSync,
): Promise<void> {
  const update: Record<string, unknown> = {
    billing_subject_id: billingSubjectId,
    provider: "stripe",
    plan_type: sync.planType,
    status: sync.status,
    provider_customer_id: sync.providerCustomerId,
    provider_subscription_id: sync.providerSubscriptionId,
    provider_price_id: sync.providerPriceId,
    seat_count: sync.seatCount,
    current_period_end: sync.currentPeriodEnd,
    cancel_at_period_end: sync.cancelAtPeriodEnd,
    updated_at: new Date().toISOString(),
  };

  if (sync.eventMeta?.id && Number.isFinite(sync.eventMeta.created)) {
    update.last_provider_event_id = sync.eventMeta.id;
    update.last_provider_event_created_at = new Date(
      sync.eventMeta.created * 1000,
    ).toISOString();
  }

  const client = asBillingSupabaseClient(supabase);
  await client
    .from("commerce_subscription")
    .upsert(update, { onConflict: "billing_subject_id,provider" });
}

export function normalizeSeatCount(value: unknown): number {
  return typeof value === "number" && Number.isInteger(value) && value > 0
    ? value
    : 1;
}
