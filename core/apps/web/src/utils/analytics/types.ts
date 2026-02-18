export type AnalyticsScalar = string | number | boolean;

export type AnalyticsProperties = Record<string, AnalyticsScalar>;

export type AnalyticsSurface = "web" | "desktop" | "mobile_shell";

export type AnalyticsEnvTarget = "local" | "worktree" | "remote";
