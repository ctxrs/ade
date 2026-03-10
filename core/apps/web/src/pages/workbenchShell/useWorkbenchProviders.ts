import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
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
  loadProvidersBootstrap,
  refreshProvidersBootstrap,
} from "../../state/providersBootstrapStore";
import {
  getProviderInstallProgressSnapshot,
  removeProviderInstallProgress,
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

const MODEL_DISCOVERY_PROVIDER_IDS = new Set(["codex", "claude-crp"]);
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

const sameProviderOptionsScope = (
  lhs: ProviderOptions | undefined,
  rhs: ProviderOptions | undefined,
): boolean => {
  if (!lhs || !rhs) return false;
  return lhs.provider_id === rhs.provider_id
    && lhs.workspace_id === rhs.workspace_id
    && lhs.auth_mode === rhs.auth_mode
    && lhs.has_active_auth === rhs.has_active_auth
    && lhs.source?.selected_source_kind === rhs.source?.selected_source_kind
    && lhs.source?.selected_endpoint_id === rhs.source?.selected_endpoint_id;
};

const sameProviderOptions = (
  lhs: ProviderOptions | undefined,
  rhs: ProviderOptions | undefined,
): boolean => JSON.stringify(lhs ?? null) === JSON.stringify(rhs ?? null);

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

export const resolveProviderOptionsUpdate = (
  previous: ProviderOptions | undefined,
  next: ProviderOptions | undefined,
): ProviderOptions | undefined => {
  if (!next) return previous === undefined ? previous : next;
  let resolved = next;
  if (previous && sameProviderOptionsScope(previous, next) && next.has_active_auth === true && !hasProviderModels(next)) {
    if (hasProviderModels(previous)) {
      resolved = { ...resolved, models: previous.models };
    }
    if (hasFailedProviderModelProbe(previous) && !hasFailedProviderModelProbe(resolved)) {
      resolved = {
        ...resolved,
        probe_ok: previous.probe_ok,
        probe_error: previous.probe_error ?? resolved.probe_error,
      };
    }
  }
  return sameProviderOptions(previous, resolved) ? previous : resolved;
};

const mergeProviderOptionsMap = (
  previous: Record<string, ProviderOptions | undefined>,
  next: Record<string, ProviderOptions | undefined>,
): Record<string, ProviderOptions | undefined> => {
  const merged = Object.fromEntries(
    Object.entries(next).map(([providerId, options]) => [
      providerId,
      resolveProviderOptionsUpdate(previous[providerId], options),
    ]),
  );
  const previousKeys = Object.keys(previous);
  const mergedKeys = Object.keys(merged);
  if (
    previousKeys.length === mergedKeys.length
    && mergedKeys.every((providerId) => previous[providerId] === merged[providerId])
  ) {
    return previous;
  }
  return merged;
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
  const installObserversRef = useRef<Record<string, () => void>>({});
  const previousInstallsRef = useRef<Record<string, ProviderInstallState | undefined>>({});

  useEffect(() => {
    return subscribeProviderInstallProgress((snapshot) => {
      const next = providerInstallsFromSnapshot(snapshot);
      setProviderInstallsById((prev) => (sameProviderInstallStateMap(prev, next) ? prev : next));
    });
  }, []);

  const applyProvidersBootstrap = useCallback((bootstrap: Awaited<ReturnType<typeof loadProvidersBootstrap>>) => {
    setProviders(bootstrap.providers);
    setProviderOptions((prev) => mergeProviderOptionsMap(prev, bootstrap.provider_options));
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
    refreshProvidersBootstrap(workspaceId)
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
    const existingInstallId = providerInstallsById[providerId]?.installId;
    if (existingInstallId === installId && installObserversRef.current[providerId]) {
      return;
    }
    installObserversRef.current[providerId]?.();

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
    if (!needsProviderRefresh || !workspaceId) return;

    let cancelled = false;
    void refreshProvidersBootstrap(workspaceId)
      .then((next) => {
        if (cancelled) return;
        applyProvidersBootstrap(next);
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
    async (
      providerId: string,
      opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger },
    ): Promise<ProviderOptions | undefined> => {
      if (!workspaceId) return;
      const ready = providersById[providerId]?.installed === true && providersById[providerId]?.health === "ok";
      if (!ready) return;

      const force = opts?.force ?? false;
      const trigger = opts?.trigger ?? (force ? "explicit" : "passive");
      const existing = providerAuthSummaryInFlightRef.current[providerId];
      if (existing && !force) return existing;
      const cached = providerOptions[providerId];
      if (!force && cached && !shouldHydrateProviderModels(providerId, cached, trigger)) {
        return cached;
      }

      const request = (force ? refreshProvidersBootstrap(workspaceId) : loadProvidersBootstrap(workspaceId))
        .then(async (bootstrap) => {
          applyProvidersBootstrap(bootstrap);
          let next = resolveProviderOptionsUpdate(cached, bootstrap.provider_options[providerId]);
          if (shouldHydrateProviderModels(providerId, next, trigger)) {
            try {
              const detailed = await getProviderOptions(workspaceId, providerId);
              next = resolveProviderOptionsUpdate(next, detailed);
              setProviderOptions((prev) => {
                const resolved = resolveProviderOptionsUpdate(prev[providerId], next);
                if (prev[providerId] === resolved) return prev;
                return { ...prev, [providerId]: resolved };
              });
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
