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
  cancelInstall,
  getProviderOptions,
  installAllProviders,
  installProvider,
  type InstallInfo,
  type InstallTarget,
  type ProviderOptions,
  type ProviderStatus,
} from "../../api/client";
import {
  getProvidersBootstrapSnapshot,
  loadProvidersBootstrap,
  refreshProvidersBootstrap,
  resolveProviderOptionsUpdate,
  subscribeProvidersBootstrap,
  updateProvidersBootstrap,
} from "../../state/providersBootstrapStore";
import {
  getProviderInstallProgressSnapshot,
  removeProviderInstallProgress,
  resolveProviderInstallProgressSession,
  subscribeProviderInstallProgress,
  type ProviderInstallProgressSnapshot,
} from "../../state/providerInstallProgressStore";
import { observeInstall } from "../../state/installProgressMonitor";
import type { DraftHarness } from "../../components/WorkbenchComposer";
import { computeInstallPct, parseInstallTarget } from "../../utils/providerInstallUi";

type ProviderInstallState = {
  installId: string;
  state: InstallInfo["state"];
  pct: number | null;
  target?: InstallTarget;
  errorCode?: InstallInfo["error_code"];
  error?: string;
};

const sameProviderInstallState = (
  lhs: ProviderInstallState | undefined,
  rhs: ProviderInstallState | undefined,
): boolean => {
  if (!lhs && !rhs) return true;
  if (!lhs || !rhs) return false;
  return lhs.installId === rhs.installId
    && lhs.state === rhs.state
    && lhs.pct === rhs.pct
    && lhs.target === rhs.target
    && lhs.errorCode === rhs.errorCode
    && lhs.error === rhs.error;
};

const sameProviderInstallStateMap = (
  lhs: Record<string, ProviderInstallState | undefined>,
  rhs: Record<string, ProviderInstallState | undefined>,
): boolean => {
  const lhsKeys = Object.keys(lhs);
  const rhsKeys = Object.keys(rhs);
  if (lhsKeys.length !== rhsKeys.length) return false;
  for (const key of lhsKeys) {
    if (!sameProviderInstallState(lhs[key], rhs[key])) return false;
  }
  return true;
};

type UseWorkbenchProvidersArgs = {
  workspaceId: string;
  setDraftHarness: Dispatch<SetStateAction<DraftHarness | null>>;
  onStartError: (message: string | null) => void;
};

const MODEL_DISCOVERY_PROVIDER_IDS = new Set(["codex", "claude-crp", "copilot"]);
export type ProviderAuthSummaryTrigger = "passive" | "explicit";

const hasProviderModels = (options: ProviderOptions | undefined): boolean => {
  const raw = options?.models;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return false;
  const record = raw as Record<string, unknown>;
  const current = record.currentModelId ?? record.current_model_id;
  if (typeof current === "string" && current.trim().length > 0) return true;
  const list = record.availableModels ?? record.available_models ?? record.models;
  return Array.isArray(list) && list.length > 0;
};

const hasFailedProviderModelProbe = (options: ProviderOptions | undefined): boolean => {
  if (!options) return false;
  if (options.probe_ok === false) return true;
  return typeof options.probe_error === "string" && options.probe_error.trim().length > 0;
};

export const shouldHydrateProviderModels = (
  providerId: string,
  options: ProviderOptions | undefined,
  trigger: ProviderAuthSummaryTrigger = "passive",
): boolean => {
  if (!MODEL_DISCOVERY_PROVIDER_IDS.has(providerId)) return false;
  if (!options) return false;
  if (options.has_active_auth !== true) return false;
  if (hasProviderModels(options)) return false;
  if (trigger === "passive" && hasFailedProviderModelProbe(options)) return false;
  return true;
};

export { resolveProviderOptionsUpdate } from "../../state/providersBootstrapStore";

const withScopedProviderAccountIdentity = (
  scoped: ProviderOptions | undefined,
  next: ProviderOptions | undefined,
): ProviderOptions | undefined => {
  if (!next || !scoped) return next;
  const accountIdentity = scoped.account_identity ?? null;
  if (next.account_identity === accountIdentity) return next;
  return {
    ...next,
    account_identity: accountIdentity,
  };
};

const toErrorMessage = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  return String(error);
};

