import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "https://esm.sh/@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { resolveLocalOrigin } from "../_shared/origin.ts";
import { getStripe } from "../_shared/stripe.ts";

type CheckoutRequest = {
  interval: "month" | "year";
};

function parseJson<T>(req: Request): Promise<T | null> {
  return req.json().catch(() => null);
}

function requiredEnv(name: string): string {
  const v = Deno.env.get(name) ?? "";
  if (!v) throw new Error(`Missing ${name}`);
  return v;
}

function optionalEnv(name: string): string {
  return (Deno.env.get(name) ?? "").trim();
}

function resolveAppOrigin(origin: string | null): string {
  const localOrigin = resolveLocalOrigin(origin);
  if (localOrigin) return localOrigin;
  return requiredEnv("CTX_APP_ORIGIN");
}

function buildReturnUrl(
  kind: "checkout_success" | "checkout_cancel" | "portal_return",
  origin: string | null,
): string {
  // Preferred: a stable hosted redirect page (e.g. https://ctx.rs/redirect)
  // that will attempt to deep-link back into the desktop app via ctx://focus.
  const redirectBase = optionalEnv("CTX_BILLING_REDIRECT_URL");
  if (redirectBase) {
    const url = new URL(redirectBase);
    url.searchParams.set("v", "1");
    url.searchParams.set("source", "stripe");
    url.searchParams.set("kind", kind);
    return url.toString();
  }

  // Local/dev fallback: return to the web app origin.
  const appOrigin = resolveAppOrigin(origin);
  if (kind === "portal_return") return new URL("/settings#billing", appOrigin).toString();
  const status = kind === "checkout_success" ? "success" : "cancel";
  return new URL(`/settings?checkout=${status}#billing`, appOrigin).toString();
}

function asBearerToken(req: Request): string {
  const authHeader = req.headers.get("authorization") ?? "";
  return authHeader.toLowerCase().startsWith("bearer ") ? authHeader.slice(7).trim() : "";
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

  const token = asBearerToken(req);
  if (!token) {
    return new Response(JSON.stringify({ error: "unauthorized" }), {
      status: 401,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const payload = await parseJson<CheckoutRequest>(req);
  const interval = payload?.interval === "year" ? "year" : "month";

  const supabaseUrl = requiredEnv("SUPABASE_URL");
  const serviceRoleKey = requiredEnv("SUPABASE_SERVICE_ROLE_KEY");
  const priceMonthly = requiredEnv("STRIPE_PRICE_ID_MONTHLY");
  const priceYearly = requiredEnv("STRIPE_PRICE_ID_YEARLY");

  const supabase = createClient(supabaseUrl, serviceRoleKey, {
    auth: { persistSession: false },
  });
  const { data: userRes, error: userErr } = await supabase.auth.getUser(token);
  if (userErr || !userRes?.user?.id) {
    return new Response(JSON.stringify({ error: "unauthorized" }), {
      status: 401,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const userId = userRes.user.id;
  const email = userRes.user.email ?? null;

  const { data: profile, error: profErr } = await supabase
    .from("billing_profile")
    .select("stripe_customer_id")
    .eq("user_id", userId)
    .maybeSingle();

  if (profErr) {
    return new Response(JSON.stringify({ error: "profile_load_failed" }), {
      status: 500,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const stripe = getStripe();
  let stripeCustomerId = profile?.stripe_customer_id ? String(profile.stripe_customer_id) : "";
  if (!stripeCustomerId) {
    const customer = await stripe.customers.create({
      email: email ?? undefined,
      metadata: { supabase_user_id: userId },
    });
    stripeCustomerId = customer.id;
    await supabase
      .from("billing_profile")
      .update({ stripe_customer_id: stripeCustomerId, updated_at: new Date().toISOString() })
      .eq("user_id", userId);
  }

  const priceId = interval === "year" ? priceYearly : priceMonthly;
  const session = await stripe.checkout.sessions.create({
    mode: "subscription",
    customer: stripeCustomerId,
    client_reference_id: userId,
    line_items: [{ price: priceId, quantity: 1 }],
    allow_promotion_codes: true,
    success_url: buildReturnUrl("checkout_success", origin),
    cancel_url: buildReturnUrl("checkout_cancel", origin),
    metadata: { supabase_user_id: userId, plan_type: "pro", billing_interval: interval },
  });

  return new Response(JSON.stringify({ url: session.url }), {
    status: 200,
    headers: { "content-type": "application/json", ...corsHeaders(origin) },
  });
});
