type PostHogScalar = string | number | boolean;

type PostHogCaptureInput = {
  event: string;
  distinctId: string;
  properties?: Record<string, unknown>;
};

const DEFAULT_POSTHOG_HOST = "https://telemetry.example.invalid";
const DEFAULT_TIMEOUT_MS = 1500;
const MAX_KEY_LENGTH = 80;
const MAX_STRING_LENGTH = 256;
const MAX_PROPERTY_COUNT = 32;
const PIPELINE_SMOKE_EVENT_NAME = "analytics_pipeline_smoke";
const TRUTHY_ENV_VALUES = new Set(["1", "true", "yes", "on"]);
const NON_USER_TRAFFIC_CLASSES = new Set([
  "synthetic",
  "internal",
  "load_test",
  "ci",
]);
const PRODUCTION_INCIDENT_EVENT_NAMES = new Set([
  "api_error_observed",
  "runtime_error_observed",
  "session_load_fatal_observed",
]);

const readTrimmedEnv = (name: string): string | null => {
  const raw = Deno.env.get(name);
  if (!raw) return null;
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
};

const normalizeHost = (rawHost: string): string => rawHost.replace(/\/+$/, "");

const stringProperty = (
  properties: Record<string, unknown> | undefined,
  key: string,
): string | null => {
  const value = properties?.[key];
  return typeof value === "string" ? value.trim() : null;
};

const hasLocalBuildVersion = (
  properties: Record<string, unknown> | undefined,
): boolean => {
  for (const key of ["app_version", "current_version", "version"]) {
    if (stringProperty(properties, key)?.startsWith("0.0.0")) return true;
  }
  return false;
};

const isClassifiedTelemetryProperties = (
  properties: Record<string, unknown> | undefined,
): boolean => {
  return Boolean(
    stringProperty(properties, "origin_runtime") ||
      stringProperty(properties, "broker_runtime") ||
      stringProperty(properties, "plane") ||
      stringProperty(properties, "traffic_class") ||
      stringProperty(properties, "analytics_environment"),
  );
};

const isTruthyEnv = (name: string): boolean => {
  const raw = readTrimmedEnv(name);
  return raw !== null && TRUTHY_ENV_VALUES.has(raw.toLowerCase());
};

const isProductionProductTraffic = (
  eventName: string,
  properties: Record<string, unknown> | undefined,
): boolean => {
  if (eventName === PIPELINE_SMOKE_EVENT_NAME) return false;
  if (stringProperty(properties, "analytics_environment") !== "production") {
    return false;
  }
  if (stringProperty(properties, "traffic_class") !== "user") return false;
  if (stringProperty(properties, "origin_runtime") !== "desktop") return false;
  if (stringProperty(properties, "surface") !== "desktop") return false;
  if (stringProperty(properties, "provider_id") === "fake") return false;
  if (stringProperty(properties, "model_id") === "fake-model") return false;
  if (hasLocalBuildVersion(properties)) return false;
  const plane = stringProperty(properties, "plane");
  if (plane === "product") return true;
  return plane === "incident" && PRODUCTION_INCIDENT_EVENT_NAMES.has(eventName);
};

const isCanaryTraffic = (
  eventName: string,
  properties: Record<string, unknown> | undefined,
): boolean => {
  if (eventName === PIPELINE_SMOKE_EVENT_NAME) return true;
  if (!isClassifiedTelemetryProperties(properties)) return false;
  const trafficClass = stringProperty(properties, "traffic_class");
  if (trafficClass && NON_USER_TRAFFIC_CLASSES.has(trafficClass)) return true;
  const analyticsEnvironment = stringProperty(
    properties,
    "analytics_environment",
  );
  if (analyticsEnvironment && analyticsEnvironment !== "production") {
    return true;
  }
  if (stringProperty(properties, "provider_id") === "fake") return true;
  if (stringProperty(properties, "model_id") === "fake-model") return true;
  if (hasLocalBuildVersion(properties)) return true;
  return false;
};

export const postHogCaptureTargetNames = (
  eventName: string,
  properties: Record<string, unknown> | undefined,
): string[] => {
  const normalizedEventName = eventName.trim();
  const targets: string[] = [];
  if (isProductionProductTraffic(normalizedEventName, properties)) {
    targets.push("production");
  }
  if (isCanaryTraffic(normalizedEventName, properties)) {
    targets.push("canary");
  }
  if (
    normalizedEventName === PIPELINE_SMOKE_EVENT_NAME &&
    isTruthyEnv("POSTHOG_PRODUCTION_CANARY_ENABLED")
  ) {
    targets.push("production");
  }
  return Array.from(new Set(targets));
};

const readProjectApiKeyForTarget = (target: string): string | null => {
  if (target === "production") return readTrimmedEnv("POSTHOG_PROJECT_API_KEY");
  if (target === "canary") {
    return readTrimmedEnv("POSTHOG_CANARY_PROJECT_API_KEY");
  }
  return null;
};

const sanitizeProperties = (
  raw: Record<string, unknown> | undefined,
): Record<string, PostHogScalar> => {
  if (!raw) return {};
  const out: Record<string, PostHogScalar> = {};
  for (const [key, value] of Object.entries(raw)) {
    if (Object.keys(out).length >= MAX_PROPERTY_COUNT) break;
    if (!key || key.length > MAX_KEY_LENGTH) continue;
    if (typeof value === "string") {
      out[key] = value.slice(0, MAX_STRING_LENGTH);
      continue;
    }
    if (typeof value === "boolean") {
      out[key] = value;
      continue;
    }
    if (typeof value === "number" && Number.isFinite(value)) {
      out[key] = value;
    }
  }
  return out;
};

export const capturePostHogEvent = async (
  input: PostHogCaptureInput,
): Promise<void> => {
  const eventName = input.event.trim();
  const distinctId = input.distinctId.trim();
  if (!eventName || !distinctId) return;

  const targets = postHogCaptureTargetNames(eventName, input.properties)
    .map((target) => ({
      apiKey: readProjectApiKeyForTarget(target),
      target,
    }))
    .filter((target): target is { apiKey: string; target: string } =>
      target.apiKey !== null
    );
  if (targets.length === 0) return;

  const host = normalizeHost(
    readTrimmedEnv("POSTHOG_HOST") ?? DEFAULT_POSTHOG_HOST,
  );
  const sanitizedProperties = sanitizeProperties(input.properties);

  const seenTargets = new Set<string>();
  const failures: Error[] = [];
  for (const target of targets) {
    const targetIdentity = `${target.target}:${target.apiKey}`;
    if (seenTargets.has(targetIdentity)) continue;
    seenTargets.add(targetIdentity);
    const payload = {
      api_key: target.apiKey,
      event: eventName,
      distinct_id: distinctId,
      properties: {
        ...sanitizedProperties,
        posthog_target: target.target,
        source: "supabase_edge",
      },
    };

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), DEFAULT_TIMEOUT_MS);
    try {
      const response = await fetch(`${host}/capture/`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
        signal: controller.signal,
      });
      if (!response.ok) {
        throw new Error(`PostHog capture HTTP ${response.status}`);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      failures.push(new Error(`PostHog ${target.target} capture failed: ${message}`));
    } finally {
      clearTimeout(timeoutId);
    }
  }
  if (failures.length > 0) {
    throw new AggregateError(
      failures,
      `PostHog capture failed for ${failures.length} target(s)`,
    );
  }
};
