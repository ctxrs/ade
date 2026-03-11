import type { Dispatch, SetStateAction } from "react";
import {
  authenticateProviderForWorkspace,
  getAmpLogin,
  getClaudeLogin,
  getCodexLogin,
  getCursorLogin,
  getGeminiLogin,
  getMistralLogin,
  getQwenLogin,
  startAmpLogin,
  startClaudeLogin,
  startCodexLogin,
  startCursorLogin,
  startGeminiLogin,
  startMistralLogin,
  startQwenLogin,
  upsertClaudeAccount,
  upsertCopilotAccount,
  upsertKimiAccount,
  type ClaudeAccountsResponse,
  type CodexAccountsResponse,
  type CopilotAccountsResponse,
  type GeminiAccountsResponse,
  type KimiAccountsResponse,
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
  MISTRAL_LOGIN_POLL_ATTEMPTS,
  MISTRAL_LOGIN_POLL_INTERVAL_MS,
  QWEN_LOGIN_POLL_ATTEMPTS,
  QWEN_LOGIN_POLL_INTERVAL_MS,
  messageFromError,
  shouldOpenPolledAuthUrlForStatus,
  shouldOpenPolledClaudeAuthUrl,
  takeNextAuthUrlToOpen,
  takeNextClaudeAuthUrlToOpen,
} from "./capabilities";
import type { HarnessAuthModalOperation } from "./useHarnessAuthModalController";

type RefreshAccountsOptions = {
  silent?: boolean;
};

type BrowserLoginOutcome = {
  status: "success" | "failed" | "timeout";
  error?: string | null;
};

type BrowserLoginStatus = {
  status: string;
  auth_url?: string | null;
  error?: string | null;
};

