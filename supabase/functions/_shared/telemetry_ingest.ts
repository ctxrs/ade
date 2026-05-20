import { sha256Hex } from "./hash.ts";

type TelemetryScalar = string | number | boolean | null;

type RawTelemetryEvent = {
  event_id?: unknown;
  event_name?: unknown;
  event_version?: unknown;
  occurred_at?: unknown;
  plane?: unknown;
  delivery?: unknown;
  origin_runtime?: unknown;
  origin_install_id?: unknown;
  app_version?: unknown;
  os?: unknown;
  arch?: unknown;
  surface?: unknown;
  env_target?: unknown;
  source?: unknown;
  properties?: unknown;
  provider_id?: unknown;
  model_id?: unknown;
  duration_ms?: unknown;
  duration_bucket?: unknown;
  status?: unknown;
  success?: unknown;
  session_root_kind?: unknown;
};

type TelemetryBatch = {
  broker_install_id?: unknown;
  broker_runtime?: unknown;
  broker_app_version?: unknown;
  broker_os?: unknown;
  broker_arch?: unknown;
  events?: unknown;
};

export type TelemetryTrafficClass =
  | "user"
  | "synthetic"
  | "internal"
  | "load_test"
  | "ci";

export type TelemetryRow = {
  event_id: string;
  install_id_hash: string | null;
  broker_install_id_hash: string | null;
  origin_install_id_hash: string;
  occurred_at: string;
  event_name: string;
  event_version: number;
  plane: "product" | "incident";
  broker_runtime: string;
  origin_runtime: string;
  source: string | null;
  analytics_environment: string | null;
  traffic_class: TelemetryTrafficClass;
  app_version: string;
  os: string;
  arch: string;
  surface: string | null;
  env_target: string | null;
  provider_id: string | null;
  model_id: string | null;
  duration_ms: number | null;
  duration_bucket: string | null;
  status: string | null;
  success: boolean | null;
  session_root_kind: string | null;
  properties: Record<string, TelemetryScalar>;
};

export type TelemetryPostHogCapture = {
  eventId: string;
  event: string;
  distinctId: string;
  properties: Record<string, unknown>;
};

export type TelemetryIngestPlan = {
  rows: TelemetryRow[];
  posthogCaptures: TelemetryPostHogCapture[];
};

export const selectPostHogCapturesForInsertedRows = (
  captures: TelemetryPostHogCapture[],
  rowsToInsert: TelemetryRow[],
): TelemetryPostHogCapture[] => {
  const insertedEventIds = new Set(rowsToInsert.map((row) => row.event_id));
  return captures.filter((capture) => insertedEventIds.has(capture.eventId));
};

export class TelemetryIngestError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string) {
    super(code);
    this.status = status;
    this.code = code;
  }
}

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
  "accountid",
  "orgid",
  "organizationid",
  "userid",
  "email",
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
const PIPELINE_SMOKE_EVENT_NAME = "analytics_pipeline_smoke";
const TRAFFIC_CLASSES = new Set<TelemetryTrafficClass>([
  "user",
  "synthetic",
  "internal",
  "load_test",
  "ci",
]);
const BATCH_KEYS = new Set([
  "broker_install_id",
  "broker_runtime",
  "broker_app_version",
  "broker_os",
  "broker_arch",
  "events",
]);
const EVENT_KEYS = new Set([
  "event_id",
  "event_name",
  "event_version",
  "occurred_at",
  "plane",
  "delivery",
  "origin_runtime",
  "origin_install_id",
  "app_version",
  "os",
  "arch",
  "surface",
  "env_target",
  "source",
  "properties",
  "provider_id",
  "model_id",
  "duration_ms",
  "duration_bucket",
  "status",
  "success",
  "session_root_kind",
]);

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

function rejectUnknownKeys(
  value: Record<string, unknown>,
  allowed: Set<string>,
  code: string,
): void {
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      throw new TelemetryIngestError(400, code);
    }
  }
}

function truncate(value: unknown, max: number): string | null {
  if (typeof value !== "string") return null;
  const s = value.trim();
  if (!s) return null;
  if (s.length <= max) return s;
  return s.slice(0, max);
}

function requireString(value: unknown, max: number, code: string): string {
  const normalized = truncate(value, max);
  if (!normalized) {
    throw new TelemetryIngestError(400, code);
  }
  return normalized;
}

function parseTimestamp(value: unknown): string {
  if (typeof value !== "string") {
    throw new TelemetryIngestError(400, "invalid_occurred_at");
  }
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    throw new TelemetryIngestError(400, "invalid_occurred_at");
  }
  return new Date(parsed).toISOString();
}

