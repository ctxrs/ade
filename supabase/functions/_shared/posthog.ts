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

const shouldCapturePostHogEvent = (
  eventName: string,
  properties: Record<string, unknown> | undefined,
): boolean => {
  if (eventName === "analytics_pipeline_smoke") return true;
  if (stringProperty(properties, "analytics_environment") !== "production") {
    return false;
  }
  if (stringProperty(properties, "traffic_class") !== "user") return false;
  if (stringProperty(properties, "origin_runtime") !== "desktop") return false;
  if (stringProperty(properties, "surface") !== "desktop") return false;
  if (stringProperty(properties, "provider_id") === "fake") return false;
  if (hasLocalBuildVersion(properties)) return false;
  return true;
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
  const projectApiKey = readTrimmedEnv("POSTHOG_PROJECT_API_KEY");
  if (!projectApiKey) return;

  const eventName = input.event.trim();
  const distinctId = input.distinctId.trim();
  if (!eventName || !distinctId) return;
  if (!shouldCapturePostHogEvent(eventName, input.properties)) return;

  const host = normalizeHost(
    readTrimmedEnv("POSTHOG_HOST") ?? DEFAULT_POSTHOG_HOST,
  );
  const payload = {
    api_key: projectApiKey,
    event: eventName,
    distinct_id: distinctId,
    properties: {
      ...sanitizeProperties(input.properties),
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
  } finally {
    clearTimeout(timeoutId);
  }
};