const toProviderInstallState = (
  session: NonNullable<ReturnType<typeof resolveProviderInstallProgressSession>>,
): ProviderInstallState => ({
  installId: session.installId,
  state: session.state,
  pct: session.pct,
  target: session.target,
  errorCode: session.errorCode,
  error: session.error,
});

const providerInstallTargetForProvider = (
  provider: ProviderStatus | undefined,
): InstallTarget | undefined => parseInstallTarget(provider?.details?.install_target);

const providerInstallsFromSnapshot = (
  snapshot: ProviderInstallProgressSnapshot,
  providersById: Record<string, ProviderStatus>,
): Record<string, ProviderInstallState | undefined> =>
  Object.fromEntries(
    Array.from(new Set([...Object.keys(snapshot), ...Object.keys(providersById)]))
      .map((providerId) => {
        const session = resolveProviderInstallProgressSession(
          snapshot,
          providerId,
          providerInstallTargetForProvider(providersById[providerId]),
        );
        return session ? ([providerId, toProviderInstallState(session)] as const) : null;
      })
      .filter((entry): entry is readonly [string, ProviderInstallState] => entry !== null),
  );

export function useWorkbenchProviders({
  workspaceId,
  setDraftHarness,
  onStartError,
}: UseWorkbenchProvidersArgs) {
  const [providerInstallsById, setProviderInstallsById] = useState<Record<string, ProviderInstallState | undefined>>(
    () => providerInstallsFromSnapshot(getProviderInstallProgressSnapshot(), {}),
  );
  const [installAllBusy, setInstallAllBusy] = useState(false);
  const postInstallHandledRef = useRef<Set<string>>(new Set());
  const postInstallInFlightRef = useRef<Set<string>>(new Set());
  const providerAuthSummaryInFlightRef = useRef<Record<string, Promise<ProviderOptions | undefined>>>({});
  const installObserversRef = useRef<Record<string, () => void>>({});
  const previousInstallsRef = useRef<Record<string, ProviderInstallState | undefined>>({});
  const providersByIdRef = useRef<Record<string, ProviderStatus>>({});

  const bootstrap = useSyncExternalStore(
    useCallback((onStoreChange) => subscribeProvidersBootstrap(workspaceId, onStoreChange), [workspaceId]),
    useCallback(() => getProvidersBootstrapSnapshot(workspaceId), [workspaceId]),
    useCallback(() => getProvidersBootstrapSnapshot(workspaceId), [workspaceId]),
  );
  const providers = bootstrap.providers;
  const providerOptions = bootstrap.provider_options;

  const providersById = useMemo(
    () => Object.fromEntries(providers.map((provider) => [provider.provider_id, provider])),
    [providers],
  );

  useEffect(() => {
    providersByIdRef.current = providersById;
    const next = providerInstallsFromSnapshot(getProviderInstallProgressSnapshot(), providersById);
    setProviderInstallsById((prev) => (sameProviderInstallStateMap(prev, next) ? prev : next));
  }, [providersById]);

  useEffect(() => {
    return subscribeProviderInstallProgress((snapshot) => {
      const next = providerInstallsFromSnapshot(snapshot, providersByIdRef.current);
      setProviderInstallsById((prev) => (sameProviderInstallStateMap(prev, next) ? prev : next));
    });
  }, []);

  useEffect(() => {
    providerAuthSummaryInFlightRef.current = {};
  }, [workspaceId]);

  const defaultProviderId = useMemo(() => {
    const installed = providers
      .filter((provider) => provider.installed && provider.health === "ok" && provider.details?.ui_hidden !== "true")
      .map((provider) => provider.provider_id);
    if (installed.includes("codex")) return "codex";
    if (installed.includes("claude-crp")) return "claude-crp";
    if (installed.includes("gemini")) return "gemini";
    if (installed.includes("qwen")) return "qwen";
    if (installed.includes("opencode")) return "opencode";
    if (installed.includes("mistral")) return "mistral";
    if (installed.includes("goose")) return "goose";
    if (installed.includes("kimi")) return "kimi";
    if (installed.includes("auggie")) return "auggie";
    return installed[0] ?? "codex";
  }, [providers]);

  useEffect(() => {
    loadProvidersBootstrap(workspaceId).catch(() => {});
  }, [workspaceId]);

  useEffect(() => {
    const refreshOnForeground = () => {
      refreshProvidersBootstrap(workspaceId).catch(() => {});
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
  }, [workspaceId]);

  const attachProviderInstall = useCallback((providerId: string, installId: string, initialTarget?: InstallTarget) => {
    const existingInstallId = providerInstallsById[providerId]?.installId;
    if (existingInstallId === installId && installObserversRef.current[providerId]) {
      return;
    }
    installObserversRef.current[providerId]?.();

    const nextInstallState: ProviderInstallState = {
      installId,
      state: "running",
      pct: 0,
      target: initialTarget,
      errorCode: undefined,
      error: undefined,
    };
    setProviderInstallsById((prev) => {
      if (prev[providerId]?.installId === installId) return prev;
      return {
        ...prev,
        [providerId]: {
          ...nextInstallState,
          pct: prev[providerId]?.pct ?? 0,
          target: prev[providerId]?.target,
        },
      };
    });
    installObserversRef.current[providerId] = observeInstall(installId, {
      providerId,
      initialState: nextInstallState,
    });
  }, [providerInstallsById]);

  useEffect(() => {
    for (const provider of providers) {
      const installId = provider.details?.install_id;
      const running = provider.details?.install_running === "true";
      const tracked = providerInstallsById[provider.provider_id];
      if (
        running
        && installId
        && (!tracked || tracked.installId !== installId || tracked.state !== "running")
      ) {
        attachProviderInstall(
          provider.provider_id,
          installId,
          providerInstallTargetForProvider(provider),
        );
      }
    }
  }, [attachProviderInstall, providerInstallsById, providers]);

  const runPostInstallAuthVerify = useCallback(
    async (providerId: string) => {
      const refreshed = await refreshProvidersBootstrap(workspaceId);
      return refreshed.provider_options[providerId];
    },
    [workspaceId],
  );

  useEffect(() => {
    let needsProviderRefresh = false;
    const completedProviders: string[] = [];

    for (const [providerId, install] of Object.entries(providerInstallsById)) {
      const previous = previousInstallsRef.current[providerId];
      if (!install || install.state === "running") continue;
      if (previous?.installId === install.installId && previous.state === install.state) continue;

      needsProviderRefresh = true;
      if (install.state === "succeeded" && !postInstallHandledRef.current.has(install.installId)) {
        postInstallHandledRef.current.add(install.installId);
        completedProviders.push(providerId);
      }
    }

    previousInstallsRef.current = providerInstallsById;
    if (!needsProviderRefresh) return;

    let cancelled = false;
    void refreshProvidersBootstrap(workspaceId)
      .then(() => {
        if (cancelled) return;
        for (const providerId of completedProviders) {
          if (postInstallInFlightRef.current.has(providerId)) continue;
          postInstallInFlightRef.current.add(providerId);
          runPostInstallAuthVerify(providerId).finally(() => {
            postInstallInFlightRef.current.delete(providerId);
          });
        }
      })
      .catch(() => {});

    return () => {
      cancelled = true;
    };
  }, [providerInstallsById, runPostInstallAuthVerify, workspaceId]);

  const installProviderFromMenu = useCallback(
    async (providerId: string) => {
      onStartError(null);
      try {
        const target = parseInstallTarget(providersById[providerId]?.details?.install_target);
        const started = await installProvider(providerId, target);
        attachProviderInstall(providerId, started.install_id, started.target);
      } catch (error: unknown) {
        onStartError(toErrorMessage(error));
      }
    },
    [attachProviderInstall, onStartError, providersById],
  );

  const installAllProvidersFromMenu = useCallback(async () => {
    onStartError(null);
    setInstallAllBusy(true);
    try {
      const target = parseInstallTarget(
        providers.find((provider) => provider.details?.install_target)?.details?.install_target,
      ) ?? "host";
      const installs = await installAllProviders(target);
      for (const { provider_id: providerId, install_id: installId, target: installTarget } of installs) {
        attachProviderInstall(providerId, installId, installTarget);
      }
    } catch (error: unknown) {
      onStartError(toErrorMessage(error));
    } finally {
      setInstallAllBusy(false);
    }
  }, [attachProviderInstall, onStartError, providers]);

  const cancelProviderInstallFromMenu = useCallback(
    async (providerId: string) => {
      onStartError(null);
      const installId = providerInstallsById[providerId]?.installId ?? providersById[providerId]?.details?.install_id;
      if (!installId) return;
      try {
        const info = await cancelInstall(installId);
        setProviderInstallsById((prev) => ({
          ...prev,
          [providerId]: {
            installId,
            state: info.state,
            pct: computeInstallPct(info, prev[providerId]?.pct ?? null),
            target: info.target,
            errorCode: info.error_code,
            error: info.error,
          },
        }));
      } catch (error: unknown) {
        onStartError(toErrorMessage(error));
      }
    },
    [onStartError, providerInstallsById, providersById],
  );

  useEffect(() => {
    return () => {
      for (const stop of Object.values(installObserversRef.current)) {
        stop();
      }
      installObserversRef.current = {};
    };
  }, []);

  useEffect(() => {
    if (Object.keys(providerInstallsById).length === 0) return;
    for (const [providerId, install] of Object.entries(providerInstallsById)) {
      const state = providersById[providerId];
      const stillRunning = state?.details?.install_running === "true";
      if (install?.state === "succeeded" && state?.installed && state.health === "ok" && !stillRunning) {
        removeProviderInstallProgress(
          providerId,
          install.target
            ? { target: install.target, installId: install.installId }
            : { installId: install.installId },
        );
      }
    }
  }, [providerInstallsById, providersById]);

  useEffect(() => {
    if (!providers.length) return;
    const codexInstalled =
      providersById.codex?.installed === true &&
      providersById.codex?.health === "ok" &&
      providersById.codex?.details?.ui_hidden !== "true";
    if (codexInstalled || defaultProviderId === "codex") return;
    setDraftHarness((prev) => {
      if (!prev) return prev;
      const isDefault = prev.providerId === "codex" && prev.modelId.trim().length === 0;
      if (!isDefault) return prev;
      return { ...prev, providerId: defaultProviderId };
    });
  }, [defaultProviderId, providers.length, providersById, setDraftHarness]);

  const ensureProviderAuthSummary = useCallback(
    async (
      providerId: string,
      opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger },
    ): Promise<ProviderOptions | undefined> => {
      const ready = providersById[providerId]?.installed === true && providersById[providerId]?.health === "ok";
      if (!ready) return;

      const force = opts?.force ?? false;
      const trigger = opts?.trigger ?? (force ? "explicit" : "passive");
      const requestKey = `${workspaceId}:${providerId}`;
      const existing = providerAuthSummaryInFlightRef.current[requestKey];
      if (existing && !force) return existing;

      const cached = getProvidersBootstrapSnapshot(workspaceId).provider_options[providerId];
      if (!force && cached && !shouldHydrateProviderModels(providerId, cached, trigger)) {
        return cached;
      }

      const request = (force ? refreshProvidersBootstrap(workspaceId) : loadProvidersBootstrap(workspaceId))
        .then(async (latestBootstrap) => {
          let next = resolveProviderOptionsUpdate(
            cached,
            latestBootstrap.provider_options[providerId],
          );

          if (shouldHydrateProviderModels(providerId, next, trigger)) {
            try {
              const detailedResponse = await getProviderOptions(workspaceId, providerId);
              const detailed = withScopedProviderAccountIdentity(
                getProvidersBootstrapSnapshot(workspaceId).provider_options[providerId],
                detailedResponse,
              ) ?? detailedResponse;
              const updated = updateProvidersBootstrap(workspaceId, (current) => {
                const resolved = resolveProviderOptionsUpdate(current.provider_options[providerId], detailed)
                  ?? detailed;
                if (current.provider_options[providerId] === resolved) return current;
                const nextProviderOptions: Record<string, ProviderOptions> = {
                  ...current.provider_options,
                  [providerId]: resolved,
                };
                return {
                  ...current,
                  provider_options: nextProviderOptions,
                };
              });
              next = updated.provider_options[providerId];
            } catch {
              // Keep bootstrap options when probe is unavailable; caller still gets auth summary.
            }
          }

          return next;
        })
        .finally(() => {
          if (providerAuthSummaryInFlightRef.current[requestKey] === request) {
            delete providerAuthSummaryInFlightRef.current[requestKey];
          }
        });

      providerAuthSummaryInFlightRef.current[requestKey] = request;
      return request;
    },
    [providersById, workspaceId],
  );

  return {
    providersById,
    defaultProviderId,
    providerInstallsById,
    providerOptions,
    installAllBusy,
    installProviderFromMenu,
    cancelProviderInstallFromMenu,
    installAllProvidersFromMenu,
    ensureProviderAuthSummary,
  };
}
