import { useCallback, useEffect, useRef, useState } from "react";
import {
  authenticateProviderForWorkspace,
  deleteClaudeAccount,
  deleteCopilotAccount,
  deleteCodexAccount,
  deleteGeminiAccount,
  deleteKimiAccount,
  deleteKiroAccount,
  deleteProviderHarnessEndpoint,
  getCodexLogin,
  getInstall,
  listClaudeAccounts,
  listCopilotAccounts,
  listGeminiAccounts,
  listKimiAccounts,
  listKiroAccounts,
  getProviderHarnessConfig,
  installAllProviders,
  installProvider,
  listCodexAccounts,
  listProviders,
  selectProviderHarnessSource,
  setClaudeActiveAccount,
  setCopilotActiveAccount,
  setCodexActiveAccount,
  setGeminiActiveAccount,
  setKimiActiveAccount,
  setKiroActiveAccount,
  startCodexLogin,
  upsertClaudeAccount,
  upsertCopilotAccount,
  upsertGeminiAccount,
  upsertKimiAccount,
  upsertKiroAccount,
  upsertProviderHarnessEndpoint,
  type ClaudeAccountsResponse,
  type CopilotAccountsResponse,
  type CodexAccountsResponse,
  type GeminiAccountsResponse,
  type HarnessProviderSourceConfig,
  type KimiAccountsResponse,
  type KiroAccountsResponse,
  type ProviderStatus,
} from "../../../api/client";
import { desktopStartCodexLoginRelay, isDesktopApp, openExternalLink } from "../../../utils/desktop";
import type { HarnessAuthModalState, InstallSession } from "../../SettingsPage.types";
import { clampPct } from "../../SettingsPage.utils";
import { defaultEndpointBaseUrlForProvider, type HarnessAuthRow } from "../harnessAuthRows";
import {
  defaultEndpointProviderPresetForHarness,
  defaultShapeForHarnessProvider,
  getHarnessEndpointProviderPreset,
  normalizeOptionalBaseUrl,
  nextDefaultEndpointName,
  nextTokenEndpointName,
} from "../harnessEndpointProviders";

type UseHarnessAuthenticationControllerArgs = {
  workspaceId: string | null;
  enabled: boolean;
};

type HarnessAuthenticationController = {
  providers: ProviderStatus[];
  installs: Record<string, InstallSession>;
  installBusy: string | null;
  onInstallAll: () => Promise<void>;
  onInstall: (providerId: string) => Promise<void>;
  providerHarnessConfig: Record<string, HarnessProviderSourceConfig | undefined>;
  providerHarnessBusy: Record<string, boolean>;
  codexAccounts: CodexAccountsResponse | null;
  codexAccountsBusy: boolean;
  claudeAccounts: ClaudeAccountsResponse | null;
  claudeAccountsBusy: boolean;
  geminiAccounts: GeminiAccountsResponse | null;
  geminiAccountsBusy: boolean;
  kimiAccounts: KimiAccountsResponse | null;
  kimiAccountsBusy: boolean;
  copilotAccounts: CopilotAccountsResponse | null;
  copilotAccountsBusy: boolean;
  kiroAccounts: KiroAccountsResponse | null;
  kiroAccountsBusy: boolean;
  harnessAuthModal: HarnessAuthModalState | null;
  openHarnessAuthModal: (providerId: string) => void;
  closeHarnessAuthModal: () => void;
  patchHarnessAuthModal: (patch: Partial<HarnessAuthModalState>) => void;
  submitHarnessSubscriptionModal: () => Promise<void>;
  submitHarnessApiKeyModal: () => Promise<void>;
  onSelectHarnessAuthRow: (providerId: string, row: HarnessAuthRow) => Promise<void>;
  onDeleteProviderEndpoint: (providerId: string, endpointId: string) => Promise<void>;
  onCodexDelete: (accountId: string) => Promise<void>;
  onClaudeDelete: (accountId: string) => Promise<void>;
  onGeminiDelete: (accountId: string) => Promise<void>;
  onKimiDelete: (accountId: string) => Promise<void>;
  onCopilotDelete: (accountId: string) => Promise<void>;
  onKiroDelete: (accountId: string) => Promise<void>;
  providerError: string | null;
  supportsHarnessEndpointConfig: (providerId: string) => boolean;
  harnessEndpointRequiresBaseUrl: (providerId: string) => boolean;
};

