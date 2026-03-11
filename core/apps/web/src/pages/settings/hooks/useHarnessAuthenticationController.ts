import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  completeClaudeLogin,
  listAmpAccounts,
  listClaudeAccounts,
  listCodexAccounts,
  listCopilotAccounts,
  listCursorAccounts,
  listGeminiAccounts,
  listKimiAccounts,
  listMistralAccounts,
  listQwenAccounts,
  upsertCursorAccount,
  type AmpAccountsResponse,
  type ClaudeAccountsResponse,
  type CodexAccountsResponse,
  type CopilotAccountsResponse,
  type CursorAccountsResponse,
  type GeminiAccountsResponse,
  type HarnessProviderSourceConfig,
  type KimiAccountsResponse,
  type MistralAccountsResponse,
  type ProviderStatus,
  type QwenAccountsResponse,
} from "../../../api/client";
import { subscribeDaemonConnection } from "../../../api/daemonConnection";
import { desktopStartCodexLoginRelay, isDesktopApp, openExternalLink } from "../../../utils/desktop";
import {
  EMPTY_PROVIDERS_BOOTSTRAP,
  invalidateHostProvidersBootstrap,
  invalidateProvidersBootstrap,
  updateHostProvidersBootstrap,
  updateProvidersBootstrap,
} from "../../../state/providersBootstrapStore";
import {
  useProviderOnboardingCoordinator,
  type ProviderOnboardingInstallState,
} from "../../../state/providerOnboardingCoordinator";
import {
  deleteProviderAccount as executeDeleteProviderAccount,
  deleteProviderEndpoint as executeDeleteProviderEndpoint,
  refreshProviderEndpointModels as executeRefreshProviderEndpointModels,
  selectProviderSubscriptionAccount as executeSelectProviderSubscriptionAccount,
  selectProviderSource as executeSelectProviderSource,
  selectSubscriptionSourceIfSupported as executeSelectSubscriptionSourceIfSupported,
  submitProviderEndpointAuth,
  type ProviderAccountMutationProviderId,
} from "../../../state/providerOnboardingActions";
import {
  createMissingProviderOwnerScopeError,
  getProviderOwnerScopeKeyOrNull,
  getProviderOwnerScopeOrNull,
} from "../../../state/providerScopeAdapters";
import type { HarnessAuthModalState, InstallSession } from "../../SettingsPage.types";
import type { HarnessAuthRow } from "../harnessAuthRows";
import {
  defaultShapeForHarnessProvider,
  getHarnessEndpointProviderPreset,
  normalizeOptionalBaseUrl,
  nextDefaultEndpointName,
  nextTokenEndpointName,
} from "../harnessEndpointProviders";
import {
  harnessEndpointRequiresApiShape,
  harnessEndpointRequiresBaseUrl,
  messageFromError,
  shouldCompleteClaudeLoginWithCallbackCode,
  supportsHarnessEndpointConfigStatic,
  supportsHarnessSubscriptionAuth,
  toErrorObject,
} from "./harnessAuth/capabilities";
import { runHarnessSubscriptionFlow } from "./harnessAuth/subscriptionFlow";
import { useHarnessAuthModalController } from "./harnessAuth/useHarnessAuthModalController";

export {
  CLAUDE_LOGIN_COMPLETION_TIMEOUT_MS,
  CLAUDE_LOGIN_POLL_ATTEMPTS,
  CLAUDE_LOGIN_POLL_INTERVAL_MS,
  extractGithubDeviceCodeFromAuthUrl,
  resolveHarnessAuthModalInitialStage,
  shouldAutoOpenCopilotAuthUrl,
  shouldAutoOpenKimiAuthUrl,
  shouldCompleteClaudeLoginWithCallbackCode,
  shouldOpenPolledAuthUrlForStatus,
  shouldOpenPolledClaudeAuthUrl,
  shouldSkipDuplicateAmpLoginStart,
  supportsHarnessSubscriptionAuth,
  takeNextClaudeAuthUrlToOpen,
  toErrorObject,
} from "./harnessAuth/capabilities";
export { resolveUpsertedEndpoint } from "../../../state/providerOnboardingActions";

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
  onCancelInstall: (providerId: string) => Promise<void>;
  providerHarnessConfig: Record<string, HarnessProviderSourceConfig | undefined>;
  providerHarnessBusy: Record<string, boolean>;
  codexAccounts: CodexAccountsResponse | null;
  codexAccountsBusy: boolean;
  claudeAccounts: ClaudeAccountsResponse | null;
  claudeAccountsBusy: boolean;
  geminiAccounts: GeminiAccountsResponse | null;
  geminiAccountsBusy: boolean;
  qwenAccounts: QwenAccountsResponse | null;
  qwenAccountsBusy: boolean;
  kimiAccounts: KimiAccountsResponse | null;
  kimiAccountsBusy: boolean;
  mistralAccounts: MistralAccountsResponse | null;
  mistralAccountsBusy: boolean;
  copilotAccounts: CopilotAccountsResponse | null;
  copilotAccountsBusy: boolean;
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
  onQwenDelete: (accountId: string) => Promise<void>;
  onKimiDelete: (accountId: string) => Promise<void>;
  onMistralDelete: (accountId: string) => Promise<void>;
  onCopilotDelete: (accountId: string) => Promise<void>;
  onCursorDelete: (accountId: string) => Promise<void>;
  onAmpDelete: (accountId: string) => Promise<void>;
  providerError: string | null;
  supportsHarnessEndpointConfig: (providerId: string) => boolean;
  supportsHarnessSubscriptionAuth: (providerId: string) => boolean;
  harnessEndpointRequiresBaseUrl: (providerId: string) => boolean;
};