type BrowserLoginDefinition = {
  providerId: "gemini" | "qwen" | "cursor" | "amp" | "mistral";
  waitingMessage: string;
  timeoutMessage: string;
  startLogin: (label?: string) => Promise<{ login_id: string; auth_url?: string | null }>;
  getLogin: (loginId: string) => Promise<BrowserLoginStatus>;
  maxAttempts: number;
  pollIntervalMs: number;
  refreshAccounts?: (opts?: RefreshAccountsOptions) => Promise<unknown>;
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
  setClaudePendingLoginIdForOperation: (
    operation: HarnessAuthModalOperation,
    loginId: string | null,
  ) => boolean;
  setProviderError: Dispatch<SetStateAction<string | null>>;
  refreshBootstrapAfterMutation: (providerId?: string) => Promise<void>;
  selectSubscriptionSourceIfSupported: (providerId: string) => Promise<void>;
  refreshCodexAccounts: (opts?: RefreshAccountsOptions) => Promise<CodexAccountsResponse | null>;
  refreshClaudeAccounts: (opts?: RefreshAccountsOptions) => Promise<ClaudeAccountsResponse | null>;
  refreshGeminiAccounts: (opts?: RefreshAccountsOptions) => Promise<GeminiAccountsResponse | null>;
  refreshQwenAccounts: (opts?: RefreshAccountsOptions) => Promise<QwenAccountsResponse | null>;
  refreshCursorAccounts: (opts?: RefreshAccountsOptions) => Promise<unknown>;
  refreshAmpAccounts: (opts?: RefreshAccountsOptions) => Promise<unknown>;
  refreshMistralAccounts: (opts?: RefreshAccountsOptions) => Promise<MistralAccountsResponse | null>;
  setClaudeAccounts: Dispatch<SetStateAction<ClaudeAccountsResponse | null>>;
  setKimiAccounts: Dispatch<SetStateAction<KimiAccountsResponse | null>>;
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

const waitForClaudeLoginOutcome = async (
  loginId: string,
  flow: HarnessAuthModalOperation,
  onAuthUrl?: (authUrl: string) => Promise<void>,
  opts?: { openedAuthUrl?: string | null },
): Promise<"success" | "failed" | "timeout"> => {
  const openedAuthUrls = new Set<string>();
  takeNextClaudeAuthUrlToOpen(opts?.openedAuthUrl, openedAuthUrls);
  for (let attempt = 0; attempt < CLAUDE_LOGIN_POLL_ATTEMPTS; attempt += 1) {
    flow.throwIfCancelled();
    try {
      const status = await getClaudeLogin(loginId);
      flow.throwIfCancelled();
      const authUrl = takeNextClaudeAuthUrlToOpen(status.auth_url, openedAuthUrls);
      if (authUrl && onAuthUrl) {
        await onAuthUrl(authUrl);
        flow.throwIfCancelled();
      }
      if (status.status === "success") return "success";
      if (status.status === "failed") return "failed";
    } catch (error) {
      if (isCancelledOperationError(error)) throw error;
    }
    await delayWithAbort(CLAUDE_LOGIN_POLL_INTERVAL_MS, flow.signal);
  }
  return "timeout";
};

const waitForBrowserLoginOutcome = async (params: {
  flow: HarnessAuthModalOperation;
  loginId: string;
  getStatus: (loginId: string) => Promise<BrowserLoginStatus>;
  onAuthUrl?: (authUrl: string) => Promise<void>;
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
      if (status.status === "success") return { status: "success" };
      if (status.status === "failed") return { status: "failed", error: status.error };
      if (status.status === "timeout") return { status: "timeout", error: status.error };
      if (shouldOpenPolledAuthUrlForStatus(status.status)) {
        const authUrl = takeNextAuthUrlToOpen(status.auth_url, openedAuthUrls);
        if (authUrl && params.onAuthUrl) {
          await params.onAuthUrl(authUrl);
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
): Promise<boolean> => {
  const opened = await openExternalLink(authUrl);
  if (opened) return true;
  deps.patchHarnessAuthModalForOperation(deps.flow, {
    subscription_status: `Couldn't open browser automatically. Open this URL manually: ${authUrl}`,
  });
  return false;
};

const runBrowserSubscriptionFlow = async (
  deps: SubscriptionFlowDeps,
  definition: BrowserLoginDefinition,
): Promise<void> => {
  const label = deps.modal.subscription_label.trim();
  const login = await definition.startLogin(label ? label : undefined);
  deps.flow.throwIfCancelled();

  const initialAuthUrl = takeNextAuthUrlToOpen(login.auth_url, new Set<string>());
  let initialAuthOpened = false;
  if (initialAuthUrl) {
    initialAuthOpened = await openExternalAuthUrlForFlow(deps, initialAuthUrl);
    deps.flow.throwIfCancelled();
  }

  setFlowStatus(
    deps,
    initialAuthUrl && !initialAuthOpened
      ? `Couldn't open browser automatically. Open this URL manually: ${initialAuthUrl}`
      : definition.waitingMessage,
  );

  const outcome = await waitForBrowserLoginOutcome({
    flow: deps.flow,
    loginId: login.login_id,
    getStatus: definition.getLogin,
    onAuthUrl: async (authUrl) => {
      await openExternalAuthUrlForFlow(deps, authUrl);
    },
    openedAuthUrl: initialAuthUrl,
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
    const next = await upsertClaudeAccount(token, label ? label : undefined);
    await deps.refreshBootstrapAfterMutation("claude-crp");
    if (!deps.flow.isCurrent()) return;
    deps.setClaudeAccounts(next);
    await finalizeSuccessfulSubscription(deps, "claude-crp");
    return;
  }

  const login = await startClaudeLogin(label ? label : undefined);
  deps.flow.throwIfCancelled();
  deps.setClaudePendingLoginIdForOperation(deps.flow, login.login_id);

  const loginStartedAtMs = Date.now();
  const initialAuthUrl = takeNextClaudeAuthUrlToOpen(login.auth_url, new Set<string>());
  if (initialAuthUrl) {
    await openExternalLink(initialAuthUrl);
    deps.flow.throwIfCancelled();
  }

  setFlowStatus(
    deps,
    "Waiting for browser sign-in. If Claude shows a token, paste it here and press Enter.",
  );

  const outcome = await waitForClaudeLoginOutcome(
    login.login_id,
    deps.flow,
    async (authUrl) => {
      if (!shouldOpenPolledClaudeAuthUrl({
        loginStartedAtMs,
        initialAuthUrl,
        polledAuthUrl: authUrl,
        nowMs: Date.now(),
      })) {
        return;
      }
      await openExternalLink(authUrl);
    },
    { openedAuthUrl: initialAuthUrl },
  );

  deps.setClaudePendingLoginIdForOperation(deps.flow, null);
  await refreshAccountsAfterFlow(deps.flow, deps.refreshClaudeAccounts);

  if (!deps.flow.isCurrent()) return;

  if (outcome === "success") {
    await deps.refreshBootstrapAfterMutation("claude-crp");
    if (!deps.flow.isCurrent()) return;
    await finalizeSuccessfulSubscription(deps, "claude-crp");
    return;
  }

  if (outcome === "failed") {
    setFlowStatus(deps, "Sign-in failed. Retry, or paste a token in the field above.");
    return;
  }

  setFlowStatus(deps, "Still waiting for completion. Keep this dialog open or retry.");
};

const runKimiSubscriptionFlow = async (deps: SubscriptionFlowDeps): Promise<void> => {
  const credentialsJson = deps.modal.subscription_credentials_json.trim();
  if (!credentialsJson) {
    throw new Error("Credentials JSON is required.");
  }

  const provider = deps.modal.subscription_provider.trim();
  const configToml = deps.modal.subscription_config_toml.trim();
  const label = deps.modal.subscription_label.trim();
  const email = deps.modal.subscription_email.trim();
  const next = await upsertKimiAccount(credentialsJson, {
    ...(label ? { label } : {}),
    ...(provider ? { provider } : {}),
    ...(configToml ? { configToml } : {}),
    ...(email ? { email } : {}),
  });

  await deps.refreshBootstrapAfterMutation("kimi");
  if (!deps.flow.isCurrent()) return;

  deps.setKimiAccounts(next);
  await finalizeSuccessfulSubscription(deps, "kimi");
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
        await runKimiSubscriptionFlow(deps);
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
