import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { sha256Hex } from "../_shared/hash.ts";
import { capturePostHogEvent } from "../_shared/posthog.ts";

type TelemetryScalar = string | number | boolean | null;

type RawTelemetryEvent = {
  event_id?: string | null;
  event_name?: string | null;
  name?: string | null;
  event_version?: number | null;
  occurred_at?: string | null;
  plane?: string | null;
  origin_runtime?: string | null;
  origin_install_id?: string | null;
  app_version?: string | null;
  os?: string | null;
  arch?: string | null;
  surface?: string | null;
  env_target?: string | null;
  source?: string | null;
  properties?: Record<string, unknown> | null;
  provider_id?: string | null;
  model_id?: string | null;
  execution_environment?: string | null;
  duration_ms?: number | null;
  duration_bucket?: string | null;
  status?: string | null;
  success?: boolean | null;
  session_root_kind?: string | null;
};

type TelemetryBatch = {
  broker_install_id?: string | null;
  install_id?: string | null;
  broker_runtime?: string | null;
  broker_app_version?: string | null;
  app_version?: string | null;
  broker_os?: string | null;
  os?: string | null;
  broker_arch?: string | null;
  arch?: string | null;
  events: RawTelemetryEvent[];
};

const MAX_EVENTS = 200;
const MAX_KEY_LENGTH = 80;
const MAX_STRING_LENGTH = 256;
const MAX_PROPERTY_COUNT = 48;
const FORBIDDEN_SCOPE_KEYS = new Set([
  "workspaceid",
  "taskid",
  "sessionid",
  "worktreeid",
  "runid",
  "turnid",
]);
const FORBIDDEN_KEY_FRAGMENTS = [
  "prompt",
  "code",
  "filepath",
  "reponame",
  "branch",
  "command",
  "token",
  "secret",
  "apikey",
  "password",
  "authorization",
  "cookie",
];

function truncate(value: string | null | undefined, max: number): string | null {
  if (!value) return null;
  const s = String(value).trim();
  if (!s) return null;
  if (s.length <= max) return s;
  return s.slice(0, max);
}

function parseTimestamp(value: string | null | undefined): string | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return null;
  return new Date(parsed).toISOString();
}

function normalizePlane(value: string | null | undefined): "product" | "incident" {
  return value === "incident" ? "incident" : "product";
}

function sanitizeScalar(value: unknown): TelemetryScalar | undefined {
  if (value === null) return null;
  if (typeof value === "boolean") return value;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") return value.slice(0, MAX_STRING_LENGTH);
  return undefined;
}

function normalizedKey(value: string): string {
  return value.replace(/[^a-z0-9]/gi, "").toLowerCase();
}

function isForbiddenPropertyKey(key: string): boolean {
  const normalized = normalizedKey(key);
  return FORBIDDEN_SCOPE_KEYS.has(normalized)
    || FORBIDDEN_KEY_FRAGMENTS.some((fragment) => normalized.includes(fragment));
}

function sanitizeProperties(raw: Record<string, unknown> | null | undefined): Record<string, TelemetryScalar> {
  if (!raw) return {};
  const out: Record<string, TelemetryScalar> = {};
  for (const [key, value] of Object.entries(raw)) {
    if (Object.keys(out).length >= MAX_PROPERTY_COUNT) break;
    const trimmedKey = key.trim();
    if (!trimmedKey || trimmedKey.length > MAX_KEY_LENGTH) continue;
    if (isForbiddenPropertyKey(trimmedKey)) continue;
    const normalized = sanitizeScalar(value);
    if (normalized === undefined) continue;
    out[trimmedKey] = normalized;
  }
  return out;
}

function pickString(...values: Array<unknown>): string | null {
  for (const value of values) {
    const normalized = truncate(typeof value === "string" ? value : null, 128);
    if (normalized) return normalized;
  }
  return null;
}

function pickNumber(...values: Array<unknown>): number | null {
  for (const value of values) {
    if (typeof value === "number" && Number.isFinite(value)) {
      return Math.max(0, Math.floor(value));
    }
  }
  return null;
}

