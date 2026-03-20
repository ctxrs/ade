import type { Dispatch, SetStateAction } from "react";
import {
  authenticateProviderForWorkspace,
  getAmpLogin,
  getClaudeLogin,
  getCodexLogin,
  getCursorLogin,
  getGeminiLogin,
  getKimiLogin,
  getMistralLogin,
  getQwenLogin,
  startAmpLogin,
  startClaudeLogin,
  startCodexLogin,
  startCursorLogin,
  startGeminiLogin,
  startKimiLogin,
  startMistralLogin,
  startQwenLogin,
  upsertClaudeAccount,
  upsertCopilotAccount,
  type ClaudeAccountsResponse,
  type CodexAccountsResponse,
  type CopilotAccountsResponse,
  type GeminiAccountsResponse,
  type MistralAccountsResponse,
  type QwenAccountsResponse,
} from "../../../../api/client";
import { openExternalLink } from "../../../../utils/desktop";
import type { HarnessAuthModalState } from "../../../SettingsPage.types";
import { delayWithAbort, isCancelledOperationError } from "./operationOwner";
import {
  AMP_LOGIN_POLL_ATTEMPTS,
  AMP_LOGIN_POLL_INTERVAL_MS,
  CLAUDE_LOGIN_POLL_ATTEMPTS,
  CLAUDE_LOGIN_POLL_INTERVAL_MS,
  GEMINI_LOGIN_POLL_ATTEMPTS,
  GEMINI_LOGIN_POLL_INTERVAL_MS,
  MANUAL_BROWSER_OPEN_MESSAGE,
  MISTRAL_LOGIN_POLL_ATTEMPTS,
  MISTRAL_LOGIN_POLL_INTERVAL_MS,
  QWEN_LOGIN_POLL_ATTEMPTS,
  QWEN_LOGIN_POLL_INTERVAL_MS,
  messageFromError,
  shouldAutoOpenAmpAuthUrl,
  shouldAutoOpenKimiAuthUrl,
  shouldOpenPolledAuthUrlForStatus,
  takeNextAuthUrlToOpen,
} from "./capabilities";
import type { HarnessAuthModalOperation } from "./useHarnessAuthModalController";

type RefreshAccountsOptions = {
  silent?: boolean;
};

type ReservedBrowserWindow = Window | null;

type BrowserLoginOutcome = {
  status: "success" | "failed" | "timeout";
  error?: string | null;
};

type BrowserLoginStatus = {
  status: string;
  auth_url?: string | null;
  device_code?: string | null;
  error?: string | null;
};

type BrowserLoginDefinition = {
  providerId: "claude-crp" | "gemini" | "qwen" | "cursor" | "kimi" | "amp" | "mistral";
  waitingMessage: string;
  timeoutMessage: string;
  startLogin: (label?: string) => Promise<{ login_id: string; auth_url?: string | null; device_code?: string | null }>;
  getLogin: (loginId: string) => Promise<BrowserLoginStatus>;
  maxAttempts: number;
  pollIntervalMs: number;
  refreshAccounts?: (opts?: RefreshAccountsOptions) => Promise<unknown>;
  shouldAutoOpenAuthUrl?: (params: {
    authUrl: string;
    phase: "initial" | "poll";
    status?: BrowserLoginStatus;
  }) => boolean;
  syncBrowserLoginState?: (state: { authUrl: string | null; deviceCode: string | null }) => void;
};

