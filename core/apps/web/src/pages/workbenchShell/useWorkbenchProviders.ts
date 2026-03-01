import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import {
  cancelInstall,
  getProviderOptions,
  getInstall,
  installAllProviders,
  installProvider,
  type InstallInfo,
  type InstallTarget,
  type ProviderOptions,
  type ProviderStatus,
} from "../../api/client";
import {
  loadProvidersBootstrap,
  refreshProvidersBootstrap,
} from "../../state/providersBootstrapStore";
import {
  getProviderInstallProgressSnapshot,
  removeProviderInstallProgress,
  subscribeProviderInstallProgress,
  upsertProviderInstallProgress,
  type ProviderInstallProgressSnapshot,
} from "../../state/providerInstallProgressStore";
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

const MODEL_DISCOVERY_PROVIDER_IDS = new Set(["codex", "claude-crp"]);

const hasProviderModels = (options: ProviderOptions | undefined): boolean => {
  const raw = options?.models;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return false;
  const record = raw as Record<string, unknown>;
  const current = record.currentModelId ?? record.current_model_id;
  if (typeof current === "string" && current.trim().length > 0) return true;
  const list = record.availableModels ?? record.available_models ?? record.models;
  return Array.isArray(list) && list.length > 0;
};

export const shouldHydrateProviderModels = (
  providerId: string,
  options: ProviderOptions | undefined,
): boolean => {
  if (!MODEL_DISCOVERY_PROVIDER_IDS.has(providerId)) return false;
  if (!options) return false;
  if (options.has_active_auth !== true) return false;
  if (options.source?.selected_source_kind === "endpoint") return false;
  return !hasProviderModels(options);
};

const toErrorMessage = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  return String(error);
};

const toProviderInstallState = (
  session: ProviderInstallProgressSnapshot[string],
): ProviderInstallState => ({
  installId: session.installId,
  state: session.state,
  pct: session.pct,
  target: session.target,
  errorCode: session.errorCode,
  error: session.error,
});

const providerInstallsFromSnapshot = (
  snapshot: ProviderInstallProgressSnapshot,
): Record<string, ProviderInstallState | undefined> =>
  Object.fromEntries(
    Object.entries(snapshot).map(([providerId, session]) => [providerId, toProviderInstallState(session)]),
  );

