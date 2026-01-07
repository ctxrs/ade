import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { sha256Hex } from "../_shared/hash.ts";

type UpdateCheckEvent = {
  channel: string;
  current_version?: string | null;
  platform?: string | null;
  result: string;
  latest_version?: string | null;
  install_id_hash?: string | null;
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
  // Expect: /functions/v1/releases/<channel>/<path...>
  const parts = url.pathname.split("/").filter(Boolean);
  const idx = parts.findIndex((p) => p === "releases");
  const rest = idx >= 0 ? parts.slice(idx + 1) : parts;

  const channel = rest[0];
  const tail = rest.slice(1).join("/");
  if (!channel || !tail) {
    return new Response(JSON.stringify({ error: "bad_request" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  // Supported:
  // - latest.json
  // - <version>.json
  const objectPath = tail === "latest.json"
    ? `releases/${channel}/latest.json`
    : `releases/${channel}/${tail}`;
  const urlPath = `/releases/${channel}/${tail}`;
  const redirectTo = `${supabaseUrl.replace(/\\/$/, "")}/storage/v1/object/public/${bucket}/${objectPath}`;

  try {
    const client = createClient(supabaseUrl, serviceRoleKey, {
      auth: { persistSession: false },
    });
    const ip = firstIp(req.headers.get("x-forwarded-for"));
    const ipHash = ip ? await sha256Hex(`${ipSalt}:${ip}`) : null;
    const country = req.headers.get("cf-ipcountry") ??
      req.headers.get("x-country") ??
      null;

    const event: UpdateCheckEvent = {
      channel,
      current_version: url.searchParams.get("current_version"),
      platform: url.searchParams.get("platform"),
      latest_version: url.searchParams.get("latest_version"),
      install_id_hash: url.searchParams.get("install_id_hash"),
      result: "redirected",
      user_agent: req.headers.get("user-agent"),
      ip_hash: ipHash,
      country,
    };
    await client.from("update_check_event").insert(event);
  } catch {
    // ignore
  }

  return new Response(null, {
    status: 302,
    headers: {
      location: redirectTo,
      "cache-control": "no-store",
      ...corsHeaders(origin),
    },
  });
});

