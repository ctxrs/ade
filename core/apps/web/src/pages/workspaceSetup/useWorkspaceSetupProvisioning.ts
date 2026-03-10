import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from "react";
import type {
  InstallTarget,
  ProviderAuthImportCandidate,
  TitleGenerationLocalStatus,
  TitleGenerationSettings,
  ProviderStatus,
} from "../../api/client";
import {
  cancelInstall,
  getSettings,
  getTitleGenerationLocalStatus,
  importProviderAuthCandidates,
  installProvider,
  installTitleGenerationLocal,
  listProviderAuthImportCandidates,
  listProviders,
  updateSettings,
} from "../../api/client";
import { HARNESS_CATALOG } from "../../utils/harnessCatalog";
import {
  computeInstallPct,
  parseInstallTarget,
  providerInstallSizeBytes,
} from "../../utils/providerInstallUi";
import {
  buildSessionTitlingDraft,
  buildSessionTitlingPayload,
  DEFAULT_TITLE_LOCAL_MODEL_ID,
  DEFAULT_TITLE_REMOTE_BASE_URL,
  DEFAULT_TITLE_REMOTE_MODEL,
  resolveSessionTitlingReadiness,
  sessionTitlingPayloadHash,
  type SessionTitlingMode,
} from "../WorkspaceSetupPage.logic";
import { isCurrentFlowRunToken, nextFlowRunToken, type FlowRunToken } from "./flowController";
import {
  nextAfterAuthImport,
  nextAfterHarnessDownloads,
  type WizardRoutePlan,
  type WizardStepKey,
} from "./wizardFlow";
import { buildOnboardingAfterConnectResult, buildWizardRoutePlan } from "./routePlanner";
import type { WizardSelections } from "./wizardFlowReducer";
import {
  messageFromError,
  resolveHarnessInstallCandidateStatus,
  type HarnessInstallProviderRow,
  type HarnessInstallRowState,
  type LocalInstallState,
  type RemoteStatus,
} from "./wizardTypes";
import {
  observeInstall,
  subscribeInstallProgress,
  type InstallProgressSnapshot,
} from "../../state/installProgressMonitor";
import {
  resolveProviderInstallProgressSession,
  subscribeProviderInstallProgress,
  upsertProviderInstallProgress,
} from "../../state/providerInstallProgressStore";
import type {
  EnsureOnboardingAfterDaemonConnectResult,
  WorkspaceSetupEffectiveTarget,
} from "./workflowTypes";

type UseWorkspaceSetupProvisioningArgs = {
  currentStepKey: WizardStepKey;
  currentStepKeyRef: MutableRefObject<WizardStepKey>;
  selections: WizardSelections;
  routePlan: WizardRoutePlan | null;
  setRoutePlan: (routePlan: WizardRoutePlan | null) => void;
  setRoutePlanningBusy: (busy: boolean) => void;
  invalidateRoutePlan: () => void;
  desktopApp: boolean;
  effectiveTarget: WorkspaceSetupEffectiveTarget | null;
  remoteStatus: RemoteStatus;
  remoteStatusRef: MutableRefObject<RemoteStatus>;
  connectDaemonForImport: (locationOverride?: "local" | "remote") => Promise<void>;
};

type RemoteScanKeyInput = {
  user?: string | null;
  host?: string | null;
  port?: number | null;
  dataDir?: string | null;
};

export const buildWorkspaceSetupAuthImportScanKey = (
  target: "local" | "remote",
  remote: RemoteScanKeyInput,
): string => (
  target === "local"
    ? "local|@"
    : `remote|${remote.user ?? ""}@${remote.host ?? ""}:${remote.port ?? 4399}:${remote.dataDir?.trim() ?? ""}`
);

export const buildWorkspaceSetupHarnessInstallScanKey = (
  target: "local" | "remote",
  installTarget: InstallTarget,
  remote: RemoteScanKeyInput,
): string => (
  target === "local"
    ? `local|@|${installTarget}`
    : `remote|${remote.user ?? ""}@${remote.host ?? ""}:${remote.port ?? 4399}:${remote.dataDir?.trim() ?? ""}|${installTarget}`
);

