import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import {
  ensureBillingProfile,
  type EventMeta,
  stripeId,
  syncSubscriptionFromStripe,
} from "../_shared/billing.ts";
import { getStripe } from "../_shared/stripe.ts";

function toArrayBuffer(data: Uint8Array): ArrayBuffer {
  return data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer;
}

async function sha256Hex(data: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", toArrayBuffer(data));
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

async function hmacSha256Hex(key: Uint8Array, msg: Uint8Array): Promise<string> {
  const cryptoKey = await crypto.subtle.importKey(
    "raw",
    toArrayBuffer(key),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", cryptoKey, toArrayBuffer(msg));
  return Array.from(new Uint8Array(sig))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function requiredEnv(name: string): string {
  const v = Deno.env.get(name) ?? "";
  if (!v) throw new Error(`Missing ${name}`);
  return v;
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
