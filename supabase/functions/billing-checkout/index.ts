import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { resolveLocalOrigin } from "../_shared/origin.ts";
import { getStripe } from "../_shared/stripe.ts";
import {
  CommercialOrgError,
  recordTeamCheckoutPending,
  resolveTeamCheckoutContext,
  SupabaseCommercialOrgStore,
} from "../_shared/commercial_orgs.ts";

type CheckoutRequest = {
  interval: "month" | "year";
  plan_type?: "pro" | "team";
  organization_id?: string;
  org_id?: string;
  seat_count?: number;
  return_path?: string;
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

function sanitizeReturnPath(path: string | null | undefined): string | null {
  const raw = (path ?? "").trim();
  if (!raw) return null;
  if (!raw.startsWith("/") || raw.startsWith("//")) return null;
  try {
    const url = new URL(raw, "http://local");
    if (url.origin !== "http://local") return null;
    return `${url.pathname}${url.search}${url.hash}`;
  } catch {
    return null;
  }
}

function buildReturnUrl(
  kind: "checkout_success" | "checkout_cancel" | "portal_return",
  origin: string | null,
  returnPath: string | null,
): string {
  const safeReturnPath = sanitizeReturnPath(returnPath);

  // Preferred: a stable hosted redirect page (e.g. https://ctx.rs/redirect)
  // that will attempt to deep-link back into the desktop app via ctx://focus.
  const redirectBase = optionalEnv("CTX_BILLING_REDIRECT_URL");
  if (redirectBase) {
    const url = new URL(redirectBase);
    url.searchParams.set("v", "1");
    url.searchParams.set("source", "stripe");
    url.searchParams.set("kind", kind);
    if (safeReturnPath) url.searchParams.set("return_path", safeReturnPath);
    return url.toString();
  }

  // Local/dev fallback: return to the web app origin.
  const appOrigin = resolveAppOrigin(origin);
  const fallbackPath = safeReturnPath ?? "/settings#billing";
  const url = new URL(fallbackPath, appOrigin);
  if (kind === "portal_return") return url.toString();
  const status = kind === "checkout_success" ? "success" : "cancel";
  url.searchParams.set("checkout", status);
  return url.toString();
}

function appendQueryParam(url: string, key: string, value: string): string {
  const [base, hash] = url.split("#", 2);
  const sep = base.includes("?") ? "&" : "?";
  const next = `${base}${sep}${key}=${value}`;
  return hash ? `${next}#${hash}` : next;
}

function asBearerToken(req: Request): string {
  const authHeader = req.headers.get("authorization") ?? "";
  return authHeader.toLowerCase().startsWith("bearer ")
    ? authHeader.slice(7).trim()
    : "";
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
  const planType = payload?.plan_type === "team" ? "team" : "pro";
  const returnPath = payload?.return_path ?? null;

  const supabaseUrl = requiredEnv("SUPABASE_URL");
  const serviceRoleKey = requiredEnv("SUPABASE_SERVICE_ROLE_KEY");
  const priceId = planType === "team"
    ? requiredEnv(
      interval === "year"
        ? "STRIPE_TEAM_PRICE_ID_YEARLY"
        : "STRIPE_TEAM_PRICE_ID_MONTHLY",
    )
    : requiredEnv(
      interval === "year"
        ? "STRIPE_PRICE_ID_YEARLY"
        : "STRIPE_PRICE_ID_MONTHLY",
    );

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
  const stripe = getStripe();

  if (planType === "team") {
    try {
      const store = new SupabaseCommercialOrgStore(supabase);
      const checkoutContext = await resolveTeamCheckoutContext(
        store,
        userId,
        {
          organizationId: payload?.organization_id ?? payload?.org_id,
          interval,
          seatCount: payload?.seat_count,
        },
      );
      let stripeCustomerId = checkoutContext.stripeCustomerId ?? "";
      if (!stripeCustomerId) {
        const customer = await stripe.customers.create({
          email: email ?? checkoutContext.account.primaryEmail ?? undefined,
          name: checkoutContext.organization.name,
          metadata: {
            supabase_user_id: userId,
            ctx_account_id: checkoutContext.account.id,
            ctx_organization_id: checkoutContext.organization.id,
            ctx_billing_subject_id: checkoutContext.billingSubjectId,
            plan_type: "team",
          },
        });
        stripeCustomerId = customer.id;
      }

      await recordTeamCheckoutPending(store, {
        billingSubjectId: checkoutContext.billingSubjectId,
        stripeCustomerId,
        providerPriceId: priceId,
        seatCount: checkoutContext.seatCount,
      });

      const baseSuccessUrl = buildReturnUrl(
        "checkout_success",
        origin,
        returnPath,
      );
      const successUrl = appendQueryParam(
        baseSuccessUrl,
        "session_id",
        "{CHECKOUT_SESSION_ID}",
      );
      const checkoutMetadata = {
        supabase_user_id: userId,
        ctx_account_id: checkoutContext.account.id,
        ctx_organization_id: checkoutContext.organization.id,
        ctx_billing_subject_id: checkoutContext.billingSubjectId,
        plan_type: "team",
        billing_interval: interval,
      };
      const session = await stripe.checkout.sessions.create({
        mode: "subscription",
        customer: stripeCustomerId,
        client_reference_id: userId,
        line_items: [{ price: priceId, quantity: checkoutContext.seatCount }],
        allow_promotion_codes: true,
        subscription_data: {
          metadata: checkoutMetadata,
        },
        success_url: successUrl,
        cancel_url: buildReturnUrl("checkout_cancel", origin, returnPath),
        metadata: checkoutMetadata,
      });

      return new Response(
        JSON.stringify({
          url: session.url,
          organization_id: checkoutContext.organization.id,
          billing_subject_id: checkoutContext.billingSubjectId,
          seat_count: checkoutContext.seatCount,
        }),
        {
          status: 200,
          headers: {
            "content-type": "application/json",
            ...corsHeaders(origin),
          },
        },
      );
    } catch (error: unknown) {
      if (error instanceof CommercialOrgError) {
        return new Response(
          JSON.stringify({ error: error.code, message: error.message }),
          {
            status: error.status,
            headers: {
              "content-type": "application/json",
              ...corsHeaders(origin),
            },
          },
        );
      }
      throw error;
    }
  }

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

  let stripeCustomerId = profile?.stripe_customer_id
    ? String(profile.stripe_customer_id)
    : "";
  if (!stripeCustomerId) {
    const customer = await stripe.customers.create({
      email: email ?? undefined,
      metadata: { supabase_user_id: userId },
    });
    stripeCustomerId = customer.id;
    await supabase
      .from("billing_profile")
      .upsert(
        {
          user_id: userId,
          email,
          stripe_customer_id: stripeCustomerId,
          updated_at: new Date().toISOString(),
        },
        { onConflict: "user_id" },
      );
  }

  const baseSuccessUrl = buildReturnUrl("checkout_success", origin, returnPath);
  const successUrl = appendQueryParam(
    baseSuccessUrl,
    "session_id",
    "{CHECKOUT_SESSION_ID}",
  );
  const checkoutMetadata = {
    supabase_user_id: userId,
    plan_type: "pro",
    billing_interval: interval,
  };
  const session = await stripe.checkout.sessions.create({
    mode: "subscription",
    customer: stripeCustomerId,
    client_reference_id: userId,
    line_items: [{ price: priceId, quantity: 1 }],
    allow_promotion_codes: true,
    subscription_data: {
      metadata: checkoutMetadata,
    },
    success_url: successUrl,
    cancel_url: buildReturnUrl("checkout_cancel", origin, returnPath),
    metadata: checkoutMetadata,
  });

  return new Response(JSON.stringify({ url: session.url }), {
    status: 200,
    headers: { "content-type": "application/json", ...corsHeaders(origin) },
  });
});
