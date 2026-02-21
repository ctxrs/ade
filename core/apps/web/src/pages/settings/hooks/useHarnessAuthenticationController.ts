import { useCallback, useEffect, useRef, useState } from "react";
import {
  authenticateProviderForWorkspace,
  completeClaudeLogin,
  deleteAmpAccount,
  deleteClaudeAccount,
  deleteCopilotAccount,
  deleteCodexAccount,
  deleteCursorAccount,
  deleteGeminiAccount,
  deleteKimiAccount,
  deleteKiroAccount,
  deleteProviderHarnessEndpoint,
  listAmpAccounts,
  getAmpLogin,
  getClaudeLogin,
  getCodexLogin,
  getGeminiLogin,
  getInstall,
  listClaudeAccounts,
  listCopilotAccounts,
  listGeminiAccounts,
  listKimiAccounts,
  listKiroAccounts,
  installAllProviders,
  installProvider,
  listCodexAccounts,
  listCursorAccounts,
  listProviders,
  refreshProviderHarnessEndpointModels,
  selectProviderHarnessSource,
  setClaudeActiveAccount,
  setCopilotActiveAccount,
  setCodexActiveAccount,
  setCursorActiveAccount,
  setAmpActiveAccount,
  setGeminiActiveAccount,
  setKimiActiveAccount,
  setKiroActiveAccount,
  startCodexLogin,
  startAmpLogin,
  startClaudeLogin,
  startGeminiLogin,
  upsertClaudeAccount,
  upsertCopilotAccount,
  upsertCursorAccount,
  upsertKimiAccount,
  upsertKiroAccount,
  upsertProviderHarnessEndpoint,
  verifyProviderForWorkspace,
  type ClaudeAccountsResponse,
  type CopilotAccountsResponse,
  type CodexAccountsResponse,
  type CursorAccountsResponse,
  type AmpAccountsResponse,
  type GeminiAccountsResponse,
  type HarnessEndpointRecord,
  type HarnessProviderSourceConfig,
  type KimiAccountsResponse,
  type KiroAccountsResponse,
  type ProviderStatus,
} from "../../../api/client";
import { desktopStartCodexLoginRelay, isDesktopApp, openExternalLink } from "../../../utils/desktop";
import {
  invalidateProvidersBootstrap,
  loadProvidersBootstrap,
  refreshProvidersBootstrap,
} from "../../../state/providersBootstrapStore";
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
  cursorAccounts: CursorAccountsResponse | null;
  cursorAccountsBusy: boolean;
  ampAccounts: AmpAccountsResponse | null;
  ampAccountsBusy: boolean;
  harnessAuthModal: HarnessAuthModalState | null;
  openHarnessAuthModal: (providerId: string) => void;
  closeHarnessAuthModal: () => void;
  patchHarnessAuthModal: (patch: Partial<HarnessAuthModalState>) => void;
  submitHarnessSubscriptionModal: () => Promise<void>;
  submitHarnessApiKeyModal: () => Promise<void>;
  onSelectHarnessAuthRow: (providerId: string, row: HarnessAuthRow) => Promise<void>;
  onDeleteProviderEndpoint: (providerId: string, endpointId: string) => Promise<void>;
  onRefreshProviderEndpointModels: (providerId: string, endpointId: string) => Promise<void>;
  onCodexDelete: (accountId: string) => Promise<void>;
  onClaudeDelete: (accountId: string) => Promise<void>;
  onGeminiDelete: (accountId: string) => Promise<void>;
  onKimiDelete: (accountId: string) => Promise<void>;
  onCopilotDelete: (accountId: string) => Promise<void>;
  onKiroDelete: (accountId: string) => Promise<void>;
  onCursorDelete: (accountId: string) => Promise<void>;
  onAmpDelete: (accountId: string) => Promise<void>;
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
  "pi",
]);

const supportsHarnessEndpointConfigStatic = (providerId: string): boolean =>
  HARNESSES_WITH_ENDPOINT_CONFIG.has(providerId);

