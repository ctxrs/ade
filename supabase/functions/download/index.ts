import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { attributionProperties, readAcquisitionContext } from "../_shared/acquisition.ts";
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

serve(async (req) => {
  const origin = req.headers.get("origin");
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }

  const publicArtifactOrigin = (Deno.env.get("SUPABASE_PUBLIC_URL") ?? DEFAULT_PUBLIC_ARTIFACT_ORIGIN).replace(/\/$/, "");
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
  const requestedDownloadId = normalizeDownloadId(url.searchParams.get("ctx_download_id"));
  const downloadId = requestedDownloadId ?? crypto.randomUUID();
  const redirectTo = `${publicArtifactOrigin}/storage/v1/object/public/${bucket}/${objectPath}`;
  const platform = url.searchParams.get("platform");
  const artifact = url.searchParams.get("artifact") ?? filename;
  const ip = firstIp(req.headers.get("x-forwarded-for"));
  const ipHash = ip ? await sha256Hex(`${ipSalt}:${ip}`) : null;
  const country = req.headers.get("cf-ipcountry") ??
    req.headers.get("x-country") ??
    null;
  const status = "redirected";
  const acquisition = readAcquisitionContext(req, url);

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
        ...attributionProperties(acquisition),
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
