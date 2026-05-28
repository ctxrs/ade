export interface ReleaseObject {
  readonly body?: ReadableStream<Uint8Array>;
  readonly httpEtag?: string;
  readonly range?: ReleaseRange;
  readonly size?: number;
  text?(): Promise<string>;
  writeHttpMetadata(headers: Headers): void;
}

export type ReleaseRange =
  | { readonly offset: number; readonly length?: number }
  | { readonly offset?: number; readonly length: number }
  | { readonly suffix: number };

export interface ReleaseGetOptions {
  readonly range?: ReleaseRange | Headers;
}

export interface ReleaseBucket {
  get(key: string, options?: ReleaseGetOptions): Promise<ReleaseObject | null>;
  head(key: string): Promise<ReleaseObject | null>;
}

export interface Env {
  RELEASES_BUCKET: ReleaseBucket;
  RELEASE_ARTIFACT_REDIRECT_BASE_URL?: string;
}

type Route =
  | { kind: "release"; channel: string; tail: string; objectKey: string }
  | { kind: "download"; channel: string; version: string; filename: string; objectKey: string }
  | { kind: "providerMatrix"; channel: string; tail: string; objectKey: string }
  | { kind: "storage"; objectKey: string };

type AcquisitionContext = {
  referrerDomain?: string | null;
  utmSource?: string | null;
  utmMedium?: string | null;
  utmCampaign?: string | null;
};

type DownloadAttributionContext = AcquisitionContext & {
  downloadId?: string | null;
};

const ABSOLUTE_URL_PATTERN = /^[a-zA-Z][a-zA-Z\d+\-.]*:/;
const DOWNLOAD_ID_PATTERN = /^[A-Za-z0-9._:-]{1,64}$/;
const MAX_ATTRIBUTION_VALUE_LENGTH = 120;
const MAX_HOST_LENGTH = 253;
const JSON_CONTENT_TYPE = "application/json; charset=utf-8";
const RELEASE_CACHE_CONTROL = "no-store";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    try {
      return await handleFetch(request, env);
    } catch (error) {
      return jsonError(500, "internal_error", error instanceof Error ? error.message : "internal error", null);
    }
  },
};

async function handleFetch(request: Request, env: Env): Promise<Response> {
  const origin = request.headers.get("origin");
  if (request.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }
  if (request.method !== "GET" && request.method !== "HEAD") {
    return jsonError(405, "method_not_allowed", "expected GET or HEAD", origin, {
      allow: "GET, HEAD, OPTIONS",
    });
  }

  const url = new URL(request.url);
  const route = parseRoute(url.pathname);
  if (route == null) {
    return jsonError(404, "not_found", "route not found", origin);
  }

  if (route.kind === "download") {
    return redirectDownload(route, request, env, origin);
  }

  const attribution = {
    downloadId: normalizeDownloadId(url.searchParams.get("ctx_download_id")),
    ...readAcquisitionContext(request, url),
  };
  const rewriteJson = request.method === "GET"
    && route.kind === "release"
    && route.tail.endsWith("-tauri.json")
    && hasAttributionContext(attribution);

  return serveObjectFromR2(env.RELEASES_BUCKET, route.objectKey, request, request.method, origin, rewriteJson, attribution);
}

