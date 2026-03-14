import type { HarnessAuthModalState } from "../../../SettingsPage.types";

const HARNESSES_WITH_ENDPOINT_CONFIG = new Set([
  "codex",
  "claude-crp",
  "cline",
  "gemini",
  "goose",
  "kimi",
  "mistral",
  "openhands",
  "opencode",
  "copilot",
  "pi",
  "qwen",
  "droid",
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
  "cline",
  "kimi",
  "mistral",
  "goose",
  "openhands",
  "opencode",
  "pi",
  "qwen",
  "droid",
]);

const HARNESSES_REQUIRING_CONCRETE_ENDPOINT_MODEL = new Set([
  "cline",
  "goose",
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

export const harnessEndpointRequiresConcreteModel = (providerId: string): boolean =>
  HARNESSES_REQUIRING_CONCRETE_ENDPOINT_MODEL.has(providerId);

type HarnessEndpointSummary = {
  model_override?: string | null;
  manual_model_ids?: string[] | null;
  model_catalog_models?: Array<{ id?: string | null }> | null;
};

const firstNonEmpty = (values: Array<string | null | undefined>): string | null => {
  for (const value of values) {
    const trimmed = value?.trim() ?? "";
    if (trimmed) return trimmed;
  }
  return null;
};

export const preferredModelIdFromEndpointSummary = (
  endpoint: HarnessEndpointSummary | null | undefined,
): string | null => {
  if (!endpoint) return null;
  return (
    firstNonEmpty([endpoint.model_override])
    ?? firstNonEmpty(endpoint.manual_model_ids ?? [])
    ?? firstNonEmpty((endpoint.model_catalog_models ?? []).map((entry) => entry.id))
  );
};

export const isOpenRouterBaseUrl = (baseUrl: string): boolean => {
  const trimmed = baseUrl.trim();
  if (!trimmed) return false;
  try {
    const parsed = new URL(trimmed);
    const hostname = parsed.hostname.toLowerCase();
    return hostname === "openrouter.ai" || hostname.endsWith(".openrouter.ai");
  } catch {
    return false;
  }
};

export const validateHarnessEndpointConfigForOwnerScope = (params: {
  ownerScopeKind: "host" | "workspace";
  providerId: string;
  baseUrl: string | null;
  manualModelIds: string[];
  existingPreferredModelId: string | null;
}): string | null => {
  if (params.ownerScopeKind !== "host") return null;
  if (params.providerId === "goose" && !isOpenRouterBaseUrl(params.baseUrl ?? "")) {
    return "Goose currently requires an OpenRouter base URL.";
  }
  if (
    harnessEndpointRequiresConcreteModel(params.providerId)
    && params.manualModelIds.length === 0
    && !(params.existingPreferredModelId?.trim())
  ) {
    return "Configure at least one manual model slug before saving this host-scoped endpoint.";
  }
  return null;
};

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

export const shouldAutoOpenAmpAuthUrl = (): boolean => false;

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

export const shouldAutoOpenKimiAuthUrl = (): boolean => true;