function normalizePlane(value: unknown): "product" | "incident" {
  if (value === "product" || value === "incident") return value;
  throw new TelemetryIngestError(400, "invalid_plane");
}

function normalizeDelivery(value: unknown): "remote" | "local_only" {
  if (value === "remote" || value === "local_only") return value;
  throw new TelemetryIngestError(400, "invalid_delivery");
}

function normalizeOriginRuntime(
  value: unknown,
): "web" | "desktop" | "mobile_shell" | "daemon" {
  if (
    value === "web" || value === "desktop" || value === "mobile_shell" ||
    value === "daemon"
  ) {
    return value;
  }
  throw new TelemetryIngestError(400, "invalid_origin_runtime");
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
  return FORBIDDEN_SCOPE_KEYS.has(normalized) ||
    FORBIDDEN_KEY_FRAGMENTS.some((fragment) => normalized.includes(fragment));
}

function sanitizeProperties(raw: unknown): Record<string, TelemetryScalar> {
  if (raw === null || raw === undefined) return {};
  if (!isRecord(raw)) {
    throw new TelemetryIngestError(400, "invalid_properties");
  }
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
    const normalized = truncate(value, 128);
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

function requirePositiveInteger(value: unknown, code: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new TelemetryIngestError(400, code);
  }
  const normalized = Math.floor(value);
  if (normalized <= 0) {
    throw new TelemetryIngestError(400, code);
  }
  return normalized;
}

function pickBoolean(...values: Array<unknown>): boolean | null {
  for (const value of values) {
    if (typeof value === "boolean") return value;
  }
  return null;
}

function normalizeTrafficClass(value: unknown): TelemetryTrafficClass | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim().toLowerCase();
  return TRAFFIC_CLASSES.has(normalized as TelemetryTrafficClass)
    ? normalized as TelemetryTrafficClass
    : null;
}

function classifyTraffic(
  eventName: string,
  appVersion: string,
  providerId: string | null,
  modelId: string | null,
  analyticsEnvironment: string | null,
  properties: Record<string, unknown>,
): TelemetryTrafficClass {
  if (eventName === PIPELINE_SMOKE_EVENT_NAME) return "synthetic";
  const explicit = normalizeTrafficClass(properties.traffic_class);
  if (explicit && explicit !== "user") return explicit;
  if (providerId === "fake") return "synthetic";
  if (modelId === "fake-model") return "synthetic";
  if (appVersion.startsWith("0.0.0")) return "synthetic";
  if (analyticsEnvironment && analyticsEnvironment !== "production") {
    return "internal";
  }
  return explicit ?? "user";
}

