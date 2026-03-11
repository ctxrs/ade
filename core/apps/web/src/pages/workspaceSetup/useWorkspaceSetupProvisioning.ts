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
  UpdateTitleGenerationSettingsRequest,
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
import { isVisibleHarnessProviderStatus } from "../../utils/providerInventory";
import {
  computeInstallPct,
  parseInstallTarget,
  providerInstallSizeBytes,
} from "../../utils/providerInstallUi";
import { providerDetailFlag } from "../../utils/boolish";
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
import { type WizardRoutePlan, type WizardStepKey } from "./wizardFlow";
import type { WizardSelections } from "./wizardFlowReducer";
import {
  beginWorkspaceSetupProvisioningRefresh,
  completeWorkspaceSetupAuthImportRefresh,
  completeWorkspaceSetupHarnessCandidatesRefresh,
  completeWorkspaceSetupTitlingProbeRefresh,
  createInitialWorkspaceSetupProvisioningMachineState,
  failWorkspaceSetupAuthImportRefresh,
  failWorkspaceSetupHarnessCandidatesRefresh,
  failWorkspaceSetupTitlingProbeRefresh,
  hasReadyWorkspaceSetupProvisioningStateForRouteScope,
  type WorkspaceSetupProvisioningMachineState,
  type WorkspaceSetupProvisioningRefreshReason,
  type WorkspaceSetupProvisioningRequest,
} from "./workspaceSetupProvisioningMachine";
import {
  messageFromError,
  resolveHarnessInstallCandidateStatus,
  type HarnessInstallProviderRow,
  type HarnessInstallRowState,
  type LocalInstallState,
  type RemoteStatus,
} from "./wizardTypes";
import { observeInstall, subscribeInstallProgress, type InstallProgressSnapshot } from "../../state/installProgressMonitor";
import { createHostOwnerScope, sameProvisioningScope } from "../../state/scopeIdentity";
import {
  resolveProviderInstallProgressSession,
  subscribeProviderInstallProgressForScope,
  upsertProviderInstallProgressForScope,
} from "../../state/providerInstallProgressStore";
import type {
  EnsureOnboardingAfterDaemonConnectResult,
  WorkspaceSetupEffectiveTarget,
  WorkspaceSetupRouteScope,
} from "./workflowTypes";
import {
  createWorkspaceSetupRouteScope,
  installTargetForWorkspaceSetupContainerSelection,
  sameWorkspaceSetupRouteScope,
  serializeWorkspaceSetupRouteScope,
} from "./workflowTypes";

