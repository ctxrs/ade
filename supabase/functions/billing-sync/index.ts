import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import type Stripe from "https://esm.sh/stripe@17.5.0?target=deno";
import { corsHeaders } from "../_shared/cors.ts";
import {
  ensureBillingProfile,
  pickPreferredSubscription,
  setSubscriptionFreeLocal,
  stripeId,
  syncSubscriptionFromStripe,
} from "../_shared/billing.ts";
import { getStripe } from "../_shared/stripe.ts";

type SyncRequest = {
  checkout_session_id?: string;
  session_id?: string;
};

function parseJson<T>(req: Request): Promise<T | null> {
  return req.json().catch(() => null);
}

function requiredEnv(name: string): string {
  const v = Deno.env.get(name) ?? "";
  if (!v) throw new Error(`Missing ${name}`);
  return v;
}

function asBearerToken(req: Request): string {
  const authHeader = req.headers.get("authorization") ?? "";
  return authHeader.toLowerCase().startsWith("bearer ")
    ? authHeader.slice(7).trim()
    : "";
}

function isOrgCheckoutSession(session: Stripe.Checkout.Session): boolean {
  const planType = String(session?.metadata?.plan_type ?? "").trim();
  const billingSubjectId = String(
    session?.metadata?.ctx_billing_subject_id ?? "",
  ).trim();
  return Boolean(billingSubjectId) || planType === "team" ||
    planType === "enterprise";
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

  const payload = await parseJson<SyncRequest>(req);
  const sessionId = payload?.checkout_session_id ?? payload?.session_id ?? null;

  const supabaseUrl = requiredEnv("SUPABASE_URL");
  const serviceRoleKey = requiredEnv("SUPABASE_SERVICE_ROLE_KEY");
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

  const stripe = getStripe();
  let stripeCustomerId = "";
  let stripeSubscriptionId = "";

  if (sessionId) {
    let session: Stripe.Checkout.Session;
    try {
      session = await stripe.checkout.sessions.retrieve(sessionId);
    } catch (err) {
      console.error("stripe checkout session retrieve failed", err);
      return new Response(JSON.stringify({ error: "invalid_session" }), {
        status: 400,
        headers: { "content-type": "application/json", ...corsHeaders(origin) },
      });
    }
    const sessionUser = String(
      session?.client_reference_id ?? session?.metadata?.supabase_user_id ?? "",
    ).trim();
    if (sessionUser && sessionUser !== userId) {
      return new Response(JSON.stringify({ error: "forbidden" }), {
        status: 403,
        headers: { "content-type": "application/json", ...corsHeaders(origin) },
      });
    }
    stripeCustomerId = stripeId(session?.customer);
    stripeSubscriptionId = stripeId(session?.subscription);
    if (stripeCustomerId && !isOrgCheckoutSession(session)) {
      await ensureBillingProfile(supabase, userId, stripeCustomerId);
    }
  }

  if (!stripeCustomerId) {
    const { data: profile } = await supabase
      .from("billing_profile")
      .select("stripe_customer_id")
      .eq("user_id", userId)
      .maybeSingle();
    stripeCustomerId = profile?.stripe_customer_id
      ? String(profile.stripe_customer_id)
      : "";
  }

  if (!stripeCustomerId) {
    return new Response(JSON.stringify({ error: "no_customer" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  if (stripeSubscriptionId) {
    await syncSubscriptionFromStripe(
      supabase,
      stripe,
      stripeSubscriptionId,
      undefined,
      userId,
    );
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const subs = await stripe.subscriptions.list({
    customer: stripeCustomerId,
    status: "all",
    limit: 10,
  });
  const preferred = pickPreferredSubscription(subs.data ?? []);
  if (preferred?.id) {
    await syncSubscriptionFromStripe(
      supabase,
      stripe,
      String(preferred.id),
      undefined,
      userId,
    );
  } else {
    await setSubscriptionFreeLocal(supabase, userId);
  }

  return new Response(JSON.stringify({ ok: true }), {
    status: 200,
    headers: { "content-type": "application/json", ...corsHeaders(origin) },
  });
});