function parseRoute(pathname: string): Route | null {
  const segments = safePathSegments(pathname);
  if (segments == null) return null;

  if (pathMatchesPrefix(segments, ["storage", "v1", "object", "public", "releases"])) {
    const objectKey = segments.slice(5).join("/");
    if (!objectKey) return null;
    return {
      kind: "storage",
      objectKey,
    };
  }

  const apiSegments = pathMatchesPrefix(segments, ["functions", "v1"])
    ? segments.slice(2)
    : segments;

  if (apiSegments[0] === "download") {
    const channel = apiSegments[1];
    const version = apiSegments[2];
    const filename = apiSegments.slice(3).join("/");
    if (!channel || !version || !filename) return null;
    return {
      kind: "download",
      channel,
      version,
      filename,
      objectKey: `artifacts/${channel}/${version}/${filename}`,
    };
  }

  if (apiSegments[0] === "provider-matrix") {
    const channel = apiSegments[1];
    const tail = apiSegments.slice(2).join("/") || "latest.json";
    if (!channel) return null;
    return {
      kind: "providerMatrix",
      channel,
      tail,
      objectKey: `providers/${channel}/${tail}`,
    };
  }

  if (apiSegments[0] === "releases") {
    const channel = apiSegments[1];
    const tail = apiSegments.slice(2).join("/");
    if (!channel || !tail) return null;
    return {
      kind: "release",
      channel,
      tail,
      objectKey: `releases/${channel}/${tail}`,
    };
  }

  return null;
}

function pathMatchesPrefix(segments: string[], prefix: string[]): boolean {
  return prefix.every((segment, index) => segments[index] === segment);
}

function safePathSegments(pathname: string): string[] | null {
  const rawSegments = pathname.split("/").filter(Boolean);
  const decodedSegments: string[] = [];
  for (const rawSegment of rawSegments) {
    let decoded: string;
    try {
      decoded = decodeURIComponent(rawSegment);
    } catch {
      return null;
    }
    if (decoded === "." || decoded === ".." || decoded.includes("/") || decoded.includes("\\")) {
      return null;
    }
    decodedSegments.push(decoded);
  }
  return decodedSegments;
}

async function serveObjectFromR2(
  bucket: ReleaseBucket,
  objectKey: string,
  request: Request,
  method: "GET" | "HEAD",
  origin: string | null,
  rewriteJson: boolean,
  attribution: DownloadAttributionContext,
): Promise<Response> {
  const parsedRange = method === "GET" && !rewriteJson
    ? parseRangeHeader(request.headers.get("range"))
    : null;
  if (parsedRange === "invalid") {
    return rangeNotSatisfiable(origin);
  }

  let responseRange: ResolvedResponseRange | null = null;
  let object: ReleaseObject | null;
  if (parsedRange != null) {
    const headObject = await bucket.head(objectKey);
    if (headObject == null) {
      return jsonError(404, "not_found", "release object not found", origin);
    }
    const resolvedRange = resolveResponseRange(parsedRange, headObject.size);
    if (resolvedRange == null || resolvedRange === "unsatisfiable") {
      return rangeNotSatisfiable(origin, headObject.size);
    }
    responseRange = resolvedRange;
    object = await bucket.get(objectKey, { range: parsedRange });
  } else {
    object = method === "HEAD" ? await bucket.head(objectKey) : await bucket.get(objectKey);
  }
  if (object == null) {
    return jsonError(404, "not_found", "release object not found", origin);
  }

  if (rewriteJson) {
    if (object.text == null) {
      return jsonError(500, "invalid_object", "release object is not readable as text", origin);
    }
    let manifest: unknown;
    try {
      manifest = JSON.parse(await object.text());
    } catch {
      return jsonError(502, "invalid_manifest", "release manifest is not valid JSON", origin);
    }
    return new Response(JSON.stringify(rewriteManifestDownloadUrls(manifest, attribution)), {
      status: 200,
      headers: objectHeaders(object, origin, JSON_CONTENT_TYPE, false),
    });
  }

  const headers = objectHeaders(object, origin, inferContentType(objectKey));
  let status = 200;
  if (responseRange != null) {
    status = 206;
    headers.set("content-range", responseRange.contentRange);
    headers.set("content-length", String(responseRange.contentLength));
  }
  return new Response(method === "HEAD" ? null : object.body ?? null, { status, headers });
}

