import type { HarnessAuthModalState } from "../../../SettingsPage.types";

const HARNESSES_WITH_ENDPOINT_CONFIG = new Set([
  "codex",
  "claude-crp",
  "gemini",
  "kimi",
  "qwen",
  "opencode",
  "mistral",
  "goose",
  "droid",
  "openhands",
  "copilot",
  "pi",
]);

const HARNESSES_WITH_SUBSCRIPTION_AUTH = new Set([
  "codex",
  "claude-crp",
  "gemini",
  "kimi",
  "qwen",
  "cursor",
  "amp",
  "copilot",
  "auggie",
]);

// Mistral is intentionally API-key / endpoint-only for now. Revisit a
// first-class subscription/OAuth lane once the product contract is settled.

const HARNESSES_WITH_ENDPOINT_BASE_URL = new Set([
  "codex",
  "claude-crp",
  "kimi",
  "qwen",
  "opencode",
  "mistral",
  "goose",
  "pi",
  "droid",
  "openhands",
]);

const looksLikeClaudeSetupToken = (value: string): boolean => value.trim().startsWith("sk-ant-oat");

const CLAUDE_POLLED_AUTH_URL_OPEN_GRACE_MS = 5000;

export const CLAUDE_LOGIN_POLL_INTERVAL_MS = 1600;
export const CLAUDE_LOGIN_COMPLETION_TIMEOUT_MS = 15 * 60 * 1000;
export const CLAUDE_LOGIN_POLL_ATTEMPTS =
  Math.ceil(CLAUDE_LOGIN_COMPLETION_TIMEOUT_MS / CLAUDE_LOGIN_POLL_INTERVAL_MS);
export const GEMINI_LOGIN_POLL_ATTEMPTS = 90;
export const GEMINI_LOGIN_POLL_INTERVAL_MS = 1600;
export const QWEN_LOGIN_POLL_ATTEMPTS = 90;
export const QWEN_LOGIN_POLL_INTERVAL_MS = 1600;
export const AMP_LOGIN_POLL_ATTEMPTS = 90;
export const AMP_LOGIN_POLL_INTERVAL_MS = 1600;
export const MISTRAL_LOGIN_POLL_ATTEMPTS = 90;
export const MISTRAL_LOGIN_POLL_INTERVAL_MS = 1600;

export const supportsHarnessEndpointConfigStatic = (providerId: string): boolean =>
  HARNESSES_WITH_ENDPOINT_CONFIG.has(providerId);

export const supportsHarnessSubscriptionAuth = (providerId: string): boolean =>
  HARNESSES_WITH_SUBSCRIPTION_AUTH.has(providerId);

export const harnessEndpointRequiresBaseUrl = (providerId: string): boolean =>
  HARNESSES_WITH_ENDPOINT_BASE_URL.has(providerId);

export const harnessEndpointRequiresApiShape = (providerId: string): boolean =>
  HARNESSES_WITH_ENDPOINT_BASE_URL.has(providerId);

export const resolveHarnessAuthModalInitialStage = (
  providerId: string,
): HarnessAuthModalState["stage"] => {
  const supportsApiKey = providerId === "cursor" || supportsHarnessEndpointConfigStatic(providerId);
  const supportsSubscription = supportsHarnessSubscriptionAuth(providerId);
  if (supportsApiKey && !supportsSubscription) return "api_key";
  if (!supportsApiKey && supportsSubscription) return "subscription";
  return "choose";
};

export const messageFromError = (error: unknown): string => {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return String(error);
};

export const toErrorObject = (error: unknown): Error => {
  if (error instanceof Error) return error;
  return new Error(String(error));
};

export const shouldSkipDuplicateAmpLoginStart = (params: {
  providerId: string;
  ampLoginInFlight: boolean;
}): boolean => params.providerId === "amp" && params.ampLoginInFlight;

export const shouldCompleteClaudeLoginWithCallbackCode = (params: {
  providerId: string;
  subscriptionBusy: boolean;
  pendingLoginId: string | null;
  token: string;
}): boolean => {
  if (params.providerId !== "claude-crp") return false;
  if (!params.subscriptionBusy) return false;
  if (!params.pendingLoginId) return false;
  const trimmed = params.token.trim();
  if (!trimmed) return false;
  return !looksLikeClaudeSetupToken(trimmed);
};

export const shouldOpenPolledAuthUrlForStatus = (status: string): boolean =>
  status === "pending";

export const takeNextClaudeAuthUrlToOpen = (
  authUrl: string | null | undefined,
  openedAuthUrls: Set<string>,
): string | null => {
  const normalized = authUrl?.trim() ?? "";
  if (!normalized || openedAuthUrls.has(normalized)) return null;
  openedAuthUrls.add(normalized);
  return normalized;
};

export const takeNextAuthUrlToOpen = (
  authUrl: string | null | undefined,
  openedAuthUrls: Set<string>,
): string | null => {
  const normalized = authUrl?.trim() ?? "";
  if (!normalized || openedAuthUrls.has(normalized)) return null;
  openedAuthUrls.add(normalized);
  return normalized;
};

export const extractGithubDeviceCodeFromAuthUrl = (
  authUrl: string | null | undefined,
): string | null => {
  const trimmed = authUrl?.trim() ?? "";
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.hostname.toLowerCase() !== "github.com") return null;
    const path = parsed.pathname.replace(/\/+$/, "");
    if (path !== "/login/device") return null;
    const code = parsed.searchParams.get("user_code")?.trim() ?? "";
    return code || null;
  } catch {
    return null;
  }
};

export const shouldAutoOpenCopilotAuthUrl = (authUrl: string): boolean => {
  void authUrl;
  return false;
};

export const shouldOpenPolledClaudeAuthUrl = (params: {
  loginStartedAtMs: number;
  initialAuthUrl: string | null | undefined;
  polledAuthUrl: string;
  nowMs: number;
}): boolean => {
  const polled = params.polledAuthUrl.trim();
  if (!polled) return false;
  const initial = params.initialAuthUrl?.trim() ?? "";
  if (!initial) {
    return params.nowMs - params.loginStartedAtMs >= CLAUDE_POLLED_AUTH_URL_OPEN_GRACE_MS;
  }
  return polled !== initial;
};

export const shouldAutoOpenKimiAuthUrl = (): boolean => false;
