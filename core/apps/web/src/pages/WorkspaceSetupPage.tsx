import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, useNavigate } from "react-router-dom";
import { ChevronRight, Info, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import LauncherBrand from "../components/LauncherBrand";
import {
  applyDaemonDesktopConnection,
  buildExecutionLaunchWsUrl,
  createWorkspace,
  getInstall,
  getExecutionLaunchStatus,
  getHealth,
  getSettings,
  getTitleGenerationLocalStatus,
  importProviderAuthCandidates,
  idToString,
  installTitleGenerationLocal,
  listProviderAuthImportCandidates,
  listWorkspaces,
  startExecutionLaunch,
  startExecutionRuntimePrewarm,
  type ExecutionLaunchLogLine,
  type ExecutionLaunchSnapshot,
  type ExecutionLaunchStreamEvent,
  type ProviderAuthImportCandidate,
  repoClone,
  repoInit,
  repoStatus,
  repoValidateDestination,
  repoStagingPath,
  type Settings,
  type TitleGenerationLocalStatus,
  type TitleGenerationSettings,
  updateSettings,
  updateWorkspaceExecutionConfig,
  updateWorkspaceMergeQueueConfig,
  updateWorkspaceWorktreeBootstrapConfig,
} from "../api/client";
import {
  desktopConnectLocal,
  desktopConnectSsh,
  desktopKickoffRemotePrewarm,
  desktopListSshHosts,
  desktopListSshPaths,
  desktopGetGitBranch,
  desktopPickFolder,
  desktopTestSsh,
  isDesktopApp,
  type DesktopConnectionInfo,
  type DesktopSshPathEntry,
  type DesktopSshHost,
} from "../utils/desktop";
import {
  buildSessionTitlingDraft,
  buildSessionTitlingPayload,
  DEFAULT_TITLE_LOCAL_MODEL_ID,
  DEFAULT_TITLE_REMOTE_BASE_URL,
  DEFAULT_TITLE_REMOTE_MODEL,
  deriveRepoNameFromUrl,
  getSourceStepValidation,
  parseCloneDestPath,
  resolveSessionTitlingReadiness,
  resolveWorkspaceName,
  sessionTitlingPayloadHash,
  type SessionTitlingMode,
} from "./WorkspaceSetupPage.logic";
import {
  formatLaunchElapsed,
  formatLaunchTime,
  launchErrorFromSnapshot,
  launchPhaseLabel,
  mergeLaunchLogs,
  parseUtcMs,
  phaseEntryForCurrent,
} from "./workspaceSetup/launchProgress";
import {
  loadRemoteProfiles,
  loadSshRecents,
  parseUserHost,
  remoteProfileKey,
  upsertRemoteProfile,
  upsertSshRecent,
  type RemoteProfile,
  type SshRecent,
} from "./workspaceSetup/remoteProfiles";
import {
  clampStepKey,
  isCurrentFlowRunToken,
  nextFlowRunToken,
  stepKeyOffset,
  type FlowRunToken,
} from "./workspaceSetup/flowController";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import { upsertLauncherRecent } from "../state/launcherRecentsStore";

type WizardOption = {
  id: string;
  title: string;
  desc: string;
  badge?: string;
  advanced?: boolean;
};

type WizardStep = {
  key: string;
  title: string;
  note: string;
  options?: WizardOption[];
  body?: ReactNode;
  info?: string;
};

type ImportInitDialogState = {
  path: string;
};

type LocalInstallState = {
  installId: string;
  state: "running" | "succeeded" | "failed";
  pct: number | null;
  error?: string;
};

export default function WorkspaceSetupPage() {
  const navigate = useNavigate();
  const [currentStepKey, setCurrentStepKey] = useState("location");
  const [selections, setSelections] = useState<Record<string, string>>({});
  const [sshHosts, setSshHosts] = useState<DesktopSshHost[]>([]);
  const [sshRecents, setSshRecents] = useState<SshRecent[]>(() => loadSshRecents());
  const [remoteHostInput, setRemoteHostInput] = useState("");
  const [remoteStatus, setRemoteStatus] = useState<"idle" | "connecting" | "connected" | "error">("idle");
  const [remoteError, setRemoteError] = useState<string | null>(null);
  const [remoteAdvancedOpen, setRemoteAdvancedOpen] = useState(false);
  const [remotePortInput, setRemotePortInput] = useState("4399");
  const [remoteDataDirInput, setRemoteDataDirInput] = useState("");
  const [remoteCtxBinInput, setRemoteCtxBinInput] = useState("");
  const [remoteProfiles, setRemoteProfiles] = useState<RemoteProfile[]>(() => loadRemoteProfiles());
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [launchSnapshot, setLaunchSnapshot] = useState<ExecutionLaunchSnapshot | null>(null);
  const [launchLogs, setLaunchLogs] = useState<ExecutionLaunchLogLine[]>([]);
  const [launchTick, setLaunchTick] = useState(0);
  const [launchCopyState, setLaunchCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [importRepoStatus, setImportRepoStatus] = useState<"idle" | "checking" | "ok" | "error">("idle");
  const [importRepoNote, setImportRepoNote] = useState<string | null>(null);
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
  const [importInitDialog, setImportInitDialog] = useState<ImportInitDialogState | null>(null);
  const [remotePathSuggestions, setRemotePathSuggestions] = useState<DesktopSshPathEntry[]>([]);
  const [remotePathStatus, setRemotePathStatus] = useState<"idle" | "loading" | "error">("idle");
  const [remotePathError, setRemotePathError] = useState<string | null>(null);
  const [authImportCandidates, setAuthImportCandidates] = useState<ProviderAuthImportCandidate[]>([]);
  const [authImportSelected, setAuthImportSelected] = useState<Record<string, boolean>>({});
  const [authImportBusy, setAuthImportBusy] = useState(false);
  const [authImportError, setAuthImportError] = useState<string | null>(null);
  // Keep a single auth-detection snapshot per daemon target during the wizard.
  // Back/forward navigation preserves the same rows + checkmarks.
  const [authImportScannedKey, setAuthImportScannedKey] = useState<string | null>(null);
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
  const importInitResolveRef = useRef<((confirmed: boolean) => void) | null>(null);
  const remoteProfileAutoAppliedKeyRef = useRef<string | null>(null);
  const titlingInstallPollRef = useRef<number | null>(null);
  const titlingInstallPollGenerationRef = useRef(0);
  const selectedDaemonTargetKeyRef = useRef<string | null>(null);
  const authImportScanPromiseRef = useRef<Promise<ProviderAuthImportCandidate[]> | null>(null);
  const authImportScanKeyRef = useRef<string | null>(null);
  const authImportScanRunRef = useRef<FlowRunToken | null>(null);
  const pendingLocalLocationAdvanceRef = useRef(false);
  const locationAdvanceRunRef = useRef<FlowRunToken | null>(null);
  const currentStepKeyRef = useRef<string>("location");
  const previousStepIndexRef = useRef(0);
  const titlingProbePromiseRef = useRef<Promise<boolean | null> | null>(null);
  const titlingProbePromiseTargetKeyRef = useRef<string | null>(null);
  const remoteStatusRef = useRef(remoteStatus);
  const harnessByProviderId = useMemo(() => {
    return new Map(HARNESS_CATALOG.map((entry) => [entry.id, entry]));
  }, []);

  const logoClasses = (base: string, invertInDark?: boolean, invertInLight?: boolean): string =>
    [base, invertInDark ? "wb-invert" : "", invertInLight ? "wb-invert-light" : ""]
      .filter(Boolean)
      .join(" ");

  const containerMode = selections.container;
  const authImportStepVisible = Boolean(selections.location) && authImportCandidates.length > 0;
  const titlingStepVisible = Boolean(selections.location) && titlingProbeDone && titlingStepRequired;

  const steps = useMemo<WizardStep[]>(() => {
    const out: WizardStep[] = [
      {
        key: "location",
        title: "Location",
        note: "Where will this workspace run?",
        options: [
          { id: "local", title: "Local", desc: "Agents run on this machine." },
          { id: "remote", title: "Remote", desc: "Agents run on your existing dev box (remote IDE experience)." },
        ],
      },
    ];

    if (authImportStepVisible) {
      out.push({
        key: "auth-import",
        title: "Import Existing Auth",
        note: "Optional: import provider credentials found on this host.",
      });
    }

    if (titlingStepVisible) {
      out.push({
        key: "session-titling",
        title: "Session Titling",
        note: "Choose how ctx should generate session titles on this daemon.",
      });
    }

    out.push(
      {
        key: "container",
        title: "Agent Sandbox Isolation",
        note: "Choose the containerization strategy for your agents in this workspace.",
        options: [
          {
            id: "disk-isolated",
            title: "Disk-isolated container",
            desc: "Worktrees live on the container/VM disk. The daemon mediates shells, files, and git for a fully isolated workspace.",
            badge: "Recommended",
          },
          {
            id: "no-container",
            title: "No container",
            desc: "Run directly on the host. Useful if this machine is already agent-safe (e.g. a dedicated dev box).",
          },
          {
            id: "host-mounted",
            title: "Host-mounted container",
            desc: "Agents run in a container but write directly to your project folder on the host.",
            advanced: true,
          },
        ],
      },
      {
        key: "source",
        title: "Source",
        note: "How should we create the workspace?",
        options: [
          { id: "clone", title: "Clone repo", desc: "Git URL + optional branch." },
          { id: "import", title: "Import folder", desc: "Use an existing git repo folder path." },
          { id: "new", title: "New empty", desc: "Initialize a new git repo." },
        ],
      },
    );

    if (containerMode !== "no-container") {
      out.push({
        key: "network",
        title: "Network Policy",
        note: "Restrict or permit agent network access (container mode only).",
        options: [
          {
            id: "providers",
            title: "LLM providers only",
            desc: "Only allow validated LLM provider traffic. This blocks other outbound access.",
          },
          {
            id: "allowlist",
            title: "Allowlist",
            desc: "Allow only hosts you approve (one per line). Useful for known-safe sources.",
          },
          {
            id: "full",
            title: "Full access",
            desc: "Unrestricted outbound. Only use if you understand prompt-injection / data exfil risks.",
          },
        ],
      });
    }

    out.push(
      {
        key: "setup",
        title: "Worktree Setup Hook",
        note: "Choose a single shell command to run on new worktree creation (e.g., install dependencies).",
        info: [
          "Each ctx task runs on its own git worktree. A worktree is a separate working directory attached to the same repository.",
          "This allows one or more agents to work on a task branch isolated from other tasks, so work can be done simultaneously.",
          "",
          "Depending on your project, you might want to do setup every time a new worktree is created (for example, installing dependencies).",
          "",
          "If you do not know what to put here, skip it and come back later. You can also ask an agent what the best setup hook is for your project.",
          "",
          'Example prompt: "You are in a freshly created git worktree in this project. Is there any setup that ought to have occurred? If so, is there a single setup command we can run as a worktree setup hook?"',
        ].join("\n"),
      },
      {
        key: "merge-queue",
        title: "Merge Queue",
        note: "Branch to work from, plus an optional verification command.",
        info: [
          "A merge queue is a queue of pull requests waiting to be merged to a single branch. It helps ensure changes merge cleanly and pass checks.",
          "",
          "With stacked agent-driven changes, two PRs can pass individually but fail once combined. A personal merge queue helps you test stacked changes locally.",
          "",
          "In this setup step, choose a branch to work off of, and an optional verification command. Prefer a lightweight but robust command here (lint, format, build, unit tests), and defer expensive or flaky end-to-end tests to CI or on-demand runs.",
          "",
          "Advanced settings let you automatically push to a remote after a successful local merge.",
        ].join("\n"),
      },
      {
        key: "confirm",
        title: "Confirm and create",
        note: "Review your choices before provisioning.",
      },
    );

    return out;
  }, [containerMode, authImportStepVisible, titlingStepVisible]);

  const stepKeys = useMemo<string[]>(
    () => steps.map((wizardStep) => wizardStep.key),
    [steps],
  );

  useEffect(() => {
    setCurrentStepKey((key) => clampStepKey(stepKeys, key, previousStepIndexRef.current));
  }, [stepKeys]);
  useEffect(() => {
    if (!creating || !launchSnapshot || launchSnapshot.state !== "running") return;
    const handle = window.setInterval(() => setLaunchTick((value) => value + 1), 1000);
    return () => window.clearInterval(handle);
  }, [creating, launchSnapshot?.job_id, launchSnapshot?.state]);

  const stepIndex = Math.max(0, stepKeys.indexOf(currentStepKey));
  const step = steps[stepIndex];
  const infoStep = openInfoKey ? steps.find((s) => s.key === openInfoKey) : null;
  const isFirst = stepIndex === 0;
  const isLast = stepIndex === steps.length - 1;
  const goToStepKey = useCallback((key: string) => {
    setCurrentStepKey(key);
  }, []);
  const goRelativeStep = useCallback((delta: number) => {
    setCurrentStepKey((current) => stepKeyOffset(stepKeys, current, delta));
  }, [stepKeys]);
  useEffect(() => {
    previousStepIndexRef.current = stepIndex;
  }, [stepIndex]);
  const requiresSelection = Boolean(step.options?.length);
  const hasSelection = Boolean(selections[step.key]);
  const mergeQueueSkipped = selections["merge-queue"] === "skip";
  const isRemoteStep = step.key === "location" && selections.location === "remote";
  const isSourceStep = step.key === "source";
  const useDiskIsolatedStaging =
    selections.container === "disk-isolated" &&
    (selections.source === "clone" || selections.source === "new");
  const sourceStepValidation = getSourceStepValidation({
    source: selections.source,
    sourcePath,
    repoUrl,
    useDiskIsolatedStaging,
  });
  const needsSourcePath = isSourceStep && sourceStepValidation.needsSourcePath;
  const hasSourceStepInputs = !isSourceStep || sourceStepValidation.isComplete;
  const needsTargetBranch = step.key === "merge-queue" && !mergeQueueSkipped;
  const hasTargetBranch = !needsTargetBranch || targetBranch.trim() !== "";
  const needsAllowlist = step.key === "network" && selections.network === "allowlist";
  const hasAllowlist = !needsAllowlist
    || networkAllowlist.split(/\r?\n/).map((line) => line.trim()).filter(Boolean).length > 0;
  const parsedRemote = parseUserHost(remoteHostInput);
  const desktopApp = isDesktopApp();
  const remoteCtxBinValue = remoteCtxBinInput.trim();
  const remoteCtxBinIsAbsolute = remoteCtxBinValue.startsWith("/");
  const parsedRemotePort = (() => {
    const raw = remotePortInput.trim();
    if (!raw) return null;
    const v = Number(raw);
    if (!Number.isFinite(v)) return null;
    const port = Math.trunc(v);
    if (port < 1 || port > 65535) return null;
    return port;
  })();
  const selectedDaemonTargetKey = selections.location === "remote"
    ? (parsedRemote?.host
      ? `ssh:${parsedRemote.user ?? ""}@${parsedRemote.host}:${parsedRemotePort ?? 4399}:${remoteDataDirInput.trim()}:${remoteCtxBinValue}`
      : null)
    : selections.location === "local"
      ? "local"
      : null;
  const canProbeTitling = desktopApp && (
    selections.location === "local"
    || (
      selections.location === "remote"
      && Boolean(parsedRemote?.host)
      && remoteStatus === "connected"
      && remoteCtxBinValue !== ""
      && remoteCtxBinIsAbsolute
    )
  );
  const hasRemoteHost = Boolean(parsedRemote?.host);
  const titlingRemoteValid = titlingRemoteBaseUrl.trim() !== ""
    && titlingRemoteApiKey.trim() !== ""
    && titlingRemoteModel.trim() !== "";
  const titlingSelectionComplete = !titlingStepVisible
    || titlingMode === "skip"
    || (titlingMode === "remote" && titlingRemoteValid)
    || titlingMode === "local";
  const titlingStepCanAdvance = step.key !== "session-titling"
    || (!titlingPersistBusy && titlingSelectionComplete);
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
  const titlingProbeResolved = !selections.location || titlingProbeDone || !canProbeTitling;
  const titlingFlowGateSatisfied = step.key === "location" || step.key === "auth-import" || titlingProbeResolved;
  const canAdvance = (!requiresSelection || hasSelection)
    && (!isRemoteStep || (
      hasRemoteHost
      && remoteStatus !== "connecting"
      && parsedRemotePort !== null
    ))
    && hasSourceStepInputs
    && hasTargetBranch
    && hasAllowlist
    && (step.key !== "auth-import" || !authImportBusy)
    && titlingStepCanAdvance
    && titlingFlowGateSatisfied
    ;
  const showLaunchPanel = Boolean(launchSnapshot) && (creating || launchSnapshot?.state === "error");
  const currentLaunchPhaseLabel = launchPhaseLabel(launchSnapshot?.current_phase);
  const currentLaunchPhaseEntry = launchSnapshot ? phaseEntryForCurrent(launchSnapshot) : null;
  const currentLaunchElapsed = useMemo(() => {
    if (!launchSnapshot) return "0s";
    const elapsedMs = currentLaunchPhaseEntry?.elapsed_ms ?? null;
    if (elapsedMs !== null && elapsedMs !== undefined) {
      return formatLaunchElapsed(elapsedMs);
    }
    const started = parseUtcMs(currentLaunchPhaseEntry?.started_at);
    if (started === null) return "0s";
    return formatLaunchElapsed(Date.now() - started);
  }, [currentLaunchPhaseEntry, launchSnapshot, launchTick]);
  const launchCopyLabel = launchCopyState === "copied"
    ? "Copied"
    : launchCopyState === "failed"
      ? "Copy failed"
      : "Copy diagnostics";
  const createButtonLabel = creating && launchSnapshot?.kind === "startup_prewarm"
    ? "Preparing runtime…"
    : creating
      ? "Creating…"
      : "Create workspace";

  function applyConnection(info: DesktopConnectionInfo) {
    applyDaemonDesktopConnection(info);
  }

  const sleepMs = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

  const waitForDaemonReady = async (timeoutMs: number) => {
    const started = Date.now();
    let lastErr: any = null;
    while (Date.now() - started < timeoutMs) {
      try {
        await getHealth();
        return;
      } catch (e) {
        lastErr = e;
      }
      await sleepMs(200);
    }
    throw lastErr ?? new Error("Timed out waiting for daemon health.");
  };

  const connectDaemonForImport = async (locationOverride?: "local" | "remote") => {
    const location = locationOverride ?? selections.location;
    if (!isDesktopApp()) {
      throw new Error("Auth import requires the desktop app.");
    }
    if (location === "remote") {
      const parsed = parseUserHost(remoteHostInput);
      if (!parsed?.host) {
        throw new Error("Remote host is required before scanning auth.");
      }
      if (remoteStatusRef.current !== "connected") {
        throw new Error("Verify remote host connection before scanning auth.");
      }
      const remoteCtxBin = remoteCtxBinInput.trim();
      if (!remoteCtxBin) {
        throw new Error("Remote ctx binary path is required.");
      }
      if (!remoteCtxBin.startsWith("/")) {
        throw new Error("Remote ctx binary path must be absolute (for example /opt/ctx/bin/ctx).");
      }
      const info = await desktopConnectSsh({
        host: parsed.host,
        user: parsed.user ?? null,
        remote_port: parsedRemotePort,
        start_remote: true,
        remote_data_dir: remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null,
        remote_ctx_bin: remoteCtxBin,
      });
      applyConnection(info);
      await waitForDaemonReady(15000);
      return;
    }
    const info = await desktopConnectLocal();
    applyConnection(info);
    await waitForDaemonReady(15000);
  };

  const messageFromError = (error: unknown): string =>
    error instanceof Error && error.message ? error.message : String(error);

  const clearTitlingInstallPoll = () => {
    titlingInstallPollGenerationRef.current += 1;
    if (titlingInstallPollRef.current) {
      window.clearTimeout(titlingInstallPollRef.current);
      titlingInstallPollRef.current = null;
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
        const pct =
          typeof info.last_event?.bytes === "number"
          && typeof info.last_event?.total_bytes === "number"
          && info.last_event.total_bytes > 0
            ? Math.max(0, Math.min(100, Math.round((info.last_event.bytes / info.last_event.total_bytes) * 100)))
            : null;
        setTitlingLocalInstall({
          installId,
          state: info.state,
          pct,
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

  const ensureTitlingProbeForCurrentTarget = async (): Promise<boolean | null> => {
    if (!selectedDaemonTargetKey || !desktopApp) return null;
    if (selections.location === "remote") {
      if (!parsedRemote?.host) return null;
      if (!remoteCtxBinValue || !remoteCtxBinIsAbsolute) return null;
      if (remoteStatusRef.current !== "connected") return null;
    }
    if (titlingProbeDone && titlingProbeTargetKey === selectedDaemonTargetKey) {
      return titlingStepRequired;
    }
    const targetKey = selectedDaemonTargetKey;
    if (
      titlingProbePromiseRef.current
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
    if (!payload) return false;
    if (!selectedDaemonTargetKey) return false;
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

  const scanAuthImportCandidatesForTarget = useCallback(async (target: "local" | "remote"): Promise<ProviderAuthImportCandidate[]> => {
    if (!isDesktopApp()) return [];
    if (target === "remote") {
      if (!parsedRemote?.host) return [];
      if (remoteStatusRef.current !== "connected") return [];
    }

    const scanKey = target === "local"
      ? "local|@"
      : `remote|${parsedRemote?.user ?? ""}@${parsedRemote?.host ?? ""}`;
    if (authImportScannedKey === scanKey) return authImportCandidates;

    if (
      authImportScanPromiseRef.current
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
        const resp = await listProviderAuthImportCandidates();
        // Wizard intentionally surfaces only candidates we can import now.
        const candidates = (resp.candidates ?? [])
          .filter((candidate) => candidate.parse_status === "parsed");
        if (!isCurrentFlowRunToken(authImportScanRunRef.current, scanRun)) {
          return [];
        }
        setAuthImportCandidates(candidates);
        setAuthImportSelected(
          Object.fromEntries(candidates.map((candidate) => [candidate.id, true])),
        );
        return candidates;
      } catch (error) {
        if (!isCurrentFlowRunToken(authImportScanRunRef.current, scanRun)) {
          return [];
        }
        setAuthImportCandidates([]);
        setAuthImportSelected({});
        setAuthImportError(messageFromError(error));
        return [];
      } finally {
        if (isCurrentFlowRunToken(authImportScanRunRef.current, scanRun)) {
          // Mark this target as scanned even on failure to avoid repeated races while navigating.
          setAuthImportScannedKey(scanKey);
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
    authImportScannedKey,
    parsedRemote?.host,
    parsedRemote?.user,
  ]);

  const shouldAutoAdvance = (stepKey: string, optionId: string): boolean => {
    if (stepKey === "location") return false;
    if (stepKey === "container") return true;
    if (stepKey === "network") return optionId !== "allowlist";
    return false;
  };

  const nextStepAfterLocation = (candidateCount: number, titlingRequired: boolean | null): string => {
    if (candidateCount > 0) return "auth-import";
    if (titlingRequired) return "session-titling";
    return "container";
  };

  const onSelect = (stepKey: string, optionId: string) => {
    setCreateError(null);
    setSelections((prev) => {
      const next = { ...prev, [stepKey]: optionId };
      if (stepKey === "container" && optionId === "no-container") {
        delete next.network;
      }
      return next;
    });
    if (stepKey === "location" && optionId === "local") {
      remoteStatusRef.current = "idle";
      setRemoteStatus("idle");
      setRemoteError(null);
      setImportRepoStatus("idle");
      setImportRepoNote(null);
    }
    if (stepKey === "location") {
      // Keep local prefetch snapshot so local click can advance without step-topology churn.
      if (optionId === "remote") {
        pendingLocalLocationAdvanceRef.current = false;
        const nextRun = nextFlowRunToken(locationAdvanceRunRef.current?.runId ?? 0, "local");
        locationAdvanceRunRef.current = nextRun;
        authImportScanPromiseRef.current = null;
        authImportScanKeyRef.current = null;
        authImportScanRunRef.current = null;
        setAuthImportBusy(false);
        setAuthImportScannedKey(null);
        setAuthImportCandidates([]);
        setAuthImportSelected({});
        setAuthImportError(null);
      }
      invalidateTitlingPersisted();
      setTitlingProbeError(null);
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
  };

  const onSelectOption = (stepKey: string, optionId: string) => {
    onSelect(stepKey, optionId);
    if (stepKey === "location" && optionId === "local") {
      pendingLocalLocationAdvanceRef.current = true;
      const nextRun = nextFlowRunToken(locationAdvanceRunRef.current?.runId ?? 0, "local");
      locationAdvanceRunRef.current = nextRun;
      return;
    }
    if (shouldAutoAdvance(stepKey, optionId)) {
      goRelativeStep(1);
    }
  };

  const enableMergeQueueIfSkipped = () => {
    if (!mergeQueueSkipped) return;
    setSelections((prev) => {
      if (prev["merge-queue"] !== "skip") return prev;
      const next = { ...prev };
      delete next["merge-queue"];
      return next;
    });
  };

  useEffect(() => {
    selectedDaemonTargetKeyRef.current = selectedDaemonTargetKey;
  }, [selectedDaemonTargetKey]);

  useEffect(() => {
    remoteStatusRef.current = remoteStatus;
  }, [remoteStatus]);

  useEffect(() => {
    currentStepKeyRef.current = step.key;
  }, [step.key]);

  useEffect(() => {
    if (!selectedDaemonTargetKey || !canProbeTitling) {
      titlingProbePromiseRef.current = null;
      titlingProbePromiseTargetKeyRef.current = null;
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
      resetTitlingDraft();
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
    if (selections.container === "host-mounted") {
      setContainerAdvancedOpen(true);
    }
  }, [selections.container]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (importInitDialog) {
          const resolve = importInitResolveRef.current;
          importInitResolveRef.current = null;
          setImportInitDialog(null);
          resolve?.(false);
          return;
        }
        setOpenInfoKey(null);
      }
    };
    if (openInfoKey || importInitDialog) {
      window.addEventListener("keydown", onKeyDown);
      return () => window.removeEventListener("keydown", onKeyDown);
    }
    return;
  }, [openInfoKey, importInitDialog]);

  useEffect(() => () => {
    const resolve = importInitResolveRef.current;
    importInitResolveRef.current = null;
    resolve?.(false);
  }, []);

  useEffect(() => {
    setSshRecents(loadSshRecents());
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    desktopListSshHosts()
      .then((hosts) => setSshHosts(hosts))
      .catch(() => setSshHosts([]));
  }, []);

  useEffect(() => {
    if (!pendingLocalLocationAdvanceRef.current) return;
    if (step.key !== "location") return;
    if (selections.location !== "local") return;

    const run = nextFlowRunToken(locationAdvanceRunRef.current?.runId ?? 0, "local");
    locationAdvanceRunRef.current = run;
    pendingLocalLocationAdvanceRef.current = false;
    void (async () => {
      // Resolve both auth + titling before leaving Location so step topology stays stable.
      const [candidates, titlingRequired] = await Promise.all([
        scanAuthImportCandidatesForTarget("local"),
        ensureTitlingProbeForCurrentTarget(),
      ]);
      if (!isCurrentFlowRunToken(locationAdvanceRunRef.current, run)) return;
      if (selectedDaemonTargetKeyRef.current !== "local") return;
      if (currentStepKeyRef.current !== "location") return;
      goToStepKey(nextStepAfterLocation(candidates.length, titlingRequired));
    })();
  }, [
    ensureTitlingProbeForCurrentTarget,
    goToStepKey,
    scanAuthImportCandidatesForTarget,
    selections.location,
    step.key,
  ]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    if (step.key !== "location") return;
    if (selections.location) return;
    void scanAuthImportCandidatesForTarget("local");
  }, [
    scanAuthImportCandidatesForTarget,
    selections.location,
    step.key,
  ]);

  useEffect(() => {
    if (selections.location) return;
    setAuthImportCandidates([]);
    setAuthImportSelected({});
    setAuthImportError(null);
    setAuthImportScannedKey(null);
  }, [selections.location]);

  useEffect(() => {
    const shouldSuggest = needsSourcePath
      && selections.location === "remote"
      && remoteStatus === "connected"
      && Boolean(parsedRemote?.host)
      && isDesktopApp();
    if (!shouldSuggest) {
      setRemotePathSuggestions([]);
      setRemotePathStatus("idle");
      setRemotePathError(null);
      return;
    }
    const handle = window.setTimeout(() => {
      setRemotePathStatus("loading");
      setRemotePathError(null);
      desktopListSshPaths({
        host: parsedRemote!.host,
        user: parsedRemote!.user ?? null,
        path: sourcePath,
      })
        .then((entries) => {
          setRemotePathSuggestions(entries);
          setRemotePathStatus("idle");
        })
        .catch((err: any) => {
          setRemotePathStatus("error");
          setRemotePathError(err?.message ?? String(err));
        });
    }, 250);
    return () => window.clearTimeout(handle);
  }, [needsSourcePath, selections.location, remoteStatus, parsedRemote?.host, parsedRemote?.user, sourcePath]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    if (selections.location !== "local") return;
    if (selections.source !== "import") {
      setImportRepoStatus("idle");
      setImportRepoNote(null);
      return;
    }
    if (!sourcePath.trim()) {
      setImportRepoStatus("idle");
      setImportRepoNote(null);
      return;
    }
    let cancelled = false;
    setImportRepoStatus("checking");
    setImportRepoNote(null);
    desktopGetGitBranch({ path: sourcePath })
      .then((branch) => {
        if (cancelled) return;
        setImportRepoStatus("ok");
        setImportRepoNote(null);
        if (!branch || targetBranchTouched) return;
        setTargetBranch(branch);
        if (!pushBranchTouched) setPushBranch(branch);
      })
      .catch(() => {
        if (cancelled) return;
        setImportRepoStatus("error");
        setImportRepoNote("Selected folder does not look like a git repo.");
      });
    return () => {
      cancelled = true;
    };
  }, [selections.location, selections.source, sourcePath, targetBranchTouched, pushBranchTouched]);

  useEffect(() => {
    if (pushBranchTouched) return;
    if (!targetBranch.trim()) return;
    setPushBranch(targetBranch);
  }, [targetBranch, pushBranchTouched]);

  useEffect(() => {
    if (selections.location !== "remote") return;
    const parsed = parseUserHost(remoteHostInput);
    if (!parsed?.host) {
      remoteProfileAutoAppliedKeyRef.current = null;
      return;
    }
    const key = remoteProfileKey(parsed.host, parsed.user ?? null);
    if (remoteProfileAutoAppliedKeyRef.current === key) return;
    const profile = remoteProfiles.find((entry) => remoteProfileKey(entry.host, entry.user) === key);
    if (!profile) return;

    if (typeof profile.remote_port === "number" && Number.isFinite(profile.remote_port)) {
      setRemotePortInput(String(profile.remote_port));
    }
    setRemoteDataDirInput(String(profile.remote_data_dir ?? ""));
    setRemoteCtxBinInput(String(profile.remote_ctx_bin ?? ""));
    remoteProfileAutoAppliedKeyRef.current = key;
  }, [selections.location, remoteHostInput, remoteProfiles]);

  // Remote import validation: once SSH is verified, check the selected remote folder is a repo.
  useEffect(() => {
    const shouldCheck =
      isDesktopApp()
      && step.key === "source"
      && selections.location === "remote"
      && remoteStatus === "connected"
      && selections.source === "import"
      && Boolean(parseUserHost(remoteHostInput)?.host)
      && Boolean(sourcePath.trim());
    if (!shouldCheck) return;
    const parsed = parseUserHost(remoteHostInput);
    if (!parsed?.host) return;

    let cancelled = false;
    const handle = window.setTimeout(() => {
      setImportRepoStatus("checking");
      setImportRepoNote(null);
      desktopConnectSsh({
        host: parsed.host,
        user: parsed.user ?? null,
        remote_port: parsedRemotePort,
        start_remote: true,
        remote_data_dir: remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null,
        remote_ctx_bin: remoteCtxBinValue,
      })
        .then((info) => {
          if (cancelled) return null;
          applyConnection(info);
          return repoStatus({ path: sourcePath.trim().replace(/\/+$/, "") });
        })
        .then((st) => {
          if (cancelled) return;
          if (!st) return;
          if (st.is_repo) {
            setImportRepoStatus("ok");
            setImportRepoNote(null);
          } else {
            setImportRepoStatus("error");
            const detail = String((st as any).error ?? "").trim();
            setImportRepoNote(detail ? `Not a git repo: ${detail}` : "Not a git repo.");
          }
        })
        .catch((e: any) => {
          if (cancelled) return;
          setImportRepoStatus("error");
          setImportRepoNote(e?.message ? `Remote repo check failed: ${e.message}` : "Remote repo check failed.");
        });
    }, 400);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
  }, [
    step.key,
    selections.location,
    remoteStatus,
    selections.source,
    remoteHostInput,
    sourcePath,
    parsedRemotePort,
    remoteDataDirInput,
    remoteCtxBinValue,
  ]);

  const sshSuggestions = useMemo(() => {
    const query = remoteHostInput.trim().toLowerCase();
    const list: Array<{ host: string; user?: string | null }> = [];
    const seen = new Set<string>();
    for (const recent of sshRecents) {
      const key = `${recent.user ?? ""}@${recent.host}`;
      if (seen.has(key)) continue;
      seen.add(key);
      list.push({ host: recent.host, user: recent.user ?? null });
    }
    for (const host of sshHosts) {
      const key = `${host.user ?? ""}@${host.host}`;
      if (seen.has(key)) continue;
      seen.add(key);
      list.push({ host: host.host, user: host.user ?? null });
    }
    return list.filter((entry) => {
      if (!query) return true;
      const hay = `${entry.user ?? ""}@${entry.host}`.toLowerCase();
      return hay.includes(query);
    }).slice(0, 6);
  }, [remoteHostInput, sshHosts, sshRecents]);

  const onRemoteInputChange = (value: string) => {
    setCreateError(null);
    remoteProfileAutoAppliedKeyRef.current = null;
    setRemoteHostInput(value);
    setAuthImportScannedKey(null);
    if (remoteStatus !== "idle") {
      setRemoteStatus("idle");
      setRemoteError(null);
    }
  };

  const onPickLocalFolder = async () => {
    if (!isDesktopApp()) return;
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
    if (step.key !== "source") return true;

    if (isDesktopApp()) {
      try {
        await connectDaemonForImport();
      } catch (error) {
        setCreateError(messageFromError(error));
        return false;
      }
    }

    // Validate host-path destination constraints on Next so users see path errors before Create.
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
      setImportRepoStatus("checking");
      setImportRepoNote(null);
      try {
        const st = await repoStatus({ path: rootPath });
        if (st.is_repo) {
          setImportRepoStatus("ok");
          setImportRepoNote(null);
          return true;
        }
        const detail = String(st.error ?? "").trim();
        const note = detail
          ? `Not a git repo yet (${detail}). We'll offer to initialize it during Create.`
          : "Not a git repo yet. We'll offer to initialize it during Create.";
        // Keep this flow reachable: non-repo folders are supported via confirm+repoInit at Create time.
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
        await importProviderAuthCandidates(candidateIds);
      } catch (err: any) {
        setAuthImportError(err?.message ?? String(err));
        setAuthImportBusy(false);
        return;
      }
      setAuthImportBusy(false);
    }
    await ensureTitlingProbeForCurrentTarget();
    if (currentStepKeyRef.current !== "auth-import") return;
    goRelativeStep(1);
  };

  const onNext = async () => {
    if (step.key === "location") {
      if (selections.location === "local") {
        // If local auto-advance is in flight, manual Next takes ownership so only one
        // continuation can commit the location->next-step transition.
        pendingLocalLocationAdvanceRef.current = false;
        const nextRun = nextFlowRunToken(locationAdvanceRunRef.current?.runId ?? 0, "local");
        locationAdvanceRunRef.current = nextRun;
      }
      if (selections.location === "remote") {
        if (!parsedRemote) return;
        if (!isDesktopApp()) {
          setRemoteStatus("error");
          setRemoteError("Remote connections require the desktop app.");
          return;
        }
        if (!remoteCtxBinValue) {
          setRemoteAdvancedOpen(true);
          setRemoteStatus("error");
          setRemoteError("Remote ctx binary path is required.");
          return;
        }
        if (!remoteCtxBinIsAbsolute) {
          setRemoteAdvancedOpen(true);
          setRemoteStatus("error");
          setRemoteError("Remote ctx binary path must be absolute (for example /opt/ctx/bin/ctx).");
          return;
        }
        if (remoteStatus !== "connected") {
          setRemoteStatus("connecting");
          setRemoteError(null);
          try {
            await desktopTestSsh({
              host: parsedRemote.host,
              user: parsedRemote.user ?? null,
            });
            remoteStatusRef.current = "connected";
            setRemoteStatus("connected");
            const normalizedDataDir = remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null;
            setRemoteProfiles(upsertRemoteProfile(parsedRemote.host, parsedRemote.user ?? null, {
              remote_port: parsedRemotePort ?? 4399,
              remote_data_dir: normalizedDataDir,
              remote_ctx_bin: remoteCtxBinValue,
            }));
            remoteProfileAutoAppliedKeyRef.current = remoteProfileKey(parsedRemote.host, parsedRemote.user ?? null);
            void desktopKickoffRemotePrewarm({
              host: parsedRemote.host,
              user: parsedRemote.user ?? null,
              remote_port: parsedRemotePort,
              remote_data_dir: normalizedDataDir,
            }).catch((err: any) => {
              console.debug("remote prewarm kickoff skipped/failed", err?.message ?? String(err));
            });
            const nextRecents = upsertSshRecent(parsedRemote.host, parsedRemote.user ?? null);
            setSshRecents(nextRecents);
          } catch (err: any) {
            setRemoteStatus("error");
            setRemoteError(err?.message ?? String(err));
            return;
          }
        }
      }

      let candidateCount = 0;
      let titlingRequired: boolean | null = null;
      if (selections.location === "local") {
        // Keep next-step choice deterministic from resolved scan/probe outcomes.
        const [candidates, required] = await Promise.all([
          scanAuthImportCandidatesForTarget("local"),
          ensureTitlingProbeForCurrentTarget(),
        ]);
        candidateCount = candidates.length;
        titlingRequired = required;
      } else if (selections.location === "remote") {
        const [candidates, required] = await Promise.all([
          scanAuthImportCandidatesForTarget("remote"),
          ensureTitlingProbeForCurrentTarget(),
        ]);
        candidateCount = candidates.length;
        titlingRequired = required;
      }
      if (currentStepKeyRef.current !== "location") return;
      goToStepKey(nextStepAfterLocation(candidateCount, titlingRequired));
      return;
    }
    if (step.key === "auth-import") {
      await advanceFromAuthImportStep();
      return;
    }
    if (step.key === "session-titling") {
      setTitlingPersistError(null);
      if (titlingMode === "skip") {
        goRelativeStep(1);
        return;
      }
      if (titlingMode !== "remote" && titlingMode !== "local") {
        setTitlingPersistError("Choose a titling option or skip for now.");
        return;
      }
      if (titlingMode === "remote" && !titlingRemoteValid) {
        setTitlingPersistError("Remote titling needs base URL, API key, and model.");
        return;
      }
      const persisted = await ensureTitlingPersistedForCurrentTarget();
      if (!persisted) return;
      goRelativeStep(1);
      return;
    }
    if (step.key === "source") {
      setCreateError(null);
      const preflightOk = await preflightSourceStep();
      if (!preflightOk) return;
      goRelativeStep(1);
      return;
    }
    goRelativeStep(1);
  };

  const applyLaunchSnapshot = (snapshot: ExecutionLaunchSnapshot) => {
    setLaunchSnapshot(snapshot);
    setLaunchLogs((prev) => mergeLaunchLogs(prev, snapshot.logs ?? []));
  };

  const appendLaunchLine = (line: ExecutionLaunchLogLine) => {
    setLaunchLogs((prev) => mergeLaunchLogs(prev, [line]));
  };

  const waitForLaunchTerminal = async (initial: ExecutionLaunchSnapshot) => {
    applyLaunchSnapshot(initial);

    if (initial.state === "ready") return;
    if (initial.state === "error") throw new Error(launchErrorFromSnapshot(initial));

    await new Promise<void>((resolve, reject) => {
      let settled = false;
      const ws = new WebSocket(buildExecutionLaunchWsUrl(initial.job_id));

      const settle = (err?: Error) => {
        if (settled) return;
        settled = true;
        ws.close();
        if (err) reject(err);
        else resolve();
      };

      ws.onmessage = (event) => {
        let parsed: ExecutionLaunchStreamEvent | null = null;
        try {
          parsed = JSON.parse(String(event.data ?? "")) as ExecutionLaunchStreamEvent;
        } catch {
          return;
        }
        if (!parsed) return;
        if (parsed.type === "launch_log") {
          appendLaunchLine(parsed.line);
          return;
        }
        if (parsed.type === "launch_snapshot") {
          applyLaunchSnapshot(parsed.snapshot);
          return;
        }
        if (parsed.type === "launch_complete") {
          applyLaunchSnapshot(parsed.snapshot);
          settle();
          return;
        }
        if (parsed.type === "launch_error") {
          applyLaunchSnapshot(parsed.snapshot);
          settle(new Error(launchErrorFromSnapshot(parsed.snapshot)));
        }
      };

      ws.onerror = () => {
        // Wait for close and then fall back to status query.
      };

      ws.onclose = () => {
        if (settled) return;
        getExecutionLaunchStatus(initial.job_id)
          .then((latest) => {
            applyLaunchSnapshot(latest);
            if (latest.state === "ready") {
              settle();
            } else if (latest.state === "error") {
              settle(new Error(launchErrorFromSnapshot(latest)));
            } else {
              settle(new Error("Lost workspace launch stream before setup finished."));
            }
          })
          .catch((err: any) => {
            settle(new Error(err?.message ?? String(err)));
          });
      };
    });
  };

  const waitForLaunchCompletion = async (workspaceId: string) => {
    const initial = await startExecutionLaunch(workspaceId);
    await waitForLaunchTerminal(initial);
  };

  const waitForRuntimePrewarm = async () => {
    const initial = await startExecutionRuntimePrewarm();
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
      if (!isDesktopApp()) {
        throw new Error("Workspace creation from the wizard requires the desktop app.");
      }

      const parsed = selections.location === "remote" ? parseUserHost(remoteHostInput) : null;
      if (selections.location === "remote" && !parsed?.host) {
        throw new Error("Remote host is required (user@host).");
      }
      if (selections.location === "remote" && !remoteCtxBinValue) {
        throw new Error("Remote ctx binary path is required.");
      }
      if (selections.location === "remote" && !remoteCtxBinIsAbsolute) {
        throw new Error("Remote ctx binary path must be absolute (for example /opt/ctx/bin/ctx).");
      }

      // 1. Connect to the intended daemon (reuse existing if already running).
      const info = selections.location === "remote"
        ? await desktopConnectSsh({
          host: parsed!.host,
          user: parsed!.user ?? null,
          remote_port: parsedRemotePort,
          start_remote: true,
          remote_data_dir: remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null,
          remote_ctx_bin: remoteCtxBinValue,
        })
        : await desktopConnectLocal();
      applyConnection(info);

      if (selections.location === "remote" && parsed?.host) {
        const normalizedDataDir = remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null;
        setRemoteProfiles(upsertRemoteProfile(parsed.host, parsed.user ?? null, {
          remote_port: parsedRemotePort ?? 4399,
          remote_data_dir: normalizedDataDir,
          remote_ctx_bin: remoteCtxBinValue,
        }));
        remoteProfileAutoAppliedKeyRef.current = remoteProfileKey(parsed.host, parsed.user ?? null);
      }

      // Ensure the daemon is reachable before we navigate away from the wizard.
      // This avoids landing on the workbench too early on cold start.
      await waitForDaemonReady(15000);
      const containerEnabled = selections.container !== "no-container";

      // Strict UX gate: if container mode is selected, ensure runtime+image readiness
      // before repository/workspace provisioning starts.
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

      // 2. Ensure we have a VCS repo root_path (workspace creation requires this).
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
            .map((workspace) => String((workspace as any).name ?? "").trim())
            .filter(Boolean);
        } catch {
          // Name collision avoidance is best-effort; do not block creation on listing failures.
          return [];
        }
      };
      let rootPath = "";
      let name: string | undefined;
      let wsId = "";
      if (selections.source === "import") {
        rootPath = sourcePath.trim().replace(/\/+$/, "");
        if (!rootPath) throw new Error("Folder is required.");
        let st = await repoStatus({ path: rootPath }).catch(() => null);
        if (!st) {
          setImportRepoStatus("error");
          setImportRepoNote("Could not verify the selected folder. Check the path and try again.");
          throw new Error("Could not verify the selected folder as a repository.");
        }
        if (st && !st.is_repo) {
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
          st = await repoStatus({ path: rootPath });
          if (!st.is_repo) {
            const detailAfter = String((st as any).error ?? "").trim();
            throw new Error(detailAfter ? `Selected folder is not a repo: ${detailAfter}` : "Selected folder is not a repo.");
          }
        }
        if (st?.canonical_path) {
          rootPath = String(st.canonical_path).trim() || rootPath;
        }
        setImportRepoStatus("ok");
        setImportRepoNote(null);
        // Prefer existing workspace if already registered.
        const all = await getAllWorkspaces();
        const hit = all.find((w) => String((w as any).root_path) === rootPath);
        if (hit) {
          wsId = idToString((hit as any).id);
        } else {
          name = workspaceName.trim() || undefined;
        }
      } else if (selections.source === "clone") {
        let dest_parent: string;
        let dest_name: string | null;
        if (useDiskIsolatedStaging) {
          const staging = await repoStagingPath();
          dest_parent = staging.path;
          dest_name = deriveRepoNameFromUrl(repoUrl) || workspaceName.trim() || null;
          if (!dest_name) throw new Error("Could not derive repo name from URL.");
        } else {
          const dest = parseCloneDestPath(sourcePath);
          if (!dest) throw new Error("Destination must be an absolute path (e.g. /Users/example-user/projects/ or /Users/example-user/projects/repo-name).");
          dest_parent = dest.dest_parent;
          dest_name = dest.dest_name ?? null;
        }
        const resp = await repoClone({
          repo_url: repoUrl.trim(),
          branch: repoBranch.trim() || null,
          dest_parent,
          dest_name,
        });
        rootPath = resp.path;
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
        // Allow existing empty directories (daemon still refuses non-empty dirs).
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

      // 3. Register the workspace.
      if (!wsId) {
        const created = await createWorkspace(rootPath, name);
        wsId = idToString((created as any).id);
      }

      // 4. Persist per-workspace execution settings.
      const environment = selections.container === "no-container"
        ? "host"
        : selections.container === "host-mounted"
          ? "container_host_mounted"
          : "container_disk_isolated";
      const allowlist = networkAllowlist
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean);
      const netMode = selections.network === "allowlist"
        ? "allowlist"
        : selections.network === "full"
          ? "all"
          : "llm_only";
      await updateWorkspaceExecutionConfig(wsId, {
        environment,
        network_mode: containerEnabled ? netMode : null,
        allowlist: containerEnabled && netMode === "allowlist" ? allowlist : null,
      });

      // If container execution is enabled, eagerly provision the workspace harness container so
      // we don't land in the workbench before the sandbox is actually ready.
      if (containerEnabled) {
        await waitForLaunchCompletion(wsId);
      }

      // 5. Persist local per-workspace settings only if the user opted in / provided values.
      if (!mergeQueueSkipped) {
        await updateWorkspaceMergeQueueConfig(wsId, {
          enabled: true,
          target_branch: targetBranch.trim() || null,
          verify_command: verifyCommand.trim() || null,
          push_on_success: pushOnSuccess ? true : null,
          push_remote: pushOnSuccess ? (pushRemote.trim() || "origin") : null,
          push_branch: pushOnSuccess ? (pushBranch.trim() || targetBranch.trim() || "main") : null,
        });
      }

      if (setupHook.trim()) {
        await updateWorkspaceWorktreeBootstrapConfig(wsId, { setup_command: setupHook.trim() });
      }

      // Final guard: ensure daemon is still reachable before navigating to the workbench.
      await waitForDaemonReady(15000);
      try {
        if (selections.location === "remote" && parsed?.host) {
          const normalizedDataDir = remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null;
          await upsertLauncherRecent({
            kind: "ssh",
            label: workspaceName.trim() || lastPathSegment(rootPath) || parsed.host,
            host: parsed.host,
            user: parsed.user ?? null,
            remote_port: parsedRemotePort ?? 4399,
            start_remote: true,
            remote_data_dir: normalizedDataDir,
            remote_ctx_bin: remoteCtxBinValue,
            updated_at_ms: Date.now(),
          });
        } else {
          await upsertLauncherRecent({
            kind: "local",
            label: lastPathSegment(rootPath) || rootPath,
            root_path: rootPath,
            updated_at_ms: Date.now(),
          });
        }
      } catch {
        // best-effort only; do not block workspace creation if recents persistence fails
      }
      navigate(`/workspaces/${wsId}`, { replace: true });
    } catch (e: any) {
      const msg = e?.message ?? String(e);
      setCreateError(msg);

      const stepKeyForError = (m: string): WizardStep["key"] | null => {
        const s = String(m || "");
        if (s.includes("Remote host is required")) return "location";
        if (s.includes("session titling") || s.includes("title generation")) return "session-titling";
        if (s.includes("repo_url") || s.includes("Destination") || s.includes("Folder") || s.includes("git clone") || s.includes("git init") || s.includes("root_path") || s.includes("not a repo")) {
          return "source";
        }
        return null;
      };
      const key = stepKeyForError(msg);
      if (key) {
        goToStepKey(key);
      }
    } finally {
      setCreating(false);
    }
  };

	  return (
	    <div className="launcher-shell launcher-shell--crt">
	      <LauncherBrand fullScreen>
		        <div
		          className="wizard-panel"
		          data-testid="workspace-setup"
		          data-step-key={step.key}
		        >
          {importInitDialog && (
            <div
              className="wizard-modal-backdrop"
              role="dialog"
              aria-modal="true"
              aria-label="Initialize Git repo"
              data-testid="wizard-import-init-modal"
              onClick={() => resolveImportInitDialog(false)}
            >
              <div className="wizard-modal" onClick={(e) => e.stopPropagation()}>
                <div className="wizard-modal-header">
                  <div className="wizard-modal-title">Initialize Git repo in this folder?</div>
                  <button
                    type="button"
                    className="wizard-modal-close"
                    aria-label="Close"
                    onClick={() => resolveImportInitDialog(false)}
                  >
                    <X size={16} aria-hidden="true" />
                  </button>
                </div>
                <div className="wizard-modal-body">
                  <div className="wizard-modal-copy">
                    The selected folder is not currently a repository.
                  </div>
                  <div className="wizard-modal-path">
                    <code>{importInitDialog.path}</code>
                  </div>
                  <div className="wizard-modal-note">
                    This will run <code>git init</code> and create one empty initial commit. Existing files are not staged or committed.
                  </div>
                  <div className="wizard-modal-actions">
                    <button
                      type="button"
                      className="wizard-secondary"
                      data-testid="wizard-import-init-cancel"
                      onClick={() => resolveImportInitDialog(false)}
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      className="wizard-primary"
                      data-testid="wizard-import-init-confirm"
                      onClick={() => resolveImportInitDialog(true)}
                    >
                      Initialize Git repo here
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}
	          {infoStep?.info && (
	            <div
	              className="wizard-modal-backdrop"
	              role="dialog"
	              aria-modal="true"
	              aria-label={`${infoStep.title} info`}
	              onClick={() => setOpenInfoKey(null)}
	            >
	              <div className="wizard-modal" onClick={(e) => e.stopPropagation()}>
	                <div className="wizard-modal-header">
	                  <div className="wizard-modal-title">{infoStep.title}</div>
	                  <button
	                    type="button"
	                    className="wizard-modal-close"
	                    aria-label="Close"
	                    onClick={() => setOpenInfoKey(null)}
	                  >
	                    <X size={16} aria-hidden="true" />
	                  </button>
	                </div>
	                <div className="wizard-modal-body">
	                  <div className="wizard-markdown">
	                    <ReactMarkdown remarkPlugins={[remarkGfm]}>
	                      {infoStep.info}
	                    </ReactMarkdown>
	                  </div>
	                </div>
	              </div>
	            </div>
	          )}
		          <div className="wizard-steps">
		            <div className="wizard-step" data-testid="wizard-step" data-step-key={step.key}>
	              <div className="wizard-step-header">
	                <div className="wizard-step-title-row">
	                  <div className="wizard-step-title">{step.title}</div>
	                  {step.info && (
	                    <button
	                      type="button"
	                      className="wizard-info-toggle"
	                      onClick={() => setOpenInfoKey((prev) => (prev === step.key ? null : step.key))}
	                      aria-label="Info"
	                    >
	                      <Info size={16} aria-hidden="true" />
	                    </button>
	                  )}
	                </div>
	                <div className="wizard-step-note">{step.note}</div>
	              </div>
	              <div className="wizard-step-body">
                  {createError && (
                    <div className="wizard-error">{createError}</div>
                  )}
                  {showLaunchPanel && launchSnapshot && (
                    <div className="wizard-launch-log-panel" data-testid="wizard-launch-log-panel">
                      <div className="wizard-launch-log-header">
                        <div>
                          <div className="wizard-launch-log-title">Workspace Launch Logs</div>
                          <div className="wizard-launch-log-meta">
                            <span>{currentLaunchPhaseLabel}</span>
                            <span>{currentLaunchElapsed}</span>
                            <span>{launchSnapshot.state}</span>
                          </div>
                        </div>
                        <button
                          type="button"
                          className="wizard-input-button"
                          onClick={onCopyLaunchDiagnostics}
                          data-testid="wizard-launch-copy"
                        >
                          {launchCopyLabel}
                        </button>
                      </div>
                      <div className="wizard-launch-log-body">
                        {launchLogs.length === 0 ? (
                          <div className="wizard-note">Waiting for launch logs…</div>
                        ) : (
                          launchLogs.map((line) => (
                            <div key={line.seq} className="wizard-launch-log-line">
                              <span className="wizard-launch-log-ts">{formatLaunchTime(line.ts)}</span>
                              <span className="wizard-launch-log-phase">{launchPhaseLabel(line.phase)}</span>
                              <span className={`wizard-launch-log-level wizard-launch-log-level--${line.level}`}>{line.level}</span>
                              <span className="wizard-launch-log-msg">{line.message}</span>
                            </div>
                          ))
                        )}
                      </div>
                    </div>
                  )}
                {step.options && (
                  <>
	                    <div className="wizard-option-grid">
	                      {step.options
	                        .filter((option) => step.key !== "container" || !option.advanced)
	                        .map((option) => {
	                          const selected = selections[step.key] === option.id;
	                          return (
	                            <button
	                              key={option.id}
	                              type="button"
	                              className={`wizard-option${selected ? " is-selected" : ""}`}
	                              data-testid={`wizard-option-${step.key}-${option.id}`}
	                              onClick={() => onSelectOption(step.key, option.id)}
	                              aria-pressed={selected}
	                            >
                              <div className="wizard-option-title">
                                <span className="wizard-option-title-text">{option.title}</span>
                                {option.badge && <span className="wizard-option-badge">{option.badge}</span>}
                              </div>
                              <div className="wizard-option-desc">{option.desc}</div>
                            </button>
                          );
                        })}
                    </div>
                    {step.key === "container" && (
                      <button
                        type="button"
                        className="wizard-advanced-link"
                        data-testid="wizard-container-advanced-toggle"
                        onClick={() => setContainerAdvancedOpen((open) => !open)}
                        aria-expanded={containerAdvancedOpen}
                      >
                        <ChevronRight
                          size={14}
                          className={containerAdvancedOpen ? "is-open" : undefined}
                          aria-hidden="true"
                        />
                        Advanced
                      </button>
                    )}
                    {step.key === "container" && containerAdvancedOpen && (
                      <div className="wizard-container-advanced">
                        <div className="wizard-option-grid wizard-option-grid--two">
                          {step.options
                            .filter((option) => Boolean(option.advanced))
                            .map((option) => {
                              const selected = selections[step.key] === option.id;
                              return (
                                <button
                                  key={option.id}
                                  type="button"
                                  className={`wizard-option${selected ? " is-selected" : ""}`}
                                  data-testid={`wizard-option-${step.key}-${option.id}`}
                                  onClick={() => onSelectOption(step.key, option.id)}
                                  aria-pressed={selected}
                                >
                                  <div className="wizard-option-title">
                                    <span className="wizard-option-title-text">{option.title}</span>
                                    {option.badge && <span className="wizard-option-badge">{option.badge}</span>}
                                  </div>
                                  <div className="wizard-option-desc">{option.desc}</div>
                                </button>
                              );
                            })}
                        </div>
                      </div>
                    )}
	                  </>
	                )}
                {step.key === "network" && selections.network === "allowlist" && (
                  <div className="wizard-input">
                    <label>
                      Allowed hosts (one per line)
                      <textarea
                        data-testid="wizard-network-allowlist"
                        placeholder={"github.com\nregistry.npmjs.org\npypi.org"}
                        value={networkAllowlist}
                        onChange={(e) => {
                          setCreateError(null);
                          setNetworkAllowlist(e.target.value);
                        }}
                        rows={6}
                      />
                    </label>
                  </div>
                )}
                {step.key === "location" && selections.location === "remote" && (
                  <div className="wizard-remote">
                    <div className="wizard-input">
	                      <label>
	                        Remote host
	                        <input
	                          data-testid="wizard-remote-host"
	                          placeholder="user@host"
	                          value={remoteHostInput}
	                          onChange={(e) => onRemoteInputChange(e.target.value)}
	                        />
	                      </label>
                    </div>
                    <button
                      type="button"
                      className="wizard-advanced-toggle"
                      data-testid="wizard-remote-advanced-toggle"
                      onClick={() => setRemoteAdvancedOpen((v) => !v)}
                    >
                      <span className="wizard-advanced-toggle-icon" aria-hidden="true">
                        {remoteAdvancedOpen ? "▾" : "▸"}
                      </span>
                      Advanced
                    </button>
                    {remoteAdvancedOpen && (
                      <div className="wizard-advanced">
                        <div className="wizard-input">
                          <label>
                            Remote daemon port
                            <input
                              data-testid="wizard-remote-port"
                              placeholder="4399"
                              value={remotePortInput}
                              onChange={(e) => {
                                setRemotePortInput(e.target.value);
                                if (remoteStatus !== "idle") {
                                  setRemoteStatus("idle");
                                  setRemoteError(null);
                                }
                              }}
                            />
                          </label>
                          <div className="wizard-note">
                            Use a non-default port for E2E/shared hosts so tests do not touch an existing daemon.
                          </div>
                        </div>
                        <div className="wizard-input">
                          <label>
                            Remote ctx binary path
                            <input
                              data-testid="wizard-remote-ctx-bin"
                              placeholder="/opt/ctx/bin/ctx"
                              value={remoteCtxBinInput}
                              onChange={(e) => {
                                setRemoteCtxBinInput(e.target.value);
                                if (remoteStatus !== "idle") {
                                  setRemoteStatus("idle");
                                  setRemoteError(null);
                                }
                              }}
                            />
                          </label>
                          <div className="wizard-note">
                            Required absolute path used when starting a remote daemon.
                          </div>
                        </div>
                        <div className="wizard-input">
                          <label>
                            Remote data dir (optional)
                            <input
                              data-testid="wizard-remote-data-dir"
                              placeholder="/tmp/ctx-e2e-123/daemon"
                              value={remoteDataDirInput}
                              onChange={(e) => {
                                setRemoteDataDirInput(e.target.value);
                                if (remoteStatus !== "idle") {
                                  setRemoteStatus("idle");
                                  setRemoteError(null);
                                }
                              }}
                            />
                          </label>
                          <div className="wizard-note">
                            Leave blank for remote defaults. Set for isolated test state.
                          </div>
                        </div>
                      </div>
                    )}
                    {sshSuggestions.length > 0 && (
                      <div className="wizard-remote-list">
                        {sshSuggestions.map((entry) => {
                          const label = entry.user ? `${entry.user}@${entry.host}` : entry.host;
                          return (
                            <button
                              key={label}
                              type="button"
                              className="wizard-remote-suggestion"
                              onClick={() => onRemoteInputChange(label)}
                            >
                              {label}
                            </button>
                          );
                        })}
                      </div>
                    )}
                    {remoteStatus === "connecting" && (
                      <div className="wizard-note">Connecting…</div>
                    )}
                    {remoteStatus === "connected" && (
                      <div className="wizard-note">Connection verified.</div>
                    )}
                    {remoteStatus === "error" && remoteError && (
                      <div className="wizard-error">{remoteError}</div>
                    )}
                  </div>
                )}
                {step.key === "auth-import" && (
                  <div className="wizard-input">
                    {authImportBusy ? <div className="wizard-note">Scanning/importing credentials…</div> : null}
                    {authImportError ? <div className="wizard-error">{authImportError}</div> : null}
                    {!authImportBusy && !authImportCandidates.length ? (
                      <div className="wizard-note">No import candidates found on this host.</div>
                    ) : null}
                    {authImportCandidates.length > 0 && (
                      <div className="wizard-auth-import-list">
                        {authImportCandidates.map((candidate) => {
                          const checked = Boolean(authImportSelected[candidate.id]);
                          const importable = candidate.parse_status === "parsed";
                          const harness = harnessByProviderId.get(candidate.provider_id);
                          const showSummary = candidate.summary
                            && !(candidate.provider_id === "codex" && candidate.summary === "Codex auth session");
                          const showUnsupportedReason = candidate.unsupported_reason
                            && !(
                              candidate.provider_id === "cursor"
                              && candidate.unsupported_reason.includes("secure local storage without a canonical import file path")
                            );
                          return (
                            <label
                              key={candidate.id}
                              className={`wizard-auth-import-row ${importable ? "" : "wizard-auth-import-row--disabled"}`}
                            >
                              <div className="wizard-auth-import-title">
                                <input
                                  type="checkbox"
                                  className="wizard-auth-import-checkbox"
                                  checked={checked}
                                  disabled={!importable || authImportBusy}
                                  onChange={(e) =>
                                    setAuthImportSelected((prev) => ({ ...prev, [candidate.id]: e.target.checked }))
                                  }
                                />
                                {harness?.logoSrc ? (
                                  <img
                                    className={logoClasses(
                                      "wizard-auth-import-logo",
                                      harness.invertInDark,
                                      harness.invertInLight,
                                    )}
                                    src={harness.logoSrc}
                                    alt=""
                                  />
                                ) : (
                                  <span className="wizard-auth-import-logo-fallback" aria-hidden="true" />
                                )}
                                <span className="wizard-auth-import-name">{candidate.provider_label}</span>
                              </div>
                              <div className="wizard-auth-import-path">Source: {candidate.path}</div>
                              {showSummary ? <div className="wizard-note wizard-note--tight">{candidate.summary}</div> : null}
                              {showUnsupportedReason ? (
                                <div className="wizard-note wizard-note--tight">{candidate.unsupported_reason}</div>
                              ) : null}
                            </label>
                          );
                        })}
                      </div>
                    )}
                    <button
                      type="button"
                      className="wizard-skip wizard-skip--left wizard-skip--below"
                      onClick={() => {
                        void advanceFromAuthImportStep({ clearSelections: true });
                      }}
                      disabled={authImportBusy}
                    >
                      Skip for now
                    </button>
                  </div>
                )}
                {step.key === "session-titling" && (
                  <div className="wizard-input">
                    {titlingProbeBusy ? (
                      <div className="wizard-note">Checking session titling configuration on this daemon…</div>
                    ) : null}
                    {titlingProbeError ? (
                      <div className="wizard-error">
                        Could not auto-detect titling configuration. You can still configure now or skip. ({titlingProbeError})
                      </div>
                    ) : null}
                    {titlingPersistError ? <div className="wizard-error">{titlingPersistError}</div> : null}
                    {titlingStatusError ? <div className="wizard-error">{titlingStatusError}</div> : null}
                    <div className="wizard-option-grid wizard-option-grid--two">
                      <button
                        type="button"
                        className={`wizard-option${titlingMode === "remote" ? " is-selected" : ""}`}
                        data-testid="wizard-titling-mode-remote"
                        onClick={() => {
                          invalidateTitlingPersisted();
                          setTitlingMode("remote");
                        }}
                        disabled={titlingLocalInstallBusy || titlingPersistBusy}
                        aria-pressed={titlingMode === "remote"}
                      >
                        <div className="wizard-option-title">
                          <span className="wizard-option-title-text">Remote model</span>
                        </div>
                        <div className="wizard-option-desc">
                          Use a cloud endpoint with API key + model for title generation.
                        </div>
                      </button>
                      <button
                        type="button"
                        className={`wizard-option${titlingMode === "local" ? " is-selected" : ""}`}
                        data-testid="wizard-titling-mode-local"
                        onClick={() => {
                          void onSelectTitlingLocal();
                        }}
                        disabled={titlingLocalInstallBusy || titlingPersistBusy}
                        aria-pressed={titlingMode === "local"}
                      >
                        <div className="wizard-option-title">
                          <span className="wizard-option-title-text">Local model</span>
                        </div>
                        <div className="wizard-option-desc">
                          Run titling on-daemon. Download can continue in background.
                        </div>
                      </button>
                    </div>
                    {titlingMode === "remote" && (
                      <div className="wizard-input">
                        <label>
                          Endpoint base URL
                          <input
                            data-testid="wizard-titling-remote-base-url"
                            placeholder="https://openrouter.ai/api/v1"
                            value={titlingRemoteBaseUrl}
                            onChange={(e) => {
                              invalidateTitlingPersisted();
                              setTitlingRemoteBaseUrl(e.target.value);
                            }}
                          />
                        </label>
                        <label>
                          API key
                          <input
                            data-testid="wizard-titling-remote-api-key"
                            placeholder="sk-..."
                            value={titlingRemoteApiKey}
                            type="password"
                            onChange={(e) => {
                              invalidateTitlingPersisted();
                              setTitlingRemoteApiKey(e.target.value);
                            }}
                          />
                        </label>
                        <label>
                          Model
                          <input
                            data-testid="wizard-titling-remote-model"
                            placeholder="google/gemini-3-flash-preview"
                            value={titlingRemoteModel}
                            onChange={(e) => {
                              invalidateTitlingPersisted();
                              setTitlingRemoteModel(e.target.value);
                            }}
                          />
                        </label>
                        <button
                          type="button"
                          className="wizard-advanced-link"
                          data-testid="wizard-titling-remote-advanced-toggle"
                          onClick={() => setTitlingRemoteAdvancedOpen((open) => !open)}
                          aria-expanded={titlingRemoteAdvancedOpen}
                        >
                          <ChevronRight
                            size={14}
                            className={titlingRemoteAdvancedOpen ? "is-open" : undefined}
                            aria-hidden="true"
                          />
                          Advanced
                        </button>
                        {titlingRemoteAdvancedOpen && (
                          <label className="wizard-checkbox">
                            <input
                              data-testid="wizard-titling-remote-use-json"
                              type="checkbox"
                              checked={titlingRemoteUseJson}
                              onChange={(e) => {
                                invalidateTitlingPersisted();
                                setTitlingRemoteUseJson(e.target.checked);
                              }}
                            />
                            Prefer JSON response format
                          </label>
                        )}
                      </div>
                    )}
                    <button
                      type="button"
                      className="wizard-skip wizard-skip--left wizard-skip--below"
                      data-testid="wizard-titling-skip"
                      onClick={() => {
                        invalidateTitlingPersisted();
                        setTitlingMode("skip");
                        goRelativeStep(1);
                      }}
                      disabled={titlingPersistBusy || titlingLocalInstallBusy}
                    >
                      Skip for now
                    </button>
                  </div>
                )}
                {step.key === "source" && needsSourcePath && (
                  <div className="wizard-input">
                    <label>
                      {selections.source === "import"
                        ? "Existing folder"
                        : "Destination folder (host)"}
	                      <div className="wizard-input-row">
	                        <input
	                          data-testid="wizard-source-path"
	                          placeholder={
	                            selections.source === "import"
	                              ? "/Users/example-user/project"
                              : "/Users/example-user/projects/"
                          }
                          value={sourcePath}
                          onChange={(e) => {
                            setCreateError(null);
                            setSourcePath(e.target.value);
                          }}
                        />
                        {selections.location === "local" && (
                          <button
                            type="button"
                            className="wizard-input-button"
                            onClick={onPickLocalFolder}
                          >
                            Browse
                          </button>
                        )}
                      </div>
	                    </label>
                    {selections.container !== "no-container" && (
                      <div className="wizard-note">
                        This is the project folder on the host. In disk-isolated mode, ctx will copy the workspace into a container-managed filesystem for execution.
                      </div>
                    )}
	                    {selections.source === "import" && importRepoStatus !== "idle" && importRepoNote && (
	                      <div className={importRepoStatus === "error" ? "wizard-error" : "wizard-note"}>
	                        {importRepoNote}
	                      </div>
	                    )}
                    {selections.location === "remote" && remotePathSuggestions.length > 0 && (
                      <div className="wizard-path-list">
                        {remotePathSuggestions.map((entry) => (
                          <button
                            key={entry.path}
                            type="button"
                            className="wizard-path-suggestion"
                            onClick={() => setSourcePath(`${entry.path}/`)}
                          >
                            {entry.name}
                          </button>
                        ))}
                      </div>
                    )}
                    {selections.location === "remote" && remotePathStatus === "loading" && (
                      <div className="wizard-note">Loading folders…</div>
                    )}
                    {selections.location === "remote" && remotePathStatus === "error" && remotePathError && (
                      <div className="wizard-error">{remotePathError}</div>
                    )}
                  </div>
                )}
                {step.key === "source" && selections.source === "clone" && (
                  <div className="wizard-input">
                    <label>
	                      Repo URL
		                      <input
		                        data-testid="wizard-repo-url"
		                        placeholder="https://github.com/org/repo.git"
		                        value={repoUrl}
		                        onChange={(e) => {
                            setCreateError(null);
                            setRepoUrl(e.target.value);
                          }}
	                      />
	                    </label>
	                    <label>
		                      Branch (optional)
		                      <input
		                        data-testid="wizard-repo-branch"
		                        placeholder="main"
		                        value={repoBranch}
		                        onChange={(e) => {
                            setCreateError(null);
                            setRepoBranch(e.target.value);
                          }}
	                      />
	                    </label>
                    {useDiskIsolatedStaging && (
                      <div className="wizard-note">
                        Ctx will clone into a managed staging path. Your workspace will live in the container.
                      </div>
                    )}
                    {!useDiskIsolatedStaging && (
                      <div className="wizard-note">
                        Tip: If you enter a folder ending in <code>/</code>, ctx will derive the repo name from the URL.
                      </div>
                    )}
	                  </div>
	                )}
                {step.key === "setup" && (
		                  <div className="wizard-input">
		                    <input
		                      data-testid="wizard-setup-hook"
		                      placeholder="./prepare-worktree.sh"
		                      value={setupHook}
		                      onChange={(e) => {
                          setCreateError(null);
                          setSetupHook(e.target.value);
                        }}
	                    />
	                  </div>
	                )}
                {step.key === "source" && (selections.source === "new" || selections.source === "import") && (
                  <div className="wizard-input">
	                    <label>
	                      Workspace name (optional)
		                      <input
		                        data-testid="wizard-workspace-name"
		                        placeholder="workspace"
		                        value={workspaceName}
		                        onChange={(e) => {
                            setCreateError(null);
                            setWorkspaceName(e.target.value);
                          }}
	                      />
		                    </label>
                    {selections.source === "new" && useDiskIsolatedStaging && (
                      <div className="wizard-note">
                        Ctx will create the repo in a managed staging path. Your workspace will live in the container.
                      </div>
                    )}
	                  </div>
	                )}
                {step.key === "merge-queue" && (
	                  <div className="wizard-input">
                    <label>
	                      Target branch
	                      <input
	                        data-testid="wizard-merge-target-branch"
	                        placeholder="main"
		                        value={targetBranch}
		                        onChange={(e) => {
                            setCreateError(null);
	                          enableMergeQueueIfSkipped();
	                          setTargetBranch(e.target.value);
	                          setTargetBranchTouched(true);
	                        }}
                        disabled={mergeQueueSkipped}
                      />
                    </label>
                    <label>
	                      Verification command (optional)
	                  <input
	                        data-testid="wizard-merge-verify-command"
	                        placeholder="./verify.sh"
		                        value={verifyCommand}
		                        onChange={(e) => {
                            setCreateError(null);
	                          enableMergeQueueIfSkipped();
	                          setVerifyCommand(e.target.value);
	                        }}
                        disabled={mergeQueueSkipped}
                      />
                    </label>
	                    <button
	                      type="button"
	                      className="wizard-advanced-link"
	                      data-testid="wizard-merge-advanced-toggle"
	                      onClick={() => setMergeAdvancedOpen((open) => !open)}
	                      aria-expanded={mergeAdvancedOpen}
	                      disabled={mergeQueueSkipped}
                    >
                      <ChevronRight
                        size={14}
                        className={mergeAdvancedOpen ? "is-open" : undefined}
                        aria-hidden="true"
                      />
                      Advanced
                    </button>
                    {mergeAdvancedOpen && (
                      <div className="wizard-advanced-panel">
	                        <label className="wizard-checkbox">
	                          <input
		                            data-testid="wizard-merge-push-on-success"
		                            type="checkbox"
		                            checked={pushOnSuccess}
		                            onChange={(e) => {
                                setCreateError(null);
                                setPushOnSuccess(e.target.checked);
                              }}
	                            disabled={mergeQueueSkipped}
	                          />
                          Push to remote on success
                        </label>
                        {pushOnSuccess && (
                          <div className="wizard-input">
                            <label>
	                              Push remote
	                              <input
		                                data-testid="wizard-merge-push-remote"
		                                placeholder="origin"
		                                value={pushRemote}
		                                onChange={(e) => {
                                    setCreateError(null);
                                    setPushRemote(e.target.value);
                                  }}
	                                disabled={mergeQueueSkipped}
	                              />
                            </label>
                            <label>
	                              Push branch
	                              <input
		                                data-testid="wizard-merge-push-branch"
	                                placeholder={targetBranch || "main"}
		                                value={pushBranch}
		                                onChange={(e) => {
	                                  setCreateError(null);
	                                  setPushBranch(e.target.value);
	                                  setPushBranchTouched(true);
	                                }}
	                                disabled={mergeQueueSkipped}
	                              />
                            </label>
                          </div>
                        )}
                      </div>
                    )}
	                    <button
	                      type="button"
	                      className="wizard-skip wizard-skip--left wizard-skip--below"
	                      data-testid="wizard-merge-skip"
	                      onClick={() => {
                        onSelect("merge-queue", "skip");
                        setMergeAdvancedOpen(false);
                        setPushOnSuccess(false);
                        goRelativeStep(1);
                      }}
                    >
                      Skip for now
                    </button>
	                  </div>
	                )}
                {step.key === "confirm" && (
                  <div className="wizard-step-summary">
                    <div className="wizard-summary">
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Location</div>
                        <div className="wizard-summary-v">
                          {selections.location === "remote" ? "Remote" : "Local"}
                          {selections.location === "remote" && remoteHostInput.trim()
                            ? ` (${remoteHostInput.trim()})`
                            : ""}
                        </div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Source</div>
                        <div className="wizard-summary-v">
                          {selections.source === "clone"
                            ? `Clone repo${repoBranch.trim() ? ` (${repoBranch.trim()})` : ""}`
                            : selections.source === "import"
                              ? "Import folder"
                              : "New empty"}
                        </div>
                      </div>
                      {selections.source === "clone" && repoUrl.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Repo</div>
                          <div className="wizard-summary-v">{repoUrl.trim()}</div>
                        </div>
                      )}
                      {selections.source === "clone" && (sourcePath.trim() || useDiskIsolatedStaging) && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Destination</div>
                          <div className="wizard-summary-v">
                            {useDiskIsolatedStaging ? "Managed staging (container)" : sourcePath.trim()}
                          </div>
                        </div>
                      )}
                      {selections.source === "import" && sourcePath.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Folder</div>
                          <div className="wizard-summary-v">{sourcePath.trim()}</div>
                        </div>
                      )}
                      {selections.source === "new" && (sourcePath.trim() || useDiskIsolatedStaging) && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Destination</div>
                          <div className="wizard-summary-v">
                            {useDiskIsolatedStaging ? "Managed staging (container)" : sourcePath.trim()}
                          </div>
                        </div>
                      )}
                      {(selections.source === "new" || selections.source === "import") && workspaceName.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Name</div>
                          <div className="wizard-summary-v">{workspaceName.trim()}</div>
                        </div>
                      )}
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Sandbox</div>
                        <div className="wizard-summary-v">
                          {selections.container === "no-container"
                            ? "Host (no container)"
                            : selections.container === "host-mounted"
                              ? "Container (host-mounted)"
                              : "Container (disk-isolated)"}
                        </div>
                      </div>
                      {selections.container !== "no-container" && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Network</div>
                          <div className="wizard-summary-v">
                            {selections.network === "allowlist"
                              ? "Allowlist"
                              : selections.network === "full"
                                ? "Full access"
                                : "LLM providers only"}
                          </div>
                        </div>
                      )}
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Session titling</div>
                        <div className="wizard-summary-v">{titlingSummaryValue}</div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Worktree hook</div>
                        <div className="wizard-summary-v">{setupHook.trim() || "(none)"}</div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Merge queue</div>
                        <div className="wizard-summary-v">
                          {mergeQueueSkipped
                            ? "Disabled"
                            : `Target ${targetBranch.trim() || "main"}${verifyCommand.trim() ? `, verify: ${verifyCommand.trim()}` : ""}`}
                        </div>
                      </div>
                      {!mergeQueueSkipped && pushOnSuccess && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Merge push</div>
                          <div className="wizard-summary-v">
                            {`${(pushRemote.trim() || "origin")}:${pushBranch.trim() || targetBranch.trim() || "main"}`}
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                )}
                {step.key === "setup" && (
                  <button
                    type="button"
                    className="wizard-skip wizard-skip--left wizard-skip--below"
	                    onClick={onNext}
	                  >
	                    Skip for now
	                  </button>
	                )}
              </div>
            </div>
          </div>

          <div className="wizard-pagination" role="tablist" aria-label="Setup steps">
            {(() => {
              const isStepSatisfied = (key: string): boolean => {
                if (key === "location") {
                  if (selections.location === "local") return true;
                  if (selections.location !== "remote") return false;
                  return remoteStatus === "connected" && Boolean(parseUserHost(remoteHostInput)?.host);
                }
                if (key === "auth-import") return true;
                if (key === "session-titling") {
                  return titlingMode === "skip"
                    || titlingMode === "local"
                    || (titlingMode === "remote" && titlingRemoteValid);
                }
                if (key === "source") {
                  return sourceStepValidation.isComplete;
                }
                if (key === "merge-queue") return mergeQueueSkipped || Boolean(targetBranch.trim());
                if (key === "setup") return true;
                if (key === "confirm") return true;
                return true;
              };

              let maxIdx = 0;
              for (let i = 0; i < steps.length; i++) {
                if (isStepSatisfied(steps[i].key)) {
                  maxIdx = Math.min(steps.length - 1, i + 1);
                } else {
                  maxIdx = Math.max(0, i);
                  break;
                }
              }

              return steps.map((item, idx) => {
                const disabled = idx > maxIdx;
                return (
                  <button
                    key={item.key}
                    type="button"
                    className={`wizard-dot${idx === stepIndex ? " is-active" : ""}`}
                    aria-label={`Go to step ${idx + 1}`}
                    aria-current={idx === stepIndex ? "true" : undefined}
                    disabled={disabled}
                    onClick={() => {
                      if (!disabled) goToStepKey(item.key);
                    }}
                  />
                );
              });
            })()}
          </div>

	          <div className="wizard-actions">
	            {isFirst ? (
	              <Link to="/" className="wizard-secondary" data-testid="wizard-back-link">
	                Back
	              </Link>
	            ) : (
	              <button
	                type="button"
	                className="wizard-secondary"
	                data-testid="wizard-back"
	                onClick={() => goRelativeStep(-1)}
	              >
	                Back
	              </button>
	            )}
	            {isLast ? (
	              <button
	                type="button"
	                className="wizard-primary"
	                data-testid="wizard-create"
	                disabled={!canAdvance || creating}
	                onClick={onCreate}
	              >
	                {createButtonLabel}
	              </button>
	            ) : (
	              <button
	                type="button"
	                className="wizard-primary"
	                data-testid="wizard-next"
	                disabled={!canAdvance || creating}
	                onClick={onNext}
	              >
	                Next
              </button>
            )}
          </div>
        </div>
      </LauncherBrand>
    </div>
		  );
}

function lastPathSegment(path: string): string {
  const normalized = String(path || "").trim().replace(/\/+$/, "");
  if (!normalized) return "";
  const idx = normalized.lastIndexOf("/");
  return idx >= 0 ? normalized.slice(idx + 1) : normalized;
}
