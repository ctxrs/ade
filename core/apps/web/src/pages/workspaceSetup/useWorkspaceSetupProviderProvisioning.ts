import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MutableRefObject,
} from "react";
import type {
  InstallTarget,
  ProviderAuthImportCandidate,
  ProviderStatus,
} from "../../api/client";
import {
  cancelInstall,
  installProvider,
  listProviderAuthImportCandidates,
  listProviders,
} from "../../api/client";
import { observeInstall, subscribeInstallProgress, type InstallProgressSnapshot } from "../../state/installProgressMonitor";
import { createHostOwnerScope } from "../../state/scopeIdentity";
import {
  resolveProviderInstallProgressSession,
  subscribeProviderInstallProgressForScope,
  upsertProviderInstallProgressForScope,
} from "../../state/providerInstallProgressStore";
import { providerDetailFlag } from "../../utils/boolish";
import { HARNESS_CATALOG } from "../../utils/harnessCatalog";
import {
  computeInstallPct,
  parseInstallTarget,
  providerInstallSizeBytes,
} from "../../utils/providerInstallUi";
import {
  isReadyVisibleHarnessProviderStatus,
  isVisibleHarnessProviderStatus,
} from "../../utils/providerInventory";
import type { WizardRoutePlan, WizardStepKey } from "./wizardFlow";
import type { WizardSelections } from "./wizardFlowReducer";
import {
  completeWorkspaceSetupAuthImportRefresh,
  completeWorkspaceSetupHarnessCandidatesRefresh,
  failWorkspaceSetupAuthImportRefresh,
  failWorkspaceSetupHarnessCandidatesRefresh,
  type WorkspaceSetupProvisioningMachineState,
  type WorkspaceSetupProvisioningRequest,
} from "./workspaceSetupProvisioningMachine";
import {
  installTargetForWorkspaceSetupContainerSelection,
  type WorkspaceSetupEffectiveTarget,
} from "./workflowTypes";
import {
  messageFromError,
  resolveHarnessInstallCandidateStatus,
  type HarnessInstallProviderRow,
  type HarnessInstallRowState,
  type RemoteStatus,
} from "./wizardTypes";

type UseWorkspaceSetupProviderProvisioningArgs = {
  currentStepKeyRef: MutableRefObject<WizardStepKey>;
  selections: WizardSelections;
  effectiveTarget: WorkspaceSetupEffectiveTarget | null;
  desktopApp: boolean;
  parsedRemoteHost: string | undefined;
  remoteStatusRef: MutableRefObject<RemoteStatus>;
  connectDaemonForImport: (locationOverride?: "local" | "remote") => Promise<void>;
  shouldDeferSpeculativeLocalRefresh: (location: "local" | "remote") => boolean;
  commitProvisioningMachineState: (
    updater:
      | WorkspaceSetupProvisioningMachineState
      | ((current: WorkspaceSetupProvisioningMachineState) => WorkspaceSetupProvisioningMachineState),
  ) => WorkspaceSetupProvisioningMachineState;
  isCurrentProvisioningRequest: (
    resource: WorkspaceSetupProvisioningRequest["resource"],
    request: WorkspaceSetupProvisioningRequest,
  ) => boolean;
  getCurrentRoutePlan: () => WizardRoutePlan | null;
};

