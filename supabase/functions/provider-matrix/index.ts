import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "https://esm.sh/@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { sha256Hex } from "../_shared/hash.ts";

type ProviderMatrixEvent = {
  channel: string;
  context_version?: string | null;
  platform?: string | null;
  result: string;
  user_agent?: string | null;
  ip_hash?: string | null;
  country?: string | null;
};

function firstIp(xff: string | null): string | null {
  if (!xff) return null;
  const first = xff.split(",")[0]?.trim();
  return first && first.length > 0 ? first : null;
}

serve(async (req) => {
  const origin = req.headers.get("origin");
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }

  const supabaseUrl = Deno.env.get("SUPABASE_URL") ?? "";
  const serviceRoleKey = Deno.env.get("SUPABASE_SERVICE_ROLE_KEY") ?? "";
  const bucket = Deno.env.get("SUPABASE_STORAGE_BUCKET") ?? "releases";
  const ipSalt = Deno.env.get("IP_HASH_SALT") ?? "local-dev";

  const url = new URL(req.url);
  // Expect: /functions/v1/provider-matrix/<channel>/latest.json
  const parts = url.pathname.split("/").filter(Boolean);
  const idx = parts.findIndex((p) => p === "provider-matrix");
  const rest = idx >= 0 ? parts.slice(idx + 1) : parts;

  const channel = rest[0];
  const tail = rest.slice(1).join("/") || "latest.json";
  if (!channel) {
    return new Response(JSON.stringify({ error: "bad_request" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const objectPath = `providers/${channel}/${tail}`;
  const baseUrl = supabaseUrl.endsWith("/") ? supabaseUrl.slice(0, -1) : supabaseUrl;
  const storageUrl = `${baseUrl}/storage/v1/object/public/${bucket}/${objectPath}`;

  let result = "served";
  let body: string | null = null;
  let status = 200;

  try {
    const resp = await fetch(storageUrl);
    if (!resp.ok) {
      result = "missing";
      status = 404;
    } else {
      body = await resp.text();
    }
  } catch {
    result = "error";
    status = 502;
  }

  try {
    const client = createClient(supabaseUrl, serviceRoleKey, {
      auth: { persistSession: false },
    });
    const ip = firstIp(req.headers.get("x-forwarded-for"));
    const ipHash = ip ? await sha256Hex(`${ipSalt}:${ip}`) : null;
    const country = req.headers.get("cf-ipcountry") ??
      req.headers.get("x-country") ??
      null;

    const event: ProviderMatrixEvent = {
      channel,
      context_version: url.searchParams.get("context_version"),
      platform: url.searchParams.get("platform"),
      result,
      user_agent: req.headers.get("user-agent"),
      ip_hash: ipHash,
      country,
    };
    await client.from("provider_matrix_event").insert(event);
  } catch {
    // ignore
  }

  if (!body) {
    return new Response(JSON.stringify({ error: result }), {
      status,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  return new Response(body, {
    status: 200,
    headers: {
      "content-type": "application/json",
      "cache-control": "no-store",
      ...corsHeaders(origin),
    },
  });
});