const HARNESSES_WITH_ENDPOINT_CONFIG = new Set([
  "codex",
  "claude-crp",
  "gemini",
  "kimi",
  "qwen",
  "opencode",
  "mistral",
  "goose",
  "cagent",
  "amp",
  "droid",
  "cody",
  "continue",
  "cline",
  "swe-agent",
  "openhands",
  "copilot",
  "kiro",
  "rovo",
  "auggie",
]);

const supportsHarnessEndpointConfig = (providerId: string): boolean =>
  HARNESSES_WITH_ENDPOINT_CONFIG.has(providerId);

const HARNESSES_WITH_ENDPOINT_BASE_URL = new Set([
  "codex",
  "claude-crp",
  "gemini",
  "kimi",
  "qwen",
  "opencode",
  "mistral",
  "goose",
  "cagent",
  "cline",
  "swe-agent",
  "openhands",
]);

const harnessEndpointRequiresBaseUrl = (providerId: string): boolean =>
  HARNESSES_WITH_ENDPOINT_BASE_URL.has(providerId);

const harnessEndpointRequiresApiShape = (providerId: string): boolean =>
  HARNESSES_WITH_ENDPOINT_BASE_URL.has(providerId);

const messageFromError = (error: unknown): string => {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return String(error);
};

export function useHarnessAuthenticationController({
  workspaceId,
  enabled,
}: UseHarnessAuthenticationControllerArgs): HarnessAuthenticationController {
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerError, setProviderError] = useState<string | null>(null);
  const [providerHarnessConfig, setProviderHarnessConfig] = useState<Record<string, HarnessProviderSourceConfig | undefined>>({});
  const [providerHarnessBusy, setProviderHarnessBusy] = useState<Record<string, boolean>>({});
  const [harnessAuthModal, setHarnessAuthModal] = useState<HarnessAuthModalState | null>(null);
  const [installBusy, setInstallBusy] = useState<string | null>(null);
  const [installs, setInstalls] = useState<Record<string, InstallSession>>({});

  const [codexAccounts, setCodexAccounts] = useState<CodexAccountsResponse | null>(null);
  const [codexAccountsBusy, setCodexAccountsBusy] = useState(false);
  const [claudeAccounts, setClaudeAccounts] = useState<ClaudeAccountsResponse | null>(null);
  const [claudeAccountsBusy, setClaudeAccountsBusy] = useState(false);
  const [geminiAccounts, setGeminiAccounts] = useState<GeminiAccountsResponse | null>(null);
  const [geminiAccountsBusy, setGeminiAccountsBusy] = useState(false);
  const [kimiAccounts, setKimiAccounts] = useState<KimiAccountsResponse | null>(null);
  const [kimiAccountsBusy, setKimiAccountsBusy] = useState(false);
  const [copilotAccounts, setCopilotAccounts] = useState<CopilotAccountsResponse | null>(null);
  const [copilotAccountsBusy, setCopilotAccountsBusy] = useState(false);
  const [kiroAccounts, setKiroAccounts] = useState<KiroAccountsResponse | null>(null);
  const [kiroAccountsBusy, setKiroAccountsBusy] = useState(false);

  const installPollTimeoutsRef = useRef<Record<string, number>>({});

  const refreshProviders = useCallback(async () => {
    try {
      const next = await listProviders();
      setProviders(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return [];
    }
  }, []);

  const refreshCodexAccounts = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setCodexAccountsBusy(true);
    }
    try {
      const next = await listCodexAccounts();
      setCodexAccounts(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setCodexAccountsBusy(false);
      }
    }
  }, []);

  const refreshClaudeAccounts = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setClaudeAccountsBusy(true);
    }
    try {
      const next = await listClaudeAccounts();
      setClaudeAccounts(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setClaudeAccountsBusy(false);
      }
    }
  }, []);

  const refreshGeminiAccounts = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setGeminiAccountsBusy(true);
    }
    try {
      const next = await listGeminiAccounts();
      setGeminiAccounts(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setGeminiAccountsBusy(false);
      }
    }
  }, []);

  const refreshKimiAccounts = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setKimiAccountsBusy(true);
    }
    try {
      const next = await listKimiAccounts();
      setKimiAccounts(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setKimiAccountsBusy(false);
      }
    }
  }, []);

  const refreshCopilotAccounts = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setCopilotAccountsBusy(true);
    }
    try {
      const next = await listCopilotAccounts();
      setCopilotAccounts(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setCopilotAccountsBusy(false);
      }
    }
  }, []);

  const refreshKiroAccounts = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setKiroAccountsBusy(true);
    }
    try {
      const next = await listKiroAccounts();
      setKiroAccounts(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setKiroAccountsBusy(false);
      }
    }
  }, []);

  const ensureProviderHarnessConfig = useCallback(
    async (providerId: string, opts?: { force?: boolean }) => {
      if (providerHarnessBusy[providerId]) return;
      if (!opts?.force && providerHarnessConfig[providerId]) return;
      setProviderHarnessBusy((prev) => ({ ...prev, [providerId]: true }));
      try {
        const cfg = await getProviderHarnessConfig(providerId);
        setProviderHarnessConfig((prev) => ({ ...prev, [providerId]: cfg }));
      } catch (error) {
        setProviderError(messageFromError(error));
      } finally {
        setProviderHarnessBusy((prev) => ({ ...prev, [providerId]: false }));
      }
    },
    [providerHarnessBusy, providerHarnessConfig],
  );

  const onDeleteProviderEndpoint = useCallback(async (providerId: string, endpointId: string) => {
    setProviderHarnessBusy((prev) => ({ ...prev, [providerId]: true }));
    setProviderError(null);
    try {
      const next = await deleteProviderHarnessEndpoint(providerId, endpointId);
      setProviderHarnessConfig((prev) => ({ ...prev, [providerId]: next }));
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setProviderHarnessBusy((prev) => ({ ...prev, [providerId]: false }));
    }
  }, []);

  const onSelectProviderSource = useCallback(
    async (providerId: string, sourceKind: "subscription" | "endpoint", endpointId?: string | null) => {
      setProviderHarnessBusy((prev) => ({ ...prev, [providerId]: true }));
      setProviderError(null);
      try {
        const next = await selectProviderHarnessSource(providerId, sourceKind, endpointId ?? null);
        setProviderHarnessConfig((prev) => ({ ...prev, [providerId]: next }));
      } catch (error) {
        setProviderError(messageFromError(error));
      } finally {
        setProviderHarnessBusy((prev) => ({ ...prev, [providerId]: false }));
      }
    },
    [],
  );

  const openHarnessAuthModal = useCallback((providerId: string) => {
    const defaultPresetId = defaultEndpointProviderPresetForHarness(providerId);
    const defaultPreset = getHarnessEndpointProviderPreset(defaultPresetId);
    const requiresBaseUrl = harnessEndpointRequiresBaseUrl(providerId);
    setProviderError(null);
    setHarnessAuthModal({
      provider_id: providerId,
      stage: "choose",
      endpoint_provider_id: defaultPresetId,
      endpoint_name: "",
      base_url: requiresBaseUrl
        ? (defaultPreset.base_url ?? defaultEndpointBaseUrlForProvider(providerId))
        : "",
      api_key: "",
      subscription_label: "",
      subscription_token: "",
      subscription_email: "",
      subscription_provider: "",
      subscription_credentials_json: "",
      subscription_config_toml: "",
      subscription_auth_token_json: "",
      subscription_oauth_creds_json: "",
      subscription_google_accounts_json: "",
      subscription_status: null,
      subscription_busy: false,
      api_key_busy: false,
    });
  }, []);

  const closeHarnessAuthModal = useCallback(() => {
    setHarnessAuthModal(null);
  }, []);

  const patchHarnessAuthModal = useCallback((patch: Partial<HarnessAuthModalState>) => {
    setHarnessAuthModal((prev) => (prev ? { ...prev, ...patch } : prev));
  }, []);

  const submitHarnessApiKeyModal = useCallback(async () => {
    const modal = harnessAuthModal;
    if (!modal || modal.stage !== "api_key") return;

    if (!supportsHarnessEndpointConfig(modal.provider_id)) {
      setProviderError("API key auth is not configurable for this harness yet.");
      return;
    }

    const requiresBaseUrl = harnessEndpointRequiresBaseUrl(modal.provider_id);
    const requiresApiShape = harnessEndpointRequiresApiShape(modal.provider_id);
    const nameInput = modal.endpoint_name.trim();
    const existingNames = (providerHarnessConfig[modal.provider_id]?.endpoints ?? []).map((endpoint) => endpoint.name);
    const name = nameInput
      || (requiresBaseUrl
        ? nextDefaultEndpointName(modal.endpoint_provider_id, existingNames)
        : nextTokenEndpointName(modal.provider_id, existingNames));
    const base = modal.base_url.trim();
    const normalizedBase = normalizeOptionalBaseUrl(base);
    const key = modal.api_key.trim();
    if (requiresBaseUrl && !base) {
      setProviderError("Endpoint base URL is required.");
      return;
    }
    if (!key) {
      setProviderError("API key is required.");
      return;
    }

    setHarnessAuthModal((prev) => (prev ? { ...prev, api_key_busy: true } : prev));
    setProviderError(null);
    try {
      const next = await upsertProviderHarnessEndpoint(modal.provider_id, {
        endpoint_id: null,
        name,
        base_url: normalizedBase,
        api_shape: requiresApiShape ? defaultShapeForHarnessProvider(modal.provider_id) : null,
        api_key: key,
      });
      const reversedEndpoints = [...next.endpoints].reverse();
      const createdEndpoint =
        reversedEndpoints.find(
          (endpoint) => endpoint.name === name && (endpoint.base_url ?? null) === normalizedBase,
        )
        ?? next.endpoints[next.endpoints.length - 1]
        ?? null;
      const selected = createdEndpoint?.id ?? next.selected_endpoint_id ?? null;
      const selectedNext = await selectProviderHarnessSource(modal.provider_id, "endpoint", selected);
      setProviderHarnessConfig((prev) => ({ ...prev, [modal.provider_id]: selectedNext }));
      closeHarnessAuthModal();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setHarnessAuthModal((prev) => (prev ? { ...prev, api_key_busy: false } : prev));
    }
  }, [closeHarnessAuthModal, harnessAuthModal, providerHarnessConfig]);

  const waitForCodexLoginOutcome = useCallback(async (accountId: string): Promise<"success" | "failed" | "timeout"> => {
    const attempts = 75;
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      try {
        const status = await getCodexLogin(accountId);
        if (status.status === "success") return "success";
        if (status.status === "failed") return "failed";
      } catch {
        // continue polling
      }
      await new Promise((resolve) => {
        window.setTimeout(resolve, 1600);
      });
    }
    return "timeout";
  }, []);

  const tryStartCodexDesktopRelay = useCallback(async (params: {
    accountId: string;
    expectedCallbackUrl?: string | null;
    completionToken?: string | null;
  }) => {
    if (!isDesktopApp()) return false;
    if (!params.expectedCallbackUrl) return false;
    if (!params.completionToken) return false;
    try {
      return await desktopStartCodexLoginRelay({
        login_id: params.accountId,
        callback_url: params.expectedCallbackUrl,
        completion_token: params.completionToken,
      });
    } catch {
      return false;
    }
  }, []);

  const openCodexAuthUrl = useCallback(
    async (
      url: string,
      params?: { accountId: string; expectedCallbackUrl: string | null; completionToken: string | null },
    ) => {
      if (!url) return;
      if (params) {
        await tryStartCodexDesktopRelay(params);
      }
      await openExternalLink(url);
    },
    [tryStartCodexDesktopRelay],
  );

  const submitHarnessSubscriptionModal = useCallback(async () => {
    const modal = harnessAuthModal;
    if (!modal) return;
    if (modal.stage !== "subscription") {
      setHarnessAuthModal((prev) => (prev ? { ...prev, stage: "subscription" } : prev));
    }

    setHarnessAuthModal((prev) =>
      prev ? { ...prev, subscription_busy: true, subscription_status: "Starting subscription flow..." } : prev);
    setProviderError(null);
    try {
      if (modal.provider_id === "codex") {
        const res = await startCodexLogin();
        await openCodexAuthUrl(res.auth_url, {
          accountId: res.account_id,
          expectedCallbackUrl: res.expected_callback_url ?? null,
          completionToken: res.completion_token,
        });
        setHarnessAuthModal((prev) =>
          prev
            ? {
                ...prev,
                subscription_status: "Waiting for browser sign-in to complete. You can close this dialog after finishing auth.",
              }
            : prev);
        const outcome = await waitForCodexLoginOutcome(res.account_id);
        await refreshCodexAccounts();
        if (outcome === "success") {
          await onSelectProviderSource("codex", "subscription", null);
          closeHarnessAuthModal();
          return;
        }
        if (outcome === "failed") {
          setHarnessAuthModal((prev) =>
            prev ? { ...prev, subscription_status: "Sign-in failed. Please retry or use the callback completion flow." } : prev);
          return;
        }
        setHarnessAuthModal((prev) =>
          prev
            ? {
                ...prev,
                subscription_status: "Still waiting for callback completion. Continue in Harness Subscriptions if needed.",
              }
            : prev);
        return;
      }

      if (modal.provider_id === "claude-crp") {
        const token = modal.subscription_token.trim();
        if (!token) {
          throw new Error("Subscription token is required.");
        }
        const label = modal.subscription_label.trim();
        const next = await upsertClaudeAccount(token, label ? label : undefined);
        setClaudeAccounts(next);
        if (supportsHarnessEndpointConfig(modal.provider_id)) {
          await onSelectProviderSource(modal.provider_id, "subscription", null);
        }
        closeHarnessAuthModal();
        return;
      }

      if (modal.provider_id === "gemini") {
        const oauthCredsJson = modal.subscription_oauth_creds_json.trim();
        if (!oauthCredsJson) {
          throw new Error("OAuth credentials JSON is required.");
        }
        const googleAccountsJson = modal.subscription_google_accounts_json.trim();
        const label = modal.subscription_label.trim();
        const email = modal.subscription_email.trim();
        const next = await upsertGeminiAccount(oauthCredsJson, {
          ...(label ? { label } : {}),
          ...(googleAccountsJson ? { googleAccountsJson } : {}),
          ...(email ? { email } : {}),
        });
        setGeminiAccounts(next);
        if (supportsHarnessEndpointConfig(modal.provider_id)) {
          await onSelectProviderSource(modal.provider_id, "subscription", null);
        }
        closeHarnessAuthModal();
        return;
      }

      if (modal.provider_id === "kimi") {
        const credentialsJson = modal.subscription_credentials_json.trim();
        if (!credentialsJson) {
          throw new Error("Credentials JSON is required.");
        }
        const provider = modal.subscription_provider.trim();
        const configToml = modal.subscription_config_toml.trim();
        const label = modal.subscription_label.trim();
        const email = modal.subscription_email.trim();
        const next = await upsertKimiAccount(credentialsJson, {
          ...(label ? { label } : {}),
          ...(provider ? { provider } : {}),
          ...(configToml ? { configToml } : {}),
          ...(email ? { email } : {}),
        });
        setKimiAccounts(next);
        if (supportsHarnessEndpointConfig(modal.provider_id)) {
          await onSelectProviderSource(modal.provider_id, "subscription", null);
        }
        closeHarnessAuthModal();
        return;
      }

      if (modal.provider_id === "copilot") {
        const token = modal.subscription_token.trim();
        if (!token) {
          throw new Error("Token is required.");
        }
        const label = modal.subscription_label.trim();
        const email = modal.subscription_email.trim();
        const next = await upsertCopilotAccount(token, {
          ...(label ? { label } : {}),
          ...(email ? { email } : {}),
        });
        setCopilotAccounts(next);
        if (supportsHarnessEndpointConfig(modal.provider_id)) {
          await onSelectProviderSource(modal.provider_id, "subscription", null);
        }
        closeHarnessAuthModal();
        return;
      }

      if (modal.provider_id === "kiro") {
        const authTokenJson = modal.subscription_auth_token_json.trim();
        if (!authTokenJson) {
          throw new Error("Auth token JSON is required.");
        }
        const label = modal.subscription_label.trim();
        const email = modal.subscription_email.trim();
        const next = await upsertKiroAccount(authTokenJson, {
          ...(label ? { label } : {}),
          ...(email ? { email } : {}),
        });
        setKiroAccounts(next);
        if (supportsHarnessEndpointConfig(modal.provider_id)) {
          await onSelectProviderSource(modal.provider_id, "subscription", null);
        }
        closeHarnessAuthModal();
        return;
      }

      if (!workspaceId) {
        throw new Error("Select a workspace first.");
      }
      await authenticateProviderForWorkspace(workspaceId, modal.provider_id);
      if (supportsHarnessEndpointConfig(modal.provider_id)) {
        await onSelectProviderSource(modal.provider_id, "subscription", null);
      }
      closeHarnessAuthModal();
    } catch (error) {
      const message = messageFromError(error);
      setProviderError(message);
      setHarnessAuthModal((prev) =>
        prev ? { ...prev, subscription_status: "Subscription flow failed. Check error details below." } : prev);
    } finally {
      setHarnessAuthModal((prev) => (prev ? { ...prev, subscription_busy: false } : prev));
    }
  }, [
    closeHarnessAuthModal,
    harnessAuthModal,
    onSelectProviderSource,
    openCodexAuthUrl,
    refreshCodexAccounts,
    waitForCodexLoginOutcome,
    workspaceId,
  ]);

  const onCodexDelete = useCallback(async (accountId: string) => {
    setCodexAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteCodexAccount(accountId);
      setCodexAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCodexAccountsBusy(false);
    }
  }, []);

  const onCodexSetActive = useCallback(async (accountId: string | null) => {
    setCodexAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setCodexActiveAccount(accountId);
      setCodexAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCodexAccountsBusy(false);
    }
  }, []);

  const onClaudeDelete = useCallback(async (accountId: string) => {
    setClaudeAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteClaudeAccount(accountId);
      setClaudeAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setClaudeAccountsBusy(false);
    }
  }, []);

  const onClaudeSetActive = useCallback(async (accountId: string | null) => {
    setClaudeAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setClaudeActiveAccount(accountId);
      setClaudeAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setClaudeAccountsBusy(false);
    }
  }, []);

  const onGeminiDelete = useCallback(async (accountId: string) => {
    setGeminiAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteGeminiAccount(accountId);
      setGeminiAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setGeminiAccountsBusy(false);
    }
  }, []);

  const onGeminiSetActive = useCallback(async (accountId: string | null) => {
    setGeminiAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setGeminiActiveAccount(accountId);
      setGeminiAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setGeminiAccountsBusy(false);
    }
  }, []);

  const onKimiDelete = useCallback(async (accountId: string) => {
    setKimiAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteKimiAccount(accountId);
      setKimiAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setKimiAccountsBusy(false);
    }
  }, []);

  const onKimiSetActive = useCallback(async (accountId: string | null) => {
    setKimiAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setKimiActiveAccount(accountId);
      setKimiAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setKimiAccountsBusy(false);
    }
  }, []);

  const onCopilotDelete = useCallback(async (accountId: string) => {
    setCopilotAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteCopilotAccount(accountId);
      setCopilotAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCopilotAccountsBusy(false);
    }
  }, []);

  const onCopilotSetActive = useCallback(async (accountId: string | null) => {
    setCopilotAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setCopilotActiveAccount(accountId);
      setCopilotAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCopilotAccountsBusy(false);
    }
  }, []);

  const onKiroDelete = useCallback(async (accountId: string) => {
    setKiroAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteKiroAccount(accountId);
      setKiroAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setKiroAccountsBusy(false);
    }
  }, []);

  const onKiroSetActive = useCallback(async (accountId: string | null) => {
    setKiroAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setKiroActiveAccount(accountId);
      setKiroAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setKiroAccountsBusy(false);
    }
  }, []);

  const onSelectHarnessAuthRow = useCallback(async (providerId: string, row: HarnessAuthRow) => {
    if (!row.selectable) return;
    if (row.kind === "api_key" && row.endpoint_id) {
      await onSelectProviderSource(providerId, "endpoint", row.endpoint_id);
      return;
    }
    if (providerId === "codex" && row.account_id) {
      await onCodexSetActive(row.account_id);
    } else if (providerId === "claude-crp" && row.account_id) {
      await onClaudeSetActive(row.account_id);
    } else if (providerId === "gemini" && row.account_id) {
      await onGeminiSetActive(row.account_id);
    } else if (providerId === "kimi" && row.account_id) {
      await onKimiSetActive(row.account_id);
    } else if (providerId === "copilot" && row.account_id) {
      await onCopilotSetActive(row.account_id);
    } else if (providerId === "kiro" && row.account_id) {
      await onKiroSetActive(row.account_id);
    }
    if (supportsHarnessEndpointConfig(providerId)) {
      await onSelectProviderSource(providerId, "subscription", null);
    }
  }, [
    onClaudeSetActive,
    onCodexSetActive,
    onCopilotSetActive,
    onGeminiSetActive,
    onKimiSetActive,
    onKiroSetActive,
    onSelectProviderSource,
  ]);

  const attachInstall = useCallback(async (providerId: string, installId: string) => {
    if (!providerId || !installId) return;
    if (installPollTimeoutsRef.current[providerId]) return;

    setInstalls((prev) => ({
      ...prev,
      [providerId]: {
        installId,
        state: "running",
        pct: prev[providerId]?.pct ?? null,
        streamError: prev[providerId]?.streamError,
        error: prev[providerId]?.error,
      },
    }));

    const poll = async () => {
      try {
        const info = await getInstall(installId);
        const pct =
          typeof info.last_event?.bytes === "number"
          && typeof info.last_event?.total_bytes === "number"
          && info.last_event.total_bytes > 0
            ? clampPct(Math.round((info.last_event.bytes / info.last_event.total_bytes) * 100))
            : null;
        setInstalls((prev) => ({
          ...prev,
          [providerId]: {
            installId,
            state: info.state,
            pct,
            streamError: prev[providerId]?.streamError,
            error: info.error,
          },
        }));
        if (info.state !== "running") {
          const timeout = installPollTimeoutsRef.current[providerId];
          if (timeout) {
            window.clearTimeout(timeout);
            delete installPollTimeoutsRef.current[providerId];
          }
          const nextProviders = await refreshProviders();
          if (workspaceId) {
            for (const provider of nextProviders) {
              if (provider.details?.ui_hidden === "true") continue;
              if (supportsHarnessEndpointConfig(provider.provider_id)) {
                ensureProviderHarnessConfig(provider.provider_id, { force: true }).catch(() => {});
              }
            }
          }
          return;
        }
      } catch {
        // continue polling on transient failures
      }
      installPollTimeoutsRef.current[providerId] = window.setTimeout(() => {
        poll().catch(() => {});
      }, 900);
    };

    await poll();
  }, [ensureProviderHarnessConfig, refreshProviders, workspaceId]);

  const onInstall = useCallback(async (providerId: string) => {
    setInstallBusy(providerId);
    setProviderError(null);
    try {
      const { install_id } = await installProvider(providerId);
      await attachInstall(providerId, install_id);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setInstallBusy(null);
    }
  }, [attachInstall]);

  const onInstallAll = useCallback(async () => {
    setInstallBusy("all");
    setProviderError(null);
    try {
      const started = await installAllProviders();
      for (const install of started) {
        attachInstall(install.provider_id, install.install_id).catch(() => {});
      }
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setInstallBusy(null);
    }
  }, [attachInstall]);

  useEffect(() => {
    if (!enabled) return;
    refreshProviders().catch(() => {});
  }, [enabled, refreshProviders]);

  useEffect(() => {
    if (!enabled || !workspaceId) return;
    refreshCodexAccounts({ silent: true }).catch(() => {});
    refreshClaudeAccounts({ silent: true }).catch(() => {});
    refreshGeminiAccounts({ silent: true }).catch(() => {});
    refreshKimiAccounts({ silent: true }).catch(() => {});
    refreshCopilotAccounts({ silent: true }).catch(() => {});
    refreshKiroAccounts({ silent: true }).catch(() => {});
    for (const provider of providers) {
      if (provider.details?.ui_hidden === "true") continue;
      if (supportsHarnessEndpointConfig(provider.provider_id)) {
        ensureProviderHarnessConfig(provider.provider_id).catch(() => {});
      }
    }
  }, [
    enabled,
    ensureProviderHarnessConfig,
    providers,
    refreshClaudeAccounts,
    refreshCodexAccounts,
    refreshCopilotAccounts,
    refreshGeminiAccounts,
    refreshKimiAccounts,
    refreshKiroAccounts,
    workspaceId,
  ]);

  useEffect(() => {
    if (!enabled) return;
    for (const provider of providers) {
      const installId = provider.details?.install_id;
      const running = provider.details?.install_running === "true";
      if (running && installId && !installs[provider.provider_id]) {
        attachInstall(provider.provider_id, installId).catch(() => {});
      }
    }
  }, [attachInstall, enabled, installs, providers]);

  useEffect(() => {
    if (!enabled) return;
    const pending = codexAccounts?.logins?.some((login) => login.status === "pending");
    if (!pending) return;
    const interval = window.setInterval(() => {
      refreshCodexAccounts({ silent: true }).catch(() => {});
    }, 2000);
    return () => window.clearInterval(interval);
  }, [codexAccounts, enabled, refreshCodexAccounts]);

  useEffect(() => {
    if (enabled) return;
    setHarnessAuthModal(null);
  }, [enabled]);

  useEffect(() => {
    return () => {
      for (const key of Object.keys(installPollTimeoutsRef.current)) {
        window.clearTimeout(installPollTimeoutsRef.current[key]);
      }
      installPollTimeoutsRef.current = {};
    };
  }, []);

  return {
    providers,
    installs,
    installBusy,
    onInstallAll,
    onInstall,
    providerHarnessConfig,
    providerHarnessBusy,
    codexAccounts,
    codexAccountsBusy,
    claudeAccounts,
    claudeAccountsBusy,
    geminiAccounts,
    geminiAccountsBusy,
    kimiAccounts,
    kimiAccountsBusy,
    copilotAccounts,
    copilotAccountsBusy,
    kiroAccounts,
    kiroAccountsBusy,
    harnessAuthModal,
    openHarnessAuthModal,
    closeHarnessAuthModal,
    patchHarnessAuthModal,
    submitHarnessSubscriptionModal,
    submitHarnessApiKeyModal,
    onSelectHarnessAuthRow,
    onDeleteProviderEndpoint,
    onCodexDelete,
    onClaudeDelete,
    onGeminiDelete,
    onKimiDelete,
    onCopilotDelete,
    onKiroDelete,
    providerError,
    supportsHarnessEndpointConfig,
    harnessEndpointRequiresBaseUrl,
  };
}
