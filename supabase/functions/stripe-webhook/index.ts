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

type SubscriptionLike = {
  id: string;
  customer: string | { id: string } | null;
  status?: string | null;
  cancel_at_period_end?: boolean | null;
  current_period_end?: number | null;
  items?: { data?: Array<{ price?: { id?: string | null } | null }> } | null;
};

function stripeId(obj: any): string {
  if (!obj) return "";
  if (typeof obj === "string") return obj;
  if (typeof obj === "object" && obj.id) return String(obj.id);
  return "";
}

async function upsertFromSubscription(
  supabase: ReturnType<typeof createClient>,
  sub: SubscriptionLike,
) {
  const stripeSubscriptionId = String(sub.id);
  const stripeCustomerId = stripeId(sub.customer);
  if (!stripeSubscriptionId || !stripeCustomerId) return;

  const priceId =
    String(sub.items?.data?.[0]?.price?.id ?? "").trim() || null;
  const planType = priceId ? planTypeForPriceId(priceId) : "free_local";
  const status = String(sub.status ?? "unknown");
  const cancelAtPeriodEnd = Boolean(sub.cancel_at_period_end ?? false);
  const currentPeriodEnd =
    typeof sub.current_period_end === "number"
      ? new Date(sub.current_period_end * 1000).toISOString()
      : null;

  const { data: profile } = await supabase
    .from("billing_profile")
    .select("user_id")
    .eq("stripe_customer_id", stripeCustomerId)
    .maybeSingle();

  const userId = profile?.user_id ? String(profile.user_id) : "";
  if (!userId) return;

  await supabase
    .from("billing_subscription")
    .upsert(
      {
        user_id: userId,
        plan_type: planType,
        status,
        stripe_subscription_id: stripeSubscriptionId,
        stripe_price_id: priceId,
        current_period_end: currentPeriodEnd,
        cancel_at_period_end: cancelAtPeriodEnd,
        updated_at: new Date().toISOString(),
      },
      { onConflict: "user_id" },
    );
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
  if (type.startsWith("customer.subscription.")) {
    const sub = evt.data?.object as SubscriptionLike;
    await upsertFromSubscription(supabase, sub);
  } else if (type === "checkout.session.completed") {
    const obj = evt.data?.object as any;
    const stripeCustomerId = stripeId(obj?.customer);
    const userId = String(obj?.client_reference_id ?? obj?.metadata?.supabase_user_id ?? "").trim();
    if (stripeCustomerId && userId) {
      await supabase
        .from("billing_profile")
        .upsert(
          { user_id: userId, stripe_customer_id: stripeCustomerId, updated_at: new Date().toISOString() },
          { onConflict: "user_id" },
        );
    }
  }

  return new Response(null, {
    status: 204,
    headers: { ...corsHeaders(origin) },
  });
});
