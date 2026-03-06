type BillingProfileRow = {
  user_id?: string;
  stripe_customer_id?: string;
};

type BillingSubscriptionRow = {
  stripe_last_event_created?: string;
};

type Awaitable<T> = PromiseLike<T>;

type MaybeSingleQuery<T> = {
  eq(column: string, value: string): {
    maybeSingle(): Awaitable<{ data: T | null }>;
  };
};

type UpsertQuery = {
  upsert(values: unknown, options?: { onConflict?: string }): Awaitable<unknown>;
};

type BillingSupabaseClient = {
  from(table: "billing_profile"): UpsertQuery & {
    select(columns: string): MaybeSingleQuery<BillingProfileRow>;
  };
  from(table: "billing_subscription"): UpsertQuery & {
    select(columns: string): MaybeSingleQuery<BillingSubscriptionRow>;
  };
};

function asBillingSupabaseClient(value: unknown): BillingSupabaseClient {
  return value as BillingSupabaseClient;
}

export type EventMeta = {
  id: string;
  created: number;
};

export function planTypeForPriceId(priceId: string): "pro" | "free_local" {
  const monthly = Deno.env.get("STRIPE_PRICE_ID_MONTHLY") ?? "";
  const yearly = Deno.env.get("STRIPE_PRICE_ID_YEARLY") ?? "";
  if (priceId === monthly || priceId === yearly) return "pro";
  return "free_local";
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
      { user_id: userId, stripe_customer_id: stripeCustomerId, updated_at: new Date().toISOString() },
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
  stripe: any,
  stripeSubscriptionId: string,
  eventMeta?: EventMeta,
  fallbackUserId?: string,
) {
  if (!stripeSubscriptionId) return;

  let sub: any;
  try {
    sub = await stripe.subscriptions.retrieve(stripeSubscriptionId);
  } catch (err) {
    console.error("stripe subscription retrieve failed", err);
    return;
  }

  const stripeCustomerId = stripeId(sub.customer);
  if (!stripeCustomerId) return;

  const metadataUserId = String(sub?.metadata?.supabase_user_id ?? "").trim();
  const userId = await resolveUserIdForCustomer(
    supabase,
    stripeCustomerId,
    metadataUserId || fallbackUserId,
  );
  if (!userId) return;

  const priceId = String(sub.items?.data?.[0]?.price?.id ?? "").trim() || null;
  const planType = priceId ? planTypeForPriceId(priceId) : "free_local";
  const status = String(sub.status ?? "unknown");
  const cancelAtPeriodEnd = Boolean(sub.cancel_at_period_end ?? false);
  const currentPeriodEnd =
    typeof sub.current_period_end === "number"
      ? new Date(sub.current_period_end * 1000).toISOString()
      : null;

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
    if (await shouldUpdateEventMeta(supabase, stripeSubscriptionId, eventCreatedMs)) {
      update.stripe_last_event_id = eventMeta.id;
      update.stripe_last_event_created = new Date(eventCreatedMs).toISOString();
    }
  }

  const client = asBillingSupabaseClient(supabase);
  await client.from("billing_subscription").upsert(update, { onConflict: "user_id" });
}

export async function setSubscriptionFreeLocal(
  supabase: unknown,
  userId: string,
) {
  if (!userId) return;
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
}

export function pickPreferredSubscription(subs: any[]): any | null {
  if (!Array.isArray(subs) || subs.length === 0) return null;
  const active = subs.find((sub) => sub?.status === "active");
  if (active) return active;
  const trialing = subs.find((sub) => sub?.status === "trialing");
  if (trialing) return trialing;
  return subs.reduce((latest, sub) => {
    if (!latest) return sub;
    const latestCreated = typeof latest.created === "number" ? latest.created : 0;
    const subCreated = typeof sub?.created === "number" ? sub.created : 0;
    return subCreated > latestCreated ? sub : latest;
  }, null);
}
