import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { sha256Hex } from "../_shared/hash.ts";
import { capturePostHogEvent } from "../_shared/posthog.ts";

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

function withDownloadId(urlValue: string, downloadId: string): string {
  try {
    const parsed = new URL(urlValue, "https://ctx.invalid");
    parsed.searchParams.set("ctx_download_id", downloadId);
    if (/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(urlValue)) {
      return parsed.toString();
    }
    return `${parsed.pathname}${parsed.search}${parsed.hash}`;
  } catch {
    return urlValue;
  }
}

function rewriteManifestDownloadUrls(value: unknown, downloadId: string): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => rewriteManifestDownloadUrls(entry, downloadId));
  }
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, nested] of Object.entries(value as Record<string, unknown>)) {
      if (typeof nested === "string" && (key === "url" || key === "url_path")) {
        out[key] = withDownloadId(nested, downloadId);
      } else {
        out[key] = rewriteManifestDownloadUrls(nested, downloadId);
      }
    }
    return out;
  }
  return value;
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
  const redirectTo = `${supabaseUrl.replace(/\/$/, "")}/storage/v1/object/public/${bucket}/${objectPath}`;
  const currentVersion = url.searchParams.get("current_version");
  const platform = url.searchParams.get("platform");
  const latestVersion = url.searchParams.get("latest_version");
  const installIdHash = url.searchParams.get("install_id_hash");
  const requestedDownloadId = normalizeDownloadId(url.searchParams.get("ctx_download_id"));
  const userAgent = req.headers.get("user-agent");
  const ip = firstIp(req.headers.get("x-forwarded-for"));
  const ipHash = ip ? await sha256Hex(`${ipSalt}:${ip}`) : null;
  const country = req.headers.get("cf-ipcountry") ??
    req.headers.get("x-country") ??
    null;

  const emitManifestCheckTelemetry = async (result: "redirected" | "served_json"): Promise<void> => {
    try {
      const client = createClient(supabaseUrl, serviceRoleKey, {
        auth: { persistSession: false },
      });

      const event: UpdateCheckEvent = {
        channel,
        current_version: currentVersion,
        platform,
        latest_version: latestVersion,
        install_id_hash: installIdHash,
        result,
        user_agent: userAgent,
        ip_hash: ipHash,
        country,
      };
      await client.from("update_check_event").insert(event);
    } catch {
      // ignore
    }

    try {
      void capturePostHogEvent({
        event: "release_manifest_checked",
        distinctId: installIdHash || (ipHash ? `release:${ipHash}` : "release:unknown"),
        properties: {
          channel,
          current_version: currentVersion,
          latest_version: latestVersion,
          platform,
          result,
          manifest_path: tail,
          country,
          url_path: urlPath,
          download_id: requestedDownloadId,
        },
      }).catch(() => {
        // ignore
      });
    } catch {
      // ignore
    }
  };

  if (tail === "latest-tauri.json" && requestedDownloadId) {
    try {
      const manifestResp = await fetch(redirectTo, {
        headers: {
          accept: "application/json",
        },
      });
      if (manifestResp.ok) {
        const manifestJson = await manifestResp.json();
        const rewritten = rewriteManifestDownloadUrls(manifestJson, requestedDownloadId);
        await emitManifestCheckTelemetry("served_json");
        return new Response(JSON.stringify(rewritten), {
          status: 200,
          headers: {
            "content-type": "application/json",
            "cache-control": "no-store",
            ...corsHeaders(origin),
          },
        });
      }
    } catch {
      // ignore and fall back to redirect
    }
  }

  await emitManifestCheckTelemetry("redirected");

  return new Response(null, {
    status: 302,
    headers: {
      location: redirectTo,
      "cache-control": "no-store",
      ...corsHeaders(origin),
    },
  });
});
