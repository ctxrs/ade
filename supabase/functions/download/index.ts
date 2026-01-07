import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { sha256Hex } from "../_shared/hash.ts";

type DownloadEvent = {
  channel: string;
  version: string;
  platform?: string | null;
  artifact?: string | null;
  status: string;
  url_path: string;
  referrer?: string | null;
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
  // Expect: /functions/v1/download/<channel>/<version>/<filename>
  const parts = url.pathname.split("/").filter(Boolean);
  const idx = parts.findIndex((p) => p === "download");
  const rest = idx >= 0 ? parts.slice(idx + 1) : parts;

  const [channel, version, filename] = [rest[0], rest[1], rest.slice(2).join("/")];
  if (!channel || !version || !filename) {
    return new Response(JSON.stringify({ error: "bad_request" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const urlPath = `/download/${channel}/${version}/${filename}`;
  const objectPath = `artifacts/${channel}/${version}/${filename}`;
  const baseUrl = supabaseUrl.endsWith("/") ? supabaseUrl.slice(0, -1) : supabaseUrl;
  const redirectTo = `${baseUrl}/storage/v1/object/public/${bucket}/${objectPath}`;

  // Best-effort analytics insert (never block the redirect on analytics failure).
  try {
    const client = createClient(supabaseUrl, serviceRoleKey, {
      auth: { persistSession: false },
    });
    const ip = firstIp(req.headers.get("x-forwarded-for"));
    const ipHash = ip ? await sha256Hex(`${ipSalt}:${ip}`) : null;
    const country = req.headers.get("cf-ipcountry") ??
      req.headers.get("x-country") ??
      null;

    const event: DownloadEvent = {
      channel,
      version,
      platform: url.searchParams.get("platform"),
      artifact: url.searchParams.get("artifact") ?? filename,
      status: "redirected",
      url_path: urlPath,
      referrer: req.headers.get("referer"),
      user_agent: req.headers.get("user-agent"),
      ip_hash: ipHash,
      country,
    };
    await client.from("download_event").insert(event);
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