function pickBoolean(...values: Array<unknown>): boolean | null {
  for (const value of values) {
    if (typeof value === "boolean") return value;
  }
  return null;
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

  let payload: TelemetryBatch | null = null;
  try {
    payload = await req.json();
  } catch {
    return new Response(JSON.stringify({ error: "invalid_json" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  if (!payload || !Array.isArray(payload.events)) {
    return new Response(JSON.stringify({ error: "bad_request" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const supabaseUrl = Deno.env.get("SUPABASE_URL") ?? "";
  const serviceRoleKey = Deno.env.get("SUPABASE_SERVICE_ROLE_KEY") ?? "";
  const idSalt = Deno.env.get("INSTALL_ID_HASH_SALT") ?? "local-dev";

  const candidateEvents = payload.events.slice(0, MAX_EVENTS);
  const brokerInstallId = truncate(payload.broker_install_id ?? payload.install_id ?? null, 128);
  const hasOriginInstallId = candidateEvents.some((event) =>
    Boolean(truncate(event.origin_install_id ?? null, 128))
  );
  if (!brokerInstallId && !hasOriginInstallId) {
    return new Response(JSON.stringify({ error: "missing_install_id" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }
  const brokerInstallIdHash = brokerInstallId ? await sha256Hex(`${idSalt}:${brokerInstallId}`) : null;
  const brokerRuntime = truncate(payload.broker_runtime ?? "daemon", 32);
  const brokerAppVersion = truncate(payload.broker_app_version ?? payload.app_version ?? null, 64);
  const brokerOs = truncate(payload.broker_os ?? payload.os ?? null, 32);
  const brokerArch = truncate(payload.broker_arch ?? payload.arch ?? null, 32);

  const rows = [];
  const posthogCaptures: Promise<void>[] = [];

  for (const rawEvent of candidateEvents) {
    const eventName = truncate(rawEvent.event_name ?? rawEvent.name ?? null, 64);
    if (!eventName) continue;

    const properties = sanitizeProperties(rawEvent.properties ?? null);
    const originInstallId = truncate(rawEvent.origin_install_id ?? brokerInstallId, 128);
    const originInstallIdHash = originInstallId
      ? await sha256Hex(`${idSalt}:${originInstallId}`)
      : null;
    const occurredAt = parseTimestamp(rawEvent.occurred_at) ?? new Date().toISOString();
    const eventVersion = pickNumber(rawEvent.event_version) ?? 1;
    const surface = truncate(rawEvent.surface ?? null, 32);
    const originRuntime = truncate(rawEvent.origin_runtime ?? surface ?? brokerRuntime, 32);
    const envTarget = truncate(
      rawEvent.env_target
        ?? rawEvent.execution_environment
        ?? (typeof properties.env_target === "string" ? properties.env_target : null),
      32,
    );
    const providerId = pickString(
      rawEvent.provider_id,
      typeof properties.provider_id === "string" ? properties.provider_id : null,
    );
    const modelId = pickString(
      rawEvent.model_id,
      typeof properties.model_id === "string" ? properties.model_id : null,
    );
    const durationMs = pickNumber(rawEvent.duration_ms, properties.duration_ms);
    const durationBucket = pickString(
      rawEvent.duration_bucket,
      typeof properties.duration_bucket === "string" ? properties.duration_bucket : null,
    );
    const status = pickString(
      rawEvent.status,
      typeof properties.status === "string" ? properties.status : null,
    );
    const success = pickBoolean(rawEvent.success, properties.success);
    const sessionRootKind = pickString(
      rawEvent.session_root_kind,
      typeof properties.session_root_kind === "string" ? properties.session_root_kind : null,
    );
    const sourceName = pickString(
      rawEvent.source,
      typeof properties.source === "string" ? properties.source : null,
    );
    const appVersion = truncate(rawEvent.app_version ?? brokerAppVersion, 64);
    const os = truncate(rawEvent.os ?? brokerOs, 32);
    const arch = truncate(rawEvent.arch ?? brokerArch, 32);
    const plane = normalizePlane(rawEvent.plane);

    const normalizedProperties = sanitizeProperties({
      ...properties,
      event_version: eventVersion,
      origin_runtime: originRuntime,
      source: sourceName,
      surface,
      env_target: envTarget,
      duration_bucket: durationBucket,
      session_root_kind: sessionRootKind,
    });

    rows.push({
      event_id: truncate(rawEvent.event_id ?? null, 128),
      install_id_hash: brokerInstallIdHash,
      broker_install_id_hash: brokerInstallIdHash,
      origin_install_id_hash: originInstallIdHash,
      occurred_at: occurredAt,
      event_name: eventName,
      event_version: eventVersion,
      plane,
      broker_runtime: brokerRuntime,
      origin_runtime: originRuntime,
      source: sourceName,
      app_version: appVersion,
      os,
      arch,
      surface,
      env_target: envTarget,
      provider_id: providerId,
      model_id: modelId,
      duration_ms: durationMs,
      duration_bucket: durationBucket,
      status,
      success,
      session_root_kind: sessionRootKind,
      properties: normalizedProperties,
    });

    const distinctId = originInstallIdHash
      ? `install:${originInstallIdHash}`
      : brokerInstallIdHash
        ? `install:${brokerInstallIdHash}`
        : "install:unknown";
    posthogCaptures.push(
      capturePostHogEvent({
        event: eventName,
        distinctId,
        properties: {
          plane,
          origin_runtime: originRuntime ?? "unknown",
          broker_runtime: brokerRuntime ?? "daemon",
          source: sourceName ?? "unknown",
          surface,
          env_target: envTarget,
          status,
          success,
          duration_ms: durationMs,
          duration_bucket: durationBucket,
          session_root_kind: sessionRootKind,
          app_version: appVersion,
          os,
          arch,
          ...normalizedProperties,
        },
      }).catch(() => {
        // ignore
      }),
    );
  }

  try {
    const client = createClient(supabaseUrl, serviceRoleKey, {
      auth: { persistSession: false },
    });
    if (rows.length > 0) {
      await client.from("telemetry_event").insert(rows);
    }
  } catch {
    // ignore
  }

  if (posthogCaptures.length > 0) {
    await Promise.allSettled(posthogCaptures);
  }

  return new Response(null, {
    status: 204,
    headers: {
      ...corsHeaders(origin),
    },
  });
});
