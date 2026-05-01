import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import {
  appendAttributionParamsToUrl,
  attributionProperties,
  hasAttributionContext,
  readAcquisitionContext,
  type DownloadAttributionContext,
} from "../_shared/acquisition.ts";
import { corsHeaders } from "../_shared/cors.ts";
import { sha256Hex } from "../_shared/hash.ts";
import { capturePostHogEvent } from "../_shared/posthog.ts";

const DOWNLOAD_ID_PATTERN = /^[A-Za-z0-9._:-]{1,64}$/;
const DEFAULT_PUBLIC_ARTIFACT_ORIGIN = "https://api.ctx.rs";

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

function rewriteManifestDownloadUrls(value: unknown, attribution: DownloadAttributionContext): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => rewriteManifestDownloadUrls(entry, attribution));
  }
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, nested] of Object.entries(value as Record<string, unknown>)) {
      if (typeof nested === "string" && (key === "url" || key === "url_path")) {
        out[key] = appendAttributionParamsToUrl(nested, attribution);
      } else {
        out[key] = rewriteManifestDownloadUrls(nested, attribution);
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

  const publicArtifactOrigin = (Deno.env.get("SUPABASE_PUBLIC_URL") ?? DEFAULT_PUBLIC_ARTIFACT_ORIGIN).replace(/\/$/, "");
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
  const redirectTo = `${publicArtifactOrigin}/storage/v1/object/public/${bucket}/${objectPath}`;
  const currentVersion = url.searchParams.get("current_version");
  const platform = url.searchParams.get("platform");
  const latestVersion = url.searchParams.get("latest_version");
  const installIdHash = url.searchParams.get("install_id_hash");
  const requestedDownloadId = normalizeDownloadId(url.searchParams.get("ctx_download_id"));
  const ip = firstIp(req.headers.get("x-forwarded-for"));
  const ipHash = ip ? await sha256Hex(`${ipSalt}:${ip}`) : null;
  const country = req.headers.get("cf-ipcountry") ??
    req.headers.get("x-country") ??
    null;
  const acquisition = readAcquisitionContext(req, url);
  const downloadAttribution: DownloadAttributionContext = {
    downloadId: requestedDownloadId,
    ...acquisition,
  };

  const emitManifestCheckTelemetry = async (result: "redirected" | "served_json"): Promise<void> => {
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
          ...(requestedDownloadId ? { download_id_source: "request" } : {}),
          ...attributionProperties(acquisition),
        },
      }).catch(() => {
        // ignore
      });
    } catch {
      // ignore
    }
  };

  if (tail === "latest-tauri.json" && hasAttributionContext(downloadAttribution)) {
    try {
      const manifestResp = await fetch(redirectTo, {
        headers: {
          accept: "application/json",
        },
      });
      if (manifestResp.ok) {
        const manifestJson = await manifestResp.json();
        const rewritten = rewriteManifestDownloadUrls(manifestJson, downloadAttribution);
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