type SubscriptionFlowDeps = {
  modal: HarnessAuthModalState;
  workspaceId: string | null;
  flow: HarnessAuthModalOperation;
  patchHarnessAuthModalForOperation: (
    operation: HarnessAuthModalOperation,
    patch: Partial<HarnessAuthModalState>,
  ) => boolean;
  closeHarnessAuthModalForOperation: (operation: HarnessAuthModalOperation) => boolean;
  setProviderError: Dispatch<SetStateAction<string | null>>;
  refreshBootstrapAfterMutation: (providerId?: string) => Promise<void>;
  selectSubscriptionSourceIfSupported: (providerId: string) => Promise<void>;
  refreshCodexAccounts: (opts?: RefreshAccountsOptions) => Promise<CodexAccountsResponse | null>;
  refreshClaudeAccounts: (opts?: RefreshAccountsOptions) => Promise<ClaudeAccountsResponse | null>;
  refreshGeminiAccounts: (opts?: RefreshAccountsOptions) => Promise<GeminiAccountsResponse | null>;
  refreshQwenAccounts: (opts?: RefreshAccountsOptions) => Promise<QwenAccountsResponse | null>;
  refreshKimiAccounts: (opts?: RefreshAccountsOptions) => Promise<unknown>;
  refreshCursorAccounts: (opts?: RefreshAccountsOptions) => Promise<unknown>;
  refreshAmpAccounts: (opts?: RefreshAccountsOptions) => Promise<unknown>;
  refreshMistralAccounts: (opts?: RefreshAccountsOptions) => Promise<MistralAccountsResponse | null>;
  setClaudeAccounts: Dispatch<SetStateAction<ClaudeAccountsResponse | null>>;
  setCopilotAccounts: Dispatch<SetStateAction<CopilotAccountsResponse | null>>;
  openCodexAuthUrl: (
    url: string,
    params?: { accountId: string; expectedCallbackUrl: string | null; completionToken: string | null },
  ) => Promise<void>;
};

const setFlowStatus = (
  deps: Pick<SubscriptionFlowDeps, "flow" | "patchHarnessAuthModalForOperation">,
  status: string,
): void => {
  deps.patchHarnessAuthModalForOperation(deps.flow, { subscription_status: status });
};

const setCurrentProviderError = (
  deps: Pick<SubscriptionFlowDeps, "flow" | "setProviderError">,
  message: string,
): void => {
  if (!deps.flow.isCurrent()) return;
  deps.setProviderError(message);
};

const finalizeSuccessfulSubscription = async (
  deps: Pick<
    SubscriptionFlowDeps,
    "closeHarnessAuthModalForOperation" | "flow" | "selectSubscriptionSourceIfSupported"
  >,
  providerId: string,
): Promise<void> => {
  if (!deps.flow.isCurrent()) return;
  await deps.selectSubscriptionSourceIfSupported(providerId);
  deps.closeHarnessAuthModalForOperation(deps.flow);
};

const refreshAccountsAfterFlow = async <TResponse>(
  flow: HarnessAuthModalOperation,
  refresh: ((opts?: RefreshAccountsOptions) => Promise<TResponse | null>) | undefined,
): Promise<TResponse | null> => {
  if (!refresh) return null;
  return refresh({ silent: !flow.isCurrent() });
};

const normalizeOptionalString = (value: string | null | undefined): string | null => {
  const normalized = value?.trim() ?? "";
  return normalized.length > 0 ? normalized : null;
};

const reserveBrowserWindowForFlow = (): ReservedBrowserWindow => {
  if (typeof window === "undefined") return null;
  if (typeof navigator !== "undefined" && /\bjsdom\b/i.test(navigator.userAgent)) {
    return null;
  }
  try {
    const popup = window.open("about:blank", "_blank");
    if (!popup) return null;
    try {
      popup.opener = null;
    } catch {
      // ignore browsers that forbid touching opener
    }
    return popup;
  } catch {
    return null;
  }
};

const navigateReservedBrowserWindow = (
  reservedWindow: ReservedBrowserWindow,
  authUrl: string,
): boolean => {
  if (!reservedWindow || reservedWindow.closed) return false;
  try {
    reservedWindow.location.href = authUrl;
    reservedWindow.focus?.();
    return true;
  } catch {
    return false;
  }
};

const closeReservedBrowserWindow = (reservedWindow: ReservedBrowserWindow): void => {
  if (!reservedWindow || reservedWindow.closed) return;
  try {
    reservedWindow.close();
  } catch {
    // ignore
  }
};

