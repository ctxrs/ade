import { getAnalyticsEnvironment } from "./config";
import type { AnalyticsProperties, AnalyticsSurface } from "./types";
import { getAppShellKind } from "../runtime";

declare const __CTX_APP_VERSION__: string;

const UNKNOWN = "unknown";

const detectOs = (): string => {
  if (typeof navigator === "undefined") return UNKNOWN;
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("windows")) return "windows";
  if (ua.includes("mac os x") || ua.includes("macintosh")) return "macos";
  if (ua.includes("android")) return "android";
  if (ua.includes("iphone") || ua.includes("ipad") || ua.includes("ios")) return "ios";
  if (ua.includes("linux")) return "linux";
  return UNKNOWN;
};

const detectArch = (): string => {
  if (typeof navigator === "undefined") return UNKNOWN;
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("arm64") || ua.includes("aarch64")) return "arm64";
  if (ua.includes("x86_64") || ua.includes("win64") || ua.includes("x64")) return "x64";
  if (ua.includes("i686") || ua.includes("i386") || ua.includes("x86")) return "x86";
  return UNKNOWN;
};

const detectSurface = (): AnalyticsSurface => {
  switch (getAppShellKind()) {
    case "desktop":
      return "desktop";
    case "mobile":
      return "mobile_shell";
    default:
      return "web";
  }
};

export const getAnalyticsSurface = (): AnalyticsSurface => detectSurface();

const RESERVED_ENVELOPE_PROPERTY_KEYS = new Set([
  "app_version",
  "arch",
  "analytics_environment",
  "delivery",
  "env_target",
  "event_id",
  "event_name",
  "event_version",
  "occurred_at",
  "origin_install_id",
  "origin_runtime",
  "os",
  "plane",
  "properties",
  "source",
  "surface",
  "traffic_class",
]);

export const getAppVersion = (): string => {
  const raw = typeof __CTX_APP_VERSION__ === "string" ? __CTX_APP_VERSION__.trim() : "";
  return raw || "0.0.0";
};

const omitReservedEnvelopeProperties = (properties: AnalyticsProperties): AnalyticsProperties => {
  const out: AnalyticsProperties = {};
  for (const [key, value] of Object.entries(properties)) {
    if (!RESERVED_ENVELOPE_PROPERTY_KEYS.has(key)) out[key] = value;
  }
  return out;
};

const normalizeEnvTarget = (
  value: AnalyticsProperties[string],
): "local" | "worktree" | "remote" | null => {
  return value === "local" || value === "worktree" || value === "remote"
    ? value
    : null;
};

export const buildEventEnvelope = (
  eventVersion: number,
  properties: AnalyticsProperties = {},
): AnalyticsProperties => {
  const safeProperties = omitReservedEnvelopeProperties(properties);
  const envTarget = normalizeEnvTarget(properties.env_target);
  return {
    ...safeProperties,
    event_version: eventVersion,
    occurred_at: new Date().toISOString(),
    app_version: getAppVersion(),
    os: detectOs(),
    arch: detectArch(),
    surface: detectSurface(),
    analytics_environment: getAnalyticsEnvironment(),
    traffic_class: "user",
    ...(envTarget ? { env_target: envTarget } : {}),
  };
};
