import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { launchPhaseLabel } from "./workspaceSetup/launchProgress";
import { WorkspaceSetupPageView } from "./workspaceSetup/WorkspaceSetupPageView";
import { useWorkspaceSetupCreate } from "./workspaceSetup/useWorkspaceSetupCreate";
import { useWorkspaceSetupFlow } from "./workspaceSetup/useWorkspaceSetupFlow";
import { useWorkspaceSetupProvisioning } from "./workspaceSetup/useWorkspaceSetupProvisioning";
import { useWorkspaceSetupRemote } from "./workspaceSetup/useWorkspaceSetupRemote";
import { nextBoundaryStep } from "./workspaceSetup/wizardFlow";
import {
  trackWizardAbandoned,
  trackWizardCompleted,
  trackWizardStarted,
  trackWizardStepViewed,
} from "../utils/analytics";

type ImportRepoStatus = "idle" | "checking" | "ok" | "error";

export function WorkspaceSetupPageController() {
  const navigate = useNavigate();
  const [sourcePath, setSourcePath] = useState("");
  const [repoUrl, setRepoUrl] = useState("");
  const [repoBranch, setRepoBranch] = useState("");
  const [workspaceName, setWorkspaceName] = useState("");
  const [networkAllowlist, setNetworkAllowlist] = useState("");
  const [containerAdvancedOpen, setContainerAdvancedOpen] = useState(false);
  const [setupHook, setSetupHook] = useState("");
  const [targetBranch, setTargetBranch] = useState("main");
  const [targetBranchTouched, setTargetBranchTouched] = useState(false);
  const [verifyCommand, setVerifyCommand] = useState("");
  const [mergeAdvancedOpen, setMergeAdvancedOpen] = useState(false);
  const [pushOnSuccess, setPushOnSuccess] = useState(false);
  const [pushRemote, setPushRemote] = useState("origin");
  const [pushBranch, setPushBranch] = useState("main");
  const [pushBranchTouched, setPushBranchTouched] = useState(false);
  const [openInfoKey, setOpenInfoKey] = useState<string | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const [importRepoStatus, setImportRepoStatus] = useState<ImportRepoStatus>("idle");
  const [importRepoNote, setImportRepoNote] = useState<string | null>(null);
  const [harnessDownloadsCanScroll, setHarnessDownloadsCanScroll] = useState(false);
  const [harnessDownloadsAtBottom, setHarnessDownloadsAtBottom] = useState(true);

  const wizardKey = "workspace_setup" as const;
  const wizardCompletedRef = useRef(false);
  const wizardStartedRef = useRef(false);
  const lastWizardStepViewedRef = useRef<{ key: string; index: number } | null>(null);
  const harnessDownloadsScrollRef = useRef<HTMLDivElement | null>(null);

  const flow = useWorkspaceSetupFlow({
    sourcePath,
    repoUrl,
  });

  const remote = useWorkspaceSetupRemote({
    selections: flow.selections,
    stepKey: flow.currentStepKey,
    needsSourcePath: flow.needsSourcePath,
    sourcePath,
    setImportRepoStatus,
    setImportRepoNote,
    setTargetBranch,
    setPushBranch,
    targetBranchTouched,
    pushBranchTouched,
    onRemoteEndpointChanged: flow.invalidateRoutePlan,
  });

  const provisioning = useWorkspaceSetupProvisioning({
    currentStepKey: flow.currentStepKey,
    currentStepKeyRef: flow.currentStepKeyRef,
    selections: flow.selections,
    routePlan: flow.routePlan,
    setRoutePlan: flow.setRoutePlan,
    setRoutePlanningBusy: flow.setRoutePlanningBusy,
    invalidateRoutePlan: flow.invalidateRoutePlan,
    goToStepKey: flow.goToStepKey,
    goRelativeStep: flow.goRelativeStep,
    desktopApp: remote.desktopApp,
    selectedDaemonTargetKey: remote.selectedDaemonTargetKey,
    parsedRemoteHost: remote.parsedRemote?.host,
    parsedRemoteUser: remote.parsedRemote?.user,
    parsedRemotePort: remote.parsedRemotePort,
    remoteDataDirInput: remote.remoteDataDirInput,
    remoteStatus: remote.remoteStatus,
    remoteStatusRef: remote.remoteStatusRef,
    connectDaemonForImport: remote.connectDaemonForImport,
  });

  const create = useWorkspaceSetupCreate({
    currentStepKey: flow.currentStepKey,
    currentStepKeyRef: flow.currentStepKeyRef,
    selections: flow.selections,
    titlingStepVisible: provisioning.titlingStepVisible,
    titlingMode: provisioning.titlingMode,
    titlingRemoteValid: provisioning.titlingRemoteValid,
    titlingPersistError: provisioning.titlingPersistError,
    ensureTitlingPersistedForCurrentTarget: provisioning.ensureTitlingPersistedForCurrentTarget,
    sourcePath,
    setSourcePath,
    repoUrl,
    repoBranch,
    workspaceName,
    networkAllowlist,
    useDiskIsolatedStaging: flow.useDiskIsolatedStaging,
    importRepoStatus,
    setImportRepoStatus,
    importRepoNote,
    setImportRepoNote,
    targetBranch,
    verifyCommand,
    mergeQueueSkipped: flow.mergeQueueSkipped,
    pushOnSuccess,
    pushRemote,
    pushBranch,
    setupHook,
    goToStepKey: flow.goToStepKey,
    navigate: (path, opts) => navigate(path, opts),
    wizardCompletedRef,
    wizardKey,
    trackWizardCompleted: (payload: { wizardKey: string; workspaceKind: string }) => {
      trackWizardCompleted({
        wizardKey: payload.wizardKey as "workspace_setup",
        workspaceKind: payload.workspaceKind as "local" | "remote" | "unknown",
      });
    },
    desktopApp: remote.desktopApp,
    parsedRemoteHost: remote.parsedRemote?.host,
    parsedRemoteUser: remote.parsedRemote?.user,
    remoteHostInput: remote.remoteHostInput,
    remotePasswordOnce: remote.remotePasswordOnce,
    parsedRemotePort: remote.parsedRemotePort,
    remoteDataDirInput: remote.remoteDataDirInput,
    connectDaemonForImport: remote.connectDaemonForImport,
    ensureOnboardingAfterDaemonConnect: provisioning.ensureOnboardingAfterDaemonConnect,
    waitForDaemonReady: remote.waitForDaemonReady,
    applyConnection: remote.applyConnection,
    rememberRemoteProfile: remote.rememberRemoteProfile,
    createError,
    setCreateError,
  });

  const infoStep = openInfoKey ? flow.steps.find((step) => step.key === openInfoKey) ?? null : null;
  const hasAllowlist = flow.step.key !== "network"
    || flow.selections.network !== "allowlist"
    || networkAllowlist.split(/\r?\n/).map((line) => line.trim()).filter(Boolean).length > 0;
  const hasSourceStepInputs = flow.step.key !== "source" || flow.sourceStepValidation.isComplete;
  const hasTargetBranch = flow.step.key !== "merge-queue"
    || flow.mergeQueueSkipped
    || targetBranch.trim() !== "";
  const titlingSelectionComplete = !provisioning.titlingStepVisible
    || provisioning.titlingMode === "skip"
    || (provisioning.titlingMode === "remote" && provisioning.titlingRemoteValid)
    || provisioning.titlingMode === "local";
  const titlingStepCanAdvance = flow.step.key !== "session-titling"
    || (!provisioning.titlingPersistBusy && titlingSelectionComplete);
  const canAdvance = (!flow.requiresSelection || flow.hasSelection)
    && !(flow.step.key === "location" && flow.selections.location === "remote" && (
      !remote.hasRemoteHost
      || remote.remoteStatus === "connecting"
      || remote.parsedRemotePort === null
    ))
    && !(flow.step.key === "container" && flow.routePlanningBusy)
    && hasSourceStepInputs
    && hasTargetBranch
    && hasAllowlist
    && (flow.step.key !== "auth-import" || !provisioning.authImportBusy)
    && (flow.step.key !== "harness-downloads" || !provisioning.harnessInstallBusy)
    && titlingStepCanAdvance;
  const nextButtonLabel = flow.step.key === "container" && flow.routePlanningBusy
    ? "Working..."
    : flow.step.key === "harness-downloads"
      ? (
        provisioning.harnessInstallBusy
          ? "Working..."
          : provisioning.selectedHarnessReadyToStartCount > 0
            ? "Start and continue"
            : "Continue"
      )
      : "Next";

  const updateHarnessDownloadsScrollState = useCallback(() => {
    const node = harnessDownloadsScrollRef.current;
    if (!node) {
      setHarnessDownloadsCanScroll(false);
      setHarnessDownloadsAtBottom(true);
      return;
    }
    const canScroll = node.scrollHeight - node.clientHeight > 2;
    const atBottom = !canScroll || node.scrollTop + node.clientHeight >= node.scrollHeight - 2;
    setHarnessDownloadsCanScroll(canScroll);
    setHarnessDownloadsAtBottom(atBottom);
  }, []);

  const onSelect = useCallback((stepKey: string, optionId: string) => {
    setCreateError(null);
    flow.selectOption(stepKey, optionId);
    if (stepKey === "location" && optionId === "local") {
      remote.resetForLocalSelection();
    }
    if (stepKey === "location") {
      flow.invalidateRoutePlan();
    }
    if (stepKey === "container" && optionId === "no-container") {
      setNetworkAllowlist("");
    }
    if (stepKey === "network" && optionId !== "allowlist") {
      setNetworkAllowlist("");
    }
    if (stepKey === "source") {
      if (optionId !== "clone") {
        setRepoUrl("");
        setRepoBranch("");
      }
      if (optionId !== "new") {
        setWorkspaceName("");
      }
      if (optionId !== "import") {
        setImportRepoStatus("idle");
        setImportRepoNote(null);
      }
    }
  }, [flow, remote]);

  const onSelectOption = useCallback((stepKey: string, optionId: string) => {
    onSelect(stepKey, optionId);
    if (stepKey === "location" && optionId === "local") {
      flow.goToStepKey("container");
      return;
    }
    if (stepKey === "container") {
      void (async () => {
        const plan = await provisioning.ensureRoutePlanForSelection(optionId);
        if (!plan) return;
        if (flow.currentStepKeyRef.current !== "container") return;
        flow.goToStepKey(nextBoundaryStep(plan));
      })();
      return;
    }
    if (stepKey === "network" && optionId !== "allowlist") {
      flow.goRelativeStep(1);
    }
  }, [flow, onSelect, provisioning]);

  const onNext = useCallback(async () => {
    if (flow.step.key === "location") {
      if (flow.selections.location === "remote") {
        const connected = await remote.verifyRemoteConnection();
        if (!connected) return;
      }
      if (flow.currentStepKeyRef.current !== "location") return;
      flow.goToStepKey("container");
      return;
    }
    if (flow.step.key === "container") {
      const plan = await provisioning.ensureRoutePlanForSelection();
      if (!plan) return;
      if (flow.currentStepKeyRef.current !== "container") return;
      flow.goToStepKey(nextBoundaryStep(plan));
      return;
    }
    if (flow.step.key === "auth-import") {
      await provisioning.advanceFromAuthImportStep();
      return;
    }
    if (flow.step.key === "harness-downloads") {
      await provisioning.advanceFromHarnessDownloadsStep();
      return;
    }
    if (flow.step.key === "session-titling") {
      provisioning.setTitlingPersistError(null);
      if (provisioning.titlingMode === "skip") {
        flow.goRelativeStep(1);
        return;
      }
      if (provisioning.titlingMode !== "remote" && provisioning.titlingMode !== "local") {
        provisioning.setTitlingPersistError("Choose a titling option or skip for now.");
        return;
      }
      if (provisioning.titlingMode === "remote" && !provisioning.titlingRemoteValid) {
        provisioning.setTitlingPersistError("Remote titling needs base URL, API key, and model.");
        return;
      }
      const persisted = await provisioning.ensureTitlingPersistedForCurrentTarget();
      if (!persisted) return;
      flow.goRelativeStep(1);
      return;
    }
    if (flow.step.key === "source") {
      setCreateError(null);
      const preflightOk = await create.preflightSourceStep();
      if (!preflightOk) return;
      flow.goRelativeStep(1);
      return;
    }
    flow.goRelativeStep(1);
  }, [create, flow, provisioning, remote]);

  useEffect(() => {
    if (flow.currentStepKey !== "harness-downloads") {
      setHarnessDownloadsCanScroll(false);
      setHarnessDownloadsAtBottom(true);
      return;
    }
    const handle = window.requestAnimationFrame(() => {
      updateHarnessDownloadsScrollState();
    });
    window.addEventListener("resize", updateHarnessDownloadsScrollState);
    return () => {
      window.cancelAnimationFrame(handle);
      window.removeEventListener("resize", updateHarnessDownloadsScrollState);
    };
  }, [
    flow.currentStepKey,
    provisioning.harnessInstallBusy,
    provisioning.harnessInstallCandidates,
    provisioning.harnessInstallRows,
    updateHarnessDownloadsScrollState,
  ]);

  useEffect(() => {
    if (wizardStartedRef.current) return;
    wizardStartedRef.current = true;
    trackWizardStarted({ wizardKey });
    return () => {
      if (wizardCompletedRef.current) return;
      const last = lastWizardStepViewedRef.current ?? {
        key: flow.currentStepKeyRef.current,
        index: flow.stepIndex,
      };
      trackWizardAbandoned({
        wizardKey,
        lastStepKey: last.key,
        lastStepIndex: last.index,
      });
    };
  }, [flow.currentStepKeyRef, flow.stepIndex, wizardKey]);

  useEffect(() => {
    lastWizardStepViewedRef.current = { key: flow.step.key, index: flow.stepIndex };
    trackWizardStepViewed({
      wizardKey,
      stepKey: flow.step.key,
      stepIndex: flow.stepIndex,
    });
  }, [flow.step.key, flow.stepIndex, wizardKey]);

  useEffect(() => {
    if (flow.selections.container === "host-mounted") {
      setContainerAdvancedOpen(true);
    }
  }, [flow.selections.container]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (create.importInitDialog) {
        create.resolveImportInitDialog(false);
        return;
      }
      setOpenInfoKey(null);
    };
    if (openInfoKey || create.importInitDialog) {
      window.addEventListener("keydown", onKeyDown);
      return () => window.removeEventListener("keydown", onKeyDown);
    }
    return;
  }, [create.importInitDialog, create.resolveImportInitDialog, openInfoKey]);

  useEffect(() => {
    if (pushBranchTouched) return;
    if (!targetBranch.trim()) return;
    setPushBranch(targetBranch);
  }, [pushBranchTouched, targetBranch]);

  return (
    <WorkspaceSetupPageView
      importInitDialog={create.importInitDialog}
      resolveImportInitDialog={create.resolveImportInitDialog}
      infoStep={infoStep}
      openInfoKey={openInfoKey}
      setOpenInfoKey={setOpenInfoKey}
      step={flow.step}
      steps={flow.steps}
      stepIndex={flow.stepIndex}
      selections={flow.selections}
      createError={createError}
      setCreateError={setCreateError}
      showLaunchPanel={create.showLaunchPanel}
      launchSnapshot={create.launchSnapshot}
      currentLaunchPhaseLabel={launchPhaseLabel(create.launchSnapshot?.current_phase)}
      currentLaunchElapsed={create.currentLaunchElapsed}
      launchCopyLabel={create.launchCopyLabel}
      onCopyLaunchDiagnostics={() => {
        void create.onCopyLaunchDiagnostics();
      }}
      launchLogs={create.launchLogs}
      onSelectOption={onSelectOption}
      containerAdvancedOpen={containerAdvancedOpen}
      setContainerAdvancedOpen={setContainerAdvancedOpen}
      networkAllowlist={networkAllowlist}
      setNetworkAllowlist={setNetworkAllowlist}
      remoteHostInput={remote.remoteHostInput}
      onRemoteInputChange={remote.onRemoteInputChange}
      remotePasswordPromptVisible={remote.remotePasswordPromptVisible}
      remotePasswordInput={remote.remotePasswordInput}
      setRemotePasswordInput={remote.setRemotePasswordInput}
      remoteStatus={remote.remoteStatus}
      setRemoteStatus={remote.setRemoteStatus}
      remoteError={remote.remoteError}
      setRemoteError={remote.setRemoteError}
      sshSuggestions={remote.sshSuggestions}
      authImportBusy={provisioning.authImportBusy}
      authImportError={provisioning.authImportError}
      authImportCandidates={provisioning.authImportCandidates}
      authImportSelected={provisioning.authImportSelected}
      setAuthImportSelected={provisioning.setAuthImportSelected}
      harnessByProviderId={provisioning.harnessByProviderId}
      onSkipAuthImport={() => {
        void provisioning.advanceFromAuthImportStep({ clearSelections: true });
      }}
      harnessInstallBusy={provisioning.harnessInstallBusy}
      harnessInstallError={provisioning.harnessInstallError}
      selectedHarnessRunningCount={provisioning.selectedHarnessRunningCount}
      selectedHarnessBlockedCount={provisioning.selectedHarnessBlockedCount}
      harnessInstallCandidates={provisioning.harnessInstallCandidates}
      harnessDownloadsCanScroll={harnessDownloadsCanScroll}
      harnessDownloadsAtBottom={harnessDownloadsAtBottom}
      harnessDownloadsScrollRef={harnessDownloadsScrollRef}
      updateHarnessDownloadsScrollState={updateHarnessDownloadsScrollState}
      harnessInstallSelected={provisioning.harnessInstallSelected}
      setHarnessInstallSelected={provisioning.setHarnessInstallSelected}
      harnessInstallRows={provisioning.harnessInstallRows}
      selectedHarnessInstallTarget={provisioning.selectedHarnessInstallTarget}
      cancelHarnessInstall={(providerId) => {
        void provisioning.cancelHarnessInstall(providerId);
      }}
      onSkipHarnessDownloads={() => {
        void provisioning.advanceFromHarnessDownloadsStep({ clearSelections: true });
      }}
      titlingProbeBusy={provisioning.titlingProbeBusy}
      titlingProbeError={provisioning.titlingProbeError}
      titlingPersistError={provisioning.titlingPersistError}
      titlingStatusError={provisioning.titlingStatusError}
      titlingMode={provisioning.titlingMode}
      setTitlingMode={provisioning.setTitlingMode}
      titlingLocalInstallBusy={provisioning.titlingLocalInstallBusy}
      titlingPersistBusy={provisioning.titlingPersistBusy}
      onSelectTitlingLocal={provisioning.onSelectTitlingLocal}
      titlingLocalStatus={provisioning.titlingLocalStatus}
      titlingLocalInstall={provisioning.titlingLocalInstall}
      titlingRemoteBaseUrl={provisioning.titlingRemoteBaseUrl}
      setTitlingRemoteBaseUrl={provisioning.setTitlingRemoteBaseUrl}
      titlingRemoteApiKey={provisioning.titlingRemoteApiKey}
      setTitlingRemoteApiKey={provisioning.setTitlingRemoteApiKey}
      titlingRemoteModel={provisioning.titlingRemoteModel}
      setTitlingRemoteModel={provisioning.setTitlingRemoteModel}
      titlingRemoteAdvancedOpen={provisioning.titlingRemoteAdvancedOpen}
      setTitlingRemoteAdvancedOpen={provisioning.setTitlingRemoteAdvancedOpen}
      titlingRemoteUseJson={provisioning.titlingRemoteUseJson}
      setTitlingRemoteUseJson={provisioning.setTitlingRemoteUseJson}
      invalidateTitlingPersisted={provisioning.invalidateTitlingPersisted}
      onSkipTitling={() => {
        provisioning.invalidateTitlingPersisted();
        provisioning.setTitlingMode("skip");
        flow.goRelativeStep(1);
      }}
      needsSourcePath={flow.needsSourcePath}
      sourcePath={sourcePath}
      setSourcePath={setSourcePath}
      onPickLocalFolder={() => {
        void create.onPickLocalFolder();
      }}
      importRepoStatus={importRepoStatus}
      importRepoNote={importRepoNote}
      remotePathSuggestions={remote.remotePathSuggestions}
      remotePathStatus={remote.remotePathStatus}
      remotePathError={remote.remotePathError}
      repoUrl={repoUrl}
      setRepoUrl={setRepoUrl}
      repoBranch={repoBranch}
      setRepoBranch={setRepoBranch}
      useDiskIsolatedStaging={flow.useDiskIsolatedStaging}
      setupHook={setupHook}
      setSetupHook={setSetupHook}
      workspaceName={workspaceName}
      setWorkspaceName={setWorkspaceName}
      mergeQueueSkipped={flow.mergeQueueSkipped}
      targetBranch={targetBranch}
      setTargetBranch={setTargetBranch}
      setTargetBranchTouched={setTargetBranchTouched}
      verifyCommand={verifyCommand}
      setVerifyCommand={setVerifyCommand}
      mergeAdvancedOpen={mergeAdvancedOpen}
      setMergeAdvancedOpen={setMergeAdvancedOpen}
      pushOnSuccess={pushOnSuccess}
      setPushOnSuccess={setPushOnSuccess}
      pushRemote={pushRemote}
      setPushRemote={setPushRemote}
      pushBranch={pushBranch}
      setPushBranch={setPushBranch}
      setPushBranchTouched={setPushBranchTouched}
      enableMergeQueueIfSkipped={() => {
        if (!flow.mergeQueueSkipped) return;
        flow.clearSelection("merge-queue");
      }}
      onMergeSkip={() => {
        onSelect("merge-queue", "skip");
        setMergeAdvancedOpen(false);
        setPushOnSuccess(false);
        flow.goRelativeStep(1);
      }}
      harnessSummaryValue={provisioning.harnessSummaryValue}
      titlingSummaryValue={provisioning.titlingSummaryValue}
      sourceStepComplete={flow.sourceStepValidation.isComplete}
      titlingRemoteValid={provisioning.titlingRemoteValid}
      hasRemoteHost={remote.hasRemoteHost}
      goToStepKey={flow.goToStepKey}
      isFirst={flow.isFirst}
      isLast={flow.isLast}
      canAdvance={canAdvance}
      creating={create.creating}
      onCreate={() => {
        void create.onCreate();
      }}
      onNext={() => {
        void onNext();
      }}
      goRelativeStep={flow.goRelativeStep}
      createButtonLabel={create.createButtonLabel}
      nextButtonLabel={nextButtonLabel}
    />
  );
}
