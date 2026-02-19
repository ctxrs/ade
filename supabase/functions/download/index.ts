import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { sha256Hex } from "../_shared/hash.ts";
import { capturePostHogEvent } from "../_shared/posthog.ts";

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

const DOWNLOAD_ID_PATTERN = /^[A-Za-z0-9._:-]{1,64}$/;

function firstIp(xff: string | null): string | null {
  if (!xff) return null;
  const first = xff.split(",")[0]?.trim();
  return first && first.length > 0 ? first : null;
}

function normalizeDownloadId(raw: string | null): string | null {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!DOWNLOAD_ID_PATTERN.test(trimmed)) return null;
  return trimmed;
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
  const requestedDownloadId = normalizeDownloadId(url.searchParams.get("ctx_download_id"));
  const downloadId = requestedDownloadId ?? crypto.randomUUID();
  const redirectTo = `${baseUrl}/storage/v1/object/public/${bucket}/${objectPath}`;
  const platform = url.searchParams.get("platform");
  const artifact = url.searchParams.get("artifact") ?? filename;
  const referrer = req.headers.get("referer");
  const userAgent = req.headers.get("user-agent");
  const ip = firstIp(req.headers.get("x-forwarded-for"));
  const ipHash = ip ? await sha256Hex(`${ipSalt}:${ip}`) : null;
  const country = req.headers.get("cf-ipcountry") ??
    req.headers.get("x-country") ??
    null;
  const status = "redirected";

  // Best-effort analytics insert (never block the redirect on analytics failure).
  try {
    const client = createClient(supabaseUrl, serviceRoleKey, {
      auth: { persistSession: false },
    });

    const event: DownloadEvent = {
      channel,
      version,
      platform,
      artifact,
      status,
      url_path: urlPath,
      referrer,
      user_agent: userAgent,
      ip_hash: ipHash,
      country,
    };
    await client.from("download_event").insert(event);
  } catch {
    // ignore
  }

  // Best-effort product analytics mirror (never block redirect).
  try {
    void capturePostHogEvent({
      event: "release_download_redirected",
      distinctId: ipHash ? `download:${ipHash}` : "download:unknown",
      properties: {
        channel,
        version,
        platform,
        artifact,
        result: status,
        country,
        url_path: urlPath,
        download_id: downloadId,
        download_id_source: requestedDownloadId ? "request" : "generated",
      },
    }).catch(() => {
      // ignore
    });
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