const waitForCodexLoginOutcome = async (
  accountId: string,
  flow: HarnessAuthModalOperation,
): Promise<"success" | "failed" | "timeout"> => {
  const attempts = 75;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    flow.throwIfCancelled();
    try {
      const status = await getCodexLogin(accountId);
      flow.throwIfCancelled();
      if (status.status === "success") return "success";
      if (status.status === "failed") return "failed";
    } catch (error) {
      if (isCancelledOperationError(error)) throw error;
    }
    await delayWithAbort(1600, flow.signal);
  }
  return "timeout";
};

const waitForBrowserLoginOutcome = async (params: {
  flow: HarnessAuthModalOperation;
  loginId: string;
  getStatus: (loginId: string) => Promise<BrowserLoginStatus>;
  onAuthUrl?: (authUrl: string, status: BrowserLoginStatus) => Promise<void>;
  onStatus?: (status: BrowserLoginStatus) => void;
  openedAuthUrl?: string | null;
  maxAttempts: number;
  intervalMs: number;
}): Promise<BrowserLoginOutcome> => {
  const openedAuthUrls = new Set<string>();
  takeNextAuthUrlToOpen(params.openedAuthUrl, openedAuthUrls);
  for (let attempt = 0; attempt < params.maxAttempts; attempt += 1) {
    params.flow.throwIfCancelled();
    try {
      const status = await params.getStatus(params.loginId);
      params.flow.throwIfCancelled();
      params.onStatus?.(status);
      if (status.status === "success") return { status: "success" };
      if (status.status === "failed") return { status: "failed", error: status.error };
      if (status.status === "timeout") return { status: "timeout", error: status.error };
      if (shouldOpenPolledAuthUrlForStatus(status.status)) {
        const authUrl = takeNextAuthUrlToOpen(status.auth_url, openedAuthUrls);
        if (authUrl && params.onAuthUrl) {
          await params.onAuthUrl(authUrl, status);
          params.flow.throwIfCancelled();
        }
      }
    } catch (error) {
      if (isCancelledOperationError(error)) throw error;
    }
    await delayWithAbort(params.intervalMs, params.flow.signal);
  }
  return { status: "timeout" };
};

const openExternalAuthUrlForFlow = async (
  deps: Pick<SubscriptionFlowDeps, "flow" | "patchHarnessAuthModalForOperation">,
  authUrl: string,
  reservedWindow: ReservedBrowserWindow = null,
): Promise<boolean> => {
  if (navigateReservedBrowserWindow(reservedWindow, authUrl)) {
    return true;
  }
  const opened = await openExternalLink(authUrl);
  if (opened) return true;
  deps.patchHarnessAuthModalForOperation(deps.flow, {
    subscription_status: MANUAL_BROWSER_OPEN_MESSAGE,
  });
  return false;
};

