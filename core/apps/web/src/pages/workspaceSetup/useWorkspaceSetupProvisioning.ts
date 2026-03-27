import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MutableRefObject,
} from "react";
import type {
  TitleGenerationLocalStatus,
  TitleGenerationSettings,
  UpdateTitleGenerationSettingsRequest,
} from "../../api/client";
import {
  getSettings,
  getTitleGenerationLocalStatus,
  installTitleGenerationLocal,
  updateSettings,
} from "../../api/client";
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
  type LocalInstallState,
  type RemoteStatus,
} from "./wizardTypes";
import { observeInstall, subscribeInstallProgress, type InstallProgressSnapshot } from "../../state/installProgressMonitor";
import { sameProvisioningScope } from "../../state/scopeIdentity";
import type {
  EnsureOnboardingAfterDaemonConnectResult,
  WorkspaceSetupEffectiveTarget,
  WorkspaceSetupRouteScope,
} from "./workflowTypes";
import {
  createWorkspaceSetupRouteScope,
  sameWorkspaceSetupRouteScope,
  serializeWorkspaceSetupRouteScope,
} from "./workflowTypes";
import { withTimeout } from "./promiseTimeout";
import { useWorkspaceSetupProviderProvisioning } from "./useWorkspaceSetupProviderProvisioning";
import { advanceWorkspaceSetupAuthImportStep } from "./advanceWorkspaceSetupAuthImportStep";
import { ensureWorkspaceSetupSandboxWarmup } from "./ensureWorkspaceSetupSandboxWarmup";
import { buildTitlingSummaryValue, isTitlingRemoteValid } from "./workspaceSetupTitlingSummary";