type RefreshOptions = {
  silent?: boolean;
};

type StateSetter<T> = Dispatch<SetStateAction<T>>;

const resolveNextNullableState = <T,>(update: SetStateAction<T | null>, current: T | null): T | null =>
  typeof update === "function"
    ? (update as (previous: T | null) => T | null)(current)
    : update;

const toInstallSession = (
  session: ProviderOnboardingInstallState,
): InstallSession => ({
  installId: session.installId,
  state: session.state,
  pct: session.pct,
  target: session.target,
  errorCode: session.errorCode,
  streamError: undefined,
  error: session.error,
});

const toInstallSessionMap = (
  installsById: Record<string, ProviderOnboardingInstallState>,
): Record<string, InstallSession> =>
  Object.fromEntries(
    Object.entries(installsById).map(([providerId, install]) => [providerId, toInstallSession(install)]),
  );

const refreshAccountCollection = async <TResponse>(params: {
  silent?: boolean;
  list: () => Promise<TResponse>;
  applyData: (next: TResponse) => void;
  setBusy: StateSetter<boolean>;
  setProviderError: StateSetter<string | null>;
}): Promise<TResponse | null> => {
  if (!params.silent) {
    params.setBusy(true);
  }
  try {
    const next = await params.list();
    params.applyData(next);
    return next;
  } catch (error) {
    params.setProviderError(messageFromError(error));
    return null;
  } finally {
    if (!params.silent) {
      params.setBusy(false);
    }
  }
};

const mutateProviderAccount = async (params: {
  mutate: () => Promise<void>;
  setBusy: StateSetter<boolean>;
  setProviderError: StateSetter<string | null>;
}): Promise<void> => {
  params.setBusy(true);
  params.setProviderError(null);
  try {
    await params.mutate();
  } catch (error) {
    params.setProviderError(messageFromError(error));
  } finally {
    params.setBusy(false);
  }
};