export function useWorkbenchProviders({
  workspaceId,
  setDraftHarness,
  onStartError,
}: UseWorkbenchProvidersArgs) {
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerInstallsById, setProviderInstallsById] = useState<Record<string, ProviderInstallState | undefined>>(
    () => providerInstallsFromSnapshot(getProviderInstallProgressSnapshot()),
  );
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions | undefined>>({});
  const [installAllBusy, setInstallAllBusy] = useState(false);
  const postInstallHandledRef = useRef<Set<string>>(new Set());
  const postInstallInFlightRef = useRef<Set<string>>(new Set());
  const providerAuthSummaryInFlightRef = useRef<Record<string, Promise<ProviderOptions | undefined>>>({});

  useEffect(() => {
    return subscribeProviderInstallProgress((snapshot) => {
      const next = providerInstallsFromSnapshot(snapshot);
      setProviderInstallsById((prev) => (sameProviderInstallStateMap(prev, next) ? prev : next));
    });
  }, []);

  const applyProvidersBootstrap = useCallback((bootstrap: Awaited<ReturnType<typeof loadProvidersBootstrap>>) => {
    setProviders(bootstrap.providers);
    setProviderOptions(bootstrap.provider_options);
  }, []);

  const providersById = useMemo(
    () => Object.fromEntries(providers.map((provider) => [provider.provider_id, provider])),
    [providers],
  );

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

  const refreshProviders = useCallback(async () => {
    if (!workspaceId) {
      setProviders([]);
      setProviderOptions({});
      return;
    }
    try {
      const next = await refreshProvidersBootstrap(workspaceId);
      applyProvidersBootstrap(next);
    } catch {
      setProviders([]);
      setProviderOptions({});
    }
  }, [applyProvidersBootstrap, workspaceId]);

  useEffect(() => {
    if (!workspaceId) {
      setProviders([]);
      setProviderOptions({});
      return;
    }
    loadProvidersBootstrap(workspaceId)
      .then((bootstrap) => {
        applyProvidersBootstrap(bootstrap);
      })
      .catch(() => {
        setProviders([]);
        setProviderOptions({});
      });
  }, [applyProvidersBootstrap, workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    const refreshOnForeground = () => {
      refreshProvidersBootstrap(workspaceId)
        .then((bootstrap) => applyProvidersBootstrap(bootstrap))
        .catch(() => {});
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
  }, [applyProvidersBootstrap, workspaceId]);

  const attachProviderInstall = useCallback((providerId: string, installId: string) => {
    const nextInstallState: ProviderInstallState = {
      installId,
      state: "running",
      pct: 0,
      target: undefined,
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
    upsertProviderInstallProgress(providerId, nextInstallState);
  }, []);

  useEffect(() => {
    for (const provider of providers) {
      const installId = provider.details?.install_id;
      const running = provider.details?.install_running === "true";
      if (running && installId && !providerInstallsById[provider.provider_id]) {
        attachProviderInstall(provider.provider_id, installId);
      }
    }
  }, [attachProviderInstall, providerInstallsById, providers]);

  const runPostInstallAuthVerify = useCallback(
    async (providerId: string) => {
      if (!workspaceId) return;
      const bootstrap = await refreshProvidersBootstrap(workspaceId);
      applyProvidersBootstrap(bootstrap);
      return bootstrap.provider_options[providerId];
    },
    [applyProvidersBootstrap, workspaceId],
  );

  useEffect(() => {
    const active = Object.entries(providerInstallsById).flatMap(([providerId, install]) =>
      install && (install.state === "running" || install.state === "succeeded") ? ([[providerId, install]] as const) : [],
    );
    if (active.length === 0) return;

    let cancelled = false;
    const tick = async () => {
      if (cancelled) return;
      let needsProviderRefresh = false;
      const completedProviders: string[] = [];
      await Promise.all(
        active.map(async ([providerId, install]) => {
          try {
            const info = await getInstall(install.installId);
            const pct = computeInstallPct(info, install.pct);

            setProviderInstallsById((prev) => {
              const existing = prev[providerId];
              if (!existing || existing.installId !== install.installId) return prev;
              const stablePct =
                info.state === "succeeded"
                  ? 100
                  : typeof pct === "number" && Number.isFinite(pct)
                    ? Math.max(existing.pct ?? 0, pct)
                    : (existing.pct ?? 0);
              const nextInstallState: ProviderInstallState = {
                installId: install.installId,
                state: info.state,
                pct: stablePct,
                target: info.target,
                errorCode: info.error_code,
                error: info.error,
              };
              if (sameProviderInstallState(existing, nextInstallState)) {
                return prev;
              }
              return {
                ...prev,
                [providerId]: nextInstallState,
              };
            });
            upsertProviderInstallProgress(providerId, {
              installId: install.installId,
              state: info.state,
              pct:
                info.state === "succeeded"
                  ? 100
                  : typeof pct === "number" && Number.isFinite(pct)
                    ? Math.max(install.pct ?? 0, pct)
                    : (install.pct ?? 0),
              target: info.target,
              errorCode: info.error_code,
              error: info.error,
            });

            if (info.state !== "running") {
              needsProviderRefresh = true;
            }

            if (info.state === "succeeded" && !postInstallHandledRef.current.has(install.installId)) {
              postInstallHandledRef.current.add(install.installId);
              completedProviders.push(providerId);
            }
          } catch {
            // ignore poll errors
          }
        }),
      );

      if (needsProviderRefresh) {
        try {
          const next = await refreshProvidersBootstrap(workspaceId);
          if (!cancelled) {
            applyProvidersBootstrap(next);
          }

          if (!cancelled && completedProviders.length > 0) {
            for (const providerId of completedProviders) {
              if (postInstallInFlightRef.current.has(providerId)) continue;
              postInstallInFlightRef.current.add(providerId);
              runPostInstallAuthVerify(providerId).finally(() => {
                postInstallInFlightRef.current.delete(providerId);
              });
            }
          }
        } catch {
          // ignore provider refresh errors
        }
      }
    };

    void tick();
    const timerId = window.setInterval(() => void tick(), 1000);
    return () => {
      cancelled = true;
      window.clearInterval(timerId);
    };
  }, [applyProvidersBootstrap, providerInstallsById, runPostInstallAuthVerify, workspaceId]);

  const installProviderFromMenu = useCallback(
    async (providerId: string) => {
      onStartError(null);
      try {
        const target = parseInstallTarget(providersById[providerId]?.details?.install_target);
        const { install_id: installId } = await installProvider(providerId, target);
        attachProviderInstall(providerId, installId);
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
      for (const { provider_id: providerId, install_id: installId } of installs) {
        attachProviderInstall(providerId, installId);
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
        upsertProviderInstallProgress(providerId, {
          installId,
          state: info.state,
          pct: computeInstallPct(info, providerInstallsById[providerId]?.pct ?? null),
          target: info.target,
          errorCode: info.error_code,
          error: info.error,
        });
      } catch (error: unknown) {
        onStartError(toErrorMessage(error));
      }
    },
    [onStartError, providerInstallsById, providersById],
  );

  useEffect(() => {
    if (Object.keys(providerInstallsById).length === 0) return;
    for (const [providerId, install] of Object.entries(providerInstallsById)) {
      const state = providersById[providerId];
      const stillRunning = state?.details?.install_running === "true";
      if (install?.state === "succeeded" && state?.installed && state.health === "ok" && !stillRunning) {
        removeProviderInstallProgress(providerId);
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

  // This loads workspace-scoped auth/config summary from providers/bootstrap.
  // For model-capable subscription providers (Codex/Claude), it additionally
  // hydrates detailed provider options once after auth so model catalogs appear.
  const ensureProviderAuthSummary = useCallback(
    async (providerId: string, opts?: { force?: boolean }): Promise<ProviderOptions | undefined> => {
      if (!workspaceId) return;
      const ready = providersById[providerId]?.installed === true && providersById[providerId]?.health === "ok";
      if (!ready) return;

      const force = opts?.force ?? false;
      const existing = providerAuthSummaryInFlightRef.current[providerId];
      if (existing && !force) return existing;
      const cached = providerOptions[providerId];
      if (!force && cached && !shouldHydrateProviderModels(providerId, cached)) {
        return cached;
      }

      const request = (force ? refreshProvidersBootstrap(workspaceId) : loadProvidersBootstrap(workspaceId))
        .then(async (bootstrap) => {
          applyProvidersBootstrap(bootstrap);
          let next = bootstrap.provider_options[providerId];
          if (shouldHydrateProviderModels(providerId, next)) {
            try {
              const detailed = await getProviderOptions(workspaceId, providerId);
              setProviderOptions((prev) => ({ ...prev, [providerId]: detailed }));
              next = detailed;
            } catch {
              // Keep bootstrap options when probe is unavailable; caller still gets auth summary.
            }
          }
          return next;
        })
        .finally(() => {
          if (providerAuthSummaryInFlightRef.current[providerId] === request) {
            delete providerAuthSummaryInFlightRef.current[providerId];
          }
        });

      providerAuthSummaryInFlightRef.current[providerId] = request;
      return request;
    },
    [applyProvidersBootstrap, providerOptions, providersById, workspaceId],
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
