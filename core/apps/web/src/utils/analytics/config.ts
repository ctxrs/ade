const POSTHOG_INGEST_HOST = import.meta.env.VITE_POSTHOG_INGEST_HOST ?? "https://telemetry.example.invalid";
const POSTHOG_UI_HOST = import.meta.env.VITE_POSTHOG_UI_HOST ?? "https://telemetry.example.invalid";

const POSTHOG_STAGING_PROJECT_ID = import.meta.env.VITE_POSTHOG_STAGING_PROJECT_ID ?? "ade-staging";
const POSTHOG_PRODUCTION_PROJECT_ID = import.meta.env.VITE_POSTHOG_PRODUCTION_PROJECT_ID ?? "ade-production";

const POSTHOG_STAGING_KEY = import.meta.env.VITE_POSTHOG_STAGING_KEY ?? "ADE_PUBLIC_KEY_PLACEHOLDER";
const POSTHOG_PRODUCTION_KEY = import.meta.env.VITE_POSTHOG_PRODUCTION_KEY ?? "ADE_PUBLIC_KEY_PLACEHOLDER";

export type AnalyticsEnvironment = "staging" | "production";

const readTrimmed = (value: string | undefined): string | undefined => {
  const next = value?.trim();
  return next ? next : undefined;
};

export const resolveAnalyticsEnvironment = (
  explicitEnv: string | undefined,
  mode: string | undefined,
): AnalyticsEnvironment => {
  const normalizedMode = String(mode ?? "").trim().toLowerCase();
  if (normalizedMode === "development" || normalizedMode === "dev") return "staging";
  const env = readTrimmed(explicitEnv)?.toLowerCase();
  if (env === "production") return "production";
  if (env === "staging") return "staging";
  if (normalizedMode === "production" || normalizedMode === "prod") return "production";
  return "staging";
};

export const getAnalyticsEnvironment = (): AnalyticsEnvironment =>
  resolveAnalyticsEnvironment(import.meta.env.VITE_POSTHOG_ENV, import.meta.env.MODE);

export const getPostHogProjectId = (): string =>
  getAnalyticsEnvironment() === "production"
    ? POSTHOG_PRODUCTION_PROJECT_ID
    : POSTHOG_STAGING_PROJECT_ID;

export const getPostHogHost = (): string =>
  POSTHOG_INGEST_HOST;

export const getPostHogUiHost = (): string =>
  readTrimmed(import.meta.env.VITE_POSTHOG_UI_HOST) ?? POSTHOG_UI_HOST;

export const getPostHogKey = (): string =>
  readTrimmed(import.meta.env.VITE_POSTHOG_KEY)
  ?? (getAnalyticsEnvironment() === "production" ? POSTHOG_PRODUCTION_KEY : POSTHOG_STAGING_KEY);