type UseWorkspaceSetupProvisioningArgs = {
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

export function useWorkspaceSetupProvisioning({
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
  const [harnessInstallCandidates, setHarnessInstallCandidates] = useState<HarnessInstallProviderRow[]>([]);
  const [harnessInstallSelected, setHarnessInstallSelected] = useState<Record<string, boolean>>({});
  const [harnessInstallBusy, setHarnessInstallBusy] = useState(false);
  const [harnessInstallError, setHarnessInstallError] = useState<string | null>(null);
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
  const provisioningMachineStateRef = useRef<WorkspaceSetupProvisioningMachineState>(
    createInitialWorkspaceSetupProvisioningMachineState(),
  );
  const [, setProvisioningMachineState] = useState<WorkspaceSetupProvisioningMachineState>(
    () => provisioningMachineStateRef.current,
  );

  const selectedDaemonTargetKeyRef = useRef<string | null>(null);
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
  const currentRouteScope = useMemo<WorkspaceSetupRouteScope | null>(() => {
    const containerSelection = (selections.container ?? "").trim();
    if (!effectiveTarget || !containerSelection) {
      return null;
    }
    return createWorkspaceSetupRouteScope(effectiveTarget, containerSelection);
  }, [effectiveTarget, selections.container]);
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
  const selectedHarnessInstallTarget: InstallTarget = installTargetForWorkspaceSetupContainerSelection(
    selections.container,
  );
  const providerProgressOwnerScope = useMemo(
    () => (effectiveTarget ? createHostOwnerScope(effectiveTarget.daemonScope) : null),
    [effectiveTarget],
  );

  const commitProvisioningMachineState = useCallback((
    updater:
      | WorkspaceSetupProvisioningMachineState
      | ((current: WorkspaceSetupProvisioningMachineState) => WorkspaceSetupProvisioningMachineState),
  ): WorkspaceSetupProvisioningMachineState => {
    const nextState = typeof updater === "function"
      ? updater(provisioningMachineStateRef.current)
      : updater;
    provisioningMachineStateRef.current = nextState;
    setProvisioningMachineState(nextState);
    return nextState;
  }, []);

  const isCurrentProvisioningRequest = useCallback((
    resource: WorkspaceSetupProvisioningRequest["resource"],
    request: WorkspaceSetupProvisioningRequest,
  ): boolean => {
    const current = provisioningMachineStateRef.current[resource];
    return current.requestId === request.requestId
      && Boolean(current.scope)
      && sameProvisioningScope(current.scope!, request.scope);
  }, []);

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
    clearHarnessInstallObserver();
    clearTitlingInstallObserver();
    commitProvisioningMachineState(createInitialWorkspaceSetupProvisioningMachineState());
    setAuthImportBusy(false);
    setAuthImportCandidates([]);
    setAuthImportSelected({});
    setAuthImportError(null);
    setHarnessInstallBusy(false);
    setHarnessInstallCandidates([]);
    setHarnessInstallSelected({});
    setHarnessInstallRows({});
    setHarnessInstallError(null);
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
  }, [commitProvisioningMachineState]);

  const mapHarnessInstallCandidate = (
    provider: ProviderStatus,
    fallbackInstallTarget: InstallTarget = selectedHarnessInstallTarget,
  ): HarnessInstallProviderRow | null => {
    if (!isVisibleHarnessProviderStatus(provider)) return null;
    const installSupported = providerDetailFlag(provider.details, "install_supported");
    if (!installSupported) return null;
    const harness = harnessByProviderId.get(provider.provider_id);
    const installTarget = parseInstallTarget(provider.details?.install_target) ?? fallbackInstallTarget;
    return {
      providerId: provider.provider_id,
      label: harness?.label ?? provider.provider_id,
      installed: provider.installed === true,
      healthy: provider.health === "ok",
      installSupported,
      installRunning: providerDetailFlag(provider.details, "install_running"),
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
        ownerScope: providerProgressOwnerScope ?? undefined,
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
      if (providerProgressOwnerScope) {
        upsertProviderInstallProgressForScope(providerProgressOwnerScope, providerId, nextInstallState);
      }
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

  const probeTitlingForTarget = async (
    request: WorkspaceSetupProvisioningRequest,
    targetKey: string,
  ): Promise<boolean | null> => {
    setTitlingProbeBusy(true);
    setTitlingProbeTargetKey(targetKey);
    setTitlingProbeDone(false);
    setTitlingProbeError(null);
    setTitlingStatusError(null);
    try {
      await connectDaemonForImport();
      if (!isCurrentProvisioningRequest("titlingProbe", request) || selectedDaemonTargetKeyRef.current !== targetKey) {
        return null;
      }

      const settings = await getSettings();
      if (!isCurrentProvisioningRequest("titlingProbe", request) || selectedDaemonTargetKeyRef.current !== targetKey) {
        return null;
      }

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
        if (!isCurrentProvisioningRequest("titlingProbe", request) || selectedDaemonTargetKeyRef.current !== targetKey) {
          return null;
        }
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
      if (!isCurrentProvisioningRequest("titlingProbe", request) || selectedDaemonTargetKeyRef.current !== targetKey) {
        return null;
      }
      setTitlingProbeTargetKey(targetKey);
      setTitlingProbeDone(true);
      setTitlingConfiguredReady(false);
      setTitlingStepRequired(true);
      setTitlingProbeError(messageFromError(error));
      throw error;
    } finally {
      if (isCurrentProvisioningRequest("titlingProbe", request) && selectedDaemonTargetKeyRef.current === targetKey) {
        setTitlingProbeBusy(false);
      }
    }
  };

  const currentTitlingPayload = (modeOverride?: "remote" | "local"): UpdateTitleGenerationSettingsRequest | null => {
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
      const next = await updateSettings({ title_generation: payload });
      setTitlingExistingSettings(next.title_generation ?? null);
      setTitlingPersistedTargetKey(targetKey);
      setTitlingPersistedHash(payloadHash);
      if (next.title_generation?.mode === "remote") {
        const readiness = resolveSessionTitlingReadiness(next, null);
        setTitlingConfiguredReady(readiness.ready);
      } else {
        const localStatus = await refreshTitlingLocalStatus({ silent: true });
        const readiness = resolveSessionTitlingReadiness(next, localStatus);
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

  const shouldDeferSpeculativeLocalRefresh = useCallback((location: "local" | "remote"): boolean => {
    if (location !== "local") {
      return false;
    }
    const refreshReason = provisioningMachineStateRef.current.refreshReason;
    return refreshReason === "refresh_auth_import" || refreshReason === "ensure_route_plan";
  }, []);

  const scanAuthImportCandidatesForRequest = useCallback(async (
    location: "local" | "remote",
    request: WorkspaceSetupProvisioningRequest,
  ): Promise<void> => {
    if (!desktopApp) {
      if (!isCurrentProvisioningRequest("authImport", request)) {
        return;
      }
      setAuthImportBusy(false);
      setAuthImportCandidates([]);
      setAuthImportSelected({});
      setAuthImportError(null);
      commitProvisioningMachineState((current) => completeWorkspaceSetupAuthImportRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        data: [],
      }));
      return;
    }
    if (location === "remote") {
      if (!parsedRemoteHost || remoteStatusRef.current !== "connected") {
        return;
      }
    }

    setAuthImportBusy(true);
    setAuthImportError(null);
    try {
      try {
        await connectDaemonForImport(location);
      } catch (error) {
        if (shouldDeferSpeculativeLocalRefresh(location)) {
          if (!isCurrentProvisioningRequest("authImport", request)) {
            return;
          }
          setAuthImportCandidates([]);
          setAuthImportSelected({});
          setAuthImportError(null);
          commitProvisioningMachineState((current) => completeWorkspaceSetupAuthImportRefresh(current, {
            scope: request.scope,
            requestId: request.requestId,
            data: [],
          }));
          return;
        }
        throw error;
      }
      const response = await listProviderAuthImportCandidates();
      const candidates = (response.candidates ?? [])
        .filter((candidate) => candidate.parse_status === "parsed");
      if (!isCurrentProvisioningRequest("authImport", request)) {
        return;
      }
      setAuthImportCandidates(candidates);
      setAuthImportSelected((prev) =>
        Object.fromEntries(
          candidates.map((candidate) => [
            candidate.id,
            Object.prototype.hasOwnProperty.call(prev, candidate.id) ? Boolean(prev[candidate.id]) : true,
          ]),
        ),
      );
      commitProvisioningMachineState((current) => completeWorkspaceSetupAuthImportRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        data: candidates,
      }));
    } catch (error) {
      const message = messageFromError(error);
      if (!isCurrentProvisioningRequest("authImport", request)) {
        return;
      }
      setAuthImportCandidates([]);
      setAuthImportSelected({});
      setAuthImportError(message);
      commitProvisioningMachineState((current) => failWorkspaceSetupAuthImportRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        error: message,
      }));
    } finally {
      if (isCurrentProvisioningRequest("authImport", request)) {
        setAuthImportBusy(false);
      }
    }
  }, [
    commitProvisioningMachineState,
    connectDaemonForImport,
    desktopApp,
    isCurrentProvisioningRequest,
    parsedRemoteHost,
    remoteStatusRef,
    shouldDeferSpeculativeLocalRefresh,
  ]);

  const scanHarnessInstallCandidatesForRequest = useCallback(async (
    location: "local" | "remote",
    request: WorkspaceSetupProvisioningRequest,
  ): Promise<void> => {
    if (!desktopApp) {
      if (!isCurrentProvisioningRequest("harnessCandidates", request)) {
        return;
      }
      setHarnessInstallBusy(false);
      setHarnessInstallCandidates([]);
      setHarnessInstallSelected({});
      setHarnessInstallRows({});
      setHarnessInstallError(null);
      commitProvisioningMachineState((current) => completeWorkspaceSetupHarnessCandidatesRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        data: [],
      }));
      return;
    }
    if (location === "remote") {
      if (!parsedRemoteHost || remoteStatusRef.current !== "connected") {
        return;
      }
    }

    const installTarget = request.scope.installTarget;
    setHarnessInstallBusy(true);
    setHarnessInstallError(null);
    try {
      try {
        await connectDaemonForImport(location);
      } catch (error) {
        if (shouldDeferSpeculativeLocalRefresh(location)) {
          if (!isCurrentProvisioningRequest("harnessCandidates", request)) {
            return;
          }
          setHarnessInstallCandidates([]);
          setHarnessInstallSelected({});
          setHarnessInstallRows({});
          setHarnessInstallError(null);
          commitProvisioningMachineState((current) => completeWorkspaceSetupHarnessCandidatesRefresh(current, {
            scope: request.scope,
            requestId: request.requestId,
            data: [],
          }));
          return;
        }
        throw error;
      }
      const providers = await listProviders(installTarget);
      if (!isCurrentProvisioningRequest("harnessCandidates", request)) {
        return;
      }
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
      if (runningRows.length > 0) {
        setHarnessInstallRows((prev) => ({
          ...prev,
          ...Object.fromEntries(
            runningRows.map((row) => [
              row.providerId,
              {
                installId: row.installId!,
                state: "running" as const,
                pct: prev[row.providerId]?.pct ?? null,
                target: row.installTarget,
                errorCode: undefined,
                error: undefined,
              },
            ]),
          ),
        }));
      }
      for (const row of runningRows) {
        await attachHarnessInstall(row.providerId, row.installId!);
      }
      commitProvisioningMachineState((current) => completeWorkspaceSetupHarnessCandidatesRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        data: rows,
      }));
    } catch (error) {
      const message = messageFromError(error);
      if (!isCurrentProvisioningRequest("harnessCandidates", request)) {
        return;
      }
      setHarnessInstallCandidates([]);
      setHarnessInstallSelected({});
      setHarnessInstallRows({});
      setHarnessInstallError(message);
      commitProvisioningMachineState((current) => failWorkspaceSetupHarnessCandidatesRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        error: message,
      }));
    } finally {
      if (isCurrentProvisioningRequest("harnessCandidates", request)) {
        setHarnessInstallBusy(false);
      }
    }
  }, [
    commitProvisioningMachineState,
    connectDaemonForImport,
    desktopApp,
    isCurrentProvisioningRequest,
    parsedRemoteHost,
    remoteStatusRef,
    shouldDeferSpeculativeLocalRefresh,
  ]);

  const scanTitlingProbeForRequest = useCallback(async (
    request: WorkspaceSetupProvisioningRequest,
  ): Promise<void> => {
    if (!desktopApp) {
      if (!isCurrentProvisioningRequest("titlingProbe", request)) {
        return;
      }
      setTitlingProbeBusy(false);
      setTitlingProbeError(null);
      setTitlingProbeDone(true);
      setTitlingConfiguredReady(false);
      setTitlingStepRequired(false);
      setTitlingProbeTargetKey(selectedDaemonTargetKeyRef.current);
      commitProvisioningMachineState((current) => completeWorkspaceSetupTitlingProbeRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        data: { required: false },
      }));
      return;
    }
    const targetKey = selectedDaemonTargetKeyRef.current;
    if (!targetKey) return;
    try {
      const required = await probeTitlingForTarget(request, targetKey);
      if (!isCurrentProvisioningRequest("titlingProbe", request)) {
        return;
      }
      commitProvisioningMachineState((current) => completeWorkspaceSetupTitlingProbeRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        data: { required: required === true },
      }));
    } catch (error) {
      const message = messageFromError(error);
      if (!isCurrentProvisioningRequest("titlingProbe", request)) {
        return;
      }
      commitProvisioningMachineState((current) => failWorkspaceSetupTitlingProbeRefresh(current, {
        scope: request.scope,
        requestId: request.requestId,
        error: message,
      }));
    }
  }, [
    commitProvisioningMachineState,
    desktopApp,
    isCurrentProvisioningRequest,
    probeTitlingForTarget,
  ]);

  const refreshProvisioningForRouteScope = useCallback(async (
    location: "local" | "remote",
    routeScope: WorkspaceSetupRouteScope,
    refreshReason: WorkspaceSetupProvisioningRefreshReason,
    options?: {
      allowTitlingInsertion?: boolean;
      resources?: WorkspaceSetupProvisioningRequest["resource"][];
      force?: boolean;
    },
  ): Promise<WorkspaceSetupProvisioningMachineState | null> => {
    const refreshStart = beginWorkspaceSetupProvisioningRefresh(provisioningMachineStateRef.current, {
      routeScope,
      refreshReason,
      titlingMode,
      previousPlan: routePlan,
      allowTitlingInsertion: options?.allowTitlingInsertion,
      resources: options?.resources,
      force: options?.force,
    });
    const started = commitProvisioningMachineState(refreshStart.state);
    const { requests } = refreshStart;
    if (requests.length === 0) {
      if (started.routeScope && sameWorkspaceSetupRouteScope(started.routeScope, routeScope)) {
        setRoutePlan(started.routePlan);
        return started;
      }
      return null;
    }

    await Promise.all(requests.map(async (request) => {
      switch (request.resource) {
        case "authImport":
          await scanAuthImportCandidatesForRequest(location, request);
          return;
        case "harnessCandidates":
          await scanHarnessInstallCandidatesForRequest(location, request);
          return;
        case "titlingProbe":
          await scanTitlingProbeForRequest(request);
          return;
        default: {
          const exhaustiveCheck: never = request.resource;
          return exhaustiveCheck;
        }
      }
    }));

    const current = provisioningMachineStateRef.current;
    if (!current.routeScope || !sameWorkspaceSetupRouteScope(current.routeScope, routeScope)) {
      return null;
    }
    setRoutePlan(current.routePlan);
    return current;
  }, [
    commitProvisioningMachineState,
    routePlan,
    scanAuthImportCandidatesForRequest,
    scanHarnessInstallCandidatesForRequest,
    scanTitlingProbeForRequest,
    setRoutePlan,
    titlingMode,
  ]);

  const refreshAuthImportForRouteScope = useCallback(async (
    location: "local" | "remote",
    routeScope: WorkspaceSetupRouteScope,
    options?: { force?: boolean },
  ): Promise<void> => {
    await refreshProvisioningForRouteScope(
      location,
      routeScope,
      "refresh_auth_import",
      {
        force: options?.force,
        resources: ["authImport"],
      },
    );
  }, [refreshProvisioningForRouteScope]);

  const ensureTitlingProbeForCurrentTarget = useCallback(async (
    options?: { force?: boolean },
  ): Promise<boolean | null> => {
    const location = selections.location;
    if (!currentRouteScope || (location !== "local" && location !== "remote")) {
      return null;
    }
    if (location === "remote") {
      if (!parsedRemoteHost || remoteStatusRef.current !== "connected") {
        return null;
      }
    }
    const nextState = await refreshProvisioningForRouteScope(
      location,
      currentRouteScope,
      "refresh_titling_probe",
      {
        force: options?.force,
        resources: ["titlingProbe"],
      },
    );
    if (!nextState) return null;
    return nextState.titlingProbe.status === "error"
      ? true
      : nextState.titlingProbe.data?.required === true;
  }, [
    currentRouteScope,
    parsedRemoteHost,
    refreshProvisioningForRouteScope,
    remoteStatusRef,
    selections.location,
  ]);

  const resetRoutePlan = useCallback(() => {
    invalidateRoutePlan();
  }, [invalidateRoutePlan]);

  const ensureOnboardingAfterDaemonConnect = useCallback(async (
    options?: { allowTitlingInsertion?: boolean },
  ): Promise<EnsureOnboardingAfterDaemonConnectResult | null> => {
    const location = selections.location;
    if (!currentRouteScope || (location !== "local" && location !== "remote")) {
      return null;
    }
    const nextState = await refreshProvisioningForRouteScope(
      location,
      currentRouteScope,
      "ensure_onboarding_after_connect",
      {
        allowTitlingInsertion: options?.allowTitlingInsertion,
        force: true,
      },
    );
    if (!nextState?.routePlan) {
      return null;
    }
    return {
      routePlan: nextState.routePlan,
      insertionStep: nextState.insertionStep,
    };
  }, [
    currentRouteScope,
    refreshProvisioningForRouteScope,
    selections.location,
  ]);

  const ensureRoutePlanForSelection = useCallback(async (
    containerSelectionOverride?: string,
  ): Promise<WizardRoutePlan | null> => {
    const location = selections.location;
    const containerSelection = (containerSelectionOverride ?? selections.container ?? "").trim();
    if (!effectiveTarget || !containerSelection || (location !== "local" && location !== "remote")) {
      return null;
    }
    const requestedRouteScope = createWorkspaceSetupRouteScope(effectiveTarget, containerSelection);
    const requestedRouteKey = serializeWorkspaceSetupRouteScope(requestedRouteScope);
    const currentPlan = provisioningMachineStateRef.current.routePlan ?? routePlan;
    if (
      currentPlan?.targetKey === requestedRouteKey
      && hasReadyWorkspaceSetupProvisioningStateForRouteScope(
        provisioningMachineStateRef.current,
        requestedRouteScope,
      )
    ) {
      return currentPlan;
    }

    setRoutePlanningBusy(true);
    try {
      const nextState = await refreshProvisioningForRouteScope(location, requestedRouteScope, "ensure_route_plan");
      return nextState?.routePlan ?? null;
    } finally {
      setRoutePlanningBusy(false);
    }
  }, [
    effectiveTarget,
    refreshProvisioningForRouteScope,
    routePlan,
    selections.container,
    selections.location,
    setRoutePlanningBusy,
  ]);

  const getCurrentRoutePlan = useCallback(
    (): WizardRoutePlan | null => provisioningMachineStateRef.current.routePlan ?? routePlan,
    [routePlan],
  );

  const advanceFromAuthImportStep = async (
    options?: { clearSelections?: boolean },
  ): Promise<WizardRoutePlan | null> => {
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
    await ensureTitlingProbeForCurrentTarget();
    if (currentStepKeyRef.current !== "auth-import") return null;
    return getCurrentRoutePlan();
  };

  const advanceFromHarnessDownloadsStep = async (
    options?: { clearSelections?: boolean },
  ): Promise<WizardRoutePlan | null> => {
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
    const currentRoutePlan = getCurrentRoutePlan();

    if (selectedRows.length === 0 || selectedRows.every(({ status }) => status === "installed" || status === "succeeded")) {
      setHarnessInstallError(null);
      if (currentStepKeyRef.current !== "harness-downloads") return null;
      return currentRoutePlan;
    }
    if (runningRows.length > 0 && startableRows.length === 0) {
      setHarnessInstallError(null);
      if (currentStepKeyRef.current !== "harness-downloads") return null;
      return currentRoutePlan;
    }
    if (blockingRows.length > 0 && startableRows.length === 0) {
      if (currentStepKeyRef.current !== "harness-downloads") return null;
      return currentRoutePlan;
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
            if (providerProgressOwnerScope) {
              upsertProviderInstallProgressForScope(
                providerProgressOwnerScope,
                row.providerId,
                nextInstallState,
              );
            }
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
    return getCurrentRoutePlan();
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
    if (!providerProgressOwnerScope) {
      return () => {};
    }
    return subscribeProviderInstallProgressForScope(providerProgressOwnerScope, (snapshot) => {
      const providerIds = new Set([
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
  }, [
    harnessInstallCandidates,
    harnessInstallRows,
    providerProgressOwnerScope,
    selectedHarnessInstallTarget,
  ]);

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
    if (!routePlan) return;
    const activeTargetKey = currentRouteScope
      ? serializeWorkspaceSetupRouteScope(currentRouteScope)
      : null;
    if (activeTargetKey === routePlan.targetKey) return;
    resetRoutePlan();
  }, [
    currentRouteScope,
    resetRoutePlan,
    routePlan,
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
    refreshAuthImportForRouteScope,
    onSelectTitlingLocal,
    ensureRoutePlanForSelection,
    resetRoutePlan,
  };
}