export function useHarnessAuthenticationController({
  workspaceId,
  enabled,
}: UseHarnessAuthenticationControllerArgs): HarnessAuthenticationController {
  const [providerError, setProviderError] = useState<string | null>(null);
  const [providerHarnessBusy, setProviderHarnessBusy] = useState<Record<string, boolean>>({});
  const [providerEndpointUnsupported, setProviderEndpointUnsupported] = useState<Record<string, boolean>>({});
  const {
    harnessAuthModal,
    claudePendingLoginId,
    openHarnessAuthModal: baseOpenHarnessAuthModal,
    closeHarnessAuthModal: baseCloseHarnessAuthModal,
    patchHarnessAuthModal: basePatchHarnessAuthModal,
    startOperation: startHarnessAuthModalOperation,
    finishOperation: finishHarnessAuthModalOperation,
    hasActiveOperation: hasActiveHarnessAuthModalOperation,
    patchHarnessAuthModalForOperation,
    closeHarnessAuthModalForOperation,
    setClaudePendingLoginIdForOperation,
  } = useHarnessAuthModalController();
  const [installBusy, setInstallBusy] = useState<string | null>(null);

  const [codexAccountsBusy, setCodexAccountsBusy] = useState(false);
  const [claudeAccountsBusy, setClaudeAccountsBusy] = useState(false);
  const [geminiAccountsBusy, setGeminiAccountsBusy] = useState(false);
  const [qwenAccountsBusy, setQwenAccountsBusy] = useState(false);
  const [kimiAccountsBusy, setKimiAccountsBusy] = useState(false);
  const [mistralAccountsBusy, setMistralAccountsBusy] = useState(false);
  const [copilotAccountsBusy, setCopilotAccountsBusy] = useState(false);
  const [cursorAccountsBusy, setCursorAccountsBusy] = useState(false);
  const [ampAccountsBusy, setAmpAccountsBusy] = useState(false);
  const ownerScopeKey = useSyncExternalStore(
    useCallback((listener) => subscribeDaemonConnection((_connection) => listener()), []),
    useCallback(() => getProviderOwnerScopeKeyOrNull(workspaceId), [workspaceId]),
    useCallback(() => getProviderOwnerScopeKeyOrNull(workspaceId), [workspaceId]),
  );
  const ownerScope = useMemo(
    () => getProviderOwnerScopeOrNull(workspaceId),
    [ownerScopeKey, workspaceId],
  );
  const requireOwnerScope = useCallback(() => {
    if (!ownerScope) {
      throw createMissingProviderOwnerScopeError();
    }
    return ownerScope;
  }, [ownerScope]);
  const handleOnboardingLoadError = useCallback((error: unknown) => {
    setProviderError(messageFromError(error));
  }, []);
  const onboarding = useProviderOnboardingCoordinator({
    workspaceId,
    enabled,
    onLoadError: workspaceId ? undefined : handleOnboardingLoadError,
  });
  const providers = onboarding.bootstrap.providers;
  const providerHarnessConfig = onboarding.bootstrap.provider_harness_config;
  const codexAccounts = onboarding.bootstrap.codex_accounts;
  const claudeAccounts = onboarding.bootstrap.claude_accounts;
  const geminiAccounts = onboarding.bootstrap.gemini_accounts;
  const qwenAccounts = onboarding.bootstrap.qwen_accounts;
  const kimiAccounts = onboarding.bootstrap.kimi_accounts;
  const mistralAccounts = onboarding.bootstrap.mistral_accounts;
  const copilotAccounts = onboarding.bootstrap.copilot_accounts;
  const cursorAccounts = onboarding.bootstrap.cursor_accounts;
  const ampAccounts = onboarding.bootstrap.amp_accounts;
  const installs = useMemo(
    () => toInstallSessionMap(onboarding.installsById),
    [onboarding.installsById],
  );
  const providerHarnessConfigRef = useRef<Record<string, HarnessProviderSourceConfig | undefined>>({});
  const providerEndpointUnsupportedRef = useRef<Record<string, boolean>>({});

  const supportsHarnessEndpointConfig = useCallback(
    (providerId: string): boolean =>
      supportsHarnessEndpointConfigStatic(providerId) && providerEndpointUnsupportedRef.current[providerId] !== true,
    [],
  );

  useEffect(() => {
    providerHarnessConfigRef.current = providerHarnessConfig;
  }, [providerHarnessConfig]);

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
      if (workspaceId) {
        const next = updateProvidersBootstrap(workspaceId, (current) => ({
          ...current,
          provider_harness_config: {
            ...current.provider_harness_config,
            [providerId]: nextConfig,
          },
        }));
        providerHarnessConfigRef.current = next.provider_harness_config;
        return;
      }
      const next = updateHostProvidersBootstrap((current) => ({
        ...current,
        provider_harness_config: {
          ...current.provider_harness_config,
          [providerId]: nextConfig,
        },
      }));
      providerHarnessConfigRef.current = next.provider_harness_config;
    },
    [workspaceId],
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
    setProviderHarnessBusy((prev) => ({ ...prev, [providerId]: busy }));
  }, []);

  const refreshProvidersBootstrapState = useCallback(async (opts?: { force?: boolean; silent?: boolean }) => {
    if (!workspaceId) return null;
    try {
      return opts?.force
        ? await onboarding.refreshBootstrap()
        : await onboarding.loadBootstrap();
    } catch (error) {
      if (!opts?.silent) {
        setProviderError(messageFromError(error));
      }
      return null;
    }
  }, [onboarding, workspaceId]);

  const refreshProviderSlicesAfterMutation = useCallback(async (providerId?: string) => {
    if (workspaceId) {
      invalidateProvidersBootstrap(workspaceId);
      await refreshProvidersBootstrapState({ force: true });
      if (providerId) {
        try {
          await onboarding.ensureProviderAuthSummary(providerId, { trigger: "explicit" });
        } catch {
          // Keep the refreshed bootstrap snapshot even when live model hydration fails.
        }
      }
      return;
    }
    try {
      invalidateHostProvidersBootstrap();
      await onboarding.refreshBootstrap();
    } catch (error) {
      setProviderError(messageFromError(error));
    }
  }, [onboarding, refreshProvidersBootstrapState, workspaceId]);

  const refreshProviders = useCallback(async () => {
    if (workspaceId) {
      const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: true });
      return bootstrap?.providers ?? [];
    }
    try {
      const bootstrap = await onboarding.refreshBootstrap();
      return bootstrap.providers;
    } catch (error) {
      setProviderError(messageFromError(error));
      return [];
    }
  }, [onboarding, refreshProvidersBootstrapState, workspaceId]);

  const applyCodexAccounts = useCallback((next: CodexAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, codex_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, codex_accounts: next }));
  }, [workspaceId]);

  const applyClaudeAccounts = useCallback((next: ClaudeAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, claude_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, claude_accounts: next }));
  }, [workspaceId]);

  const applyGeminiAccounts = useCallback((next: GeminiAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, gemini_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, gemini_accounts: next }));
  }, [workspaceId]);

  const applyQwenAccounts = useCallback((next: QwenAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, qwen_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, qwen_accounts: next }));
  }, [workspaceId]);

  const applyKimiAccounts = useCallback((next: KimiAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, kimi_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, kimi_accounts: next }));
  }, [workspaceId]);

  const applyMistralAccounts = useCallback((next: MistralAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, mistral_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, mistral_accounts: next }));
  }, [workspaceId]);

  const applyCopilotAccounts = useCallback((next: CopilotAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, copilot_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, copilot_accounts: next }));
  }, [workspaceId]);

  const applyCursorAccounts = useCallback((next: CursorAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, cursor_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, cursor_accounts: next }));
  }, [workspaceId]);

  const applyAmpAccounts = useCallback((next: AmpAccountsResponse) => {
    if (workspaceId) {
      updateProvidersBootstrap(workspaceId, (current) => ({ ...current, amp_accounts: next }));
      return;
    }
    updateHostProvidersBootstrap((current) => ({ ...current, amp_accounts: next }));
  }, [workspaceId]);

  const setScopedClaudeAccounts = useCallback((update: SetStateAction<ClaudeAccountsResponse | null>) => {
    const next = resolveNextNullableState(update, claudeAccounts);
    if (next) {
      applyClaudeAccounts(next);
      return;
    }
    if (!workspaceId) {
      updateHostProvidersBootstrap((current) => ({
        ...current,
        claude_accounts: EMPTY_PROVIDERS_BOOTSTRAP.claude_accounts,
      }));
    }
  }, [applyClaudeAccounts, claudeAccounts, workspaceId]);

  const setScopedKimiAccounts = useCallback((update: SetStateAction<KimiAccountsResponse | null>) => {
    const next = resolveNextNullableState(update, kimiAccounts);
    if (next) {
      applyKimiAccounts(next);
      return;
    }
    if (!workspaceId) {
      updateHostProvidersBootstrap((current) => ({
        ...current,
        kimi_accounts: EMPTY_PROVIDERS_BOOTSTRAP.kimi_accounts,
      }));
    }
  }, [applyKimiAccounts, kimiAccounts, workspaceId]);

  const setScopedCopilotAccounts = useCallback((update: SetStateAction<CopilotAccountsResponse | null>) => {
    const next = resolveNextNullableState(update, copilotAccounts);
    if (next) {
      applyCopilotAccounts(next);
      return;
    }
    if (!workspaceId) {
      updateHostProvidersBootstrap((current) => ({
        ...current,
        copilot_accounts: EMPTY_PROVIDERS_BOOTSTRAP.copilot_accounts,
      }));
    }
  }, [applyCopilotAccounts, copilotAccounts, workspaceId]);

  const refreshCodexAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.codex_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listCodexAccounts,
        applyData: applyCodexAccounts,
        setBusy: setCodexAccountsBusy,
        setProviderError,
      });
    },
    [applyCodexAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const refreshClaudeAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.claude_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listClaudeAccounts,
        applyData: applyClaudeAccounts,
        setBusy: setClaudeAccountsBusy,
        setProviderError,
      });
    },
    [applyClaudeAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const refreshGeminiAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.gemini_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listGeminiAccounts,
        applyData: applyGeminiAccounts,
        setBusy: setGeminiAccountsBusy,
        setProviderError,
      });
    },
    [applyGeminiAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const refreshQwenAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.qwen_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listQwenAccounts,
        applyData: applyQwenAccounts,
        setBusy: setQwenAccountsBusy,
        setProviderError,
      });
    },
    [applyQwenAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const refreshKimiAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.kimi_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listKimiAccounts,
        applyData: applyKimiAccounts,
        setBusy: setKimiAccountsBusy,
        setProviderError,
      });
    },
    [applyKimiAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const refreshMistralAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.mistral_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listMistralAccounts,
        applyData: applyMistralAccounts,
        setBusy: setMistralAccountsBusy,
        setProviderError,
      });
    },
    [applyMistralAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const refreshCopilotAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.copilot_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listCopilotAccounts,
        applyData: applyCopilotAccounts,
        setBusy: setCopilotAccountsBusy,
        setProviderError,
      });
    },
    [applyCopilotAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const refreshCursorAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.cursor_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listCursorAccounts,
        applyData: applyCursorAccounts,
        setBusy: setCursorAccountsBusy,
        setProviderError,
      });
    },
    [applyCursorAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const refreshAmpAccounts = useCallback(
    async (opts?: RefreshOptions) => {
      if (workspaceId) {
        const bootstrap = await refreshProvidersBootstrapState({ force: true, silent: opts?.silent });
        return bootstrap?.amp_accounts ?? null;
      }
      return refreshAccountCollection({
        silent: opts?.silent,
        list: listAmpAccounts,
        applyData: applyAmpAccounts,
        setBusy: setAmpAccountsBusy,
        setProviderError,
      });
    },
    [applyAmpAccounts, refreshProvidersBootstrapState, workspaceId],
  );

  const runDeleteProviderAccount = useCallback(
    async <TKey extends ProviderAccountMutationProviderId>(
      providerId: TKey,
      accountId: string,
      setBusy: StateSetter<boolean>,
    ) => {
      await mutateProviderAccount({
        mutate: async () => {
          await executeDeleteProviderAccount(requireOwnerScope(), providerId, accountId);
        },
        setBusy,
        setProviderError,
      });
    },
    [requireOwnerScope],
  );

  const runSelectProviderSubscriptionAccount = useCallback(
    async <TKey extends ProviderAccountMutationProviderId>(
      providerId: TKey,
      accountId: string | null,
      setBusy: StateSetter<boolean>,
    ) => {
      await mutateProviderAccount({
        mutate: async () => {
          await executeSelectProviderSubscriptionAccount({
            ownerScope: requireOwnerScope(),
            providerId,
            accountId,
            supportsEndpointConfig: supportsHarnessEndpointConfig(providerId),
          });
          await onboarding.ensureProviderAuthSummary(providerId, { trigger: "explicit" });
        },
        setBusy,
        setProviderError,
      });
    },
    [onboarding, requireOwnerScope, supportsHarnessEndpointConfig],
  );

  const onDeleteProviderEndpoint = useCallback(async (providerId: string, endpointId: string) => {
    setProviderHarnessBusyForProvider(providerId, true);
    setProviderError(null);
    try {
      await executeDeleteProviderEndpoint(requireOwnerScope(), providerId, endpointId);
      await refreshProviderSlicesAfterMutation(providerId);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setProviderHarnessBusyForProvider(providerId, false);
    }
  }, [refreshProviderSlicesAfterMutation, requireOwnerScope, setProviderHarnessBusyForProvider]);

  const onRefreshProviderEndpointModels = useCallback(async (providerId: string, endpointId: string) => {
    setProviderHarnessBusyForProvider(providerId, true);
    setProviderError(null);
    try {
      await executeRefreshProviderEndpointModels(requireOwnerScope(), providerId, endpointId);
      await refreshProviderSlicesAfterMutation(providerId);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setProviderHarnessBusyForProvider(providerId, false);
    }
  }, [refreshProviderSlicesAfterMutation, requireOwnerScope, setProviderHarnessBusyForProvider]);

  const onSelectProviderSource = useCallback(
    async (providerId: string, sourceKind: "subscription" | "endpoint", endpointId?: string | null) => {
      setProviderHarnessBusyForProvider(providerId, true);
      setProviderError(null);
      try {
        await executeSelectProviderSource(requireOwnerScope(), providerId, {
          sourceKind,
          endpointId: endpointId ?? null,
        });
      } catch (error) {
        setProviderError(messageFromError(error));
        throw toErrorObject(error);
      } finally {
        setProviderHarnessBusyForProvider(providerId, false);
      }
    },
    [requireOwnerScope, setProviderHarnessBusyForProvider],
  );

  const selectSubscriptionSourceIfSupported = useCallback(
    async (providerId: string) => {
      await executeSelectSubscriptionSourceIfSupported({
        ownerScope: requireOwnerScope(),
        providerId,
        supportsEndpointConfig: supportsHarnessEndpointConfig(providerId),
        onEndpointUnsupported: () => {
          markProviderEndpointUnsupported(providerId);
        },
      });
    },
    [
      markProviderEndpointUnsupported,
      requireOwnerScope,
      supportsHarnessEndpointConfig,
    ],
  );

  const openHarnessAuthModal = useCallback((providerId: string) => {
    setProviderError(null);
    baseOpenHarnessAuthModal(providerId);
  }, [baseOpenHarnessAuthModal]);

  const closeHarnessAuthModal = useCallback(() => {
    baseCloseHarnessAuthModal();
  }, [baseCloseHarnessAuthModal]);

  const patchHarnessAuthModal = useCallback((patch: Partial<HarnessAuthModalState>) => {
    basePatchHarnessAuthModal(patch);
  }, [basePatchHarnessAuthModal]);

  const submitHarnessApiKeyModal = useCallback(async () => {
    const modal = harnessAuthModal;
    if (!modal || modal.stage !== "api_key") return;
    if (modal.api_key_busy) return;

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
    const usesGeminiVertexServiceAccount = modal.provider_id === "gemini" && geminiAuthType === "vertex_ai";
    const base = modal.base_url.trim();
    const normalizedBase = normalizeOptionalBaseUrl(base);
    const effectiveBaseUrl = modal.provider_id === "gemini" ? null : normalizedBase;
    const key = modal.api_key.trim();
    const serviceAccountJson = modal.service_account_json.trim();
    const projectId = modal.project_id.trim();
    const location = modal.location.trim();
    const manualModelIds = modal.manual_model_ids
      .split(/[\n,]/)
      .map((value: string) => value.trim())
      .filter((value: string) => value.length > 0);

    if (requiresBaseUrl && !base) {
      setProviderError("Endpoint base URL is required.");
      return;
    }
    if (usesGeminiVertexServiceAccount) {
      if (!serviceAccountJson) {
        setProviderError("Service account JSON is required.");
        return;
      }
    } else if (!key) {
      setProviderError("API key is required.");
      return;
    }

    const operation = startHarnessAuthModalOperation("modal-action");
    patchHarnessAuthModalForOperation(operation, { api_key_busy: true });
    setProviderError(null);

    const previousSourceConfig = providerHarnessConfigRef.current[modal.provider_id];
    const previousSourceKind = previousSourceConfig?.selected_source_kind ?? "subscription";
    const previousEndpointId = previousSourceKind === "endpoint"
      ? previousSourceConfig?.selected_endpoint_id ?? null
      : null;

    try {
      if (isCursor) {
        const label = name.trim();
        const next = await upsertCursorAccount(key, label ? { label } : undefined);
        await refreshProviderSlicesAfterMutation(modal.provider_id);
        if (!operation.isCurrent()) return;
        applyCursorAccounts(next);
        await selectSubscriptionSourceIfSupported(modal.provider_id);
        if (!operation.isCurrent()) return;
        setSubscriptionSourceFallback(modal.provider_id);
        closeHarnessAuthModalForOperation(operation);
        return;
      }

      const result = await submitProviderEndpointAuth({
        ownerScope: requireOwnerScope(),
        providerId: modal.provider_id,
        requestedEndpointId: modal.endpoint_id?.trim() || null,
        name,
        baseUrl: effectiveBaseUrl,
        apiShape: requiresApiShape ? defaultShapeForHarnessProvider(modal.provider_id) : null,
        authType: geminiAuthType,
        apiKey: usesGeminiVertexServiceAccount ? null : key,
        serviceAccountJson: usesGeminiVertexServiceAccount ? serviceAccountJson : null,
        projectId: usesGeminiVertexServiceAccount ? (projectId || null) : null,
        location: usesGeminiVertexServiceAccount ? (location || null) : null,
        manualModelIds,
        previousSelection: {
          sourceKind: previousSourceKind,
          endpointId: previousEndpointId,
        },
        isStale: () => !operation.isCurrent(),
      });

      if (operation.isCurrent()) {
        patchHarnessAuthModalForOperation(operation, { endpoint_id: result.selectedEndpointId });
      }

      if (result.status === "applied") {
        closeHarnessAuthModalForOperation(operation);
        return;
      }

      if (result.status === "rolled_back") {
        if (operation.isCurrent()) {
          setProviderError(result.message);
        }
        return;
      }

      if (result.status === "rollback_failed") {
        if (operation.isCurrent()) {
          setProviderError(
            result.message
              ? `${result.message} Failed to restore previous provider source: ${messageFromError(result.rollbackError)}`
              : `Failed to restore previous provider source: ${messageFromError(result.rollbackError)}`,
          );
        }
        return;
      }
    } catch (error) {
      if (operation.isCurrent()) {
        setProviderError(messageFromError(error));
      }
    } finally {
      patchHarnessAuthModalForOperation(operation, { api_key_busy: false });
      finishHarnessAuthModalOperation(operation);
    }
  }, [
    harnessAuthModal,
    closeHarnessAuthModalForOperation,
    finishHarnessAuthModalOperation,
    patchHarnessAuthModalForOperation,
    refreshProviderSlicesAfterMutation,
    requireOwnerScope,
    selectSubscriptionSourceIfSupported,
    setSubscriptionSourceFallback,
    startHarnessAuthModalOperation,
  ]);

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

    const shouldCompleteClaudeCallback = shouldCompleteClaudeLoginWithCallbackCode({
      providerId: modal.provider_id,
      subscriptionBusy: modal.subscription_busy,
      pendingLoginId: claudePendingLoginId,
      token: modal.subscription_token,
    });

    if (modal.subscription_busy && !shouldCompleteClaudeCallback) {
      return;
    }

    if (hasActiveHarnessAuthModalOperation("subscription-flow") && !shouldCompleteClaudeCallback) {
      return;
    }

    if (shouldCompleteClaudeCallback) {
      const operation = startHarnessAuthModalOperation("modal-action");
      setProviderError(null);
      try {
        if (!claudePendingLoginId) {
          throw new Error("Claude login callback requires an active pending login.");
        }
        await completeClaudeLogin(claudePendingLoginId, modal.subscription_token.trim());
        patchHarnessAuthModalForOperation(operation, {
          subscription_status: "Submitted callback code. Waiting for Claude setup-token completion...",
        });
      } catch (error) {
        if (operation.isCurrent()) {
          setProviderError(messageFromError(error));
          patchHarnessAuthModalForOperation(operation, {
            subscription_status: "Subscription flow failed. Check error details below.",
          });
        }
      } finally {
        finishHarnessAuthModalOperation(operation);
      }
      return;
    }

    const flow = startHarnessAuthModalOperation("subscription-flow");
    patchHarnessAuthModalForOperation(flow, {
      stage: "subscription",
      subscription_busy: true,
      subscription_status: "Starting subscription flow...",
    });
    setProviderError(null);
    try {
      await runHarnessSubscriptionFlow({
        modal,
        workspaceId,
        flow,
        patchHarnessAuthModalForOperation,
        closeHarnessAuthModalForOperation,
        setClaudePendingLoginIdForOperation,
        setProviderError,
        refreshBootstrapAfterMutation: refreshProviderSlicesAfterMutation,
        selectSubscriptionSourceIfSupported,
        refreshCodexAccounts,
        refreshClaudeAccounts,
        refreshGeminiAccounts,
        refreshQwenAccounts,
        refreshAmpAccounts,
        refreshMistralAccounts,
        setClaudeAccounts: setScopedClaudeAccounts,
        setKimiAccounts: setScopedKimiAccounts,
        setCopilotAccounts: setScopedCopilotAccounts,
        openCodexAuthUrl,
      });
    } finally {
      patchHarnessAuthModalForOperation(flow, { subscription_busy: false });
      finishHarnessAuthModalOperation(flow);
    }
  }, [
    claudePendingLoginId,
    harnessAuthModal,
    closeHarnessAuthModalForOperation,
    finishHarnessAuthModalOperation,
    hasActiveHarnessAuthModalOperation,
    openCodexAuthUrl,
    patchHarnessAuthModalForOperation,
    refreshAmpAccounts,
    refreshProviderSlicesAfterMutation,
    refreshClaudeAccounts,
    refreshCodexAccounts,
    refreshGeminiAccounts,
    refreshMistralAccounts,
    refreshQwenAccounts,
    setScopedClaudeAccounts,
    setScopedCopilotAccounts,
    setScopedKimiAccounts,
    selectSubscriptionSourceIfSupported,
    setClaudePendingLoginIdForOperation,
    startHarnessAuthModalOperation,
    workspaceId,
  ]);

  const onCodexDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("codex", accountId, setCodexAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onClaudeDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("claude-crp", accountId, setClaudeAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onGeminiDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("gemini", accountId, setGeminiAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onQwenDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("qwen", accountId, setQwenAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onKimiDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("kimi", accountId, setKimiAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onMistralDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("mistral", accountId, setMistralAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onCopilotDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("copilot", accountId, setCopilotAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onCursorDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("cursor", accountId, setCursorAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onAmpDelete = useCallback(async (accountId: string) => {
    await runDeleteProviderAccount("amp", accountId, setAmpAccountsBusy);
  }, [runDeleteProviderAccount]);

  const onSelectHarnessAuthRow = useCallback(async (providerId: string, row: HarnessAuthRow) => {
    try {
      if (!row.selectable) return;
      if (row.kind === "api_key" && row.endpoint_id) {
        await onSelectProviderSource(providerId, "endpoint", row.endpoint_id);
        return;
      }
      if (providerId === "codex" && row.account_id) {
        await runSelectProviderSubscriptionAccount("codex", row.account_id, setCodexAccountsBusy);
      } else if (providerId === "claude-crp" && row.account_id) {
        await runSelectProviderSubscriptionAccount("claude-crp", row.account_id, setClaudeAccountsBusy);
      } else if (providerId === "gemini" && row.account_id) {
        await runSelectProviderSubscriptionAccount("gemini", row.account_id, setGeminiAccountsBusy);
      } else if (providerId === "qwen" && row.account_id) {
        await runSelectProviderSubscriptionAccount("qwen", row.account_id, setQwenAccountsBusy);
      } else if (providerId === "kimi" && row.account_id) {
        await runSelectProviderSubscriptionAccount("kimi", row.account_id, setKimiAccountsBusy);
      } else if (providerId === "mistral" && row.account_id) {
        await runSelectProviderSubscriptionAccount("mistral", row.account_id, setMistralAccountsBusy);
      } else if (providerId === "copilot" && row.account_id) {
        await runSelectProviderSubscriptionAccount("copilot", row.account_id, setCopilotAccountsBusy);
      } else if (providerId === "cursor" && row.account_id) {
        await runSelectProviderSubscriptionAccount("cursor", row.account_id, setCursorAccountsBusy);
      } else if (providerId === "amp" && row.account_id) {
        await runSelectProviderSubscriptionAccount("amp", row.account_id, setAmpAccountsBusy);
      }
    } catch (error) {
      setProviderError(messageFromError(error));
    }
  }, [
    onSelectProviderSource,
    runSelectProviderSubscriptionAccount,
  ]);

  const onInstall = useCallback(async (providerId: string) => {
    setInstallBusy(providerId);
    setProviderError(null);
    try {
      await onboarding.startProviderInstall(providerId);
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setInstallBusy(null);
    }
  }, [onboarding]);

  const onInstallAll = useCallback(async () => {
    setInstallBusy("all");
    setProviderError(null);
    try {
      await onboarding.startAllProviderInstalls();
    } catch (error) {
      setProviderError(messageFromError(error));
    } finally {
      setInstallBusy(null);
    }
  }, [onboarding]);

  const onCancelInstall = useCallback(async (providerId: string) => {
    setProviderError(null);
    try {
      await onboarding.cancelProviderInstall(providerId);
    } catch (error) {
      setProviderError(messageFromError(error));
    }
  }, [onboarding]);

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
    closeHarnessAuthModal();
  }, [closeHarnessAuthModal, enabled]);

  return {
    providers,
    installs,
    installBusy,
    onInstallAll,
    onInstall,
    onCancelInstall,
    providerHarnessConfig,
    providerHarnessBusy,
    codexAccounts,
    codexAccountsBusy,
    claudeAccounts,
    claudeAccountsBusy,
    geminiAccounts,
    geminiAccountsBusy,
    qwenAccounts,
    qwenAccountsBusy,
    kimiAccounts,
    kimiAccountsBusy,
    mistralAccounts,
    mistralAccountsBusy,
    copilotAccounts,
    copilotAccountsBusy,
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
    onQwenDelete,
    onKimiDelete,
    onMistralDelete,
    onCopilotDelete,
    onCursorDelete,
    onAmpDelete,
    providerError,
    supportsHarnessEndpointConfig,
    supportsHarnessSubscriptionAuth,
    harnessEndpointRequiresBaseUrl,
  };
}
