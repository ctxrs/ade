import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "https://esm.sh/@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { getStripe } from "../_shared/stripe.ts";

async function sha256Hex(data: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", data);
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

async function hmacSha256Hex(key: Uint8Array, msg: Uint8Array): Promise<string> {
  const cryptoKey = await crypto.subtle.importKey(
    "raw",
    key,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", cryptoKey, msg);
  return Array.from(new Uint8Array(sig))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function requiredEnv(name: string): string {
  const v = Deno.env.get(name) ?? "";
  if (!v) throw new Error(`Missing ${name}`);
  return v;
}

function planTypeForPriceId(priceId: string): "pro" | "free_local" {
  const monthly = Deno.env.get("STRIPE_PRICE_ID_MONTHLY") ?? "";
  const yearly = Deno.env.get("STRIPE_PRICE_ID_YEARLY") ?? "";
  if (priceId === monthly || priceId === yearly) return "pro";
  return "free_local";
}

function stripeId(obj: any): string {
  if (!obj) return "";
  if (typeof obj === "string") return obj;
  if (typeof obj === "object" && obj.id) return String(obj.id);
  return "";
}

type EventMeta = {
  id: string;
  created: number;
};

async function ensureBillingProfile(
  supabase: ReturnType<typeof createClient>,
  userId: string,
  stripeCustomerId: string,
) {
  if (!userId || !stripeCustomerId) return;
  await supabase
    .from("billing_profile")
    .upsert(
      { user_id: userId, stripe_customer_id: stripeCustomerId, updated_at: new Date().toISOString() },
      { onConflict: "user_id" },
    );
}

async function resolveUserIdForCustomer(
  supabase: ReturnType<typeof createClient>,
  stripeCustomerId: string,
  fallbackUserId?: string,
): Promise<string> {
  if (!stripeCustomerId) return "";
  const { data: profile } = await supabase
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

async function shouldUpdateEventMeta(
  supabase: ReturnType<typeof createClient>,
  stripeSubscriptionId: string,
  eventCreatedMs: number,
): Promise<boolean> {
  const { data } = await supabase
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

async function syncSubscriptionFromStripe(
  supabase: ReturnType<typeof createClient>,
  stripe: ReturnType<typeof getStripe>,
  stripeSubscriptionId: string,
  eventMeta?: EventMeta,
  fallbackUserId?: string,
) {
  if (!stripeSubscriptionId) return;

  const sub = await stripe.subscriptions.retrieve(stripeSubscriptionId);
  const stripeCustomerId = stripeId(sub.customer);
  if (!stripeCustomerId) return;

  const metadataUserId = String((sub as any)?.metadata?.supabase_user_id ?? "").trim();
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

  await supabase.from("billing_subscription").upsert(update, { onConflict: "user_id" });
}

serve(async (req) => {
  const origin = req.headers.get("origin");
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }
  if (req.method !== "POST") {
    return new Response(JSON.stringify({ error: "method_not_allowed" }), {
      status: 405,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const webhookSecret = requiredEnv("STRIPE_WEBHOOK_SECRET");
  const sig = req.headers.get("stripe-signature") ?? "";

  const bodyBuf = new Uint8Array(await req.arrayBuffer());
  const bodyText = new TextDecoder().decode(bodyBuf);

  const stripe = getStripe();
  let evt: any;
  try {
    evt = await stripe.webhooks.constructEventAsync(bodyText, sig, webhookSecret);
  } catch (e) {
    const debug = Deno.env.get("CTX_DEBUG_STRIPE_WEBHOOKS") === "1";
    if (debug) {
      const errorMessage = e instanceof Error ? e.message : String(e);
      const sigParts = sig.split(",").map((p) => p.trim());
      const tStr = sigParts.find((p) => p.startsWith("t="))?.slice(2) ?? "";
      const v1 =
        sigParts.find((p) => p.startsWith("v1="))?.slice(3).trim() ?? "";

      const msg = new Uint8Array(
        [
          ...new TextEncoder().encode(`${tStr}.`),
          ...bodyBuf,
        ],
      );
      const expectedV1 = tStr
        ? await hmacSha256Hex(new TextEncoder().encode(webhookSecret), msg)
        : "";

      return new Response(
        JSON.stringify({
          error: "bad_signature",
          debug: {
            error_message: errorMessage,
            sig_present: Boolean(sig),
            sig_len: sig.length,
            sig_prefix: sig.slice(0, 16),
            body_len: bodyBuf.length,
            body_sha256: await sha256Hex(bodyBuf),
            webhook_secret_sha256: await sha256Hex(
              new TextEncoder().encode(webhookSecret),
            ),
            parsed_t: tStr || null,
            provided_v1_prefix: v1 ? v1.slice(0, 8) : null,
            expected_v1_prefix: expectedV1 ? expectedV1.slice(0, 8) : null,
            provided_matches_expected: Boolean(v1 && expectedV1 && v1 === expectedV1),
          },
        }),
        {
          status: 400,
          headers: {
            "content-type": "application/json",
            ...corsHeaders(origin),
          },
        },
      );
    }
    return new Response(JSON.stringify({ error: "bad_signature" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const supabaseUrl = requiredEnv("SUPABASE_URL");
  const serviceRoleKey = requiredEnv("SUPABASE_SERVICE_ROLE_KEY");
  const supabase = createClient(supabaseUrl, serviceRoleKey, {
    auth: { persistSession: false },
  });

  const type = String(evt.type ?? "");
  const eventMeta: EventMeta | undefined =
    evt && typeof evt.id === "string" && typeof evt.created === "number"
      ? { id: evt.id, created: evt.created }
      : undefined;

  if (type === "checkout.session.completed") {
    const obj = evt.data?.object as any;
    const stripeCustomerId = stripeId(obj?.customer);
    const userId = String(obj?.client_reference_id ?? obj?.metadata?.supabase_user_id ?? "").trim();
    if (stripeCustomerId && userId) {
      await ensureBillingProfile(supabase, userId, stripeCustomerId);
    }
    const stripeSubscriptionId = stripeId(obj?.subscription);
    if (stripeSubscriptionId) {
      await syncSubscriptionFromStripe(
        supabase,
        stripe,
        stripeSubscriptionId,
        eventMeta,
        userId || undefined,
      );
    }
  }

  if (type.startsWith("customer.subscription.")) {
    const sub = evt.data?.object as any;
    const stripeSubscriptionId = stripeId(sub?.id);
    if (stripeSubscriptionId) {
      const fallbackUserId = String(sub?.metadata?.supabase_user_id ?? "").trim() || undefined;
      await syncSubscriptionFromStripe(supabase, stripe, stripeSubscriptionId, eventMeta, fallbackUserId);
    }
  }

  if (type.startsWith("invoice.")) {
    const obj = evt.data?.object as any;
    const stripeSubscriptionId = stripeId(obj?.subscription);
    if (stripeSubscriptionId) {
      await syncSubscriptionFromStripe(supabase, stripe, stripeSubscriptionId, eventMeta);
    }
  }

  return new Response(null, {
    status: 204,
    headers: { ...corsHeaders(origin) },
  });
});
