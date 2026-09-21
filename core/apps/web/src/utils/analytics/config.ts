const viteEnv = import.meta.env ?? {};

const POSTHOG_INGEST_HOST = viteEnv.VITE_POSTHOG_INGEST_HOST ?? "https://telemetry.example.invalid";
const POSTHOG_UI_HOST = viteEnv.VITE_POSTHOG_UI_HOST ?? "https://telemetry.example.invalid";

const POSTHOG_STAGING_PROJECT_ID = viteEnv.VITE_POSTHOG_STAGING_PROJECT_ID ?? "ade-staging";
const POSTHOG_PRODUCTION_PROJECT_ID = viteEnv.VITE_POSTHOG_PRODUCTION_PROJECT_ID ?? "example-project";

const POSTHOG_STAGING_KEY = viteEnv.VITE_POSTHOG_STAGING_KEY ?? "ADE_PUBLIC_KEY_PLACEHOLDER";
const POSTHOG_PRODUCTION_KEY = viteEnv.VITE_POSTHOG_PRODUCTION_KEY ?? "ADE_PUBLIC_KEY_PLACEHOLDER";

export type AnalyticsEnvironment = "staging" | "production";

const readTrimmed = (value: string | undefined): string | undefined => {
  const next = value?.trim();
  return next ? next : undefined;
};

export type ProductionAnalyticsBuildConfig = {
  explicitAnalyticsEnv: string | undefined;
  explicitAppVersion: string | undefined;
  mode: string | undefined;
  packageVersion: string | undefined;
};

export const validateProductionAnalyticsBuildConfig = ({
  explicitAnalyticsEnv,
  explicitAppVersion,
  mode,
  packageVersion,
}: ProductionAnalyticsBuildConfig): void => {
  const analyticsEnv = resolveAnalyticsEnvironment(explicitAnalyticsEnv, mode, explicitAppVersion);
  if (analyticsEnv !== "production") return;

  const appVersion = readTrimmed(explicitAppVersion);
  if (!appVersion) {
    throw new Error(
      "VITE_POSTHOG_ENV=production requires VITE_CTX_APP_VERSION to be set to the desktop release version.",
    );
  }

  if (appVersion === readTrimmed(packageVersion)) {
    throw new Error(
      "VITE_POSTHOG_ENV=production cannot use the web package version as VITE_CTX_APP_VERSION.",
    );
  }
};

export const resolveAnalyticsEnvironment = (
  explicitEnv: string | undefined,
  mode: string | undefined,
  appVersion?: string | undefined,
): AnalyticsEnvironment => {
  const normalizedMode = String(mode ?? "").trim().toLowerCase();
  if (normalizedMode === "development" || normalizedMode === "dev") return "staging";
  const env = readTrimmed(explicitEnv)?.toLowerCase();
  if (env === "production") return "production";
  if (env === "staging") return "staging";
  if (normalizedMode === "production" && readTrimmed(appVersion)) return "production";
  return "staging";
};

export const getAnalyticsEnvironment = (): AnalyticsEnvironment =>
  resolveAnalyticsEnvironment(
    viteEnv.VITE_POSTHOG_ENV,
    viteEnv.MODE,
    viteEnv.VITE_CTX_APP_VERSION,
  );

export const getPostHogProjectId = (): string =>
  getAnalyticsEnvironment() === "production"
    ? POSTHOG_PRODUCTION_PROJECT_ID
    : POSTHOG_STAGING_PROJECT_ID;

export const getPostHogHost = (): string =>
  POSTHOG_INGEST_HOST;

export const getPostHogUiHost = (): string =>
  readTrimmed(viteEnv.VITE_POSTHOG_UI_HOST) ?? POSTHOG_UI_HOST;

export const getPostHogKey = (): string =>
  readTrimmed(viteEnv.VITE_POSTHOG_KEY)
  ?? (getAnalyticsEnvironment() === "production" ? POSTHOG_PRODUCTION_KEY : POSTHOG_STAGING_KEY);
