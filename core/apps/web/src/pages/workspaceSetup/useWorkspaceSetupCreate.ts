import { type MutableRefObject, useEffect, useRef, useState } from "react";
import type { ExecutionLaunchSnapshot } from "../../api/client";
import {
  createWorkspace,
  deleteWorkspace,
  idToString,
  listWorkspaces,
  repoClone,
  repoInit,
  repoStatus,
  repoStagingPath,
  repoValidateDestination,
  updateWorkspaceExecutionConfig,
  updateWorkspaceMergeQueueConfig,
  updateWorkspaceWorktreeBootstrapConfig,
} from "../../api/client";
import { desktopConnectLocal, desktopConnectSsh, desktopPickFolder } from "../../utils/desktop";
import { trackWorkspaceLaunchCompleted } from "../../utils/analytics";
import { upsertLauncherRecent } from "../../state/launcherRecentsStore";
import { deriveRepoNameFromUrl, parseCloneDestPath, resolveWorkspaceName } from "../WorkspaceSetupPage.logic";
import {
  currentLaunchStepLabel as deriveCurrentLaunchStepLabel,
  formatLaunchElapsed,
  formatLaunchRemaining,
  formatLaunchTime,
  launchElapsedMs,
  launchEtaRemainingMs,
  stabilizeLaunchEtaRemainingMs,
  workspaceSetupProvisioningPhaseLabel,
  workspaceSetupProvisioningRemainingMs,
  type WorkspaceSetupProvisioningExecutionMode,
  type WorkspaceSetupProvisioningPhase,
  type WorkspaceSetupProvisioningSource,
  type WorkspaceSetupLaunchLogLine,
} from "./launchProgress";
import type { WizardStepKey } from "./wizardFlow";
import { lastPathSegment, messageFromError, type ImportInitDialogState } from "./wizardTypes";
import { buildWorkspaceSetupCreateIntent, parseNetworkAllowlist, resolveCreateErrorStepKey, type WorkspaceSetupCreateIntent } from "./createHandoff";
import { waitForWorkspaceBootstrapBeforeNavigation } from "../workspaceBootstrapGate";
import type {
  RoutePlanInsertionStep,
  WorkspaceSetupEffectiveTarget,
} from "./workflowTypes";
import {
  copyWorkspaceSetupLaunchDiagnostics,
  prepareWorkspaceSetupSandboxRuntime,
  waitForWorkspaceSetupLaunchCompletion,
} from "./workspaceSetupLaunchHelpers";

type UseWorkspaceSetupCreateArgs = {
  currentStepKey: WizardStepKey;
  intent: WorkspaceSetupCreateIntent;
  ensureTitlingPersistedForCurrentTarget: () => Promise<boolean>;
  setSourcePath: (value: string) => void;
  setImportRepoStatus: (status: "idle" | "checking" | "ok" | "error") => void;
  setImportRepoNote: (note: string | null) => void;
  onOnboardingInsertionRequested: (stepKey: RoutePlanInsertionStep) => void;
  onCreateErrorStep: (stepKey: WizardStepKey) => void;
  navigate: (path: string, opts: { replace: boolean }) => void;
  wizardCompletedRef: MutableRefObject<boolean>;
  wizardKey: string;
  trackWizardCompleted: (payload: { wizardKey: string; workspaceKind: string }) => void;
  desktopApp: boolean;
  remoteSshPasswordOnce: string | null;
  remoteSshPasswordCandidate: string | null;
  remoteAdminPasswordOnce: string | null;
  remoteAdminPasswordCandidate: string | null;
  effectiveTarget: WorkspaceSetupEffectiveTarget | null;
  connectDaemonForImport: (locationOverride?: "local" | "remote") => Promise<void>;
  ensureOnboardingAfterDaemonConnect: (options?: { allowTitlingInsertion?: boolean }) => Promise<{
    insertionStep: RoutePlanInsertionStep | null;
  } | null>;
  waitForDaemonReady: (timeoutMs: number) => Promise<void>;
  applyConnection: (info: Awaited<ReturnType<typeof desktopConnectLocal>>) => void;
  rememberRemoteProfile: (host: string, user: string | null) => void;
  requestRemotePasswordPrompt: (mode?: "ssh" | "admin") => void;
  setCreateError: (message: string | null) => void;
};

type WorkspaceProvisioningState = {
  phase: WorkspaceSetupProvisioningPhase;
  stepLabel: string;
  startedAtMs: number;
  phaseStartedAtMs: number;
  updatedAtMs: number;
  state: "running" | "ready" | "error";
  error: string | null;
  workspaceId: string | null;
};