export function useWorkspaceSetupProvisioning({
  currentStepKey,
  currentStepKeyRef,
  selections,
  routePlan,
  setRoutePlan,
  setRoutePlanningBusy,
  invalidateRoutePlan,
  desktopApp,
  effectiveTarget,
  remoteStatus,
  remoteStatusRef,
  connectDaemonForImport,
}: UseWorkspaceSetupProvisioningArgs) {
  const [authImportCandidates, setAuthImportCandidates] = useState<ProviderAuthImportCandidate[]>([]);
  const [authImportSelected, setAuthImportSelected] = useState<Record<string, boolean>>({});
  const [authImportBusy, setAuthImportBusy] = useState(false);
  const [authImportError, setAuthImportError] = useState<string | null>(null);
  const [authImportScannedKey, setAuthImportScannedKey] = useState<string | null>(null);
  const [authImportDeferredKey, setAuthImportDeferredKey] = useState<string | null>(null);
  const [harnessInstallCandidates, setHarnessInstallCandidates] = useState<HarnessInstallProviderRow[]>([]);
  const [harnessInstallSelected, setHarnessInstallSelected] = useState<Record<string, boolean>>({});
  const [harnessInstallBusy, setHarnessInstallBusy] = useState(false);
  const [harnessInstallError, setHarnessInstallError] = useState<string | null>(null);
  const [harnessInstallScannedKey, setHarnessInstallScannedKey] = useState<string | null>(null);
  const [harnessInstallDeferredKey, setHarnessInstallDeferredKey] = useState<string | null>(null);
  const [harnessInstallRows, setHarnessInstallRows] = useState<Record<string, HarnessInstallRowState>>({});
  const [titlingProbeBusy, setTitlingProbeBusy] = useState(false);
  const [titlingProbeError, setTitlingProbeError] = useState<string | null>(null);
  const [titlingProbeDone, setTitlingProbeDone] = useState(false);
  const [titlingConfiguredReady, setTitlingConfiguredReady] = useState(false);
  const [titlingStepRequired, setTitlingStepRequired] = useState(false);
  const [titlingProbeTargetKey, setTitlingProbeTargetKey] = useState<string | null>(null);
  const [titlingMode, setTitlingMode] = useState<SessionTitlingMode>("unset");
  const [titlingRemoteBaseUrl, setTitlingRemoteBaseUrl] = useState(DEFAULT_TITLE_REMOTE_BASE_URL);
  const [titlingRemoteApiKey, setTitlingRemoteApiKey] = useState("");
  const [titlingRemoteModel, setTitlingRemoteModel] = useState(DEFAULT_TITLE_REMOTE_MODEL);
  const [titlingRemoteUseJson, setTitlingRemoteUseJson] = useState(true);
  const [titlingRemoteAdvancedOpen, setTitlingRemoteAdvancedOpen] = useState(false);
  const [titlingLocalUseJson, setTitlingLocalUseJson] = useState(true);
  const [titlingLocalStatus, setTitlingLocalStatus] = useState<TitleGenerationLocalStatus | null>(null);
  const [titlingLocalStatusBusy, setTitlingLocalStatusBusy] = useState(false);
  const [titlingLocalStatusRequestedTargetKey, setTitlingLocalStatusRequestedTargetKey] = useState<string | null>(null);
  const [titlingStatusError, setTitlingStatusError] = useState<string | null>(null);
  const [titlingLocalInstallBusy, setTitlingLocalInstallBusy] = useState(false);
  const [titlingLocalInstall, setTitlingLocalInstall] = useState<LocalInstallState | null>(null);
  const [titlingPersistBusy, setTitlingPersistBusy] = useState(false);
  const [titlingPersistError, setTitlingPersistError] = useState<string | null>(null);
  const [titlingPersistedTargetKey, setTitlingPersistedTargetKey] = useState<string | null>(null);
  const [titlingPersistedHash, setTitlingPersistedHash] = useState<string | null>(null);
  const [titlingExistingSettings, setTitlingExistingSettings] = useState<TitleGenerationSettings | null>(null);

  const selectedDaemonTargetKeyRef = useRef<string | null>(null);
  const authImportScanPromiseRef = useRef<Promise<ProviderAuthImportCandidate[]> | null>(null);
  const authImportScanKeyRef = useRef<string | null>(null);
  const authImportScanRunRef = useRef<FlowRunToken | null>(null);
  const harnessInstallScanPromiseRef = useRef<Promise<HarnessInstallProviderRow[]> | null>(null);
  const harnessInstallScanKeyRef = useRef<string | null>(null);
  const harnessInstallScanRunRef = useRef<FlowRunToken | null>(null);
  const routePlanRunRef = useRef<FlowRunToken | null>(null);
  const titlingProbePromiseRef = useRef<Promise<boolean | null> | null>(null);
  const titlingProbePromiseTargetKeyRef = useRef<string | null>(null);
  const titlingInstallObserverRef = useRef<{ installId: string; stop: () => void } | null>(null);
  const titlingInstallStateRef = useRef<LocalInstallState | null>(null);
  const harnessInstallObserversRef = useRef<Record<string, { installId: string; stop: () => void }>>({});
  const previousTargetKeyRef = useRef<string | null>(null);

  const harnessByProviderId = useMemo(() => {
    return new Map(HARNESS_CATALOG.map((entry) => [entry.id, entry]));
  }, []);

  const selectedDaemonTargetKey = effectiveTarget?.targetKey ?? null;
  const remoteTarget = effectiveTarget?.kind === "remote" ? effectiveTarget : null;
  const parsedRemoteHost = remoteTarget?.host;
  const parsedRemoteUser = remoteTarget?.user;
  const parsedRemotePort = remoteTarget?.port ?? null;
  const remoteDataDirInput = remoteTarget?.dataDirInput ?? "";
  const authImportStepVisible = Boolean(routePlan?.includeAuthImport);
  const harnessInstallStepVisible = Boolean(routePlan?.includeHarnessDownloads);
  const titlingStepVisible = Boolean(routePlan?.includeTitling);
  const canProbeTitling = desktopApp && (
    selections.location === "local"
    || (
      selections.location === "remote"
      && Boolean(parsedRemoteHost)
      && remoteStatus === "connected"
    )
  );
  const selectedHarnessInstallTarget: InstallTarget =
    selections.container && selections.container !== "no-container" ? "container" : "host";

  const authImportScanKeyForTarget = useCallback((target: "local" | "remote"): string => (
    buildWorkspaceSetupAuthImportScanKey(target, {
      user: parsedRemoteUser,
      host: parsedRemoteHost,
      port: parsedRemotePort,
      dataDir: remoteDataDirInput,
    })
  ), [parsedRemoteHost, parsedRemotePort, parsedRemoteUser, remoteDataDirInput]);

  const harnessInstallScanKeyForTarget = useCallback((
    target: "local" | "remote",
    containerSelectionOverride?: string,
  ): string => {
    const containerSelection = containerSelectionOverride ?? selections.container;
    const installTarget: InstallTarget =
      containerSelection && containerSelection !== "no-container" ? "container" : "host";
    return buildWorkspaceSetupHarnessInstallScanKey(target, installTarget, {
      user: parsedRemoteUser,
      host: parsedRemoteHost,
      port: parsedRemotePort,
      dataDir: remoteDataDirInput,
    });
  }, [parsedRemoteHost, parsedRemotePort, parsedRemoteUser, remoteDataDirInput, selections.container]);

  const resetTitlingDraft = () => {
    setTitlingMode("unset");
    setTitlingRemoteBaseUrl(DEFAULT_TITLE_REMOTE_BASE_URL);
    setTitlingRemoteApiKey("");
    setTitlingRemoteModel(DEFAULT_TITLE_REMOTE_MODEL);
    setTitlingRemoteUseJson(true);
    setTitlingLocalUseJson(true);
  };

  const invalidateTitlingPersisted = () => {
    setTitlingPersistError(null);
    setTitlingPersistedTargetKey(null);
    setTitlingPersistedHash(null);
  };

  const clearHarnessInstallObserver = (providerId?: string) => {
    if (providerId) {
      const active = harnessInstallObserversRef.current[providerId];
      if (!active) return;
      active.stop();
      delete harnessInstallObserversRef.current[providerId];
      return;
    }
    for (const active of Object.values(harnessInstallObserversRef.current)) {
      active.stop();
    }
    harnessInstallObserversRef.current = {};
  };

  const clearTitlingInstallObserver = () => {
    titlingInstallObserverRef.current?.stop();
    titlingInstallObserverRef.current = null;
  };

  const resetProvisioningState = useCallback(() => {
    authImportScanPromiseRef.current = null;
    authImportScanKeyRef.current = null;
    authImportScanRunRef.current = null;
    harnessInstallScanPromiseRef.current = null;
    harnessInstallScanKeyRef.current = null;
    harnessInstallScanRunRef.current = null;
    titlingProbePromiseRef.current = null;
    titlingProbePromiseTargetKeyRef.current = null;
    clearHarnessInstallObserver();
    clearTitlingInstallObserver();
    setAuthImportBusy(false);
    setAuthImportCandidates([]);
    setAuthImportSelected({});
    setAuthImportError(null);
    setAuthImportScannedKey(null);
    setAuthImportDeferredKey(null);
    setHarnessInstallBusy(false);
    setHarnessInstallCandidates([]);
    setHarnessInstallSelected({});
    setHarnessInstallRows({});
    setHarnessInstallError(null);
    setHarnessInstallScannedKey(null);
    setHarnessInstallDeferredKey(null);
    setTitlingProbeBusy(false);
    setTitlingProbeError(null);
    setTitlingProbeDone(false);
    setTitlingConfiguredReady(false);
    setTitlingStepRequired(false);
    setTitlingProbeTargetKey(null);
    setTitlingLocalStatus(null);
    setTitlingLocalStatusRequestedTargetKey(null);
    setTitlingStatusError(null);
    setTitlingLocalInstall(null);
    setTitlingExistingSettings(null);
    invalidateTitlingPersisted();
    resetTitlingDraft();
  }, []);

  const mapHarnessInstallCandidate = (
    provider: ProviderStatus,
    fallbackInstallTarget: InstallTarget = selectedHarnessInstallTarget,
  ): HarnessInstallProviderRow | null => {
    if (provider.details?.ui_hidden === "true") return null;
    const installSupported = provider.details?.install_supported === "true";
    if (!installSupported) return null;
    const harness = harnessByProviderId.get(provider.provider_id);
    const installTarget = parseInstallTarget(provider.details?.install_target) ?? fallbackInstallTarget;
    return {
      providerId: provider.provider_id,
      label: harness?.label ?? provider.provider_id,
      installed: provider.installed === true,
      healthy: provider.health === "ok",
      installSupported,
      installRunning: provider.details?.install_running === "true",
      installId: provider.details?.install_id,
      installTarget,
      installSizeBytes: providerInstallSizeBytes(provider),
    };
  };

  const attachHarnessInstall = async (providerId: string, installId: string) => {
    if (!providerId || !installId) return;
    const active = harnessInstallObserversRef.current[providerId];
    if (active?.installId === installId) return;
    active?.stop();
    harnessInstallObserversRef.current[providerId] = {
      installId,
      stop: observeInstall(installId, {
        providerId,
        initialState: harnessInstallRows[providerId],
      }),
    };
  };

  const cancelHarnessInstall = async (providerId: string) => {
    const installId = harnessInstallRows[providerId]?.installId
      ?? harnessInstallCandidates.find((candidate) => candidate.providerId === providerId)?.installId;
    if (!installId) return;
    try {
      const info = await cancelInstall(installId);
      const fallbackPct = harnessInstallRows[providerId]?.pct ?? null;
      const nextInstallState = {
        installId,
        state: info.state,
        pct: computeInstallPct(info, fallbackPct),
        target: info.target,
        errorCode: info.error_code,
        error: info.error,
      };
      setHarnessInstallRows((prev) => ({
        ...prev,
        [providerId]: {
          ...nextInstallState,
          pct: computeInstallPct(info, prev[providerId]?.pct ?? fallbackPct),
        },
      }));
      setHarnessInstallCandidates((prev) =>
        prev.map((candidate) =>
          candidate.providerId === providerId
            ? {
                ...candidate,
                installRunning: info.state === "running",
                installId,
              }
            : candidate,
        ),
      );
      if (info.state !== "running") {
        clearHarnessInstallObserver(providerId);
      }
      upsertProviderInstallProgress(providerId, nextInstallState);
    } catch (error) {
      setHarnessInstallError(messageFromError(error));
    }
  };

  const refreshTitlingLocalStatus = async (opts?: { silent?: boolean }): Promise<TitleGenerationLocalStatus | null> => {
    if (!opts?.silent) {
      setTitlingLocalStatusBusy(true);
    }
    setTitlingStatusError(null);
    try {
      const status = await getTitleGenerationLocalStatus();
      setTitlingLocalStatus(status);
      return status;
    } catch (error) {
      setTitlingStatusError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setTitlingLocalStatusBusy(false);
      }
    }
  };

  const attachTitlingInstall = async (installId: string) => {
    if (!installId) return;
    if (titlingInstallObserverRef.current?.installId === installId) return;
    clearTitlingInstallObserver();
    setTitlingLocalInstall({
      installId,
      state: "running",
      pct: null,
    });
    titlingInstallObserverRef.current = {
      installId,
      stop: observeInstall(installId, {
        providerId: "title_generation_local",
        initialState: titlingLocalInstall ?? { installId, state: "running", pct: null },
      }),
    };
  };

  const probeTitlingForTarget = async (targetKey: string): Promise<boolean | null> => {
    setTitlingProbeBusy(true);
    setTitlingProbeTargetKey(targetKey);
    setTitlingProbeDone(false);
    setTitlingProbeError(null);
    setTitlingStatusError(null);
    try {
      await connectDaemonForImport();
      if (selectedDaemonTargetKeyRef.current !== targetKey) return null;

      const settings = await getSettings();
      if (selectedDaemonTargetKeyRef.current !== targetKey) return null;

      setTitlingExistingSettings(settings.title_generation ?? null);
      const draft = buildSessionTitlingDraft(settings);
      setTitlingRemoteBaseUrl(draft.remote.baseUrl);
      setTitlingRemoteApiKey(draft.remote.apiKey);
      setTitlingRemoteModel(draft.remote.model);
      setTitlingRemoteUseJson(draft.remote.useJson);
      setTitlingLocalUseJson(draft.local.useJson);
      setTitlingMode((currentMode) => (currentMode === "skip" ? "skip" : draft.mode));

      let localStatus: TitleGenerationLocalStatus | null = null;
      if (!settings.title_generation || settings.title_generation.mode === "local") {
        localStatus = await refreshTitlingLocalStatus({ silent: true });
        if (selectedDaemonTargetKeyRef.current !== targetKey) return null;
      } else {
        setTitlingLocalStatus(null);
        setTitlingStatusError(null);
      }

      const readiness = resolveSessionTitlingReadiness(settings, localStatus);
      setTitlingConfiguredReady(readiness.ready);
      setTitlingStepRequired(!readiness.ready);
      setTitlingProbeTargetKey(targetKey);
      setTitlingProbeDone(true);
      if (localStatus?.install_running && localStatus.install_id) {
        void attachTitlingInstall(localStatus.install_id).catch(() => {});
      }
      return !readiness.ready;
    } catch (error) {
      if (selectedDaemonTargetKeyRef.current !== targetKey) return null;
      setTitlingProbeTargetKey(targetKey);
      setTitlingProbeDone(true);
      setTitlingConfiguredReady(false);
      setTitlingStepRequired(true);
      setTitlingProbeError(messageFromError(error));
      return true;
    } finally {
      if (selectedDaemonTargetKeyRef.current === targetKey) {
        setTitlingProbeBusy(false);
      }
    }
  };

  const ensureTitlingProbeForCurrentTarget = async (
    options?: { force?: boolean },
  ): Promise<boolean | null> => {
    if (!selectedDaemonTargetKey || !desktopApp) return null;
    if (selections.location === "remote") {
      if (!parsedRemoteHost) return null;
      if (remoteStatusRef.current !== "connected") return null;
    }
    if (!options?.force && titlingProbeDone && titlingProbeTargetKey === selectedDaemonTargetKey) {
      return titlingStepRequired;
    }
    const targetKey = selectedDaemonTargetKey;
    if (
      !options?.force
      && titlingProbePromiseRef.current
      && titlingProbePromiseTargetKeyRef.current === targetKey
    ) {
      return await titlingProbePromiseRef.current;
    }
    const probePromise = probeTitlingForTarget(targetKey);
    titlingProbePromiseRef.current = probePromise;
    titlingProbePromiseTargetKeyRef.current = targetKey;
    try {
      return await probePromise;
    } finally {
      if (titlingProbePromiseRef.current === probePromise) {
        titlingProbePromiseRef.current = null;
        titlingProbePromiseTargetKeyRef.current = null;
      }
    }
  };

  const currentTitlingPayload = (modeOverride?: "remote" | "local"): TitleGenerationSettings | null => {
    const mode = modeOverride ?? titlingMode;
    if (mode !== "remote" && mode !== "local") return null;
    return buildSessionTitlingPayload({
      mode,
      draft: {
        mode,
        remote: {
          baseUrl: titlingRemoteBaseUrl,
          apiKey: titlingRemoteApiKey,
          model: titlingRemoteModel,
          useJson: titlingRemoteUseJson,
        },
        local: {
          modelId: DEFAULT_TITLE_LOCAL_MODEL_ID,
          useJson: titlingLocalUseJson,
        },
      },
      existing: titlingExistingSettings,
    });
  };

  const ensureTitlingPersistedForCurrentTarget = async (
    modeOverride?: "remote" | "local",
  ): Promise<boolean> => {
    if (!modeOverride && titlingMode === "skip") return true;
    const payload = currentTitlingPayload(modeOverride);
    if (!payload || !selectedDaemonTargetKey) return false;
    const targetKey = selectedDaemonTargetKey;
    const payloadHash = sessionTitlingPayloadHash(payload);
    if (titlingPersistedTargetKey === targetKey && titlingPersistedHash === payloadHash) {
      return true;
    }

    setTitlingPersistBusy(true);
    setTitlingPersistError(null);
    try {
      await connectDaemonForImport();
      if (selectedDaemonTargetKeyRef.current !== targetKey) {
        return false;
      }
      await updateSettings({ title_generation: payload });
      setTitlingExistingSettings(payload);
      setTitlingPersistedTargetKey(targetKey);
      setTitlingPersistedHash(payloadHash);
      if (payload.mode === "remote") {
        setTitlingConfiguredReady(true);
      } else {
        const localStatus = await refreshTitlingLocalStatus({ silent: true });
        const readiness = resolveSessionTitlingReadiness({ title_generation: payload }, localStatus);
        setTitlingConfiguredReady(readiness.ready);
        if (localStatus?.install_running && localStatus.install_id) {
          void attachTitlingInstall(localStatus.install_id).catch(() => {});
        }
      }
      return true;
    } catch (error) {
      setTitlingPersistError(messageFromError(error));
      return false;
    } finally {
      setTitlingPersistBusy(false);
    }
  };

  const onSelectTitlingLocal = () => {
    if (titlingLocalInstallBusy || titlingPersistBusy) return false;
    invalidateTitlingPersisted();
    setTitlingMode("local");
    setTitlingLocalInstallBusy(true);
    setTitlingStatusError(null);
    setTitlingPersistError(null);
    void (async () => {
      try {
        const persisted = await ensureTitlingPersistedForCurrentTarget("local");
        if (!persisted) return;
        const { install_id } = await installTitleGenerationLocal();
        void attachTitlingInstall(install_id).catch((error) => {
          setTitlingStatusError(messageFromError(error));
        });
      } catch (error) {
        setTitlingStatusError(messageFromError(error));
      } finally {
        setTitlingLocalInstallBusy(false);
      }
    })();
    return true;
  };

  const scanAuthImportCandidatesForTarget = useCallback(async (
    target: "local" | "remote",
    options?: { force?: boolean },
  ): Promise<ProviderAuthImportCandidate[]> => {
    if (!desktopApp) return [];
    if (target === "remote") {
      if (!parsedRemoteHost) return [];
      if (remoteStatusRef.current !== "connected") return [];
    }

    const scanKey = authImportScanKeyForTarget(target);
    if (!options?.force && authImportScannedKey === scanKey) return authImportCandidates;

    if (
      !options?.force
      && authImportScanPromiseRef.current
      && authImportScanKeyRef.current === scanKey
    ) {
      return await authImportScanPromiseRef.current;
    }

    setAuthImportBusy(true);
    setAuthImportError(null);
    const scanRun = nextFlowRunToken(authImportScanRunRef.current?.runId ?? 0, scanKey);
    authImportScanRunRef.current = scanRun;

    const scanPromise = (async () => {
      try {
        await connectDaemonForImport(target);
        const response = await listProviderAuthImportCandidates();
        const candidates = (response.candidates ?? [])
          .filter((candidate) => candidate.parse_status === "parsed");
        if (!isCurrentFlowRunToken(authImportScanRunRef.current, scanRun)) {
          return [];
        }
        setAuthImportCandidates(candidates);
        setAuthImportSelected(
          Object.fromEntries(candidates.map((candidate) => [candidate.id, true])),
        );
        setAuthImportScannedKey(scanKey);
        setAuthImportDeferredKey(null);
        return candidates;
      } catch (error) {
        if (!isCurrentFlowRunToken(authImportScanRunRef.current, scanRun)) {
          return [];
        }
        setAuthImportCandidates([]);
        setAuthImportSelected({});
        setAuthImportError(messageFromError(error));
        setAuthImportScannedKey(scanKey);
        setAuthImportDeferredKey(scanKey);
        return [];
      } finally {
        if (isCurrentFlowRunToken(authImportScanRunRef.current, scanRun)) {
          setAuthImportBusy(false);
        }
      }
    })();

    authImportScanPromiseRef.current = scanPromise;
    authImportScanKeyRef.current = scanKey;
    try {
      return await scanPromise;
    } finally {
      if (authImportScanPromiseRef.current === scanPromise) {
        authImportScanPromiseRef.current = null;
        authImportScanKeyRef.current = null;
      }
    }
  }, [
    authImportCandidates,
    authImportDeferredKey,
    authImportScannedKey,
    authImportScanKeyForTarget,
    connectDaemonForImport,
    desktopApp,
    parsedRemoteHost,
    remoteStatusRef,
  ]);

  const scanHarnessInstallCandidatesForTarget = useCallback(async (
    target: "local" | "remote",
    containerSelectionOverride?: string,
    options?: { force?: boolean },
  ): Promise<HarnessInstallProviderRow[]> => {
    if (!desktopApp) return [];
    if (target === "remote") {
      if (!parsedRemoteHost) return [];
      if (remoteStatusRef.current !== "connected") return [];
    }

    const containerSelection = containerSelectionOverride ?? selections.container;
    const installTarget: InstallTarget =
      containerSelection && containerSelection !== "no-container" ? "container" : "host";
    const scanKey = harnessInstallScanKeyForTarget(target, containerSelectionOverride);
    if (!options?.force && harnessInstallScannedKey === scanKey) return harnessInstallCandidates;

    if (
      !options?.force
      && harnessInstallScanPromiseRef.current
      && harnessInstallScanKeyRef.current === scanKey
    ) {
      return await harnessInstallScanPromiseRef.current;
    }

    setHarnessInstallBusy(true);
    setHarnessInstallError(null);
    const scanRun = nextFlowRunToken(harnessInstallScanRunRef.current?.runId ?? 0, scanKey);
    harnessInstallScanRunRef.current = scanRun;

    const scanPromise = (async () => {
      try {
        await connectDaemonForImport(target);
        const providers = await listProviders(installTarget);
        if (!isCurrentFlowRunToken(harnessInstallScanRunRef.current, scanRun)) return [];
        const rows = providers
          .map((provider) => mapHarnessInstallCandidate(provider, installTarget))
          .filter((row): row is HarnessInstallProviderRow => row !== null)
          .sort((a, b) => a.label.localeCompare(b.label));
        setHarnessInstallCandidates(rows);
        setHarnessInstallSelected((prev) =>
          Object.fromEntries(
            rows.map((row) => {
              if (row.installed && row.healthy) {
                return [row.providerId, false];
              }
              if (Object.prototype.hasOwnProperty.call(prev, row.providerId)) {
                return [row.providerId, Boolean(prev[row.providerId])];
              }
              return [row.providerId, row.installSupported];
            }),
          ),
        );
        const runningRows = rows.filter((row) => row.installRunning && row.installId);
        for (const row of runningRows) {
          await attachHarnessInstall(row.providerId, row.installId!);
        }
        setHarnessInstallScannedKey(scanKey);
        setHarnessInstallDeferredKey(null);
        return rows;
      } catch (error) {
        if (!isCurrentFlowRunToken(harnessInstallScanRunRef.current, scanRun)) return [];
        setHarnessInstallCandidates([]);
        setHarnessInstallSelected({});
        setHarnessInstallRows({});
        setHarnessInstallError(messageFromError(error));
        setHarnessInstallScannedKey(scanKey);
        setHarnessInstallDeferredKey(scanKey);
        return [];
      } finally {
        if (isCurrentFlowRunToken(harnessInstallScanRunRef.current, scanRun)) {
          setHarnessInstallBusy(false);
        }
      }
    })();

    harnessInstallScanPromiseRef.current = scanPromise;
    harnessInstallScanKeyRef.current = scanKey;
    try {
      return await scanPromise;
    } finally {
      if (harnessInstallScanPromiseRef.current === scanPromise) {
        harnessInstallScanPromiseRef.current = null;
        harnessInstallScanKeyRef.current = null;
      }
    }
  }, [
    connectDaemonForImport,
    desktopApp,
    harnessInstallScanKeyForTarget,
    harnessInstallCandidates,
    harnessInstallDeferredKey,
    harnessInstallScannedKey,
    parsedRemoteHost,
    remoteStatusRef,
    selections.container,
  ]);

  const resetRoutePlan = useCallback(() => {
    invalidateRoutePlan();
    routePlanRunRef.current = nextFlowRunToken(routePlanRunRef.current?.runId ?? 0, "reset");
  }, [invalidateRoutePlan]);

  const ensureOnboardingAfterDaemonConnect = useCallback(async (
    options?: { allowTitlingInsertion?: boolean },
  ): Promise<EnsureOnboardingAfterDaemonConnectResult | null> => {
    const location = selections.location;
    if (location !== "local" && location !== "remote") return null;
    const targetKey = selectedDaemonTargetKeyRef.current;
    const containerSelection = (selections.container ?? "").trim();
    if (!targetKey || !containerSelection) return null;
    const allowTitlingInsertion = options?.allowTitlingInsertion ?? true;

    const authScanKey = authImportScanKeyForTarget(location);
    const harnessScanKey = harnessInstallScanKeyForTarget(location, containerSelection);
    const needsAuthRefresh = authImportScannedKey !== authScanKey || authImportDeferredKey === authScanKey;
    const needsHarnessRefresh =
      harnessInstallScannedKey !== harnessScanKey || harnessInstallDeferredKey === harnessScanKey;

    const [authCandidates, harnessRows, titlingRequired] = await Promise.all([
      needsAuthRefresh
        ? scanAuthImportCandidatesForTarget(location, { force: true })
        : Promise.resolve(authImportCandidates),
      needsHarnessRefresh
        ? scanHarnessInstallCandidatesForTarget(location, containerSelection, { force: true })
        : Promise.resolve(harnessInstallCandidates),
      ensureTitlingProbeForCurrentTarget({ force: true }),
    ]);

    const result = buildOnboardingAfterConnectResult({
      targetKey,
      containerSelection,
      authImportCandidateCount: authCandidates.length,
      missingHarnessCount: harnessRows.filter(
        (candidate) => candidate.installSupported && !(candidate.installed && candidate.healthy),
      ).length,
      titlingRequired: titlingRequired === true,
      titlingMode,
    }, routePlan, { allowTitlingInsertion });
    setRoutePlan(result.routePlan);
    return result;
  }, [
    authImportCandidates,
    authImportDeferredKey,
    authImportScannedKey,
    ensureTitlingProbeForCurrentTarget,
    harnessInstallCandidates,
    harnessInstallDeferredKey,
    harnessInstallScannedKey,
    routePlan,
    scanAuthImportCandidatesForTarget,
    scanHarnessInstallCandidatesForTarget,
    selections.container,
    selections.location,
    setRoutePlan,
    titlingMode,
  ]);

  const ensureRoutePlanForSelection = useCallback(async (
    containerSelectionOverride?: string,
  ): Promise<WizardRoutePlan | null> => {
    const location = selections.location;
    if (location !== "local" && location !== "remote") return null;
    const targetKey = selectedDaemonTargetKeyRef.current ?? (location === "local" ? "local" : null);
    const containerSelection = (containerSelectionOverride ?? selections.container ?? "").trim();
    if (!targetKey || !containerSelection) return null;

    const routeKey = `${targetKey}|${containerSelection}`;
    if (routePlan?.targetKey === routeKey) {
      return routePlan;
    }

    const run = nextFlowRunToken(routePlanRunRef.current?.runId ?? 0, routeKey);
    routePlanRunRef.current = run;
    setRoutePlanningBusy(true);
    try {
      const [authCandidates, titlingRequired, harnessRows] = await Promise.all([
        scanAuthImportCandidatesForTarget(location),
        ensureTitlingProbeForCurrentTarget(),
        scanHarnessInstallCandidatesForTarget(location, containerSelection),
      ]);
      if (!isCurrentFlowRunToken(routePlanRunRef.current, run)) return null;

      const nextPlan = buildWizardRoutePlan({
        targetKey,
        containerSelection,
        authImportCandidateCount: authCandidates.length,
        missingHarnessCount: harnessRows.filter(
          (candidate) => candidate.installSupported && !(candidate.installed && candidate.healthy),
        ).length,
        titlingRequired: titlingRequired === true,
        titlingMode,
      });
      setRoutePlan(nextPlan);
      return nextPlan;
    } finally {
      if (isCurrentFlowRunToken(routePlanRunRef.current, run)) {
        setRoutePlanningBusy(false);
      }
    }
  }, [
    ensureTitlingProbeForCurrentTarget,
    routePlan,
    scanAuthImportCandidatesForTarget,
    scanHarnessInstallCandidatesForTarget,
    selections.container,
    selections.location,
    setRoutePlan,
    setRoutePlanningBusy,
    titlingMode,
  ]);

  const advanceFromAuthImportStep = async (
    options?: { clearSelections?: boolean },
  ): Promise<WizardStepKey | null> => {
    if (authImportBusy) return null;
    const selectionSnapshot = options?.clearSelections ? {} : authImportSelected;
    if (options?.clearSelections) {
      setAuthImportSelected({});
    }
    const candidateIds = authImportCandidates
      .filter((candidate) => selectionSnapshot[candidate.id])
      .map((candidate) => candidate.id);
    if (candidateIds.length) {
      setAuthImportBusy(true);
      setAuthImportError(null);
      try {
        await connectDaemonForImport();
        const response = await importProviderAuthCandidates(candidateIds);
        const acceptableStatuses = new Set(["imported", "updated", "already_imported"]);
        const failures = (response.results ?? [])
          .filter((result) => !acceptableStatuses.has(result.status))
          .map((result) => {
            const label = authImportCandidates.find((candidate) => candidate.id === result.candidate_id)?.provider_label
              ?? result.provider_id;
            const detail = (result.message ?? `Import status: ${result.status}`).trim();
            return `${label}: ${detail}`;
          });
        if (failures.length > 0) {
          setAuthImportError(`Some auth imports did not apply. ${failures.join(" ; ")}`);
          setAuthImportBusy(false);
          return null;
        }
      } catch (error) {
        setAuthImportError(messageFromError(error));
        setAuthImportBusy(false);
        return null;
      }
      setAuthImportBusy(false);
    }
    const titlingRequired = await ensureTitlingProbeForCurrentTarget();
    if (currentStepKeyRef.current !== "auth-import") return null;
    if (
      titlingRequired === true
      && titlingMode !== "skip"
      && routePlan?.includeTitling !== true
    ) {
      if (routePlan) {
        setRoutePlan({ ...routePlan, includeTitling: true });
      }
      return "session-titling";
    }
    return nextAfterAuthImport(routePlan);
  };

  const advanceFromHarnessDownloadsStep = async (
    options?: { clearSelections?: boolean },
  ): Promise<WizardStepKey | null> => {
    if (harnessInstallBusy) return null;
    const selectionSnapshot = options?.clearSelections ? {} : harnessInstallSelected;
    if (options?.clearSelections) {
      setHarnessInstallSelected({});
    }
    const selectedRows = harnessInstallCandidates
      .filter((row) => selectionSnapshot[row.providerId])
      .filter((row) => row.installSupported)
      .map((row) => ({
        row,
        installUi: harnessInstallRows[row.providerId],
        status: resolveHarnessInstallCandidateStatus(row, harnessInstallRows[row.providerId]),
      }));
    const startableRows = selectedRows
      .filter(({ status }) => status === "ready_to_start")
      .map(({ row }) => row);
    const blockingRows = selectedRows.filter(
      ({ status }) => status === "failed" || status === "cancelled",
    );
    const runningRows = selectedRows.filter(({ status }) => status === "running");

    if (selectedRows.length === 0 || selectedRows.every(({ status }) => status === "installed" || status === "succeeded")) {
      setHarnessInstallError(null);
      if (currentStepKeyRef.current !== "harness-downloads") return null;
      return nextAfterHarnessDownloads(routePlan);
    }
    if (runningRows.length > 0 && startableRows.length === 0) {
      setHarnessInstallError(null);
      if (currentStepKeyRef.current !== "harness-downloads") return null;
      return nextAfterHarnessDownloads(routePlan);
    }
    if (blockingRows.length > 0 && startableRows.length === 0) {
      if (currentStepKeyRef.current !== "harness-downloads") return null;
      return nextAfterHarnessDownloads(routePlan);
    }

    setHarnessInstallBusy(true);
    setHarnessInstallError(null);
    let shouldAdvance = false;
    try {
      await connectDaemonForImport();
      const startResults = await Promise.all(
        startableRows.map(async (row) => {
          try {
            const started = await installProvider(row.providerId, selectedHarnessInstallTarget);
            const installId = started.install_id;
            const nextInstallState = {
              installId,
              state: "running" as const,
              pct: null,
              target: started.target,
              errorCode: undefined,
              error: undefined,
            };
            setHarnessInstallRows((prev) => ({
              ...prev,
              [row.providerId]: nextInstallState,
            }));
            setHarnessInstallCandidates((prev) =>
              prev.map((candidate) =>
                candidate.providerId === row.providerId
                  ? {
                      ...candidate,
                      installRunning: true,
                      installId,
                    }
                  : candidate,
              ),
            );
            upsertProviderInstallProgress(row.providerId, nextInstallState);
            void attachHarnessInstall(row.providerId, installId);
            return {
              providerId: row.providerId,
              ok: true as const,
            };
          } catch (error) {
            return {
              providerId: row.providerId,
              ok: false as const,
              error: messageFromError(error),
            };
          }
        }),
      );

      const failures = startResults
        .filter((result) => !result.ok)
        .map((result) => {
          const label = harnessInstallCandidates.find((candidate) => candidate.providerId === result.providerId)?.label ?? result.providerId;
          return `${label}: ${result.error}`;
        });
      if (failures.length > 0) {
        setHarnessInstallRows((prev) => ({
          ...prev,
          ...Object.fromEntries(
            startResults
              .filter((result) => !result.ok)
              .map((result) => [
                result.providerId,
                {
                  installId: prev[result.providerId]?.installId ?? "",
                  state: "failed" as const,
                  pct: null,
                  target: selectedHarnessInstallTarget,
                  errorCode: undefined,
                  error: result.error,
                },
              ]),
          ),
        }));
        const prefix = failures.length === startableRows.length
          ? "Selected downloads failed to start."
          : "Some selected downloads failed to start.";
        setHarnessInstallError(`${prefix} ${failures.join(" ; ")} Continuing without those downloads.`);
        shouldAdvance = true;
      }

      const startedAny = startResults.some((result) => result.ok);
      if (!shouldAdvance && !startedAny && runningRows.length === 0 && blockingRows.length === 0) {
        return null;
      }
      shouldAdvance = true;
    } catch (error) {
      setHarnessInstallError(`Could not start selected downloads: ${messageFromError(error)} Continuing without those downloads.`);
      shouldAdvance = true;
    } finally {
      setHarnessInstallBusy(false);
    }
    if (!shouldAdvance) return null;
    if (currentStepKeyRef.current !== "harness-downloads") return null;
    return nextAfterHarnessDownloads(routePlan);
  };

  const harnessCandidateStatuses = harnessInstallCandidates.map((candidate) => {
    const installUi = harnessInstallRows[candidate.providerId];
    return {
      candidate,
      installUi,
      status: resolveHarnessInstallCandidateStatus(candidate, installUi),
    };
  });
  const harnessMissingCount = harnessCandidateStatuses.filter(
    ({ candidate, status }) => candidate.installSupported && status !== "installed" && status !== "succeeded",
  ).length;
  const selectedHarnessStatuses = harnessCandidateStatuses.filter(
    ({ candidate }) => harnessInstallSelected[candidate.providerId] && candidate.installSupported,
  );
  const selectedHarnessReadyToStartCount = selectedHarnessStatuses.filter(
    ({ status }) => status === "ready_to_start",
  ).length;
  const selectedHarnessRunningCount = selectedHarnessStatuses.filter(
    ({ status }) => status === "running",
  ).length;
  const selectedHarnessBlockedCount = selectedHarnessStatuses.filter(
    ({ status }) => status === "failed" || status === "cancelled",
  ).length;
  const selectedHarnessFailedCount = selectedHarnessStatuses.filter(
    ({ status }) => status === "failed",
  ).length;
  const selectedHarnessCompletedCount = selectedHarnessStatuses.filter(
    ({ status }) => status === "installed" || status === "succeeded",
  ).length;
  const harnessSummaryValue = harnessMissingCount === 0
    ? "All detectable harnesses are ready"
    : selectedHarnessRunningCount > 0
      ? `${selectedHarnessRunningCount} selected download${selectedHarnessRunningCount === 1 ? "" : "s"} in progress`
      : selectedHarnessBlockedCount > 0
        ? `${selectedHarnessBlockedCount} selected download${selectedHarnessBlockedCount === 1 ? "" : "s"} failed or were canceled`
        : selectedHarnessReadyToStartCount > 0
          ? `${selectedHarnessReadyToStartCount} selected for download`
          : selectedHarnessCompletedCount > 0
            ? `${selectedHarnessCompletedCount} selected download${selectedHarnessCompletedCount === 1 ? "" : "s"} ready`
            : "Skipped for now";

  const titlingRemoteValid = titlingRemoteBaseUrl.trim() !== ""
    && titlingRemoteApiKey.trim() !== ""
    && titlingRemoteModel.trim() !== "";
  const titlingSummaryValue = titlingMode === "skip"
    ? "Skipped (fallback titles)"
    : titlingMode === "remote"
      ? `Configured remote (${titlingRemoteModel.trim() || "model pending"})`
      : titlingMode === "local"
        ? (titlingLocalStatus?.ready
          ? "Configured local (ready)"
          : "Configured local (install pending; fallback until ready)")
      : titlingConfiguredReady
          ? (titlingExistingSettings?.mode === "local" ? "Configured local (ready)" : "Configured remote")
        : "Not configured";

  useEffect(() => {
    titlingInstallStateRef.current = titlingLocalInstall;
  }, [titlingLocalInstall]);

  useEffect(() => {
    return subscribeProviderInstallProgress((snapshot) => {
      const providerIds = new Set([
        ...Object.keys(snapshot),
        ...Object.keys(harnessInstallRows),
        ...harnessInstallCandidates.map((candidate) => candidate.providerId),
      ]);
      if (providerIds.size === 0) return;

      setHarnessInstallRows((prev) => {
        let changed = false;
        const next = { ...prev };
        for (const providerId of providerIds) {
          const session = resolveProviderInstallProgressSession(snapshot, providerId, selectedHarnessInstallTarget);
          if (!session) continue;
          const nextRow: HarnessInstallRowState = {
            installId: session.installId,
            state: session.state,
            pct: session.pct,
            target: session.target,
            errorCode: session.errorCode,
            error: session.error,
          };
          const current = prev[providerId];
          if (
            current?.installId === nextRow.installId
            && current.state === nextRow.state
            && current.pct === nextRow.pct
            && current.target === nextRow.target
            && current.errorCode === nextRow.errorCode
            && current.error === nextRow.error
          ) {
            continue;
          }
          next[providerId] = nextRow;
          changed = true;
        }
        return changed ? next : prev;
      });

      setHarnessInstallCandidates((prev) => {
        let changed = false;
        const next = prev.map((candidate) => {
          const session = resolveProviderInstallProgressSession(
            snapshot,
            candidate.providerId,
            selectedHarnessInstallTarget,
          );
          if (!session) return candidate;
          const installRunning = session.state === "running";
          if (candidate.installRunning === installRunning && candidate.installId === session.installId) {
            return candidate;
          }
          changed = true;
          return {
            ...candidate,
            installRunning,
            installId: session.installId,
          };
        });
        return changed ? next : prev;
      });

      const terminalProviderIds = Array.from(providerIds).filter((providerId) => {
        const session = resolveProviderInstallProgressSession(snapshot, providerId, selectedHarnessInstallTarget);
        return session ? session.state !== "running" : false;
      });
      if (terminalProviderIds.length === 0) return;
      for (const providerId of terminalProviderIds) {
        clearHarnessInstallObserver(providerId);
      }
    });
  }, [harnessInstallCandidates, harnessInstallRows, selectedHarnessInstallTarget]);

  useEffect(() => {
    return subscribeInstallProgress((snapshot: InstallProgressSnapshot) => {
      const trackedInstallId =
        titlingInstallObserverRef.current?.installId
        ?? titlingInstallStateRef.current?.installId
        ?? null;
      if (!trackedInstallId) return;
      const entry = snapshot[trackedInstallId];
      if (!entry) return;

      const nextState: LocalInstallState = {
        installId: entry.installId,
        state: entry.state,
        pct: entry.pct,
        errorCode: entry.errorCode,
        error: entry.error,
      };
      const previousState = titlingInstallStateRef.current?.state ?? null;
      titlingInstallStateRef.current = nextState;
      setTitlingLocalInstall((prev) => {
        if (
          prev?.installId === nextState.installId
          && prev.state === nextState.state
          && prev.pct === nextState.pct
          && prev.errorCode === nextState.errorCode
          && prev.error === nextState.error
        ) {
          return prev;
        }
        return nextState;
      });
      if (previousState === "running" && entry.state !== "running") {
        clearTitlingInstallObserver();
        void refreshTitlingLocalStatus({ silent: true });
      }
    });
  }, [refreshTitlingLocalStatus]);

  useEffect(() => {
    selectedDaemonTargetKeyRef.current = selectedDaemonTargetKey;
  }, [selectedDaemonTargetKey]);

  useEffect(() => {
    const targetChanged = previousTargetKeyRef.current !== selectedDaemonTargetKey;
    previousTargetKeyRef.current = selectedDaemonTargetKey;
    if (!targetChanged) return;
    resetProvisioningState();
    resetRoutePlan();
  }, [resetProvisioningState, resetRoutePlan, selectedDaemonTargetKey]);

  useEffect(() => {
    if (!selectedDaemonTargetKey || !canProbeTitling) {
      resetTitlingDraft();
      setTitlingProbeBusy(false);
      setTitlingProbeError(null);
      setTitlingProbeDone(false);
      setTitlingConfiguredReady(false);
      setTitlingStepRequired(false);
      setTitlingProbeTargetKey(null);
      setTitlingPersistError(null);
      setTitlingPersistedTargetKey(null);
      setTitlingPersistedHash(null);
      setTitlingExistingSettings(null);
      setTitlingLocalStatus(null);
      setTitlingLocalStatusRequestedTargetKey(null);
      setTitlingStatusError(null);
      setTitlingLocalInstall(null);
      clearTitlingInstallObserver();
      return;
    }

    if (titlingProbeTargetKey !== selectedDaemonTargetKey) {
      titlingProbePromiseRef.current = null;
      titlingProbePromiseTargetKeyRef.current = null;
      setTitlingProbeError(null);
      setTitlingProbeDone(false);
      setTitlingConfiguredReady(false);
      setTitlingStepRequired(false);
      setTitlingPersistError(null);
      setTitlingPersistedTargetKey(null);
      setTitlingPersistedHash(null);
      setTitlingExistingSettings(null);
      setTitlingLocalStatus(null);
      setTitlingLocalStatusRequestedTargetKey(null);
      setTitlingStatusError(null);
      setTitlingLocalInstall(null);
      clearTitlingInstallObserver();
      resetTitlingDraft();
    }
  }, [
    canProbeTitling,
    selectedDaemonTargetKey,
    titlingProbeTargetKey,
  ]);

  useEffect(() => {
    return () => {
      clearTitlingInstallObserver();
      clearHarnessInstallObserver();
    };
  }, []);

  useEffect(() => {
    if (titlingMode === "local") return;
    setTitlingLocalStatusRequestedTargetKey(null);
  }, [titlingMode]);

  useEffect(() => {
    if (titlingMode !== "local") return;
    if (!selectedDaemonTargetKey || !canProbeTitling) return;
    if (titlingLocalStatus || titlingLocalStatusBusy) return;
    if (titlingLocalStatusRequestedTargetKey === selectedDaemonTargetKey) return;
    setTitlingLocalStatusRequestedTargetKey(selectedDaemonTargetKey);
    void refreshTitlingLocalStatus().catch(() => {});
  }, [
    canProbeTitling,
    selectedDaemonTargetKey,
    titlingLocalStatus,
    titlingLocalStatusBusy,
    titlingLocalStatusRequestedTargetKey,
    titlingMode,
  ]);

  useEffect(() => {
    if (!desktopApp) return;
    if (currentStepKey !== "location") return;
    if (selections.location) return;
    void scanAuthImportCandidatesForTarget("local");
  }, [
    currentStepKey,
    desktopApp,
    scanAuthImportCandidatesForTarget,
    selections.location,
  ]);

  useEffect(() => {
    if (!desktopApp) return;
    if (!selectedDaemonTargetKey) return;
    if (selections.location !== "local" && selections.location !== "remote") return;
    if (selections.location === "remote") {
      if (!parsedRemoteHost) return;
      if (remoteStatus !== "connected") return;
    }
    void scanHarnessInstallCandidatesForTarget(selections.location).catch(() => {});
  }, [
    desktopApp,
    parsedRemoteHost,
    remoteStatus,
    scanHarnessInstallCandidatesForTarget,
    selections.location,
    selectedDaemonTargetKey,
  ]);

  useEffect(() => {
    if (!routePlan) return;
    const activeTargetKey = selectedDaemonTargetKey
      ? `${selectedDaemonTargetKey}|${(selections.container ?? "").trim()}`
      : null;
    if (activeTargetKey === routePlan.targetKey) return;
    resetRoutePlan();
  }, [
    resetRoutePlan,
    routePlan,
    selectedDaemonTargetKey,
    selections.container,
  ]);

  return {
    authImportStepVisible,
    authImportCandidates,
    authImportSelected,
    setAuthImportSelected,
    authImportBusy,
    authImportError,
    advanceFromAuthImportStep,
    harnessInstallStepVisible,
    harnessInstallCandidates,
    harnessInstallSelected,
    setHarnessInstallSelected,
    harnessInstallBusy,
    harnessInstallError,
    harnessInstallRows,
    cancelHarnessInstall,
    advanceFromHarnessDownloadsStep,
    selectedHarnessInstallTarget,
    harnessByProviderId,
    selectedHarnessReadyToStartCount,
    selectedHarnessRunningCount,
    selectedHarnessBlockedCount,
    selectedHarnessFailedCount,
    harnessSummaryValue,
    titlingStepVisible,
    titlingProbeBusy,
    titlingProbeError,
    titlingConfiguredReady,
    titlingMode,
    setTitlingMode,
    titlingRemoteBaseUrl,
    setTitlingRemoteBaseUrl,
    titlingRemoteApiKey,
    setTitlingRemoteApiKey,
    titlingRemoteModel,
    setTitlingRemoteModel,
    titlingRemoteUseJson,
    setTitlingRemoteUseJson,
    titlingRemoteAdvancedOpen,
    setTitlingRemoteAdvancedOpen,
    titlingLocalStatus,
    titlingStatusError,
    titlingLocalInstallBusy,
    titlingLocalInstall,
    titlingPersistBusy,
    titlingPersistError,
    setTitlingPersistError,
    titlingRemoteValid,
    titlingSummaryValue,
    invalidateTitlingPersisted,
    ensureTitlingProbeForCurrentTarget,
    ensureTitlingPersistedForCurrentTarget,
    ensureOnboardingAfterDaemonConnect,
    onSelectTitlingLocal,
    ensureRoutePlanForSelection,
    resetRoutePlan,
  };
}
