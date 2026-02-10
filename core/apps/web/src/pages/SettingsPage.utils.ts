import type { ProviderUsageSnapshot } from "../api/client";
import { desktopSaveTextFile, isDesktopApp } from "../utils/desktop";
import { SECTIONS } from "./SettingsPage.constants";
import type { SectionId } from "./SettingsPage.types";

export const saveTextFile = async (name: string, contents: string) => {
  if (isDesktopApp()) {
    await desktopSaveTextFile({ suggested_name: name, contents });
    return;
  }
  const blob = new Blob([contents], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  try {
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    a.rel = "noopener";
    a.click();
  } finally {
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
};

export function sectionFromHash(hash: string): SectionId | null {
  const raw = String(hash || "").replace(/^#/, "").trim();
  if (!raw) return null;
  const match = SECTIONS.find((s) => s.id === raw);
  if (!match) return null;
  if (match.id === "dev_tools" && !import.meta.env.DEV) return null;
  return (match.id ?? null) as any;
}

export function clampPct(n: number): number {
  if (!Number.isFinite(n)) return 0;
  return Math.max(0, Math.min(100, n));
}

export function formatPct(value?: number | null): string {
  if (!Number.isFinite(value)) return "—";
  return `${Math.round(value as number)}%`;
}

export function formatBytes(value?: number | null): string {
  if (!Number.isFinite(value)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let idx = 0;
  let v = value as number;
  while (v >= 1024 && idx < units.length - 1) {
    v /= 1024;
    idx += 1;
  }
  const precision = v >= 100 ? 0 : v >= 10 ? 1 : 2;
  return `${v.toFixed(precision)} ${units[idx]}`;
}

export function formatAge(ms?: number | null): string {
  if (!Number.isFinite(ms)) return "—";
  const totalSeconds = Math.max(0, Math.round((ms as number) / 1000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const mins = Math.floor(totalSeconds / 60);
  const secs = totalSeconds % 60;
  return `${mins}m ${secs}s`;
}

export function isLinuxPlatform(): boolean {
  if (typeof navigator === "undefined") return false;
  const platform = navigator.platform?.toLowerCase() ?? "";
  const agent = navigator.userAgent?.toLowerCase() ?? "";
  return platform.includes("linux") || agent.includes("linux");
}

export function codexResetAtMs(window?: any): number | null {
  if (!window) return null;
  if (Number.isFinite(window.reset_at)) return (window.reset_at as number) * 1000;
  if (Number.isFinite(window.resetAt)) return (window.resetAt as number) * 1000;
  if (Number.isFinite(window.reset_after_seconds)) {
    return Date.now() + (window.reset_after_seconds as number) * 1000;
  }
  if (Number.isFinite(window.resetAfterSeconds)) {
    return Date.now() + (window.resetAfterSeconds as number) * 1000;
  }
  return null;
}

export function codexRemainingPct(window?: any): number | null {
  if (!window) return null;
  if (Number.isFinite(window.remaining_percent)) return clampPct(window.remaining_percent as number);
  if (Number.isFinite(window.remainingPercent)) return clampPct(window.remainingPercent as number);
  if (Number.isFinite(window.used_percent)) return clampPct(100 - (window.used_percent as number));
  if (Number.isFinite(window.usedPercent)) return clampPct(100 - (window.usedPercent as number));
  return null;
}

export function formatResetLabel(resetAtMs?: number | null): string {
  if (!Number.isFinite(resetAtMs)) return "Reset time unavailable";
  const date = new Date(resetAtMs as number);
  if (!Number.isFinite(date.getTime())) return "Reset time unavailable";
  const now = new Date();
  const options: Intl.DateTimeFormatOptions = {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  };
  if (date.getFullYear() !== now.getFullYear()) {
    options.year = "numeric";
  }
  return `Resets ${date.toLocaleString(undefined, options)}`;
}

type CodexUsageSummary = {
  planType: string | null;
  primaryRemaining: number | null;
  secondaryRemaining: number | null;
  primaryResetAt: number | null;
  secondaryResetAt: number | null;
  creditsValue: string;
  creditsSub: string;
  updatedLabel: string;
  source: string | null;
  error: string | null;
};

export function summarizeCodexUsage(snapshot?: ProviderUsageSnapshot | null): CodexUsageSummary {
  const payload = (snapshot?.payload ?? null) as any;
  const planType = payload?.plan_type ?? payload?.planType ?? null;
  const rateLimit = payload?.rate_limit ?? payload?.rateLimit ?? null;
  const primaryWindow = rateLimit?.primary_window ?? rateLimit?.primaryWindow ?? null;
  const secondaryWindow = rateLimit?.secondary_window ?? rateLimit?.secondaryWindow ?? null;
  const credits = payload?.credits ?? null;
  const primaryRemaining = codexRemainingPct(primaryWindow);
  const secondaryRemaining = codexRemainingPct(secondaryWindow);
  const primaryResetAt = codexResetAtMs(primaryWindow);
  const secondaryResetAt = codexResetAtMs(secondaryWindow);
  const creditsValue = (() => {
    if (!credits) return "—";
    if (credits.unlimited) return "Unlimited";
    if (credits.balance !== undefined && credits.balance !== null) {
      return String(credits.balance);
    }
    if (credits.has_credits === false) return "None";
    return "—";
  })();
  const creditsSub = (() => {
    if (!credits) return "Credits unavailable";
    if (credits.unlimited) return "No spend cap";
    if (credits.has_credits === false) return "Credits exhausted";
    return "Credits balance";
  })();
  const updatedLabel = (() => {
    if (!snapshot?.fetched_at) return "";
    const ts = Date.parse(snapshot.fetched_at);
    if (!Number.isFinite(ts)) return "";
    return `Updated ${formatAge(Date.now() - ts)} ago`;
  })();

  return {
    planType,
    primaryRemaining,
    secondaryRemaining,
    primaryResetAt,
    secondaryResetAt,
    creditsValue,
    creditsSub,
    updatedLabel,
    source: snapshot?.source ?? null,
    error: snapshot?.error ?? null,
  };
}

export function formatGiB(mb?: number | null): string {
  if (!Number.isFinite(mb) || !mb) return "";
  const gb = (mb as number) / 1024;
  const precision = gb >= 10 ? 0 : 1;
  return gb.toFixed(precision);
}

export function parseGiB(value: string): number | null {
  const v = Number(value);
  if (!Number.isFinite(v) || v <= 0) return null;
  return Math.round(v * 1024);
}

export function truncateText(value: string, maxLen: number): string {
  const s = String(value ?? "");
  if (s.length <= maxLen) return s;
  return `${s.slice(0, Math.max(0, maxLen - 1))}…`;
}

export function guessAttachmentName(source: string): string {
  let cleaned = String(source ?? "").trim();
  if (!cleaned) return "";
  cleaned = cleaned.replace(/[\\/]+$/, "");
  const slashIdx = Math.max(cleaned.lastIndexOf("/"), cleaned.lastIndexOf(":"));
  let name = slashIdx >= 0 ? cleaned.slice(slashIdx + 1) : cleaned;
  if (name.endsWith(".git")) name = name.slice(0, -4);
  return name;
}