const runBrowserSubscriptionFlow = async (
  deps: SubscriptionFlowDeps,
  definition: BrowserLoginDefinition,
): Promise<void> => {
  definition.syncBrowserLoginState?.({ authUrl: null, deviceCode: null });
  const reservedWindow =
    definition.providerId === "amp" || definition.providerId === "claude-crp"
      ? null
      : reserveBrowserWindowForFlow();
  const label = deps.modal.subscription_label.trim();
  let reservedWindowUsed = false;
  try {
    const login = await definition.startLogin(label ? label : undefined);
    deps.flow.throwIfCancelled();

    const initialAuthUrl = takeNextAuthUrlToOpen(login.auth_url, new Set<string>());
    definition.syncBrowserLoginState?.({
      authUrl: normalizeOptionalString(login.auth_url),
      deviceCode: normalizeOptionalString(login.device_code),
    });
    let initialAuthOpened = false;
    let attemptedInitialAuthOpen = false;
    const shouldAutoOpenInitialAuthUrl = initialAuthUrl
      ? (definition.shouldAutoOpenAuthUrl?.({
        authUrl: initialAuthUrl,
        phase: "initial",
      }) ?? true)
      : false;
    if (initialAuthUrl && shouldAutoOpenInitialAuthUrl) {
      attemptedInitialAuthOpen = true;
      initialAuthOpened = await openExternalAuthUrlForFlow(deps, initialAuthUrl, reservedWindow);
      reservedWindowUsed = initialAuthOpened;
      deps.flow.throwIfCancelled();
    }

    setFlowStatus(
      deps,
      attemptedInitialAuthOpen && !initialAuthOpened && initialAuthUrl
        ? MANUAL_BROWSER_OPEN_MESSAGE
        : definition.waitingMessage,
    );

    const outcome = await waitForBrowserLoginOutcome({
      flow: deps.flow,
      loginId: login.login_id,
      getStatus: definition.getLogin,
      onAuthUrl: async (authUrl, status) => {
        if (!(definition.shouldAutoOpenAuthUrl?.({
          authUrl,
          phase: "poll",
          status,
        }) ?? true)) {
          return;
        }
        const opened = await openExternalAuthUrlForFlow(deps, authUrl, reservedWindow);
        if (opened) {
          reservedWindowUsed = true;
        }
      },
      onStatus: (status) => {
        definition.syncBrowserLoginState?.({
          authUrl: normalizeOptionalString(status.auth_url),
          deviceCode: normalizeOptionalString(status.device_code),
        });
      },
      openedAuthUrl: initialAuthOpened ? initialAuthUrl : null,
      maxAttempts: definition.maxAttempts,
      intervalMs: definition.pollIntervalMs,
    });

    await refreshAccountsAfterFlow(deps.flow, definition.refreshAccounts);

    if (!deps.flow.isCurrent()) return;

    if (outcome.status === "success") {
      await deps.refreshBootstrapAfterMutation(definition.providerId);
      if (!deps.flow.isCurrent()) return;
      await finalizeSuccessfulSubscription(deps, definition.providerId);
      return;
    }

    if (outcome.error && outcome.error.trim()) {
      deps.setProviderError(outcome.error);
    }

    if (outcome.status === "failed") {
      setFlowStatus(deps, outcome.error?.trim() || "Sign-in failed. Retry.");
      return;
    }

    if (outcome.status === "timeout") {
      setFlowStatus(deps, outcome.error?.trim() || definition.timeoutMessage);
      return;
    }

    setFlowStatus(deps, "Still waiting for completion. Keep this dialog open or retry.");
  } finally {
    if (!reservedWindowUsed) {
      closeReservedBrowserWindow(reservedWindow);
    }
  }
};

const runCodexSubscriptionFlow = async (deps: SubscriptionFlowDeps): Promise<void> => {
  const login = await startCodexLogin();
  deps.flow.throwIfCancelled();

  await deps.openCodexAuthUrl(login.auth_url, {
    accountId: login.account_id,
    expectedCallbackUrl: login.expected_callback_url ?? null,
    completionToken: login.completion_token,
  });
  deps.flow.throwIfCancelled();

  setFlowStatus(
    deps,
    "Waiting for browser sign-in to complete. You can close this dialog after finishing auth.",
  );

  const outcome = await waitForCodexLoginOutcome(login.account_id, deps.flow);
  await refreshAccountsAfterFlow(deps.flow, deps.refreshCodexAccounts);

  if (!deps.flow.isCurrent()) return;

  if (outcome === "success") {
    await deps.refreshBootstrapAfterMutation("codex");
    if (!deps.flow.isCurrent()) return;
    await finalizeSuccessfulSubscription(deps, "codex");
    return;
  }

  if (outcome === "failed") {
    setFlowStatus(deps, "Sign-in failed. Please retry or use the callback completion flow.");
    return;
  }

  setFlowStatus(
    deps,
    "Still waiting for callback completion. Continue in Harness Subscriptions if needed.",
  );
};

