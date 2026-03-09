import { useEffect, useRef, useState, type MutableRefObject } from "react";
import type {
  ExecutionLaunchLogLine,
  ExecutionLaunchSnapshot,
} from "../../api/client";
import {
  createWorkspace,
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
import { upsertLauncherRecent } from "../../state/launcherRecentsStore";
import {
  deriveRepoNameFromUrl,
  parseCloneDestPath,
  resolveWorkspaceName,
} from "../WorkspaceSetupPage.logic";
import {
  formatLaunchElapsed,
  parseUtcMs,
  phaseEntryForCurrent,
} from "./launchProgress";
import type { WizardStepKey } from "./wizardFlow";
import {
  lastPathSegment,
  messageFromError,
  type ImportInitDialogState,
} from "./wizardTypes";
import {
  buildWorkspaceSetupCreateIntent,
  parseNetworkAllowlist,
  resolveCreateErrorStepKey,
  type WorkspaceSetupCreateIntent,
} from "./createHandoff";
import {
  mergeWorkspaceSetupLaunchLogs,
  startWorkspaceSetupLaunchHandoff,
  startWorkspaceSetupRuntimePrewarm,
  waitForLaunchHandoffTerminal,
} from "./launchHandoff";
import type { RoutePlanInsertionStep } from "./workflowTypes";

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
  parsedRemoteHost: string | undefined;
  parsedRemoteUser: string | null | undefined;
  remotePasswordOnce: string | null;
  parsedRemotePort: number | null;
  remoteDataDirInput: string;
  connectDaemonForImport: (locationOverride?: "local" | "remote") => Promise<void>;
  ensureOnboardingAfterDaemonConnect: (options?: { allowTitlingInsertion?: boolean }) => Promise<{
    insertionStep: RoutePlanInsertionStep | null;
  } | null>;
  waitForDaemonReady: (timeoutMs: number) => Promise<void>;
  applyConnection: (info: Awaited<ReturnType<typeof desktopConnectLocal>>) => void;
  rememberRemoteProfile: (host: string, user: string | null) => void;
  setCreateError: (message: string | null) => void;
};

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
  parsedRemoteHost,
  parsedRemoteUser,
  remotePasswordOnce,
  parsedRemotePort,
  remoteDataDirInput,
  connectDaemonForImport,
  ensureOnboardingAfterDaemonConnect,
  waitForDaemonReady,
  applyConnection,
  rememberRemoteProfile,
  setCreateError,
}: UseWorkspaceSetupCreateArgs) {
  const [creating, setCreating] = useState(false);
  const [launchSnapshot, setLaunchSnapshot] = useState<ExecutionLaunchSnapshot | null>(null);
  const [launchLogs, setLaunchLogs] = useState<ExecutionLaunchLogLine[]>([]);
  const [launchTick, setLaunchTick] = useState(0);
  const [launchCopyState, setLaunchCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [importInitDialog, setImportInitDialog] = useState<ImportInitDialogState | null>(null);
  const importInitResolveRef = useRef<((confirmed: boolean) => void) | null>(null);
  const createIntent = buildWorkspaceSetupCreateIntent(intent);
  const {
    selections,
    sourcePath,
    repoUrl,
    repoBranch,
    workspaceName,
    networkAllowlist,
    useDiskIsolatedStaging,
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

  useEffect(() => {
    if (!creating || !launchSnapshot || launchSnapshot.state !== "running") return;
    const handle = window.setInterval(() => setLaunchTick((value) => value + 1), 1000);
    return () => window.clearInterval(handle);
  }, [creating, launchSnapshot?.job_id, launchSnapshot?.state]);

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

    if (selections.source === "clone" && !useDiskIsolatedStaging) {
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

    if (selections.source === "new" && !useDiskIsolatedStaging) {
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

  const applyLaunchSnapshot = (snapshot: ExecutionLaunchSnapshot) => {
    setLaunchSnapshot(snapshot);
    setLaunchLogs((prev) => mergeWorkspaceSetupLaunchLogs(prev, snapshot.logs ?? []));
  };

  const appendLaunchLine = (line: ExecutionLaunchLogLine) => {
    setLaunchLogs((prev) => mergeWorkspaceSetupLaunchLogs(prev, [line]));
  };

  const waitForLaunchTerminal = async (initial: ExecutionLaunchSnapshot) => {
    await waitForLaunchHandoffTerminal(initial, {
      applySnapshot: applyLaunchSnapshot,
      appendLine: appendLaunchLine,
    });
  };

  const waitForLaunchCompletion = async (workspaceId: string) => {
    const initial = await startWorkspaceSetupLaunchHandoff(workspaceId);
    await waitForLaunchTerminal(initial);
  };

  const waitForRuntimePrewarm = async () => {
    const initial = await startWorkspaceSetupRuntimePrewarm();
    await waitForLaunchTerminal(initial);
  };

  const onCopyLaunchDiagnostics = async () => {
    if (!launchSnapshot) return;
    const payload = {
      snapshot: launchSnapshot,
      logs: launchLogs,
    };
    try {
      await navigator.clipboard.writeText(JSON.stringify(payload, null, 2));
      setLaunchCopyState("copied");
    } catch {
      setLaunchCopyState("failed");
    }
  };

  const onCreate = async () => {
    setCreateError(null);
    setLaunchSnapshot(null);
    setLaunchLogs([]);
    setLaunchCopyState("idle");
    setLaunchTick(0);
    setCreating(true);
    try {
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
            password_once: remotePasswordOnce,
            remote_port: parsedRemotePort,
            start_remote: true,
            remote_data_dir: remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null,
          })
          : await desktopConnectLocal();
        applyConnection(info);
      }

      if (selections.location === "remote" && parsedRemoteHost) {
        rememberRemoteProfile(parsedRemoteHost, parsedRemoteUser ?? null);
      }

      await waitForDaemonReady(15000);
      const onboardingResult = await ensureOnboardingAfterDaemonConnect({
        allowTitlingInsertion: selections.location === "remote",
      });
      if (onboardingResult?.insertionStep) {
        onOnboardingInsertionRequested(onboardingResult.insertionStep);
        return;
      }
      const containerEnabled = selections.container !== "no-container";
      if (containerEnabled) {
        await waitForRuntimePrewarm();
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
        let destParent: string;
        let destName: string | null;
        if (useDiskIsolatedStaging) {
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
          destPath: useDiskIsolatedStaging ? null : sourcePath,
          useDiskIsolatedStaging,
          existingWorkspaceNames,
        });
      } else if (selections.source === "new") {
        let destPath: string;
        if (useDiskIsolatedStaging) {
          const staging = await repoStagingPath();
          destPath = staging.path;
        } else {
          destPath = sourcePath.trim().replace(/\/+$/, "");
          if (!destPath) throw new Error("Destination folder is required.");
        }
        await repoInit({ path: destPath, allow_existing: true });
        rootPath = destPath;
        const existingWorkspaceNames = await getExistingWorkspaceNamesForGenerated();
        name = resolveWorkspaceName({
          source: selections.source,
          workspaceName,
          repoUrl,
          destPath,
          useDiskIsolatedStaging,
          existingWorkspaceNames,
        });
      } else {
        throw new Error("Choose a source option.");
      }

      const workspaceKind = selections.location === "remote" ? "remote" : "local";
      if (!workspaceId) {
        const created = await createWorkspace(rootPath, name, workspaceKind, "wizard");
        workspaceId = idToString(created.id);
      }

      const environment = selections.container === "no-container"
        ? "host"
        : selections.container === "host-mounted"
          ? "container_host_mounted"
          : "container_disk_isolated";
      const allowlist = parseNetworkAllowlist(networkAllowlist);
      const netMode = selections.network === "allowlist"
        ? "allowlist"
        : selections.network === "full"
          ? "all"
          : "llm_only";
      await updateWorkspaceExecutionConfig(workspaceId, {
        environment,
        network_mode: selections.container !== "no-container" ? netMode : null,
        allowlist: selections.container !== "no-container" && netMode === "allowlist" ? allowlist : null,
      });

      if (selections.container !== "no-container") {
        await waitForLaunchCompletion(workspaceId);
      }

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
          const normalizedDataDir = remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null;
          await upsertLauncherRecent({
            kind: "ssh",
            label: workspaceName.trim() || lastPathSegment(rootPath) || parsedRemoteHost,
            host: parsedRemoteHost,
            user: parsedRemoteUser ?? null,
            remote_port: parsedRemotePort ?? 4399,
            start_remote: true,
            remote_data_dir: normalizedDataDir,
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
      wizardCompletedRef.current = true;
      trackWizardCompleted({
        wizardKey,
        workspaceKind,
      });
      navigate(`/workspaces/${workspaceId}`, { replace: true });
    } catch (error) {
      const message = messageFromError(error);
      setCreateError(message);
      const key = resolveCreateErrorStepKey(message);
      if (key) onCreateErrorStep(key);
    } finally {
      setCreating(false);
    }
  };

  const currentLaunchPhaseEntry = launchSnapshot ? phaseEntryForCurrent(launchSnapshot) : null;
  const currentLaunchElapsed = (() => {
    if (!launchSnapshot) return "0s";
    const elapsedMs = currentLaunchPhaseEntry?.elapsed_ms ?? null;
    if (elapsedMs !== null && elapsedMs !== undefined) {
      return formatLaunchElapsed(elapsedMs);
    }
    const started = parseUtcMs(currentLaunchPhaseEntry?.started_at);
    if (started === null) return "0s";
    return formatLaunchElapsed(Date.now() - started);
  })();

  return {
    creating,
    setCreateError,
    importInitDialog,
    resolveImportInitDialog,
    onPickLocalFolder,
    preflightSourceStep,
    launchSnapshot,
    launchLogs,
    showLaunchPanel: Boolean(launchSnapshot) && (creating || launchSnapshot?.state === "error"),
    currentLaunchElapsed,
    launchCopyLabel: launchCopyState === "copied"
      ? "Copied"
      : launchCopyState === "failed"
        ? "Copy failed"
        : "Copy diagnostics",
    createButtonLabel: creating && launchSnapshot?.kind === "startup_prewarm"
      ? "Preparing runtime…"
      : creating
        ? "Creating…"
        : "Create workspace",
    onCopyLaunchDiagnostics,
    onCreate,
  };
}