const SYNTHETIC_WORKSPACE_SETUP_JOB_ID = "workspace-setup-provisioning";

export function useWorkspaceSetupCreate({
  currentStepKey,
  intent,
  ensureTitlingPersistedForCurrentTarget,
  setSourcePath,
  setImportRepoStatus,
  setImportRepoNote,
  onOnboardingInsertionRequested,
  onCreateErrorStep,
  navigate,
  wizardCompletedRef,
  wizardKey,
  trackWizardCompleted,
  desktopApp,
  remoteSshPasswordOnce,
  remoteSshPasswordCandidate,
  remoteAdminPasswordOnce,
  remoteAdminPasswordCandidate,
  effectiveTarget,
  connectDaemonForImport,
  ensureOnboardingAfterDaemonConnect,
  waitForDaemonReady,
  applyConnection,
  rememberRemoteProfile,
  requestRemotePasswordPrompt,
  setCreateError,
}: UseWorkspaceSetupCreateArgs) {
  const [creating, setCreating] = useState(false);
  const [localAdminPasswordPromptVisible, setLocalAdminPasswordPromptVisible] = useState(false);
  const [localAdminPasswordInput, setLocalAdminPasswordInput] = useState("");
  const [sandboxPrepareMessage, setSandboxPrepareMessage] = useState<string | null>(null);
  const [launchSnapshot, setLaunchSnapshot] = useState<ExecutionLaunchSnapshot | null>(null);
  const [launchLogs, setLaunchLogs] = useState<WorkspaceSetupLaunchLogLine[]>([]);
  const [provisioningState, setProvisioningState] = useState<WorkspaceProvisioningState | null>(null);
  const [launchEtaDisplayMs, setLaunchEtaDisplayMs] = useState<number | null>(null);
  const [launchTick, setLaunchTick] = useState(0);
  const [launchCopyState, setLaunchCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [importInitDialog, setImportInitDialog] = useState<ImportInitDialogState | null>(null);
  const importInitResolveRef = useRef<((confirmed: boolean) => void) | null>(null);
  const provisioningStateRef = useRef<WorkspaceProvisioningState | null>(null);
  const syntheticLogSeqRef = useRef(-1);
  const launchEtaDisplayRef = useRef<{
    nowMs: number | null;
    rawRemainingMs: number | null;
    recalibrationTargetMs: number | null;
    remainingMs: number | null;
  }>({
    nowMs: null,
    rawRemainingMs: null,
    recalibrationTargetMs: null,
    remainingMs: null,
  });
  const createIntent = buildWorkspaceSetupCreateIntent(intent);
  const {
    selections,
    sourcePath,
    repoUrl,
    repoBranch,
    workspaceName,
    networkAllowlist,
    useSandboxStaging,
    targetBranch,
    verifyCommand,
    mergeQueueSkipped,
    pushOnSuccess,
    pushRemote,
    pushBranch,
    setupHook,
    titlingStepVisible,
    titlingMode,
    titlingRemoteValid,
    titlingPersistError,
  } = createIntent;
  const remoteTarget = effectiveTarget?.kind === "remote" ? effectiveTarget : null;
  const parsedRemoteHost = remoteTarget?.host;
  const parsedRemoteUser = remoteTarget?.user;
  const parsedRemotePort = remoteTarget?.port ?? null;
  const remoteDataDir = remoteTarget?.dataDir ?? null;
  const provisioningSource = selections.source as WorkspaceSetupProvisioningSource;
  const provisioningExecutionMode =
    (selections.container === "host" ? "host" : "sandbox") satisfies WorkspaceSetupProvisioningExecutionMode;
  const localAdminPasswordOnce = localAdminPasswordInput.length > 0 ? localAdminPasswordInput : null;

  useEffect(() => {
    if (selections.location === "local") {
      return;
    }
    setLocalAdminPasswordPromptVisible(false);
    setLocalAdminPasswordInput("");
  }, [selections.location]);

  useEffect(() => {
    provisioningStateRef.current = provisioningState;
  }, [provisioningState]);

  const buildSyntheticLaunchSnapshot = (
    state: WorkspaceProvisioningState | null,
  ): ExecutionLaunchSnapshot | null => {
    if (!state) return null;
    const startedAt = new Date(state.startedAtMs).toISOString();
    return {
      job_id: SYNTHETIC_WORKSPACE_SETUP_JOB_ID,
      workspace_id: state.workspaceId ?? "pending",
      kind: "workspace_launch",
      state: state.state,
      created_at: startedAt,
      started_at: startedAt,
      updated_at: new Date(state.updatedAtMs).toISOString(),
      finished_at: state.state === "running" ? null : new Date(state.updatedAtMs).toISOString(),
      current_phase: null,
      current_step_label: state.stepLabel,
      progress_pct: null,
      eta_ms: null,
      active_download: null,
      phases: [],
      logs: [],
      error: state.error,
    };
  };

  const appendSyntheticLaunchLog = (
    phase: WorkspaceSetupProvisioningPhase,
    message: string,
    level: "info" | "warn" | "error" = "info",
  ) => {
    const ts = new Date().toISOString();
    const seq = syntheticLogSeqRef.current;
    syntheticLogSeqRef.current -= 1;
    setLaunchLogs((prev) => prev.concat({
      seq,
      ts,
      phase: "machine_check",
      level,
      message,
      phaseLabel: workspaceSetupProvisioningPhaseLabel(phase),
      provisioningPhase: phase,
      timeLabel: formatLaunchTime(ts),
    }).slice(-400));
  };

  const beginProvisioningPhase = (
    phase: WorkspaceSetupProvisioningPhase,
    stepLabel: string,
    message: string,
    workspaceId?: string | null,
  ) => {
    const nowMs = Date.now();
    const startedAtMs = provisioningStateRef.current?.startedAtMs ?? nowMs;
    const nextState: WorkspaceProvisioningState = {
      phase,
      stepLabel,
      startedAtMs,
      phaseStartedAtMs: nowMs,
      updatedAtMs: nowMs,
      state: "running",
      error: null,
      workspaceId: workspaceId ?? provisioningStateRef.current?.workspaceId ?? null,
    };
    provisioningStateRef.current = nextState;
    setProvisioningState(nextState);
    appendSyntheticLaunchLog(phase, message);
  };

  const clearProvisioningState = () => {
    provisioningStateRef.current = null;
    setProvisioningState(null);
  };

  const markProvisioningError = (message: string) => {
    const current = provisioningStateRef.current;
    if (!current) return;
    const nowMs = Date.now();
    const nextState: WorkspaceProvisioningState = {
      ...current,
      updatedAtMs: nowMs,
      state: "error",
      error: message,
    };
    provisioningStateRef.current = nextState;
    setProvisioningState(nextState);
    appendSyntheticLaunchLog(current.phase, message, "error");
  };

  const syntheticLaunchSnapshot = buildSyntheticLaunchSnapshot(provisioningState);
  const effectiveLaunchSnapshot =
    provisioningState?.state === "running"
      ? syntheticLaunchSnapshot
      : launchSnapshot ?? syntheticLaunchSnapshot;

  useEffect(() => {
    if (!creating || !effectiveLaunchSnapshot || effectiveLaunchSnapshot.state !== "running") return;
    const handle = window.setInterval(() => setLaunchTick((value) => value + 1), 1000);
    return () => window.clearInterval(handle);
  }, [
    creating,
    launchSnapshot?.job_id,
    launchSnapshot?.state,
    provisioningState?.phase,
    provisioningState?.state,
  ]);

  useEffect(() => {
    if (
      provisioningState?.state !== "running"
      || provisioningState.phase !== "launch_runtime"
      || !launchSnapshot
    ) {
      return;
    }
    clearProvisioningState();
  }, [
    launchSnapshot,
    provisioningState?.phase,
    provisioningState?.state,
  ]);

  useEffect(() => {
    const nowMs = Date.now();
    if (!creating || !effectiveLaunchSnapshot) {
      launchEtaDisplayRef.current = {
        nowMs: null,
        rawRemainingMs: null,
        recalibrationTargetMs: null,
        remainingMs: null,
      };
      setLaunchEtaDisplayMs(null);
      return;
    }
    if (effectiveLaunchSnapshot.state === "ready") {
      launchEtaDisplayRef.current = {
        nowMs,
        rawRemainingMs: 0,
        recalibrationTargetMs: null,
        remainingMs: 0,
      };
      setLaunchEtaDisplayMs(0);
      return;
    }
    if (effectiveLaunchSnapshot.state === "error") {
      launchEtaDisplayRef.current = {
        nowMs,
        rawRemainingMs: null,
        recalibrationTargetMs: null,
        remainingMs: null,
      };
      setLaunchEtaDisplayMs(null);
      return;
    }
    const rawRemainingMs = provisioningState?.state === "running"
      ? workspaceSetupProvisioningRemainingMs({
        phase: provisioningState.phase,
        source: provisioningSource,
        executionMode: provisioningExecutionMode,
        phaseStartedAtMs: provisioningState.phaseStartedAtMs,
        nowMs,
      })
      : launchEtaRemainingMs(launchSnapshot, nowMs);
    const nextRemainingMs = stabilizeLaunchEtaRemainingMs({
      previousRemainingMs: launchEtaDisplayRef.current.remainingMs,
      previousRawRemainingMs: launchEtaDisplayRef.current.rawRemainingMs,
      previousRecalibrationTargetMs: launchEtaDisplayRef.current.recalibrationTargetMs,
      previousNowMs: launchEtaDisplayRef.current.nowMs,
      nowMs,
      rawRemainingMs,
    });
    launchEtaDisplayRef.current = {
      nowMs,
      rawRemainingMs,
      recalibrationTargetMs: nextRemainingMs.recalibrationTargetMs,
      remainingMs: nextRemainingMs.remainingMs,
    };
    setLaunchEtaDisplayMs(nextRemainingMs.remainingMs);
  }, [
    creating,
    launchSnapshot?.job_id,
    launchSnapshot?.state,
    launchSnapshot?.updated_at,
    launchSnapshot?.current_phase,
    launchSnapshot?.current_step_label,
    provisioningExecutionMode,
    provisioningSource,
    provisioningState?.phase,
    provisioningState?.phaseStartedAtMs,
    provisioningState?.state,
    provisioningState?.stepLabel,
    provisioningState?.updatedAtMs,
    provisioningState?.workspaceId,
    launchTick,
  ]);

  useEffect(() => () => {
    const resolve = importInitResolveRef.current;
    importInitResolveRef.current = null;
    resolve?.(false);
  }, []);

  const onPickLocalFolder = async () => {
    if (!desktopApp) return;
    try {
      const picked = await desktopPickFolder();
      if (picked) {
        setCreateError(null);
        setSourcePath(picked);
      }
    } catch {
      // ignore
    }
  };

  const resolveImportInitDialog = (confirmed: boolean) => {
    const resolve = importInitResolveRef.current;
    importInitResolveRef.current = null;
    setImportInitDialog(null);
    resolve?.(confirmed);
  };

  const confirmInitImportFolder = (path: string) =>
    new Promise<boolean>((resolve) => {
      if (importInitResolveRef.current) {
        importInitResolveRef.current(false);
      }
      importInitResolveRef.current = resolve;
      setImportInitDialog({ path });
    });

  const preflightSourceStep = async (): Promise<boolean> => {
    if (currentStepKey !== "source") return true;

    if (desktopApp && selections.location === "local") {
      try {
        await connectDaemonForImport();
        const onboardingResult = await ensureOnboardingAfterDaemonConnect({ allowTitlingInsertion: true });
        if (onboardingResult?.insertionStep) {
          onOnboardingInsertionRequested(onboardingResult.insertionStep);
          return false;
        }
      } catch (error) {
        setCreateError(messageFromError(error));
        return false;
      }
    }

    if (selections.source === "clone" && !useSandboxStaging) {
      const dest = parseCloneDestPath(sourcePath);
      if (!dest) {
        setCreateError("Destination must be an absolute path (e.g. /Users/example-user/projects/ or /Users/example-user/projects/repo-name).");
        return false;
      }
      const destName = dest.dest_name ?? deriveRepoNameFromUrl(repoUrl);
      if (!destName) {
        setCreateError("Could not derive repo name from URL.");
        return false;
      }
      const normalizedParent = dest.dest_parent.replace(/\/+$/, "") || "/";
      const fullDestPath = normalizedParent === "/" ? `/${destName}` : `${normalizedParent}/${destName}`;
      if (selections.location === "remote") {
        return true;
      }
      try {
        await repoValidateDestination({ path: fullDestPath, must_not_exist: true });
      } catch (error) {
        setCreateError(messageFromError(error));
        return false;
      }
      return true;
    }

    if (selections.source === "new" && !useSandboxStaging) {
      const destPath = sourcePath.trim().replace(/\/+$/, "");
      if (!destPath) {
        setCreateError("Destination folder is required.");
        return false;
      }
      if (selections.location === "remote") {
        return true;
      }
      try {
        await repoValidateDestination({
          path: destPath,
          require_empty_if_exists: true,
        });
      } catch (error) {
        setCreateError(messageFromError(error));
        return false;
      }
      return true;
    }

    if (selections.source === "import") {
      const rootPath = sourcePath.trim().replace(/\/+$/, "");
      if (!rootPath) {
        setCreateError("Folder is required.");
        return false;
      }
      if (selections.location === "remote") {
        setImportRepoStatus("ok");
        setImportRepoNote("Remote folder checks run during Create so the remote daemon stays cold until launch.");
        return true;
      }
      setImportRepoStatus("checking");
      setImportRepoNote(null);
      try {
        const status = await repoStatus({ path: rootPath });
        if (status.is_repo) {
          setImportRepoStatus("ok");
          setImportRepoNote(null);
          return true;
        }
        const detail = String(status.error ?? "").trim();
        const note = detail
          ? `Not a git repo yet (${detail}). We'll offer to initialize it during Create.`
          : "Not a git repo yet. We'll offer to initialize it during Create.";
        setImportRepoStatus("ok");
        setImportRepoNote(note);
        return true;
      } catch (error) {
        const message = `Could not verify the selected folder. ${messageFromError(error)}`;
        setImportRepoStatus("error");
        setImportRepoNote(message);
        setCreateError(message);
        return false;
      }
    }

    return true;
  };

  const prepareSandboxRuntimeIfNeeded = async () => {
    await prepareWorkspaceSetupSandboxRuntime({
      activationMode: selections.location === "remote" ? "remote" : "local",
      containerSelection: selections.container === "host" ? "host" : "sandbox",
      desktopApp,
      localAdminPasswordOnce,
      remoteAdminPasswordOnce,
      remoteAdminPasswordCandidate,
      requestRemoteAdminPasswordPrompt: () => requestRemotePasswordPrompt("admin"),
      setSandboxPrepareMessage,
      onLocalAdminPasswordRequired: () => {
        setLocalAdminPasswordPromptVisible(true);
        setLocalAdminPasswordInput("");
      },
      onLocalAdminPasswordReady: () => {
        setLocalAdminPasswordPromptVisible(false);
        setLocalAdminPasswordInput("");
      },
      onCreateErrorStep: (stepKey) => onCreateErrorStep(stepKey),
    });
  };

  const onCopyLaunchDiagnostics = async () => {
    await copyWorkspaceSetupLaunchDiagnostics(
      effectiveLaunchSnapshot,
      launchLogs,
      setLaunchCopyState,
    );
  };

  const onCreate = async () => {
    setCreateError(null);
    setLaunchSnapshot(null);
    setLaunchLogs([]);
    clearProvisioningState();
    launchEtaDisplayRef.current = {
      nowMs: null,
      rawRemainingMs: null,
      recalibrationTargetMs: null,
      remainingMs: null,
    };
    setLaunchEtaDisplayMs(null);
    setLaunchCopyState("idle");
    setLaunchTick(0);
    syntheticLogSeqRef.current = -1;
    setSandboxPrepareMessage(null);
    setCreating(true);
    const launchStartedAtMs = Date.now();
    let createdWorkspaceId: string | null = null;
    let shouldCleanupCreatedWorkspace = false;
    try {
      beginProvisioningPhase(
        "connect_daemon",
        selections.location === "remote" ? "Connecting to remote daemon" : "Connecting to local daemon",
        selections.location === "remote"
          ? "Connecting to the selected remote daemon."
          : "Connecting to the local daemon.",
      );
      if (selections.location === "remote" && !desktopApp) {
        throw new Error("Workspace creation from the wizard requires the desktop app.");
      }

      if (selections.location === "remote" && !parsedRemoteHost) {
        throw new Error("Remote host is required (user@host).");
      }

      if (desktopApp) {
        const info = selections.location === "remote"
          ? await desktopConnectSsh({
            host: parsedRemoteHost!,
            user: parsedRemoteUser ?? null,
            password_once: remoteSshPasswordOnce ?? remoteSshPasswordCandidate,
            remote_port: parsedRemotePort,
            start_remote: true,
            remote_data_dir: remoteDataDir,
          })
          : await desktopConnectLocal();
        applyConnection(info);
      }

      if (selections.location === "remote" && parsedRemoteHost) {
        rememberRemoteProfile(parsedRemoteHost, parsedRemoteUser ?? null);
      }

      await waitForDaemonReady(15000);
      await prepareSandboxRuntimeIfNeeded();
      beginProvisioningPhase(
        "prepare_source",
        "Preparing workspace source",
        "Daemon ready. Preparing workspace source.",
      );
      const onboardingResult = await ensureOnboardingAfterDaemonConnect({
        allowTitlingInsertion: selections.location === "remote",
      });
      const blockingInsertionStep = onboardingResult?.insertionStep === "harness-downloads"
        ? null
        : onboardingResult?.insertionStep;
      if (blockingInsertionStep) {
        // Harness downloads remain optional even when a freshly connected remote daemon reports
        // missing installs. Once the user has committed Create, keep the workspace launch moving
        // and let the remote daemon surface optional downloads separately.
        onOnboardingInsertionRequested(blockingInsertionStep);
        return;
      }
      if (titlingStepVisible && titlingMode !== "skip") {
        if (titlingMode !== "remote" && titlingMode !== "local") {
          throw new Error("Choose a session titling option or skip for now.");
        }
        if (titlingMode === "remote" && !titlingRemoteValid) {
          throw new Error("Remote session titling requires base URL, API key, and model.");
        }
        const persisted = await ensureTitlingPersistedForCurrentTarget();
        if (!persisted) {
          throw new Error(titlingPersistError ?? "Failed to save session titling settings.");
        }
      }

      let allWorkspaces: Awaited<ReturnType<typeof listWorkspaces>> | null = null;
      const getAllWorkspaces = async () => {
        if (!allWorkspaces) {
          allWorkspaces = await listWorkspaces();
        }
        return allWorkspaces;
      };
      const getExistingWorkspaceNamesForGenerated = async () => {
        try {
          const all = await getAllWorkspaces();
          return all
            .map((workspace) => String(workspace.name ?? "").trim())
            .filter(Boolean);
        } catch {
          return [];
        }
      };

      let rootPath = "";
      let name: string | undefined;
      let workspaceId = "";
      if (selections.source === "import") {
        beginProvisioningPhase(
          "import_repo",
          "Checking import source",
          "Checking the selected folder before importing it as a workspace.",
        );
        rootPath = sourcePath.trim().replace(/\/+$/, "");
        if (!rootPath) throw new Error("Folder is required.");
        let status = await repoStatus({ path: rootPath }).catch(() => null);
        if (!status) {
          setImportRepoStatus("error");
          setImportRepoNote("Could not verify the selected folder. Check the path and try again.");
          throw new Error("Could not verify the selected folder as a repository.");
        }
        if (status && !status.is_repo) {
          const confirmed = await confirmInitImportFolder(rootPath);
          if (!confirmed) {
            setImportRepoStatus("error");
            setImportRepoNote("Folder is not a repo. Initialization cancelled.");
            throw new Error("Selected folder is not a repo.");
          }
          beginProvisioningPhase(
            "init_repo",
            "Initializing Git repository",
            "Initializing a new Git repository in the selected folder.",
          );
          setImportRepoStatus("checking");
          setImportRepoNote("Initializing Git repo in selected folder…");
          const init = await repoInit({ path: rootPath, allow_existing: true, allow_non_empty: true });
          rootPath = String(init.path ?? "").trim() || rootPath;
          status = await repoStatus({ path: rootPath });
          if (!status.is_repo) {
            const detailAfter = String(status.error ?? "").trim();
            throw new Error(detailAfter ? `Selected folder is not a repo: ${detailAfter}` : "Selected folder is not a repo.");
          }
        }
        if (status?.canonical_path) {
          rootPath = String(status.canonical_path).trim() || rootPath;
        }
        setImportRepoStatus("ok");
        setImportRepoNote(null);
        const all = await getAllWorkspaces();
        const hit = all.find((workspace) => String(workspace.root_path) === rootPath);
        if (hit) {
          workspaceId = idToString(hit.id);
        } else {
          name = workspaceName.trim() || undefined;
        }
      } else if (selections.source === "clone") {
        beginProvisioningPhase(
          "prepare_source",
          "Preparing clone destination",
          useSandboxStaging
            ? "Allocating sandbox staging before cloning the repository."
            : "Preparing the destination folder for the repository clone.",
        );
        let destParent: string;
        let destName: string | null;
        if (useSandboxStaging) {
          const staging = await repoStagingPath();
          destParent = staging.path;
          destName = deriveRepoNameFromUrl(repoUrl) || workspaceName.trim() || null;
          if (!destName) throw new Error("Could not derive repo name from URL.");
        } else {
          const dest = parseCloneDestPath(sourcePath);
          if (!dest) throw new Error("Destination must be an absolute path (e.g. /Users/example-user/projects/ or /Users/example-user/projects/repo-name).");
          destParent = dest.dest_parent;
          destName = dest.dest_name ?? null;
        }
        beginProvisioningPhase(
          "clone_repo",
          "Cloning repository",
          `Cloning ${repoUrl.trim()}${repoBranch.trim() ? ` (${repoBranch.trim()})` : ""}.`,
        );
        const response = await repoClone({
          repo_url: repoUrl.trim(),
          branch: repoBranch.trim() || null,
          dest_parent: destParent,
          dest_name: destName,
        });
        rootPath = response.path;
        const existingWorkspaceNames = await getExistingWorkspaceNamesForGenerated();
        name = resolveWorkspaceName({
          source: selections.source,
          workspaceName,
          repoUrl,
          destPath: useSandboxStaging ? null : sourcePath,
          useSandboxStaging,
          existingWorkspaceNames,
        });
      } else if (selections.source === "new") {
        beginProvisioningPhase(
          "init_repo",
          "Initializing repository",
          useSandboxStaging
            ? "Allocating sandbox staging before creating the new repository."
            : "Initializing a new Git repository for the workspace.",
        );
        let destPath: string;
        if (useSandboxStaging) {
          const staging = await repoStagingPath();
          destPath = staging.path;
        } else {
          destPath = sourcePath.trim().replace(/\/+$/, "");
          if (!destPath) throw new Error("Destination folder is required.");
        }
        const init = await repoInit({ path: destPath, allow_existing: true });
        rootPath = String(init.path ?? "").trim() || destPath;
        const existingWorkspaceNames = await getExistingWorkspaceNamesForGenerated();
        name = resolveWorkspaceName({
          source: selections.source,
          workspaceName,
          repoUrl,
          destPath,
          useSandboxStaging,
          existingWorkspaceNames,
        });
      } else {
        throw new Error("Choose a source option.");
      }

      const workspaceKind = selections.location === "remote" ? "remote" : "local";
      const executionMode = selections.container === "host" ? "host" : "sandbox";
      let pendingRouteLaunch: {
        workspaceId: string;
        workspaceKind: "local" | "remote";
        executionMode: "host" | "sandbox";
        source: "wizard";
        startedAtMs: number;
      } | null = null;
      if (!workspaceId) {
        beginProvisioningPhase(
          "register_workspace",
          "Registering workspace",
          "Registering the workspace with the daemon.",
        );
        const created = await createWorkspace(rootPath, name, workspaceKind, "wizard");
        workspaceId = idToString(created.id);
        createdWorkspaceId = workspaceId;
        shouldCleanupCreatedWorkspace = true;
        setProvisioningState((current) => current ? { ...current, workspaceId } : current);
      }

      beginProvisioningPhase(
        "configure_workspace",
        "Configuring workspace runtime",
        "Saving workspace runtime settings.",
        workspaceId,
      );
      const environment = selections.container === "host"
        ? "host"
        : "sandbox";
      const allowlist = parseNetworkAllowlist(networkAllowlist);
      const netMode = selections.network === "allowlist"
        ? "allowlist"
        : selections.network === "full"
          ? "all"
          : "llm_only";
      await updateWorkspaceExecutionConfig(workspaceId, {
        environment,
        network_mode: selections.container !== "host" ? netMode : null,
          allowlist: selections.container !== "host" && netMode === "allowlist" ? allowlist : null,
      });

      if (selections.container !== "host") {
        appendSyntheticLaunchLog(
          "launch_runtime",
          "Handing off to sandbox launch and waiting for runtime readiness.",
        );
        try {
          await waitForWorkspaceSetupLaunchCompletion(
            workspaceId,
            setLaunchSnapshot,
            setLaunchLogs,
          );
          trackWorkspaceLaunchCompleted({
            workspaceId,
            workspaceKind,
            executionMode,
            source: "wizard",
            startedAtMs: launchStartedAtMs,
            result: "ready",
            persistPendingRoute: false,
          });
          pendingRouteLaunch = {
            workspaceId,
            workspaceKind,
            executionMode,
            source: "wizard",
            startedAtMs: launchStartedAtMs,
          };
        } catch (error) {
          trackWorkspaceLaunchCompleted({
            workspaceId,
            workspaceKind,
            executionMode,
            source: "wizard",
            startedAtMs: launchStartedAtMs,
            result: "error",
          });
          throw error;
        }
      }

      beginProvisioningPhase(
        "bootstrap_workspace",
        "Finalizing workspace setup",
        "Finalizing workspace setup before opening it.",
        workspaceId,
      );

      if (!mergeQueueSkipped) {
        await updateWorkspaceMergeQueueConfig(workspaceId, {
          enabled: true,
          target_branch: targetBranch.trim() || null,
          verify_command: verifyCommand.trim() || null,
          push_on_success: pushOnSuccess ? true : null,
          push_remote: pushOnSuccess ? (pushRemote.trim() || "origin") : null,
          push_branch: pushOnSuccess ? (pushBranch.trim() || targetBranch.trim() || "main") : null,
        });
      }

      if (setupHook.trim()) {
        await updateWorkspaceWorktreeBootstrapConfig(workspaceId, { setup_command: setupHook.trim() });
      }

      await waitForDaemonReady(15000);
      try {
        if (selections.location === "remote" && parsedRemoteHost) {
          await upsertLauncherRecent({
            kind: "ssh",
            label: workspaceName.trim() || lastPathSegment(rootPath) || parsedRemoteHost,
            host: parsedRemoteHost,
            user: parsedRemoteUser ?? null,
            remote_port: parsedRemotePort ?? 4399,
            start_remote: true,
            remote_data_dir: remoteDataDir,
            workspace_root_path: rootPath,
            execution_environment: environment,
            updated_at_ms: Date.now(),
          });
        } else {
          await upsertLauncherRecent({
            kind: "local",
            label: lastPathSegment(rootPath) || rootPath,
            root_path: rootPath,
            execution_environment: environment,
            updated_at_ms: Date.now(),
          });
        }
      } catch {
        // best-effort only; do not block workspace creation if recents persistence fails
      }
      // The workspace now exists and has been fully configured. If provider bootstrap
      // fails while gating navigation, preserve the created workspace for recovery.
      shouldCleanupCreatedWorkspace = false;
      await waitForWorkspaceBootstrapBeforeNavigation(workspaceId);
      setProvisioningState((current) => current ? {
        ...current,
        updatedAtMs: Date.now(),
        state: "ready",
      } : current);
      if (pendingRouteLaunch) {
        trackWorkspaceLaunchCompleted({
          ...pendingRouteLaunch,
          result: "ready",
          emitEvent: false,
        });
      }
      shouldCleanupCreatedWorkspace = false;
      wizardCompletedRef.current = true;
      trackWizardCompleted({
        wizardKey,
        workspaceKind,
      });
      navigate(`/workspaces/${workspaceId}`, { replace: true });
    } catch (error) {
      if (shouldCleanupCreatedWorkspace && createdWorkspaceId) {
        try {
          await deleteWorkspace(createdWorkspaceId);
        } catch {
          // Best-effort rollback only; preserve the original create failure.
        }
      }
      const message = messageFromError(error);
      markProvisioningError(message);
      setCreateError(message);
      const key = resolveCreateErrorStepKey(message);
      if (key) onCreateErrorStep(key);
    } finally {
      setSandboxPrepareMessage(null);
      setCreating(false);
    }
  };

  const currentLaunchElapsed = (() => {
    return formatLaunchElapsed(launchElapsedMs(effectiveLaunchSnapshot, Date.now()));
  })();
  const currentLaunchStepLabel = deriveCurrentLaunchStepLabel(effectiveLaunchSnapshot);
  const currentLaunchEtaLabel = effectiveLaunchSnapshot?.state === "ready"
    ? "Ready"
    : effectiveLaunchSnapshot?.state === "error"
      ? "Launch failed"
      : formatLaunchRemaining(launchEtaDisplayMs);

  return {
    creating,
    localAdminPasswordPromptVisible,
    localAdminPasswordInput,
    setLocalAdminPasswordInput,
    setCreateError,
    importInitDialog,
    resolveImportInitDialog,
    onPickLocalFolder,
    preflightSourceStep,
    launchSnapshot: effectiveLaunchSnapshot,
    launchLogs,
    showLaunchPanel: Boolean(effectiveLaunchSnapshot) && (creating || effectiveLaunchSnapshot?.state === "error"),
    currentLaunchStepLabel,
    currentLaunchElapsed,
    currentLaunchEtaLabel,
    launchCopyLabel: launchCopyState === "copied"
      ? "Copied"
      : launchCopyState === "failed"
        ? "Copy failed"
        : "Copy diagnostics",
    createButtonLabel: creating
      ? (sandboxPrepareMessage ?? "Creating…")
      : "Create workspace",
    onCopyLaunchDiagnostics,
    onCreate,
  };
}