const TITLING_PROBE_TIMEOUT_MS = 2_000;

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
  const previousTargetKeyRef = useRef<string | null>(null);
  const sandboxWarmupTargetKeyRef = useRef<string | null>(null);

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

  const getCurrentRoutePlan = useCallback(
    (): WizardRoutePlan | null => provisioningMachineStateRef.current.routePlan ?? routePlan,
    [routePlan],
  );

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

  const clearTitlingInstallObserver = () => {
    titlingInstallObserverRef.current?.stop();
    titlingInstallObserverRef.current = null;
  };

  const shouldDeferSpeculativeLocalRefresh = useCallback((location: "local" | "remote"): boolean => {
    if (location !== "local") {
      return false;
    }
    const refreshReason = provisioningMachineStateRef.current.refreshReason;
    return refreshReason === "refresh_auth_import" || refreshReason === "ensure_route_plan";
  }, []);

  const {
    authImportCandidates,
    authImportSelected,
    setAuthImportSelected,
    authImportBusy,
    authImportError,
    setAuthImportBusy,
    setAuthImportError,
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
    resetProviderProvisioningState,
    scanAuthImportCandidatesForRequest,
    scanHarnessInstallCandidatesForRequest,
  } = useWorkspaceSetupProviderProvisioning({
    currentStepKeyRef,
    selections,
    effectiveTarget,
    desktopApp,
    parsedRemoteHost,
    remoteStatusRef,
    connectDaemonForImport,
    shouldDeferSpeculativeLocalRefresh,
    commitProvisioningMachineState,
    isCurrentProvisioningRequest,
    getCurrentRoutePlan,
  });

  const resetProvisioningState = useCallback(() => {
    resetProviderProvisioningState();
    clearTitlingInstallObserver();
    sandboxWarmupTargetKeyRef.current = null;
    commitProvisioningMachineState(createInitialWorkspaceSetupProvisioningMachineState());
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
  }, [commitProvisioningMachineState, resetProviderProvisioningState]);

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
      await withTimeout(
        connectDaemonForImport(),
        TITLING_PROBE_TIMEOUT_MS,
        "Timed out loading daemon settings.",
      );
      if (!isCurrentProvisioningRequest("titlingProbe", request) || selectedDaemonTargetKeyRef.current !== targetKey) {
        return null;
      }

      const settings = await withTimeout(
        getSettings(),
        TITLING_PROBE_TIMEOUT_MS,
        "Timed out loading daemon settings.",
      );
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
        localStatus = await withTimeout(
          refreshTitlingLocalStatus({ silent: true }),
          TITLING_PROBE_TIMEOUT_MS,
          "Timed out loading daemon settings.",
        );
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

  const prefetchTitlingForCurrentTarget = useCallback(async (
    locationOverride?: "local" | "remote",
  ): Promise<void> => {
    if (!desktopApp) return;
    const location = locationOverride ?? selections.location;
    if (location !== "local" && location !== "remote") {
      return;
    }
    if (location === "remote" && (!parsedRemoteHost || remoteStatusRef.current !== "connected")) {
      return;
    }
    await withTimeout(
      connectDaemonForImport(location),
      TITLING_PROBE_TIMEOUT_MS,
      "Timed out loading daemon settings.",
    );
    const settings = await withTimeout(
      getSettings(),
      TITLING_PROBE_TIMEOUT_MS,
      "Timed out loading daemon settings.",
    );
    if (!settings.title_generation || settings.title_generation.mode === "local") {
      await withTimeout(
        refreshTitlingLocalStatus({ silent: true }),
        TITLING_PROBE_TIMEOUT_MS,
        "Timed out loading daemon settings.",
      );
    }
  }, [
    connectDaemonForImport,
    desktopApp,
    parsedRemoteHost,
    refreshTitlingLocalStatus,
    remoteStatusRef,
    selections.location,
  ]);

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

  const ensureSandboxWarmupForRouteScope = useCallback(async (
    location: "local" | "remote",
    routeScope: WorkspaceSetupRouteScope,
  ): Promise<void> => ensureWorkspaceSetupSandboxWarmup(
    { desktopApp, location, routeScope, sandboxWarmupTargetKeyRef, connectDaemonForImport },
  ), [
    connectDaemonForImport,
    desktopApp,
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
      await ensureSandboxWarmupForRouteScope(location, requestedRouteScope);
      return currentPlan;
    }

    setRoutePlanningBusy(true);
    try {
      const nextState = await refreshProvisioningForRouteScope(location, requestedRouteScope, "ensure_route_plan");
      const nextRoutePlan = nextState?.routePlan ?? null;
      if (nextRoutePlan) {
        await ensureSandboxWarmupForRouteScope(location, requestedRouteScope);
      }
      return nextRoutePlan;
    } finally {
      setRoutePlanningBusy(false);
    }
  }, [
    ensureSandboxWarmupForRouteScope,
    effectiveTarget,
    refreshProvisioningForRouteScope,
    routePlan,
    selections.container,
    selections.location,
    setRoutePlanningBusy,
  ]);

  const advanceFromAuthImportStep = async (
    options?: { clearSelections?: boolean },
  ): Promise<WizardRoutePlan | null> => advanceWorkspaceSetupAuthImportStep(
    {
      authImportBusy,
      authImportSelected,
      setAuthImportSelected,
      authImportCandidates,
      setAuthImportBusy,
      setAuthImportError,
      connectDaemonForImport,
      ensureTitlingProbeForCurrentTarget,
      currentStepKeyRef,
      getCurrentRoutePlan,
    },
    options,
  );

  const titlingRemoteValid = isTitlingRemoteValid(
    titlingRemoteBaseUrl,
    titlingRemoteApiKey,
    titlingRemoteModel,
  );
  const titlingSummaryValue = buildTitlingSummaryValue({
    titlingMode,
    titlingRemoteBaseUrl,
    titlingRemoteApiKey,
    titlingRemoteModel,
    titlingConfiguredReady,
    titlingLocalStatus,
    titlingExistingSettings,
  });

  useEffect(() => {
    titlingInstallStateRef.current = titlingLocalInstall;
  }, [titlingLocalInstall]);

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
    if (selections.location !== "local" || (selections.container ?? "").trim() !== "sandbox") {
      sandboxWarmupTargetKeyRef.current = null;
    }
  }, [selections.container, selections.location]);

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
    prefetchTitlingForCurrentTarget,
    onSelectTitlingLocal,
    ensureRoutePlanForSelection,
    resetRoutePlan,
  };
}
