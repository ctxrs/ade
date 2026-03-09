import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  type SetStateAction,
  type MutableRefObject,
} from "react";
import { nextAfterHarnessDownloads, nextBoundaryStep } from "./wizardFlow";
import { useWorkspaceSetupCreate } from "./useWorkspaceSetupCreate";
import { useWorkspaceSetupFlow } from "./useWorkspaceSetupFlow";
import { useWorkspaceSetupProvisioning } from "./useWorkspaceSetupProvisioning";
import { useWorkspaceSetupRemote } from "./useWorkspaceSetupRemote";
import {
  createInitialWorkflowDraftState,
  makeDraftFieldSetter,
} from "./workflowReducer";
import type { WorkspaceSetupDraftState } from "./workflowTypes";
import { workspaceSetupWorkflowReducer } from "./workflowReducer";
import type { WizardStepKey } from "./wizardFlow";

type UseWorkspaceSetupWorkflowArgs = {
  navigate: (path: string, opts: { replace: boolean }) => void;
  wizardCompletedRef: MutableRefObject<boolean>;
  wizardKey: string;
  trackWizardCompleted: (payload: { wizardKey: string; workspaceKind: string }) => void;
};

type FieldSetters = {
  [K in keyof WorkspaceSetupDraftState]: (value: SetStateAction<WorkspaceSetupDraftState[K]>) => void;
};