const HARNESSES_WITH_ENDPOINT_BASE_URL = new Set([
  "codex",
  "claude-crp",
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

export const toErrorObject = (error: unknown): Error => {
  if (error instanceof Error) return error;
  return new Error(String(error));
};

const looksLikeClaudeSetupToken = (value: string): boolean => value.trim().startsWith("sk-ant-oat");
const CLAUDE_POLLED_AUTH_URL_OPEN_GRACE_MS = 5000;
const GEMINI_LOGIN_POLL_ATTEMPTS = 90;
const GEMINI_LOGIN_POLL_INTERVAL_MS = 1600;
const AMP_LOGIN_POLL_ATTEMPTS = 90;
const AMP_LOGIN_POLL_INTERVAL_MS = 1600;

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

const takeNextAuthUrlToOpen = (
  authUrl: string | null | undefined,
  openedAuthUrls: Set<string>,
): string | null => {
  const normalized = authUrl?.trim() ?? "";
  if (!normalized || openedAuthUrls.has(normalized)) return null;
  openedAuthUrls.add(normalized);
  return normalized;
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

type ResolveUpsertedEndpointArgs = {
  requestedEndpointId: string | null;
  previousEndpointIds: Set<string>;
  nextEndpoints: HarnessEndpointRecord[];
  name: string;
  normalizedBase: string | null;
  geminiAuthType: "gemini_api_key" | "vertex_ai" | null;
};

export const resolveUpsertedEndpoint = ({
  requestedEndpointId,
  previousEndpointIds,
  nextEndpoints,
  name,
  normalizedBase,
  geminiAuthType,
}: ResolveUpsertedEndpointArgs): HarnessEndpointRecord | null => {
  if (requestedEndpointId) {
    return nextEndpoints.find((endpoint) => endpoint.id === requestedEndpointId) ?? null;
  }

  const newlyAdded = nextEndpoints.filter((endpoint) => !previousEndpointIds.has(endpoint.id));
  if (newlyAdded.length === 1) {
    return newlyAdded[0];
  }

  const reversedEndpoints = [...nextEndpoints].reverse();
  return reversedEndpoints.find(
    (endpoint) => endpoint.name === name
      && (endpoint.base_url ?? null) === normalizedBase
      && (geminiAuthType === null || endpoint.auth_type === geminiAuthType),
  )
    ?? nextEndpoints[nextEndpoints.length - 1]
    ?? null;
};

export function useHarnessAuthenticationController({
  workspaceId,
  enabled,
}: UseHarnessAuthenticationControllerArgs): HarnessAuthenticationController {
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerError, setProviderError] = useState<string | null>(null);
  const [providerHarnessConfig, setProviderHarnessConfig] = useState<Record<string, HarnessProviderSourceConfig | undefined>>({});
  const [providerHarnessBusy, setProviderHarnessBusy] = useState<Record<string, boolean>>({});
  const [providerEndpointUnsupported, setProviderEndpointUnsupported] = useState<Record<string, boolean>>({});
  const [harnessAuthModal, setHarnessAuthModal] = useState<HarnessAuthModalState | null>(null);
  const [claudePendingLoginId, setClaudePendingLoginId] = useState<string | null>(null);
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
  const [cursorAccounts, setCursorAccounts] = useState<CursorAccountsResponse | null>(null);
  const [cursorAccountsBusy, setCursorAccountsBusy] = useState(false);
  const [ampAccounts, setAmpAccounts] = useState<AmpAccountsResponse | null>(null);
  const [ampAccountsBusy, setAmpAccountsBusy] = useState(false);

  const installPollTimeoutsRef = useRef<Record<string, number>>({});
  const providerHarnessConfigRef = useRef<Record<string, HarnessProviderSourceConfig | undefined>>({});
  const providerHarnessBusyRef = useRef<Record<string, boolean>>({});
  const providerEndpointUnsupportedRef = useRef<Record<string, boolean>>({});
  const ampLoginInFlightRef = useRef(false);

  const supportsHarnessEndpointConfig = useCallback(
    (providerId: string): boolean =>
      supportsHarnessEndpointConfigStatic(providerId) && providerEndpointUnsupportedRef.current[providerId] !== true,
    [],
  );

  useEffect(() => {
    providerHarnessConfigRef.current = providerHarnessConfig;
  }, [providerHarnessConfig]);

  useEffect(() => {
    providerHarnessBusyRef.current = providerHarnessBusy;
  }, [providerHarnessBusy]);

  useEffect(() => {
    providerEndpointUnsupportedRef.current = providerEndpointUnsupported;
  }, [providerEndpointUnsupported]);

  const markProviderEndpointUnsupported = useCallback((providerId: string) => {
    setProviderEndpointUnsupported((prev) => {
      if (prev[providerId]) return prev;
      const next = { ...prev, [providerId]: true };
      providerEndpointUnsupportedRef.current = next;
      return next;
    });
  }, []);

  const setProviderHarnessConfigForProvider = useCallback(
    (providerId: string, nextConfig: HarnessProviderSourceConfig) => {
      setProviderHarnessConfig((prev) => {
        const next = { ...prev, [providerId]: nextConfig };
        providerHarnessConfigRef.current = next;
        return next;
      });
    },
    [],
  );

  const setSubscriptionSourceFallback = useCallback(
    (providerId: string) => {
      setProviderHarnessConfigForProvider(providerId, {
        provider_id: providerId,
        selected_source_kind: "subscription",
        selected_endpoint_id: null,
        endpoints: providerHarnessConfigRef.current[providerId]?.endpoints ?? [],
      });
    },
    [setProviderHarnessConfigForProvider],
  );

  const setProviderHarnessBusyForProvider = useCallback((providerId: string, busy: boolean) => {
    providerHarnessBusyRef.current = { ...providerHarnessBusyRef.current, [providerId]: busy };
    setProviderHarnessBusy((prev) => ({ ...prev, [providerId]: busy }));
  }, []);

  const applyProvidersBootstrap = useCallback((bootstrap: Awaited<ReturnType<typeof loadProvidersBootstrap>>) => {
    setProviders(bootstrap.providers);
    setProviderHarnessConfig(bootstrap.provider_harness_config);
    providerHarnessConfigRef.current = bootstrap.provider_harness_config;
    setCodexAccounts(bootstrap.codex_accounts);
    setClaudeAccounts(bootstrap.claude_accounts);
    setGeminiAccounts(bootstrap.gemini_accounts);
    setKimiAccounts(bootstrap.kimi_accounts);
    setCopilotAccounts(bootstrap.copilot_accounts);
    setKiroAccounts(bootstrap.kiro_accounts);
    setCursorAccounts(bootstrap.cursor_accounts);
    setAmpAccounts(bootstrap.amp_accounts);
  }, []);

  const refreshProvidersBootstrapState = useCallback(async (opts?: { force?: boolean; silent?: boolean }) => {
    if (!workspaceId) return null;
    try {
      const bootstrap = opts?.force
        ? await refreshProvidersBootstrap(workspaceId)
        : await loadProvidersBootstrap(workspaceId);
      applyProvidersBootstrap(bootstrap);
      return bootstrap;
    } catch (error) {
      if (!opts?.silent) {
        setProviderError(messageFromError(error));
      }
      return null;
    }
  }, [applyProvidersBootstrap, workspaceId]);

  const refreshBootstrapAfterMutation = useCallback(async () => {
    if (!workspaceId) return;
    invalidateProvidersBootstrap(workspaceId);
    await refreshProvidersBootstrapState({ force: true, silent: true });
  }, [refreshProvidersBootstrapState, workspaceId]);

  const refreshProviders = useCallback(async () => {
    if (workspaceId) {
      const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: true });
      return bootstrap?.providers ?? [];
    }
    try {
      const next = await listProviders();
      setProviders(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return [];
    }
  }, [refreshProvidersBootstrapState, workspaceId]);

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

  const refreshCursorAccounts = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setCursorAccountsBusy(true);
    }
    try {
      const next = await listCursorAccounts();
      setCursorAccounts(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setCursorAccountsBusy(false);
      }
    }
  }, []);

  const refreshAmpAccounts = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setAmpAccountsBusy(true);
    }
    try {
      const next = await listAmpAccounts();
      setAmpAccounts(next);
      return next;
    } catch (error) {
      setProviderError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setAmpAccountsBusy(false);
      }
    }
  }, []);
  const onDeleteProviderEndpoint = useCallback(async (providerId: string, endpointId: string) => {
    setProviderHarnessBusyForProvider(providerId, true);
    setProviderError(null);
    try {
      const next = await deleteProviderHarnessEndpoint(providerId, endpointId);
      setProviderHarnessConfigForProvider(providerId, next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setProviderHarnessBusyForProvider(providerId, false);
    }
  }, [refreshBootstrapAfterMutation, setProviderHarnessBusyForProvider, setProviderHarnessConfigForProvider]);

  const onRefreshProviderEndpointModels = useCallback(async (providerId: string, endpointId: string) => {
    setProviderHarnessBusyForProvider(providerId, true);
    setProviderError(null);
    try {
      const next = await refreshProviderHarnessEndpointModels(providerId, endpointId);
      setProviderHarnessConfigForProvider(providerId, next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setProviderHarnessBusyForProvider(providerId, false);
    }
  }, [refreshBootstrapAfterMutation, setProviderHarnessBusyForProvider, setProviderHarnessConfigForProvider]);

  const onSelectProviderSource = useCallback(
    async (providerId: string, sourceKind: "subscription" | "endpoint", endpointId?: string | null) => {
      setProviderHarnessBusyForProvider(providerId, true);
      setProviderError(null);
      try {
        const next = await selectProviderHarnessSource(providerId, sourceKind, endpointId ?? null);
        setProviderHarnessConfigForProvider(providerId, next);
        await refreshBootstrapAfterMutation();
      } catch (error) {
        setProviderError(messageFromError(error));
        throw toErrorObject(error);
      } finally {
        setProviderHarnessBusyForProvider(providerId, false);
      }
    },
    [refreshBootstrapAfterMutation, setProviderHarnessBusyForProvider, setProviderHarnessConfigForProvider],
  );

  const selectSubscriptionSourceIfSupported = useCallback(
    async (providerId: string) => {
      if (!supportsHarnessEndpointConfig(providerId)) return;
      try {
        await onSelectProviderSource(providerId, "subscription", null);
      } catch (error) {
        const message = messageFromError(error);
        if (message.includes("provider does not support harness endpoints")) {
          markProviderEndpointUnsupported(providerId);
          setSubscriptionSourceFallback(providerId);
          return;
        }
        throw error;
      }
    },
    [
      markProviderEndpointUnsupported,
      onSelectProviderSource,
      setSubscriptionSourceFallback,
      supportsHarnessEndpointConfig,
    ],
  );

  const openHarnessAuthModal = useCallback((providerId: string) => {
    const defaultPresetId = defaultEndpointProviderPresetForHarness(providerId);
    const defaultPreset = getHarnessEndpointProviderPreset(defaultPresetId);
    const requiresBaseUrl = harnessEndpointRequiresBaseUrl(providerId);
    setProviderError(null);
    setHarnessAuthModal({
      provider_id: providerId,
      stage: "choose",
      endpoint_id: null,
      endpoint_provider_id: defaultPresetId,
      gemini_endpoint_auth_type: "gemini_api_key",
      endpoint_name: "",
      base_url: requiresBaseUrl
        ? (defaultPreset.base_url ?? defaultEndpointBaseUrlForProvider(providerId))
        : "",
      api_key: "",
      manual_model_ids: "",
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
    setClaudePendingLoginId(null);
  }, []);

  const patchHarnessAuthModal = useCallback((patch: Partial<HarnessAuthModalState>) => {
    setHarnessAuthModal((prev) => (prev ? { ...prev, ...patch } : prev));
  }, []);

  const submitHarnessApiKeyModal = useCallback(async () => {
    const modal = harnessAuthModal;
    if (!modal || modal.stage !== "api_key") return;

    const isCursor = modal.provider_id === "cursor";
    if (!isCursor && !supportsHarnessEndpointConfig(modal.provider_id)) {
      setProviderError("API key auth is not configurable for this harness yet.");
      return;
    }

    const requiresBaseUrl = harnessEndpointRequiresBaseUrl(modal.provider_id);
    const requiresApiShape = harnessEndpointRequiresApiShape(modal.provider_id);
    const nameInput = modal.endpoint_name.trim();
    const existingNames = (providerHarnessConfigRef.current[modal.provider_id]?.endpoints ?? []).map((endpoint) => endpoint.name);
    const name = nameInput
      || (requiresBaseUrl
        ? nextDefaultEndpointName(modal.endpoint_provider_id, existingNames)
        : nextTokenEndpointName(modal.provider_id, existingNames));
    const geminiAuthType = modal.provider_id === "gemini" ? modal.gemini_endpoint_auth_type : null;
    const base = modal.base_url.trim();
    const normalizedBase = normalizeOptionalBaseUrl(base);
    const key = modal.api_key.trim();
    const manualModelIds = modal.manual_model_ids
      .split(/[\n,]/)
      .map((value) => value.trim())
      .filter((value) => value.length > 0);
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
      if (isCursor) {
        const label = name.trim();
        const next = await upsertCursorAccount(key, label ? { label } : undefined);
        setCursorAccounts(next);
        await refreshBootstrapAfterMutation();
        await selectSubscriptionSourceIfSupported(modal.provider_id);
        setSubscriptionSourceFallback(modal.provider_id);
        closeHarnessAuthModal();
        return;
      }

      const previousEndpointIds = new Set(
        (providerHarnessConfigRef.current[modal.provider_id]?.endpoints ?? []).map((endpoint) => endpoint.id),
      );
      const requestedEndpointId = modal.endpoint_id?.trim() || null;
      const next = await upsertProviderHarnessEndpoint(modal.provider_id, {
        endpoint_id: requestedEndpointId,
        name,
        base_url: normalizedBase,
        api_shape: requiresApiShape ? defaultShapeForHarnessProvider(modal.provider_id) : null,
        auth_type: geminiAuthType,
        api_key: key,
        manual_model_ids: manualModelIds,
      });
      const upsertedEndpoint = resolveUpsertedEndpoint({
        requestedEndpointId,
        previousEndpointIds,
        nextEndpoints: next.endpoints,
        name,
        normalizedBase,
        geminiAuthType,
      });
      const selected = upsertedEndpoint?.id ?? next.selected_endpoint_id ?? requestedEndpointId ?? null;
      setHarnessAuthModal((prev) => {
        if (!prev || prev.provider_id !== modal.provider_id || prev.stage !== "api_key") return prev;
        return { ...prev, endpoint_id: selected };
      });
      await onSelectProviderSource(modal.provider_id, "endpoint", selected);
      if (workspaceId) {
        const verify = await verifyProviderForWorkspace(workspaceId, modal.provider_id);
        if (verify.status !== "ok") {
          setProviderError(
            verify.message?.trim()
            || `Endpoint verification failed for ${modal.provider_id} (${verify.status}).`,
          );
          return;
        }
      }
      closeHarnessAuthModal();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setHarnessAuthModal((prev) => (prev ? { ...prev, api_key_busy: false } : prev));
    }
  }, [
    closeHarnessAuthModal,
    harnessAuthModal,
    onSelectProviderSource,
    refreshBootstrapAfterMutation,
    selectSubscriptionSourceIfSupported,
    setSubscriptionSourceFallback,
    workspaceId,
  ]);

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

  const waitForClaudeLoginOutcome = useCallback(async (
    loginId: string,
    onAuthUrl?: (authUrl: string) => Promise<void>,
    opts?: { openedAuthUrl?: string | null },
  ): Promise<"success" | "failed" | "timeout"> => {
    const attempts = 90;
    const openedAuthUrls = new Set<string>();
    takeNextClaudeAuthUrlToOpen(opts?.openedAuthUrl, openedAuthUrls);
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      try {
        const status = await getClaudeLogin(loginId);
        const authUrl = takeNextClaudeAuthUrlToOpen(status.auth_url, openedAuthUrls);
        if (authUrl && onAuthUrl) {
          await onAuthUrl(authUrl);
        }
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

  const waitForGeminiLoginOutcome = useCallback(async (
    loginId: string,
    onAuthUrl?: (authUrl: string) => Promise<void>,
    opts?: { openedAuthUrl?: string | null },
  ): Promise<{ status: "success" | "failed" | "timeout"; error?: string | null }> => {
    const openedAuthUrls = new Set<string>();
    takeNextAuthUrlToOpen(opts?.openedAuthUrl, openedAuthUrls);
    for (let attempt = 0; attempt < GEMINI_LOGIN_POLL_ATTEMPTS; attempt += 1) {
      try {
        const status = await getGeminiLogin(loginId);
        if (status.status === "success") return { status: "success" };
        if (status.status === "failed") return { status: "failed", error: status.error };
        if (status.status === "timeout") return { status: "timeout", error: status.error };
        if (shouldOpenPolledAuthUrlForStatus(status.status)) {
          const authUrl = takeNextAuthUrlToOpen(status.auth_url, openedAuthUrls);
          if (authUrl && onAuthUrl) {
            await onAuthUrl(authUrl);
          }
        }
      } catch {
        // continue polling
      }
      await new Promise((resolve) => {
        window.setTimeout(resolve, GEMINI_LOGIN_POLL_INTERVAL_MS);
      });
    }
    return { status: "timeout" };
  }, []);

  const waitForAmpLoginOutcome = useCallback(async (
    loginId: string,
    onAuthUrl?: (authUrl: string) => Promise<void>,
    opts?: { openedAuthUrl?: string | null },
  ): Promise<{ status: "success" | "failed" | "timeout"; error?: string | null }> => {
    const openedAuthUrls = new Set<string>();
    takeNextAuthUrlToOpen(opts?.openedAuthUrl, openedAuthUrls);
    for (let attempt = 0; attempt < AMP_LOGIN_POLL_ATTEMPTS; attempt += 1) {
      try {
        const status = await getAmpLogin(loginId);
        if (status.status === "success") return { status: "success" };
        if (status.status === "failed") return { status: "failed", error: status.error };
        if (status.status === "timeout") return { status: "timeout", error: status.error };
        if (shouldOpenPolledAuthUrlForStatus(status.status)) {
          const authUrl = takeNextAuthUrlToOpen(status.auth_url, openedAuthUrls);
          if (authUrl && onAuthUrl) {
            await onAuthUrl(authUrl);
          }
        }
      } catch {
        // continue polling
      }
      await new Promise((resolve) => {
        window.setTimeout(resolve, AMP_LOGIN_POLL_INTERVAL_MS);
      });
    }
    return { status: "timeout" };
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
    if (shouldSkipDuplicateAmpLoginStart({
      providerId: modal.provider_id,
      ampLoginInFlight: ampLoginInFlightRef.current,
    })) {
      return;
    }
    if (modal.stage !== "subscription") {
      setHarnessAuthModal((prev) => (prev ? { ...prev, stage: "subscription" } : prev));
    }

    let releaseAmpLoginInFlight = false;
    if (modal.provider_id === "amp") {
      ampLoginInFlightRef.current = true;
      releaseAmpLoginInFlight = true;
    }
    let preserveClaudeBusyState = false;
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
        const label = modal.subscription_label.trim();
        if (shouldCompleteClaudeLoginWithCallbackCode({
          providerId: modal.provider_id,
          subscriptionBusy: modal.subscription_busy,
          pendingLoginId: claudePendingLoginId,
          token,
        })) {
          if (!claudePendingLoginId) {
            throw new Error("Claude login callback requires an active pending login.");
          }
          await completeClaudeLogin(claudePendingLoginId, token);
          setHarnessAuthModal((prev) =>
            prev
              ? {
                  ...prev,
                  subscription_status: "Submitted callback code. Waiting for Claude setup-token completion...",
                }
              : prev);
          preserveClaudeBusyState = true;
          return;
        }
        if (token) {
          const next = await upsertClaudeAccount(token, label ? label : undefined);
          setClaudeAccounts(next);
          setClaudePendingLoginId(null);
          await refreshBootstrapAfterMutation();
          await selectSubscriptionSourceIfSupported(modal.provider_id);
          closeHarnessAuthModal();
          return;
        }
        const login = await startClaudeLogin(label ? label : undefined);
        setClaudePendingLoginId(login.login_id);
        const loginStartedAtMs = Date.now();
        const initialAuthUrl = takeNextClaudeAuthUrlToOpen(login.auth_url, new Set<string>());
        if (initialAuthUrl) {
          await openExternalLink(initialAuthUrl);
        }
        setHarnessAuthModal((prev) =>
          prev
            ? {
                ...prev,
                subscription_status:
                  "Waiting for browser sign-in. If Claude shows a token, paste it here and press Enter.",
              }
            : prev);
        const outcome = await waitForClaudeLoginOutcome(login.login_id, async (authUrl) => {
          if (!shouldOpenPolledClaudeAuthUrl({
            loginStartedAtMs,
            initialAuthUrl,
            polledAuthUrl: authUrl,
            nowMs: Date.now(),
          })) {
            return;
          }
          await openExternalLink(authUrl);
        }, {
          openedAuthUrl: initialAuthUrl,
        });
        setClaudePendingLoginId(null);
        await refreshClaudeAccounts();
        if (outcome === "success") {
          await onSelectProviderSource("claude-crp", "subscription", null);
          closeHarnessAuthModal();
          return;
        }
        if (outcome === "failed") {
          setHarnessAuthModal((prev) =>
            prev
              ? {
                  ...prev,
                  subscription_status:
                    "Sign-in failed. Retry, or paste a token in the field above.",
                }
              : prev);
          return;
        }
        setHarnessAuthModal((prev) =>
          prev
            ? {
                ...prev,
                subscription_status:
                  "Still waiting for completion. Keep this dialog open or retry.",
              }
            : prev);
        return;
      }

      if (modal.provider_id === "gemini") {
        const label = modal.subscription_label.trim();
        const openGeminiAuthUrl = async (authUrl: string): Promise<boolean> => {
          const opened = await openExternalLink(authUrl);
          if (opened) return true;
          setHarnessAuthModal((prev) =>
            prev
              ? {
                  ...prev,
                  subscription_status:
                    `Couldn't open browser automatically. Open this URL manually: ${authUrl}`,
                }
              : prev);
          return false;
        };

        const login = await startGeminiLogin(label ? label : undefined);
        const initialAuthUrl = takeNextAuthUrlToOpen(login.auth_url, new Set<string>());
        let initialAuthOpened = false;
        if (initialAuthUrl) {
          initialAuthOpened = await openGeminiAuthUrl(initialAuthUrl);
        }
        setHarnessAuthModal((prev) =>
          prev
            ? {
                ...prev,
                subscription_status: initialAuthUrl && !initialAuthOpened
                  ? `Couldn't open browser automatically. Open this URL manually: ${initialAuthUrl}`
                  : "Waiting for Google sign-in to complete in your browser...",
              }
            : prev);
        const outcome = await waitForGeminiLoginOutcome(login.login_id, async (authUrl) => {
          await openGeminiAuthUrl(authUrl);
        }, {
          openedAuthUrl: initialAuthUrl,
        });
        await refreshGeminiAccounts();
        if (outcome.status === "success") {
          await onSelectProviderSource("gemini", "subscription", null);
          closeHarnessAuthModal();
          return;
        }
        if (outcome.error && outcome.error.trim()) {
          setProviderError(outcome.error);
        }
        if (outcome.status === "failed") {
          const failureMessage = outcome.error?.trim() || "Sign-in failed. Retry.";
          setHarnessAuthModal((prev) =>
            prev
              ? {
                  ...prev,
                  subscription_status: failureMessage,
                }
              : prev);
          return;
        }
        if (outcome.status === "timeout") {
          const timeoutMessage =
            outcome.error?.trim() || "Timed out waiting for Gemini sign-in completion. Retry.";
          setHarnessAuthModal((prev) =>
            prev
              ? {
                  ...prev,
                  subscription_status: timeoutMessage,
                }
              : prev);
          return;
        }
        setHarnessAuthModal((prev) =>
          prev
            ? {
                ...prev,
                subscription_status:
                  "Still waiting for completion. Keep this dialog open or retry.",
              }
            : prev);
        return;
      }

      if (modal.provider_id === "amp") {
        const label = modal.subscription_label.trim();
        const openAmpAuthUrl = async (authUrl: string): Promise<boolean> => {
          const opened = await openExternalLink(authUrl);
          if (opened) return true;
          setHarnessAuthModal((prev) =>
            prev
              ? {
                  ...prev,
                  subscription_status:
                    `Couldn't open browser automatically. Open this URL manually: ${authUrl}`,
                }
              : prev);
          return false;
        };

        const login = await startAmpLogin(label ? label : undefined);
        const initialAuthUrl = takeNextAuthUrlToOpen(login.auth_url, new Set<string>());
        let initialAuthOpened = false;
        if (initialAuthUrl) {
          initialAuthOpened = await openAmpAuthUrl(initialAuthUrl);
        }
        setHarnessAuthModal((prev) =>
          prev
            ? {
                ...prev,
                subscription_status: initialAuthUrl && !initialAuthOpened
                  ? `Couldn't open browser automatically. Open this URL manually: ${initialAuthUrl}`
                  : "Waiting for Amp sign-in to complete in your browser...",
              }
            : prev);
        const outcome = await waitForAmpLoginOutcome(login.login_id, async (authUrl) => {
          await openAmpAuthUrl(authUrl);
        }, {
          openedAuthUrl: initialAuthUrl,
        });
        if (outcome.status === "success") {
          await refreshAmpAccounts();
          await onSelectProviderSource("amp", "subscription", null);
          closeHarnessAuthModal();
          return;
        }
        if (outcome.error && outcome.error.trim()) {
          setProviderError(outcome.error);
        }
        if (outcome.status === "failed") {
          const failureMessage = outcome.error?.trim() || "Sign-in failed. Retry.";
          setHarnessAuthModal((prev) =>
            prev
              ? {
                  ...prev,
                  subscription_status: failureMessage,
                }
              : prev);
          return;
        }
        if (outcome.status === "timeout") {
          const timeoutMessage =
            outcome.error?.trim() || "Timed out waiting for Amp sign-in completion. Retry.";
          setHarnessAuthModal((prev) =>
            prev
              ? {
                  ...prev,
                  subscription_status: timeoutMessage,
                }
              : prev);
          return;
        }
        setHarnessAuthModal((prev) =>
          prev
            ? {
                ...prev,
                subscription_status:
                  "Still waiting for completion. Keep this dialog open or retry.",
              }
            : prev);
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
        await refreshBootstrapAfterMutation();
        await selectSubscriptionSourceIfSupported(modal.provider_id);
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
        await refreshBootstrapAfterMutation();
        await selectSubscriptionSourceIfSupported(modal.provider_id);
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
        await refreshBootstrapAfterMutation();
        await selectSubscriptionSourceIfSupported(modal.provider_id);
        closeHarnessAuthModal();
        return;
      }

      if (!workspaceId) {
        throw new Error("Select a workspace first.");
      }
      await authenticateProviderForWorkspace(workspaceId, modal.provider_id);
      await refreshBootstrapAfterMutation();
      await selectSubscriptionSourceIfSupported(modal.provider_id);
      closeHarnessAuthModal();
    } catch (error) {
      const message = messageFromError(error);
      setProviderError(message);
      setHarnessAuthModal((prev) =>
        prev ? { ...prev, subscription_status: "Subscription flow failed. Check error details below." } : prev);
    } finally {
      if (releaseAmpLoginInFlight) {
        ampLoginInFlightRef.current = false;
      }
      if (!preserveClaudeBusyState) {
        setHarnessAuthModal((prev) => (prev ? { ...prev, subscription_busy: false } : prev));
      }
    }
  }, [
    claudePendingLoginId,
    closeHarnessAuthModal,
    harnessAuthModal,
    openCodexAuthUrl,
    refreshAmpAccounts,
    onSelectProviderSource,
    refreshClaudeAccounts,
    refreshCodexAccounts,
    refreshGeminiAccounts,
    refreshBootstrapAfterMutation,
    selectSubscriptionSourceIfSupported,
    waitForAmpLoginOutcome,
    waitForClaudeLoginOutcome,
    waitForCodexLoginOutcome,
    waitForGeminiLoginOutcome,
    workspaceId,
  ]);

  const onCodexDelete = useCallback(async (accountId: string) => {
    setCodexAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteCodexAccount(accountId);
      setCodexAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCodexAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onCodexSetActive = useCallback(async (accountId: string | null) => {
    setCodexAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setCodexActiveAccount(accountId);
      setCodexAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCodexAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onClaudeDelete = useCallback(async (accountId: string) => {
    setClaudeAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteClaudeAccount(accountId);
      setClaudeAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setClaudeAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onClaudeSetActive = useCallback(async (accountId: string | null) => {
    setClaudeAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setClaudeActiveAccount(accountId);
      setClaudeAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setClaudeAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onGeminiDelete = useCallback(async (accountId: string) => {
    setGeminiAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteGeminiAccount(accountId);
      setGeminiAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setGeminiAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onGeminiSetActive = useCallback(async (accountId: string | null) => {
    setGeminiAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setGeminiActiveAccount(accountId);
      setGeminiAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setGeminiAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onKimiDelete = useCallback(async (accountId: string) => {
    setKimiAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteKimiAccount(accountId);
      setKimiAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setKimiAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onKimiSetActive = useCallback(async (accountId: string | null) => {
    setKimiAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setKimiActiveAccount(accountId);
      setKimiAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setKimiAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onCopilotDelete = useCallback(async (accountId: string) => {
    setCopilotAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteCopilotAccount(accountId);
      setCopilotAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCopilotAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onCopilotSetActive = useCallback(async (accountId: string | null) => {
    setCopilotAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setCopilotActiveAccount(accountId);
      setCopilotAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCopilotAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onKiroDelete = useCallback(async (accountId: string) => {
    setKiroAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteKiroAccount(accountId);
      setKiroAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setKiroAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onKiroSetActive = useCallback(async (accountId: string | null) => {
    setKiroAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setKiroActiveAccount(accountId);
      setKiroAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setKiroAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onCursorDelete = useCallback(async (accountId: string) => {
    setCursorAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteCursorAccount(accountId);
      setCursorAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCursorAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onCursorSetActive = useCallback(async (accountId: string | null) => {
    setCursorAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setCursorActiveAccount(accountId);
      setCursorAccounts(next);
      await refreshBootstrapAfterMutation();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setCursorAccountsBusy(false);
    }
  }, [refreshBootstrapAfterMutation]);

  const onAmpDelete = useCallback(async (accountId: string) => {
    setAmpAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await deleteAmpAccount(accountId);
      setAmpAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setAmpAccountsBusy(false);
    }
  }, []);

  const onAmpSetActive = useCallback(async (accountId: string | null) => {
    setAmpAccountsBusy(true);
    setProviderError(null);
    try {
      const next = await setAmpActiveAccount(accountId);
      setAmpAccounts(next);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setAmpAccountsBusy(false);
    }
  }, []);

  const onSelectHarnessAuthRow = useCallback(async (providerId: string, row: HarnessAuthRow) => {
    try {
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
      } else if (providerId === "cursor" && row.account_id) {
        await onCursorSetActive(row.account_id);
      } else if (providerId === "amp" && row.account_id) {
        await onAmpSetActive(row.account_id);
      }
      if (supportsHarnessEndpointConfig(providerId)) {
        await onSelectProviderSource(providerId, "subscription", null);
      }
    } catch (error) {
      setProviderError(messageFromError(error));
    }
  }, [
    onClaudeSetActive,
    onCodexSetActive,
    onCopilotSetActive,
    onGeminiSetActive,
    onKimiSetActive,
    onKiroSetActive,
    onCursorSetActive,
    onAmpSetActive,
    onSelectProviderSource,
    supportsHarnessEndpointConfig,
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
          await refreshProviders();
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
  }, [refreshProviders]);

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
    if (workspaceId) {
      refreshProvidersBootstrapState({ silent: true }).catch(() => {});
      return;
    }
    refreshProviders().catch(() => {});
  }, [enabled, refreshProviders, refreshProvidersBootstrapState, workspaceId]);

  useEffect(() => {
    if (!enabled || !workspaceId) return;
    const refreshOnForeground = () => {
      refreshProvidersBootstrapState({ force: true, silent: true }).catch(() => {});
    };
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        refreshOnForeground();
      }
    };

    window.addEventListener("focus", refreshOnForeground);
    window.addEventListener("online", refreshOnForeground);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.removeEventListener("focus", refreshOnForeground);
      window.removeEventListener("online", refreshOnForeground);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [enabled, refreshProvidersBootstrapState, workspaceId]);

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
    setClaudePendingLoginId(null);
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
    cursorAccounts,
    cursorAccountsBusy,
    ampAccounts,
    ampAccountsBusy,
    harnessAuthModal,
    openHarnessAuthModal,
    closeHarnessAuthModal,
    patchHarnessAuthModal,
    submitHarnessSubscriptionModal,
    submitHarnessApiKeyModal,
    onSelectHarnessAuthRow,
    onDeleteProviderEndpoint,
    onRefreshProviderEndpointModels,
    onCodexDelete,
    onClaudeDelete,
    onGeminiDelete,
    onKimiDelete,
    onCopilotDelete,
    onKiroDelete,
    onCursorDelete,
    onAmpDelete,
    providerError,
    supportsHarnessEndpointConfig,
    harnessEndpointRequiresBaseUrl,
  };
}