export function useWorkspaceSetupProviderProvisioning({
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
}: UseWorkspaceSetupProviderProvisioningArgs) {
  const [authImportCandidates, setAuthImportCandidates] = useState<ProviderAuthImportCandidate[]>([]);
  const [authImportSelected, setAuthImportSelected] = useState<Record<string, boolean>>({});
  const [authImportBusy, setAuthImportBusy] = useState(false);
  const [authImportError, setAuthImportError] = useState<string | null>(null);
  const [harnessInstallCandidates, setHarnessInstallCandidates] = useState<HarnessInstallProviderRow[]>([]);
  const [harnessInstallSelected, setHarnessInstallSelected] = useState<Record<string, boolean>>({});
  const [harnessInstallBusy, setHarnessInstallBusy] = useState(false);
  const [harnessInstallError, setHarnessInstallError] = useState<string | null>(null);
  const [harnessInstallRows, setHarnessInstallRows] = useState<Record<string, HarnessInstallRowState>>({});

  const harnessInstallObserversRef = useRef<Record<string, { installId: string; stop: () => void }>>({});
  const harnessByProviderId = useMemo(() => {
    return new Map(HARNESS_CATALOG.map((entry) => [entry.id, entry]));
  }, []);
  const selectedHarnessInstallTarget: InstallTarget = installTargetForWorkspaceSetupContainerSelection(
    selections.container,
  );
  const providerProgressOwnerScope = useMemo(
    () => (effectiveTarget ? createHostOwnerScope(effectiveTarget.daemonScope) : null),
    [effectiveTarget],
  );

  const clearHarnessInstallObserver = useCallback((providerId?: string) => {
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
  }, []);

  const resetProviderProvisioningState = useCallback(() => {
    clearHarnessInstallObserver();
    setAuthImportBusy(false);
    setAuthImportCandidates([]);
    setAuthImportSelected({});
    setAuthImportError(null);
    setHarnessInstallBusy(false);
    setHarnessInstallCandidates([]);
    setHarnessInstallSelected({});
    setHarnessInstallRows({});
    setHarnessInstallError(null);
  }, [clearHarnessInstallObserver]);

  const mapHarnessInstallCandidate = useCallback((
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
      installed: isReadyVisibleHarnessProviderStatus(provider),
      healthy: isReadyVisibleHarnessProviderStatus(provider),
      installSupported,
      installRunning: providerDetailFlag(provider.details, "install_running"),
      installId: provider.details?.install_id,
      installTarget,
      installSizeBytes: providerInstallSizeBytes(provider),
    };
  }, [harnessByProviderId, selectedHarnessInstallTarget]);

  const attachHarnessInstall = useCallback(async (providerId: string, installId: string) => {
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
  }, [harnessInstallRows, providerProgressOwnerScope]);

  const cancelHarnessInstall = useCallback(async (providerId: string) => {
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
  }, [
    clearHarnessInstallObserver,
    harnessInstallCandidates,
    harnessInstallRows,
    providerProgressOwnerScope,
  ]);

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
    attachHarnessInstall,
    commitProvisioningMachineState,
    connectDaemonForImport,
    desktopApp,
    isCurrentProvisioningRequest,
    mapHarnessInstallCandidate,
    parsedRemoteHost,
    remoteStatusRef,
    shouldDeferSpeculativeLocalRefresh,
  ]);

  const advanceFromHarnessDownloadsStep = useCallback(async (
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
  }, [
    attachHarnessInstall,
    connectDaemonForImport,
    currentStepKeyRef,
    getCurrentRoutePlan,
    harnessInstallBusy,
    harnessInstallCandidates,
    harnessInstallRows,
    harnessInstallSelected,
    providerProgressOwnerScope,
    selectedHarnessInstallTarget,
  ]);

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
    clearHarnessInstallObserver,
    harnessInstallCandidates,
    harnessInstallRows,
    providerProgressOwnerScope,
    selectedHarnessInstallTarget,
  ]);

  useEffect(() => {
    return subscribeInstallProgress((snapshot: InstallProgressSnapshot) => {
      const providerIds = Object.keys(harnessInstallObserversRef.current);
      if (providerIds.length === 0) return;
      for (const providerId of providerIds) {
        const trackedInstallId = harnessInstallObserversRef.current[providerId]?.installId ?? null;
        if (!trackedInstallId) continue;
        const entry = snapshot[trackedInstallId];
        if (!entry) continue;

        const nextState: HarnessInstallRowState = {
          installId: entry.installId,
          state: entry.state,
          pct: entry.pct,
          target: entry.target,
          errorCode: entry.errorCode,
          error: entry.error,
        };
        setHarnessInstallRows((prev) => {
          const current = prev[providerId];
          if (
            current?.installId === nextState.installId
            && current.state === nextState.state
            && current.pct === nextState.pct
            && current.target === nextState.target
            && current.errorCode === nextState.errorCode
            && current.error === nextState.error
          ) {
            return prev;
          }
          return {
            ...prev,
            [providerId]: nextState,
          };
        });
        if (entry.state !== "running") {
          clearHarnessInstallObserver(providerId);
        }
      }
    });
  }, [clearHarnessInstallObserver]);

  useEffect(() => {
    return () => {
      clearHarnessInstallObserver();
    };
  }, [clearHarnessInstallObserver]);

  return {
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
  };
}
