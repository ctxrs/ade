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
  getInstall,
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
  nextBoundaryStep,
  type WizardRoutePlan,
  type WizardStepKey,
} from "./wizardFlow";
import type { WizardSelections } from "./wizardFlowReducer";
import {
  messageFromError,
  resolveHarnessInstallCandidateStatus,
  type HarnessInstallProviderRow,
  type HarnessInstallRowState,
  type LocalInstallState,
  type RemoteStatus,
} from "./wizardTypes";
import { upsertProviderInstallProgress } from "../../state/providerInstallProgressStore";

type UseWorkspaceSetupProvisioningArgs = {
  currentStepKey: WizardStepKey;
  currentStepKeyRef: MutableRefObject<WizardStepKey>;
  selections: WizardSelections;
  routePlan: WizardRoutePlan | null;
  setRoutePlan: (routePlan: WizardRoutePlan | null) => void;
  setRoutePlanningBusy: (busy: boolean) => void;
  invalidateRoutePlan: () => void;
  goToStepKey: (key: WizardStepKey) => void;
  goRelativeStep: (delta: number) => void;
  desktopApp: boolean;
  selectedDaemonTargetKey: string | null;
  parsedRemoteHost: string | undefined;
  parsedRemoteUser: string | null | undefined;
  parsedRemotePort: number | null;
  remoteDataDirInput: string;
  remoteStatus: RemoteStatus;
  remoteStatusRef: MutableRefObject<RemoteStatus>;
  connectDaemonForImport: (locationOverride?: "local" | "remote") => Promise<void>;
};