export function useWorkspaceSetupWorkflow({
  navigate,
  wizardCompletedRef,
  wizardKey,
  trackWizardCompleted,
}: UseWorkspaceSetupWorkflowArgs) {
  const [draft, dispatchDraft] = useReducer(
    workspaceSetupWorkflowReducer,
    undefined,
    createInitialWorkflowDraftState,
  );

  const setters = useMemo<FieldSetters>(() => ({
    sourcePath: makeDraftFieldSetter(dispatchDraft, "sourcePath"),
    repoUrl: makeDraftFieldSetter(dispatchDraft, "repoUrl"),
    repoBranch: makeDraftFieldSetter(dispatchDraft, "repoBranch"),
    workspaceName: makeDraftFieldSetter(dispatchDraft, "workspaceName"),
    networkAllowlist: makeDraftFieldSetter(dispatchDraft, "networkAllowlist"),
    setupHook: makeDraftFieldSetter(dispatchDraft, "setupHook"),
    targetBranch: makeDraftFieldSetter(dispatchDraft, "targetBranch"),
    targetBranchTouched: makeDraftFieldSetter(dispatchDraft, "targetBranchTouched"),
    verifyCommand: makeDraftFieldSetter(dispatchDraft, "verifyCommand"),
    pushOnSuccess: makeDraftFieldSetter(dispatchDraft, "pushOnSuccess"),
    pushRemote: makeDraftFieldSetter(dispatchDraft, "pushRemote"),
    pushBranch: makeDraftFieldSetter(dispatchDraft, "pushBranch"),
    pushBranchTouched: makeDraftFieldSetter(dispatchDraft, "pushBranchTouched"),
    createError: makeDraftFieldSetter(dispatchDraft, "createError"),
    importRepoStatus: makeDraftFieldSetter(dispatchDraft, "importRepoStatus"),
    importRepoNote: makeDraftFieldSetter(dispatchDraft, "importRepoNote"),
  }), []);

  const flow = useWorkspaceSetupFlow({
    sourcePath: draft.sourcePath,
    repoUrl: draft.repoUrl,
  });

  const remote = useWorkspaceSetupRemote({
    selections: flow.selections,
    stepKey: flow.currentStepKey,
    needsSourcePath: flow.needsSourcePath,
    sourcePath: draft.sourcePath,
    setImportRepoStatus: setters.importRepoStatus,
    setImportRepoNote: setters.importRepoNote,
    setTargetBranch: setters.targetBranch,
    setPushBranch: setters.pushBranch,
    targetBranchTouched: draft.targetBranchTouched,
    pushBranchTouched: draft.pushBranchTouched,
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
    intent: {
      selections: flow.selections,
      sourcePath: draft.sourcePath,
      repoUrl: draft.repoUrl,
      repoBranch: draft.repoBranch,
      workspaceName: draft.workspaceName,
      networkAllowlist: draft.networkAllowlist,
      useDiskIsolatedStaging: flow.useDiskIsolatedStaging,
      importRepoStatus: draft.importRepoStatus,
      importRepoNote: draft.importRepoNote,
      targetBranch: draft.targetBranch,
      verifyCommand: draft.verifyCommand,
      mergeQueueSkipped: flow.mergeQueueSkipped,
      pushOnSuccess: draft.pushOnSuccess,
      pushRemote: draft.pushRemote,
      pushBranch: draft.pushBranch,
      setupHook: draft.setupHook,
      titlingStepVisible: provisioning.titlingStepVisible,
      titlingMode: provisioning.titlingMode,
      titlingRemoteValid: provisioning.titlingRemoteValid,
      titlingPersistError: provisioning.titlingPersistError,
    },
    ensureTitlingPersistedForCurrentTarget: provisioning.ensureTitlingPersistedForCurrentTarget,
    setSourcePath: setters.sourcePath,
    setImportRepoStatus: setters.importRepoStatus,
    setImportRepoNote: setters.importRepoNote,
    onOnboardingInsertionRequested: flow.goToStepKey,
    onCreateErrorStep: flow.goToStepKey,
    navigate,
    wizardCompletedRef,
    wizardKey,
    trackWizardCompleted,
    desktopApp: remote.desktopApp,
    parsedRemoteHost: remote.parsedRemote?.host,
    parsedRemoteUser: remote.parsedRemote?.user,
    remotePasswordOnce: remote.remotePasswordOnce,
    parsedRemotePort: remote.parsedRemotePort,
    remoteDataDirInput: remote.remoteDataDirInput,
    connectDaemonForImport: remote.connectDaemonForImport,
    ensureOnboardingAfterDaemonConnect: provisioning.ensureOnboardingAfterDaemonConnect,
    waitForDaemonReady: remote.waitForDaemonReady,
    applyConnection: remote.applyConnection,
    rememberRemoteProfile: remote.rememberRemoteProfile,
    setCreateError: setters.createError,
  });

  const onSelect = useCallback((stepKey: string, optionId: string) => {
    setters.createError(null);
    flow.selectOption(stepKey, optionId);
    if (stepKey === "location" && optionId === "local") {
      remote.resetForLocalSelection();
    }
    if (stepKey === "location") {
      flow.invalidateRoutePlan();
    }
    if (stepKey === "container" && optionId === "no-container") {
      setters.networkAllowlist("");
    }
    if (stepKey === "network" && optionId !== "allowlist") {
      setters.networkAllowlist("");
    }
    if (stepKey === "source") {
      if (optionId !== "clone") {
        setters.repoUrl("");
        setters.repoBranch("");
      }
      if (optionId !== "new") {
        setters.workspaceName("");
      }
      if (optionId !== "import") {
        setters.importRepoStatus("idle");
        setters.importRepoNote(null);
      }
    }
  }, [flow, remote, setters]);

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

  const onSkipAuthImport = useCallback(() => {
    void (async () => {
      const nextStep = await provisioning.advanceFromAuthImportStep({ clearSelections: true });
      if (nextStep) {
        flow.goToStepKey(nextStep);
      }
    })();
  }, [flow, provisioning]);

  const onSkipHarnessDownloads = useCallback(() => {
    void (async () => {
      if (flow.currentStepKeyRef.current === "harness-downloads") {
        flow.goToStepKey(nextAfterHarnessDownloads(flow.routePlan));
      }
      const nextStep = await provisioning.advanceFromHarnessDownloadsStep({ clearSelections: true });
      if (nextStep) {
        flow.goToStepKey(nextStep);
      }
    })();
  }, [flow, provisioning]);

  const onSelectTitlingLocal = useCallback(() => {
    const started = provisioning.onSelectTitlingLocal();
    if (started) {
      flow.goRelativeStep(1);
    }
  }, [flow, provisioning]);

  const onSkipTitling = useCallback(() => {
    provisioning.invalidateTitlingPersisted();
    provisioning.setTitlingMode("skip");
    if (flow.routePlan?.includeTitling) {
      flow.setRoutePlan({ ...flow.routePlan, includeTitling: false });
    }
    flow.goRelativeStep(1);
  }, [flow, provisioning]);

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
      const nextStep = await provisioning.advanceFromAuthImportStep();
      if (nextStep) {
        flow.goToStepKey(nextStep);
      }
      return;
    }
    if (flow.step.key === "harness-downloads") {
      if (provisioning.selectedHarnessReadyToStartCount === 0) {
        flow.goToStepKey(nextAfterHarnessDownloads(flow.routePlan));
        return;
      }
      const nextStep = await provisioning.advanceFromHarnessDownloadsStep();
      if (nextStep) {
        flow.goToStepKey(nextStep);
        return;
      }
      if (
        provisioning.selectedHarnessBlockedCount > 0
        && provisioning.selectedHarnessReadyToStartCount === 0
      ) {
        flow.goToStepKey(nextAfterHarnessDownloads(flow.routePlan));
      }
      return;
    }
    if (flow.step.key === "session-titling") {
      provisioning.setTitlingPersistError(null);
      if (provisioning.titlingMode === "skip") {
        if (flow.routePlan?.includeTitling) {
          flow.setRoutePlan({ ...flow.routePlan, includeTitling: false });
        }
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
      setters.createError(null);
      const preflightOk = await create.preflightSourceStep();
      if (!preflightOk) return;
      flow.goRelativeStep(1);
      return;
    }
    flow.goRelativeStep(1);
  }, [create, flow, provisioning, remote, setters]);

  useEffect(() => {
    if (draft.pushBranchTouched) return;
    if (!draft.targetBranch.trim()) return;
    setters.pushBranch(draft.targetBranch);
  }, [draft.pushBranchTouched, draft.targetBranch, setters]);

  useEffect(() => {
    if (flow.step.key !== "harness-downloads") return;
    if (provisioning.harnessInstallBusy) return;
    if (provisioning.harnessInstallError) return;
    if (provisioning.selectedHarnessReadyToStartCount > 0) return;
    if (provisioning.selectedHarnessRunningCount > 0) return;
    if (provisioning.selectedHarnessFailedCount === 0) return;
    flow.goToStepKey(nextAfterHarnessDownloads(flow.routePlan));
  }, [
    flow,
    provisioning.harnessInstallBusy,
    provisioning.harnessInstallError,
    provisioning.selectedHarnessFailedCount,
    provisioning.selectedHarnessReadyToStartCount,
    provisioning.selectedHarnessRunningCount,
  ]);

  const hasAllowlist = flow.step.key !== "network"
    || flow.selections.network !== "allowlist"
    || draft.networkAllowlist.split(/\r?\n/).map((line) => line.trim()).filter(Boolean).length > 0;
  const hasSourceStepInputs = flow.step.key !== "source" || flow.sourceStepValidation.isComplete;
  const hasTargetBranch = flow.step.key !== "merge-queue"
    || flow.mergeQueueSkipped
    || draft.targetBranch.trim() !== "";
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

  return {
    draft,
    setters,
    flow,
    remote,
    provisioning,
    create,
    canAdvance,
    nextButtonLabel,
    onSelect,
    onSelectOption,
    onSkipAuthImport,
    onSkipHarnessDownloads,
    onSelectTitlingLocal,
    onSkipTitling,
    onNext,
  };
}
