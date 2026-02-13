import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import {
  getInstall,
  getProviderOptions,
  installAllProviders,
  installProvider,
  listProviders,
  type InstallInfo,
  type ProviderOptions,
  type ProviderStatus,
} from "../../api/client";
import type { DraftTrack } from "../../components/WorkbenchComposer";

type ProviderInstallState = {
  installId: string;
  state: InstallInfo["state"];
  pct: number | null;
};

type UseWorkbenchProvidersArgs = {
  workspaceId: string;
  setDraftTracks: Dispatch<SetStateAction<DraftTrack[]>>;
  onStartError: (message: string | null) => void;
};

const toErrorMessage = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  return String(error);
};

export function useWorkbenchProviders({
  workspaceId,
  setDraftTracks,
  onStartError,
}: UseWorkbenchProvidersArgs) {
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerInstallsById, setProviderInstallsById] = useState<Record<string, ProviderInstallState | undefined>>(
    {},
  );
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions | undefined>>({});
  const [installAllBusy, setInstallAllBusy] = useState(false);
  const postInstallHandledRef = useRef<Set<string>>(new Set());
  const postInstallInFlightRef = useRef<Set<string>>(new Set());
  const providerOptionsInFlightRef = useRef<Record<string, Promise<ProviderOptions | undefined>>>({});

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
      return;
    }
    try {
      const next = await listProviders();
      setProviders(next);
    } catch {
      setProviders([]);
    }
  }, [workspaceId]);

  useEffect(() => {
    void refreshProviders();
  }, [refreshProviders]);

  const attachProviderInstall = useCallback((providerId: string, installId: string) => {
    setProviderInstallsById((prev) => {
      if (prev[providerId]?.installId === installId) return prev;
      return {
        ...prev,
        [providerId]: {
          installId,
          state: "running",
          pct: prev[providerId]?.pct ?? 0,
        },
      };
    });
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

  const refreshProviderOptions = useCallback(
    async (providerId: string): Promise<ProviderOptions | undefined> => {
      if (!workspaceId) return;
      try {
        const options = await getProviderOptions(workspaceId, providerId);
        setProviderOptions((prev) => ({ ...prev, [providerId]: options }));
        return options;
      } catch {
        return;
      }
    },
    [workspaceId],
  );

  const runPostInstallAuthVerify = useCallback(
    async (providerId: string) => {
      if (!workspaceId) return;
      await refreshProviderOptions(providerId);
    },
    [refreshProviderOptions, workspaceId],
  );

  useEffect(() => {
    const active = Object.entries(providerInstallsById).flatMap(([providerId, install]) =>
      install && (install.state === "running" || install.state === "succeeded") ? ([[providerId, install]] as const) : [],
    );
    if (active.length === 0) return;

    const stagePct: Record<string, number> = {
      start: 2,
      download: 10,
      node: 15,
      node_download: 18,
      node_extract: 22,
      prepare: 25,
      venv: 35,
      npm_install: 65,
      pip_install: 70,
      extract: 78,
      entrypoint: 80,
      inspect: 90,
      refresh: 95,
      registry: 98,
    };

    let cancelled = false;
    const tick = async () => {
      if (cancelled) return;
      let needsProviderRefresh = false;
      const completedProviders: string[] = [];
      await Promise.all(
        active.map(async ([providerId, install]) => {
          try {
            const info = await getInstall(install.installId);
            const last = info.last_event;
            const pct =
              typeof last?.bytes === "number" && typeof last?.total_bytes === "number" && last.total_bytes > 0
                ? (() => {
                    const raw = Math.max(0, Math.min(100, Math.round((last.bytes / last.total_bytes) * 100)));
                    const stage = typeof last?.stage === "string" ? last.stage : "";
                    if (stage.includes("download")) {
                      return Math.round((raw / 100) * 75);
                    }
                    return raw;
                  })()
                : typeof last?.stage === "string"
                  ? (stagePct[last.stage] ?? install.pct ?? 0)
                  : (install.pct ?? 0);

            setProviderInstallsById((prev) => {
              const existing = prev[providerId];
              if (!existing || existing.installId !== install.installId) return prev;
              const stablePct =
                info.state === "succeeded"
                  ? 100
                  : typeof pct === "number" && Number.isFinite(pct)
                    ? Math.max(existing.pct ?? 0, pct)
                    : (existing.pct ?? 0);
              return {
                ...prev,
                [providerId]: { installId: install.installId, state: info.state, pct: stablePct },
              };
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
          const next = await listProviders();
          if (!cancelled) {
            setProviders(next);
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
  }, [providerInstallsById, runPostInstallAuthVerify]);

  const installProviderFromMenu = useCallback(
    async (providerId: string) => {
      onStartError(null);
      try {
        const { install_id: installId } = await installProvider(providerId);
        attachProviderInstall(providerId, installId);
      } catch (error: unknown) {
        onStartError(toErrorMessage(error));
      }
    },
    [attachProviderInstall, onStartError],
  );

  const installAllProvidersFromMenu = useCallback(async () => {
    onStartError(null);
    setInstallAllBusy(true);
    try {
      const installs = await installAllProviders();
      for (const { provider_id: providerId, install_id: installId } of installs) {
        attachProviderInstall(providerId, installId);
      }
    } catch (error: unknown) {
      onStartError(toErrorMessage(error));
    } finally {
      setInstallAllBusy(false);
    }
  }, [attachProviderInstall, onStartError]);

  useEffect(() => {
    if (Object.keys(providerInstallsById).length === 0) return;
    setProviderInstallsById((prev) => {
      let changed = false;
      const next: typeof prev = { ...prev };
      for (const [providerId, install] of Object.entries(prev)) {
        const state = providersById[providerId];
        const stillRunning = state?.details?.install_running === "true";
        if (install?.state === "succeeded" && state?.installed && state.health === "ok" && !stillRunning) {
          delete next[providerId];
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [providerInstallsById, providersById]);

  useEffect(() => {
    if (!providers.length) return;
    const codexInstalled =
      providersById.codex?.installed === true &&
      providersById.codex?.health === "ok" &&
      providersById.codex?.details?.ui_hidden !== "true";
    if (codexInstalled || defaultProviderId === "codex") return;
    setDraftTracks((prev) => {
      const isDefault = prev.every((track) => track.providerId === "codex" && !track.label.trim() && !track.modelId.trim());
      if (!isDefault) return prev;
      return prev.map((track) => ({ ...track, providerId: defaultProviderId }));
    });
  }, [defaultProviderId, providers.length, providersById, setDraftTracks]);

  const ensureProviderOptions = useCallback(
    async (providerId: string, opts?: { force?: boolean }): Promise<ProviderOptions | undefined> => {
      if (!workspaceId) return;
      const installed = providersById[providerId]?.installed === true && providersById[providerId]?.health === "ok";
      if (!installed) return;

      const force = opts?.force ?? false;
      const existing = providerOptionsInFlightRef.current[providerId];
      if (existing && !force) return existing;
      if (!force && providerOptions[providerId]) return providerOptions[providerId];

      const request = getProviderOptions(workspaceId, providerId)
        .then((options) => {
          setProviderOptions((prev) => ({ ...prev, [providerId]: options }));
          return options;
        })
        .finally(() => {
          if (providerOptionsInFlightRef.current[providerId] === request) {
            delete providerOptionsInFlightRef.current[providerId];
          }
        });

      providerOptionsInFlightRef.current[providerId] = request;
      return request;
    },
    [providerOptions, providersById, workspaceId],
  );

  return {
    providersById,
    defaultProviderId,
    providerInstallsById,
    providerOptions,
    installAllBusy,
    installProviderFromMenu,
    installAllProvidersFromMenu,
    ensureProviderOptions,
  };
}