export function useWorkspaceSetupProvisioning({
  currentStepKey,
  currentStepKeyRef,
  selections,
  routePlan,
  setRoutePlan,
  setRoutePlanningBusy,
  invalidateRoutePlan,
  goToStepKey,
  goRelativeStep,
  desktopApp,
  selectedDaemonTargetKey,
  parsedRemoteHost,
  parsedRemoteUser,
  parsedRemotePort,
  remoteDataDirInput,
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
  const titlingInstallPollRef = useRef<number | null>(null);
  const titlingInstallPollGenerationRef = useRef(0);
  const harnessInstallPollTimeoutsRef = useRef<Record<string, number>>({});
  const previousTargetKeyRef = useRef<string | null>(null);

  const harnessByProviderId = useMemo(() => {
    return new Map(HARNESS_CATALOG.map((entry) => [entry.id, entry]));
  }, []);

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

  const clearHarnessInstallPoll = (providerId?: string) => {
    if (providerId) {
      const timeout = harnessInstallPollTimeoutsRef.current[providerId];
      if (timeout) {
        window.clearTimeout(timeout);
        delete harnessInstallPollTimeoutsRef.current[providerId];
      }
      return;
    }
    for (const key of Object.keys(harnessInstallPollTimeoutsRef.current)) {
      const timeout = harnessInstallPollTimeoutsRef.current[key];
      window.clearTimeout(timeout);
      delete harnessInstallPollTimeoutsRef.current[key];
    }
  };

  const clearTitlingInstallPoll = () => {
    titlingInstallPollGenerationRef.current += 1;
    if (titlingInstallPollRef.current) {
      window.clearTimeout(titlingInstallPollRef.current);
      titlingInstallPollRef.current = null;
    }
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
    clearHarnessInstallPoll();
    clearTitlingInstallPoll();
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
    if (harnessInstallPollTimeoutsRef.current[providerId]) return;

    const poll = async () => {
      try {
        const info = await getInstall(installId);
        const pct = computeInstallPct(info, harnessInstallRows[providerId]?.pct ?? null);
        const nextInstallState = {
          installId,
          state: info.state,
          pct,
          target: info.target,
          errorCode: info.error_code,
          error: info.error,
        };
        setHarnessInstallRows((prev) => ({
          ...prev,
          [providerId]: nextInstallState,
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
        upsertProviderInstallProgress(providerId, nextInstallState);
        if (info.state !== "running") {
          clearHarnessInstallPoll(providerId);
          setHarnessInstallScannedKey(null);
          setHarnessInstallDeferredKey(null);
          return;
        }
      } catch {
        // keep polling while install is active
      }
      harnessInstallPollTimeoutsRef.current[providerId] = window.setTimeout(() => {
        void poll();
      }, 900);
    };

    await poll();
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
        clearHarnessInstallPoll(providerId);
        setHarnessInstallScannedKey(null);
        setHarnessInstallDeferredKey(null);
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
    clearTitlingInstallPoll();
    const generation = titlingInstallPollGenerationRef.current;
    setTitlingLocalInstall({
      installId,
      state: "running",
      pct: null,
    });

    const poll = async () => {
      if (generation !== titlingInstallPollGenerationRef.current) return;
      try {
        const info = await getInstall(installId);
        if (generation !== titlingInstallPollGenerationRef.current) return;
        const pct = computeInstallPct(info, titlingLocalInstall?.pct ?? null);
        setTitlingLocalInstall({
          installId,
          state: info.state,
          pct,
          errorCode: info.error_code,
          error: info.error,
        });
        if (info.state !== "running") {
          if (generation === titlingInstallPollGenerationRef.current) {
            clearTitlingInstallPoll();
          }
          await refreshTitlingLocalStatus({ silent: true });
          return;
        }
      } catch {
        // Keep polling: install fetches may transiently fail while daemon restarts/backgrounds.
      }

      if (generation !== titlingInstallPollGenerationRef.current) return;
      titlingInstallPollRef.current = window.setTimeout(() => {
        if (generation !== titlingInstallPollGenerationRef.current) return;
        poll().catch(() => {});
      }, 900);
    };

    await poll();
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
      setTitlingMode(draft.mode);

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
    if (titlingLocalInstallBusy || titlingPersistBusy) return;
    invalidateTitlingPersisted();
    setTitlingMode("local");
    setTitlingLocalInstallBusy(true);
    setTitlingStatusError(null);
    setTitlingPersistError(null);
    goRelativeStep(1);
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

    const scanKey = target === "local"
      ? "local|@"
      : `remote|${parsedRemoteUser ?? ""}@${parsedRemoteHost ?? ""}:${parsedRemotePort ?? 4399}:${remoteDataDirInput.trim()}`;
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
    connectDaemonForImport,
    desktopApp,
    parsedRemoteHost,
    parsedRemotePort,
    parsedRemoteUser,
    remoteDataDirInput,
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
    const scanKey = target === "local"
      ? `local|@|${installTarget}`
      : `remote|${parsedRemoteUser ?? ""}@${parsedRemoteHost ?? ""}|${installTarget}`;
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
    harnessInstallCandidates,
    harnessInstallDeferredKey,
    harnessInstallScannedKey,
    parsedRemoteHost,
    parsedRemoteUser,
    remoteStatusRef,
    selections.container,
  ]);

  const resetRoutePlan = useCallback(() => {
    invalidateRoutePlan();
    routePlanRunRef.current = nextFlowRunToken(routePlanRunRef.current?.runId ?? 0, "reset");
  }, [invalidateRoutePlan]);

  const ensureOnboardingAfterDaemonConnect = useCallback(async (
    options?: { allowTitlingInsertion?: boolean },
  ): Promise<boolean> => {
    const location = selections.location;
    if (location !== "local" && location !== "remote") return false;
    const targetKey = selectedDaemonTargetKeyRef.current;
    const containerSelection = (selections.container ?? "").trim();
    if (!targetKey || !containerSelection) return false;
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

    const nextPlan: WizardRoutePlan = {
      targetKey: `${targetKey}|${containerSelection}`,
      containerSelection,
      includeHarnessDownloads: harnessRows.some(
        (candidate) => candidate.installSupported && !(candidate.installed && candidate.healthy),
      ),
      includeAuthImport: authCandidates.length > 0,
      includeTitling: titlingMode !== "skip" && titlingRequired === true,
    };
    setRoutePlan(nextPlan);

    const shouldInsertHarnessDownloads =
      nextPlan.includeHarnessDownloads && routePlan?.includeHarnessDownloads !== true;
    const shouldInsertAuthImport =
      nextPlan.includeAuthImport && routePlan?.includeAuthImport !== true;
    const shouldInsertTitling =
      allowTitlingInsertion && nextPlan.includeTitling && routePlan?.includeTitling !== true;

    if (shouldInsertHarnessDownloads) {
      goToStepKey("harness-downloads");
      return true;
    }
    if (shouldInsertAuthImport) {
      goToStepKey("auth-import");
      return true;
    }
    if (shouldInsertTitling) {
      goToStepKey("session-titling");
      return true;
    }
    return false;
  }, [
    authImportCandidates,
    authImportDeferredKey,
    authImportScannedKey,
    ensureTitlingProbeForCurrentTarget,
    goToStepKey,
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

      const nextPlan: WizardRoutePlan = {
        targetKey: routeKey,
        containerSelection,
        includeHarnessDownloads: harnessRows.some(
          (candidate) => candidate.installSupported && !(candidate.installed && candidate.healthy),
        ),
        includeAuthImport: authCandidates.length > 0,
        includeTitling: titlingRequired === true,
      };
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
  ]);

  const advanceFromAuthImportStep = async (
    options?: { clearSelections?: boolean },
  ): Promise<void> => {
    if (authImportBusy) return;
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
          return;
        }
      } catch (error) {
        setAuthImportError(messageFromError(error));
        setAuthImportBusy(false);
        return;
      }
      setAuthImportBusy(false);
    }
    const titlingRequired = await ensureTitlingProbeForCurrentTarget();
    if (currentStepKeyRef.current !== "auth-import") return;
    if (titlingRequired === true && routePlan?.includeTitling !== true) {
      if (routePlan) {
        setRoutePlan({ ...routePlan, includeTitling: true });
      }
      goToStepKey("session-titling");
      return;
    }
    goToStepKey(nextAfterAuthImport(routePlan));
  };

  const advanceFromHarnessDownloadsStep = async (
    options?: { clearSelections?: boolean },
  ): Promise<void> => {
    if (harnessInstallBusy) return;
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
      if (currentStepKeyRef.current !== "harness-downloads") return;
      goToStepKey(nextAfterHarnessDownloads(routePlan));
      return;
    }
    if (runningRows.length > 0 && startableRows.length === 0) {
      setHarnessInstallError(null);
      if (currentStepKeyRef.current !== "harness-downloads") return;
      goToStepKey(nextAfterHarnessDownloads(routePlan));
      return;
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
        return;
      }
      shouldAdvance = true;
    } catch (error) {
      setHarnessInstallError(`Could not start selected downloads: ${messageFromError(error)} Continuing without those downloads.`);
      shouldAdvance = true;
    } finally {
      setHarnessInstallBusy(false);
    }
    if (!shouldAdvance) return;
    if (currentStepKeyRef.current !== "harness-downloads") return;
    goToStepKey(nextAfterHarnessDownloads(routePlan));
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
      clearTitlingInstallPoll();
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
      clearTitlingInstallPoll();
      resetTitlingDraft();
    }
  }, [
    canProbeTitling,
    selectedDaemonTargetKey,
    titlingProbeTargetKey,
  ]);

  useEffect(() => {
    return () => {
      clearTitlingInstallPoll();
      clearHarnessInstallPoll();
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