const runClaudeSubscriptionFlow = async (deps: SubscriptionFlowDeps): Promise<void> => {
  const token = deps.modal.subscription_token.trim();
  const label = deps.modal.subscription_label.trim();

  if (token) {
    if (!token.startsWith("sk-ant-oat")) {
      throw new Error("Claude setup token must start with sk-ant-oat.");
    }
    const next = await upsertClaudeAccount(token, label ? label : undefined);
    await deps.refreshBootstrapAfterMutation("claude-crp");
    if (!deps.flow.isCurrent()) return;
    deps.setClaudeAccounts(next);
    await finalizeSuccessfulSubscription(deps, "claude-crp");
    return;
  }

  await runBrowserSubscriptionFlow(deps, {
    providerId: "claude-crp",
    waitingMessage: "Waiting for Claude setup-token sign-in to complete in your browser...",
    timeoutMessage: "Timed out waiting for Claude setup-token completion. Retry.",
    startLogin: startClaudeLogin,
    getLogin: getClaudeLogin,
    maxAttempts: CLAUDE_LOGIN_POLL_ATTEMPTS,
    pollIntervalMs: CLAUDE_LOGIN_POLL_INTERVAL_MS,
    refreshAccounts: deps.refreshClaudeAccounts,
    shouldAutoOpenAuthUrl: () => false,
    syncBrowserLoginState: ({ authUrl }) => {
      deps.patchHarnessAuthModalForOperation(deps.flow, {
        subscription_auth_url: authUrl,
      });
    },
  });
};

const runCopilotSubscriptionFlow = async (deps: SubscriptionFlowDeps): Promise<void> => {
  const token = deps.modal.subscription_token.trim();
  if (!token) {
    throw new Error("Token is required.");
  }

  const label = deps.modal.subscription_label.trim();
  const email = deps.modal.subscription_email.trim();
  const next = await upsertCopilotAccount(token, {
    ...(label ? { label } : {}),
    ...(email ? { email } : {}),
  });

  await deps.refreshBootstrapAfterMutation("copilot");
  if (!deps.flow.isCurrent()) return;

  deps.setCopilotAccounts(next);
  await finalizeSuccessfulSubscription(deps, "copilot");
};

const runWorkspaceSubscriptionFlow = async (deps: SubscriptionFlowDeps): Promise<void> => {
  if (!deps.workspaceId) {
    throw new Error("Select a workspace first.");
  }

  await authenticateProviderForWorkspace(deps.workspaceId, deps.modal.provider_id);
  await deps.refreshBootstrapAfterMutation(deps.modal.provider_id);

  if (!deps.flow.isCurrent()) return;
  await finalizeSuccessfulSubscription(deps, deps.modal.provider_id);
};