function redirectDownload(route: Extract<Route, { kind: "download" }>, request: Request, env: Env, origin: string | null): Response {
  const url = new URL(request.url);
  const baseUrl = env.RELEASE_ARTIFACT_REDIRECT_BASE_URL?.trim().replace(/\/+$/, "")
    || `${url.origin}/storage/v1/object/public/releases`;
  const attribution = {
    downloadId: normalizeDownloadId(url.searchParams.get("ctx_download_id")),
    ...readAcquisitionContext(request, url),
  };
  const location = appendAttributionParamsToUrl(`${baseUrl}/${encodeObjectKey(route.objectKey)}`, attribution);
  return new Response(null, {
    status: 302,
    headers: {
      location,
      "cache-control": RELEASE_CACHE_CONTROL,
      ...corsHeaders(origin),
    },
  });
}

function objectHeaders(
  object: ReleaseObject,
  origin: string | null,
  defaultContentType: string,
  includeContentLength = true,
): Headers {
  const headers = new Headers(corsHeaders(origin));
  object.writeHttpMetadata(headers);
  if (!headers.has("content-type")) {
    headers.set("content-type", defaultContentType);
  }
  headers.set("cache-control", RELEASE_CACHE_CONTROL);
  headers.set("accept-ranges", "bytes");
  if (object.httpEtag != null) {
    headers.set("etag", object.httpEtag);
  }
  if (includeContentLength && object.size != null) {
    headers.set("content-length", String(object.size));
  }
  return headers;
}

type ResolvedResponseRange = {
  readonly contentLength: number;
  readonly contentRange: string;
};

function parseRangeHeader(headerValue: string | null): ReleaseRange | "invalid" | null {
  if (headerValue == null || headerValue.trim() === "") {
    return null;
  }
  const match = /^bytes=(\d*)-(\d*)$/u.exec(headerValue.trim());
  if (match == null) {
    return "invalid";
  }
  const startText = match[1] ?? "";
  const endText = match[2] ?? "";
  if (!startText && !endText) {
    return "invalid";
  }
  if (!startText) {
    const suffix = parseRangeInteger(endText);
    if (suffix == null || suffix <= 0) {
      return "invalid";
    }
    return { suffix };
  }
  const offset = parseRangeInteger(startText);
  if (offset == null) {
    return "invalid";
  }
  if (!endText) {
    return { offset };
  }
  const end = parseRangeInteger(endText);
  if (end == null || end < offset) {
    return "invalid";
  }
  return { offset, length: end - offset + 1 };
}

function parseRangeInteger(value: string): number | null {
  if (!/^\d+$/u.test(value)) {
    return null;
  }
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function resolveResponseRange(range: ReleaseRange, objectSize: number | undefined): ResolvedResponseRange | "unsatisfiable" | null {
  if (objectSize == null || !Number.isSafeInteger(objectSize) || objectSize < 0) {
    return null;
  }
  if (objectSize === 0) {
    return "unsatisfiable";
  }

  let start: number;
  let end: number;
  if ("suffix" in range) {
    const length = Math.min(range.suffix, objectSize);
    start = objectSize - length;
    end = objectSize - 1;
  } else {
    start = range.offset ?? 0;
    if (start >= objectSize) {
      return "unsatisfiable";
    }
    const length = range.length ?? (objectSize - start);
    end = Math.min(start + length - 1, objectSize - 1);
  }

  const contentLength = end - start + 1;
  if (contentLength <= 0) {
    return "unsatisfiable";
  }
  return {
    contentLength,
    contentRange: `bytes ${start}-${end}/${objectSize}`,
  };
}

function rangeNotSatisfiable(origin: string | null, objectSize?: number): Response {
  const headers = new Headers(corsHeaders(origin));
  headers.set("content-type", JSON_CONTENT_TYPE);
  headers.set("cache-control", RELEASE_CACHE_CONTROL);
  if (objectSize != null) {
    headers.set("content-range", `bytes */${objectSize}`);
  }
  return new Response(JSON.stringify({
    code: "range_not_satisfiable",
    message: "requested byte range is not satisfiable",
  }), {
    status: 416,
    headers,
  });
}

function inferContentType(objectKey: string): string {
  if (objectKey.endsWith(".json")) return JSON_CONTENT_TYPE;
  if (objectKey.endsWith(".json.sig") || objectKey.endsWith(".sig")) return "text/plain; charset=utf-8";
  return "application/octet-stream";
}

function corsHeaders(origin: string | null): Record<string, string> {
  return {
    "access-control-allow-origin": origin ?? "*",
    "access-control-allow-headers": "authorization, content-type, x-client-info, apikey",
    "access-control-allow-methods": "GET, HEAD, OPTIONS",
  };
}

function jsonError(
  status: number,
  error: string,
  message: string,
  origin: string | null,
  extraHeaders?: Record<string, string>,
): Response {
  return new Response(JSON.stringify({ error, message }), {
    status,
    headers: {
      "content-type": JSON_CONTENT_TYPE,
      "cache-control": RELEASE_CACHE_CONTROL,
      ...corsHeaders(origin),
      ...extraHeaders,
    },
  });
}

function normalizeDownloadId(raw: string | null): string | null {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!DOWNLOAD_ID_PATTERN.test(trimmed)) return null;
  return trimmed;
}

