import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { resolveLocalOrigin } from "../_shared/origin.ts";
import { getStripe } from "../_shared/stripe.ts";

type PortalRequest = {
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

function buildReturnUrl(origin: string | null, returnPath: string | null): string {
  const safeReturnPath = sanitizeReturnPath(returnPath);
  const redirectBase = optionalEnv("CTX_BILLING_REDIRECT_URL");
  if (redirectBase) {
    const url = new URL(redirectBase);
    url.searchParams.set("v", "1");
    url.searchParams.set("source", "stripe");
    url.searchParams.set("kind", "portal_return");
    if (safeReturnPath) url.searchParams.set("return_path", safeReturnPath);
    return url.toString();
  }

  const appOrigin = resolveAppOrigin(origin);
  const fallbackPath = safeReturnPath ?? "/settings#billing";
  return new URL(fallbackPath, appOrigin).toString();
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

  const payload = await parseJson<PortalRequest>(req);
  const returnPath = payload?.return_path ?? null;

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

  const { data: profile, error: profErr } = await supabase
    .from("billing_profile")
    .select("stripe_customer_id")
    .eq("user_id", userId)
    .maybeSingle();

  if (profErr || !profile?.stripe_customer_id) {
    return new Response(JSON.stringify({ error: "no_customer" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const stripe = getStripe();
  const portal = await stripe.billingPortal.sessions.create({
    customer: String(profile.stripe_customer_id),
    return_url: buildReturnUrl(origin, returnPath),
  });

  return new Response(JSON.stringify({ url: portal.url }), {
    status: 200,
    headers: { "content-type": "application/json", ...corsHeaders(origin) },
  });
});