export async function buildTelemetryIngestPlan(
  payload: unknown,
  opts: { idSalt: string; now?: () => Date },
): Promise<TelemetryIngestPlan> {
  if (!opts.idSalt.trim()) {
    throw new TelemetryIngestError(500, "missing_hash_salt");
  }
  if (!isRecord(payload)) {
    throw new TelemetryIngestError(400, "bad_request");
  }
  rejectUnknownKeys(payload, BATCH_KEYS, "unknown_batch_field");
  const batch = payload as TelemetryBatch;
  if (!Array.isArray(batch.events)) {
    throw new TelemetryIngestError(400, "bad_request");
  }
  if (batch.events.length === 0) {
    throw new TelemetryIngestError(400, "empty_events");
  }
  if (batch.events.length > MAX_EVENTS) {
    throw new TelemetryIngestError(413, "too_many_events");
  }

  const brokerInstallId = requireString(
    batch.broker_install_id,
    128,
    "missing_broker_install_id",
  );
  const brokerInstallIdHash = await sha256Hex(
    `${opts.idSalt}:${brokerInstallId}`,
  );
  const brokerRuntime = requireString(
    batch.broker_runtime,
    32,
    "missing_broker_runtime",
  );
  const brokerAppVersion = requireString(
    batch.broker_app_version,
    64,
    "missing_broker_app_version",
  );
  const brokerOs = requireString(batch.broker_os, 32, "missing_broker_os");
  const brokerArch = requireString(
    batch.broker_arch,
    32,
    "missing_broker_arch",
  );

  const rows: TelemetryRow[] = [];
  const posthogCaptures: TelemetryPostHogCapture[] = [];
  const nowIso = (opts.now ?? (() => new Date()))().toISOString();

  for (const raw of batch.events) {
    if (!isRecord(raw)) {
      throw new TelemetryIngestError(400, "invalid_event");
    }
    rejectUnknownKeys(raw, EVENT_KEYS, "unknown_event_field");
    const rawEvent = raw as RawTelemetryEvent;
    const eventId = requireString(rawEvent.event_id, 128, "missing_event_id");
    const eventName = requireString(
      rawEvent.event_name,
      64,
      "missing_event_name",
    );
    const eventVersion = requirePositiveInteger(
      rawEvent.event_version,
      "invalid_event_version",
    );
    const occurredAt = parseTimestamp(rawEvent.occurred_at);
    const plane = normalizePlane(rawEvent.plane);
    const delivery = normalizeDelivery(rawEvent.delivery);
    if (delivery !== "remote") {
      throw new TelemetryIngestError(400, "invalid_remote_delivery");
    }
    const originRuntime = normalizeOriginRuntime(rawEvent.origin_runtime);
    const originInstallId = truncate(rawEvent.origin_install_id, 128) ??
      (originRuntime === "daemon" ? brokerInstallId : null);
    if (!originInstallId) {
      throw new TelemetryIngestError(400, "missing_origin_install_id");
    }
    const originInstallIdHash = await sha256Hex(
      `${opts.idSalt}:${originInstallId}`,
    );
    const properties = sanitizeProperties(rawEvent.properties);
    const surface = truncate(rawEvent.surface, 32);
    if (originRuntime !== "daemon" && !surface) {
      throw new TelemetryIngestError(400, "missing_surface");
    }
    const propertyExecutionEnvironment =
      typeof properties.execution_environment === "string"
        ? properties.execution_environment
        : null;
    const propertyEnvTarget = typeof properties.env_target === "string"
      ? properties.env_target
      : null;
    const envTarget = truncate(
      rawEvent.env_target ?? propertyExecutionEnvironment ?? propertyEnvTarget,
      32,
    );
    const providerId = pickString(
      rawEvent.provider_id,
      typeof properties.provider_id === "string"
        ? properties.provider_id
        : null,
    );
    const modelId = pickString(
      rawEvent.model_id,
      typeof properties.model_id === "string" ? properties.model_id : null,
    );
    const durationMs = pickNumber(rawEvent.duration_ms, properties.duration_ms);
    const durationBucket = pickString(
      rawEvent.duration_bucket,
      typeof properties.duration_bucket === "string"
        ? properties.duration_bucket
        : null,
    );
    const status = pickString(
      rawEvent.status,
      typeof properties.status === "string" ? properties.status : null,
    );
    const success = pickBoolean(rawEvent.success, properties.success);
    const sessionRootKind = pickString(
      rawEvent.session_root_kind,
      typeof properties.session_root_kind === "string"
        ? properties.session_root_kind
        : null,
    );
    const sourceName = pickString(
      rawEvent.source,
      typeof properties.source === "string" ? properties.source : null,
    );
    const appVersion = requireString(
      rawEvent.app_version ?? brokerAppVersion,
      64,
      "missing_app_version",
    );
    const os = requireString(rawEvent.os ?? brokerOs, 32, "missing_os");
    const arch = requireString(rawEvent.arch ?? brokerArch, 32, "missing_arch");
    const analyticsEnvironment = pickString(
      typeof properties.analytics_environment === "string"
        ? properties.analytics_environment
        : null,
    );
    const trafficClass = classifyTraffic(
      eventName,
      appVersion,
      providerId,
      modelId,
      analyticsEnvironment,
      properties,
    );

    const canonicalProperties: Record<string, TelemetryScalar> = {
      event_version: eventVersion,
      plane,
      origin_runtime: originRuntime,
      broker_runtime: brokerRuntime,
      source: sourceName,
      surface,
      provider_id: providerId,
      model_id: modelId,
      analytics_environment: analyticsEnvironment,
      traffic_class: trafficClass,
      env_target: envTarget,
      status,
      success,
      duration_ms: durationMs,
      duration_bucket: durationBucket,
      session_root_kind: sessionRootKind,
      app_version: appVersion,
      os,
      arch,
      ingested_at: nowIso,
    };
    const normalizedProperties: Record<string, TelemetryScalar> = {
      ...canonicalProperties,
      ...properties,
      ...canonicalProperties,
    };

    rows.push({
      event_id: eventId,
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
      analytics_environment: analyticsEnvironment,
      traffic_class: trafficClass,
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

    const posthogCanonicalProperties: Record<string, TelemetryScalar> = {
      origin_install_id_hash: originInstallIdHash,
      broker_install_id_hash: brokerInstallIdHash,
      ...canonicalProperties,
      source: sourceName ?? "unknown",
    };
    const posthogProperties = {
      ...posthogCanonicalProperties,
      ...normalizedProperties,
      ...posthogCanonicalProperties,
    };
    posthogCaptures.push({
      eventId,
      event: eventName,
      distinctId: `install:${originInstallIdHash}`,
      properties: posthogProperties,
    });
  }

  return { rows, posthogCaptures };
}