function normalizeAttributionValue(raw: string | null): string | null {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  return trimmed.slice(0, MAX_ATTRIBUTION_VALUE_LENGTH);
}

function normalizeReferrerDomain(raw: string | null): string | null {
  if (!raw) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  try {
    const parsed = ABSOLUTE_URL_PATTERN.test(trimmed)
      ? new URL(trimmed)
      : new URL(`https://${trimmed}`);
    const hostname = parsed.hostname.trim().toLowerCase().replace(/\.+$/, "");
    if (!hostname || hostname.length > MAX_HOST_LENGTH) return null;
    return hostname;
  } catch {
    return null;
  }
}

function readAcquisitionContext(request: Request, url: URL): AcquisitionContext {
  return {
    referrerDomain: normalizeReferrerDomain(url.searchParams.get("referrer_domain"))
      ?? normalizeReferrerDomain(request.headers.get("referer")),
    utmSource: normalizeAttributionValue(url.searchParams.get("utm_source")),
    utmMedium: normalizeAttributionValue(url.searchParams.get("utm_medium")),
    utmCampaign: normalizeAttributionValue(url.searchParams.get("utm_campaign")),
  };
}

function hasAttributionContext(context: DownloadAttributionContext): boolean {
  return Boolean(
    context.downloadId
      || context.referrerDomain
      || context.utmSource
      || context.utmMedium
      || context.utmCampaign,
  );
}

function appendAttributionParamsToUrl(urlValue: string, context: DownloadAttributionContext): string {
  if (!hasAttributionContext(context)) return urlValue;
  try {
    const parsed = new URL(urlValue, "https://ctx.invalid");
    if (context.downloadId) parsed.searchParams.set("ctx_download_id", context.downloadId);
    if (context.referrerDomain) parsed.searchParams.set("referrer_domain", context.referrerDomain);
    if (context.utmSource) parsed.searchParams.set("utm_source", context.utmSource);
    if (context.utmMedium) parsed.searchParams.set("utm_medium", context.utmMedium);
    if (context.utmCampaign) parsed.searchParams.set("utm_campaign", context.utmCampaign);
    if (ABSOLUTE_URL_PATTERN.test(urlValue)) {
      return parsed.toString();
    }
    return `${parsed.pathname}${parsed.search}${parsed.hash}`;
  } catch {
    return urlValue;
  }
}

function rewriteManifestDownloadUrls(value: unknown, attribution: DownloadAttributionContext): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => rewriteManifestDownloadUrls(entry, attribution));
  }
  if (value != null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, nested] of Object.entries(value)) {
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

function encodeObjectKey(objectKey: string): string {
  return objectKey.split("/").map(encodeURIComponent).join("/");
}