export const runHarnessSubscriptionFlow = async (deps: SubscriptionFlowDeps): Promise<void> => {
  try {
    switch (deps.modal.provider_id) {
      case "codex":
        await runCodexSubscriptionFlow(deps);
        return;
      case "claude-crp":
        await runClaudeSubscriptionFlow(deps);
        return;
      case "gemini":
        await runBrowserSubscriptionFlow(deps, {
          providerId: "gemini",
          waitingMessage: "Waiting for Google sign-in to complete in your browser...",
          timeoutMessage: "Timed out waiting for Gemini sign-in completion. Retry.",
          startLogin: startGeminiLogin,
          getLogin: getGeminiLogin,
          maxAttempts: GEMINI_LOGIN_POLL_ATTEMPTS,
          pollIntervalMs: GEMINI_LOGIN_POLL_INTERVAL_MS,
          refreshAccounts: deps.refreshGeminiAccounts,
        });
        return;
      case "qwen":
        await runBrowserSubscriptionFlow(deps, {
          providerId: "qwen",
          waitingMessage: "Waiting for Qwen sign-in to complete in your browser...",
          timeoutMessage: "Timed out waiting for Qwen sign-in completion. Retry.",
          startLogin: startQwenLogin,
          getLogin: getQwenLogin,
          maxAttempts: QWEN_LOGIN_POLL_ATTEMPTS,
          pollIntervalMs: QWEN_LOGIN_POLL_INTERVAL_MS,
          refreshAccounts: deps.refreshQwenAccounts,
        });
        return;
      case "cursor":
        await runBrowserSubscriptionFlow(deps, {
          providerId: "cursor",
          waitingMessage: "Waiting for Cursor sign-in to complete in your browser...",
          timeoutMessage: "Timed out waiting for Cursor sign-in completion. Retry.",
          startLogin: startCursorLogin,
          getLogin: getCursorLogin,
          maxAttempts: QWEN_LOGIN_POLL_ATTEMPTS,
          pollIntervalMs: QWEN_LOGIN_POLL_INTERVAL_MS,
          refreshAccounts: deps.refreshCursorAccounts,
        });
        return;
      case "amp":
        await runBrowserSubscriptionFlow(deps, {
          providerId: "amp",
          waitingMessage: "Waiting for Amp sign-in to complete in your browser...",
          timeoutMessage: "Timed out waiting for Amp sign-in completion. Retry.",
          startLogin: startAmpLogin,
          getLogin: getAmpLogin,
          maxAttempts: AMP_LOGIN_POLL_ATTEMPTS,
          pollIntervalMs: AMP_LOGIN_POLL_INTERVAL_MS,
          refreshAccounts: deps.refreshAmpAccounts,
          shouldAutoOpenAuthUrl: () => shouldAutoOpenAmpAuthUrl(),
          syncBrowserLoginState: ({ authUrl, deviceCode }) => {
            deps.patchHarnessAuthModalForOperation(deps.flow, {
              subscription_auth_url: authUrl,
              subscription_device_code: deviceCode,
            });
          },
        });
        return;
      case "mistral":
        await runBrowserSubscriptionFlow(deps, {
          providerId: "mistral",
          waitingMessage: "Waiting for Mistral sign-in to complete in your browser...",
          timeoutMessage: "Timed out waiting for Mistral sign-in completion. Retry.",
          startLogin: startMistralLogin,
          getLogin: getMistralLogin,
          maxAttempts: MISTRAL_LOGIN_POLL_ATTEMPTS,
          pollIntervalMs: MISTRAL_LOGIN_POLL_INTERVAL_MS,
          refreshAccounts: deps.refreshMistralAccounts,
        });
        return;
      case "kimi":
        await runBrowserSubscriptionFlow(deps, {
          providerId: "kimi",
          waitingMessage: "Waiting for Kimi sign-in to complete in your browser...",
          timeoutMessage: "Timed out waiting for Kimi sign-in completion. Retry.",
          startLogin: startKimiLogin,
          getLogin: getKimiLogin,
          maxAttempts: GEMINI_LOGIN_POLL_ATTEMPTS,
          pollIntervalMs: GEMINI_LOGIN_POLL_INTERVAL_MS,
          refreshAccounts: deps.refreshKimiAccounts,
          shouldAutoOpenAuthUrl: ({ authUrl }) => shouldAutoOpenKimiAuthUrl() && authUrl.length > 0,
          syncBrowserLoginState: ({ authUrl, deviceCode }) => {
            deps.patchHarnessAuthModalForOperation(deps.flow, {
              subscription_auth_url: authUrl,
              subscription_device_code: deviceCode,
            });
          },
        });
        return;
      case "copilot":
        await runCopilotSubscriptionFlow(deps);
        return;
      default:
        await runWorkspaceSubscriptionFlow(deps);
    }
  } catch (error) {
    if (isCancelledOperationError(error)) {
      return;
    }
    const message = messageFromError(error);
    setCurrentProviderError(deps, message);
    setFlowStatus(deps, "Subscription flow failed. Check error details below.");
  }
};
