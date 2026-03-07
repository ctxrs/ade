import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, useNavigate } from "react-router-dom";
import { ChevronRight, Info, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import LauncherBrand from "../components/LauncherBrand";
import {
  applyDaemonDesktopConnection,
  buildExecutionLaunchWsUrl,
  cancelInstall,
  createWorkspace,
  getInstall,
  getExecutionLaunchStatus,
  getHealth,
  installProvider,
  listProviders,
  getSettings,
  getTitleGenerationLocalStatus,
  importProviderAuthCandidates,
  idToString,
  installTitleGenerationLocal,
  listProviderAuthImportCandidates,
  listWorkspaces,
  startExecutionLaunch,
  startExecutionRuntimePrewarm,
  type InstallInfo,
  type InstallTarget,
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
import type { ProviderStatus } from "@ctx/types";
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
  isCurrentFlowRunToken,
  nextFlowRunToken,
  type FlowRunToken,
} from "./workspaceSetup/flowController";
import {
  buildWizardStepPath,
  nextAfterAuthImport,
  nextAfterHarnessDownloads,
  nextBoundaryStep,
  resolveWizardCurrentStepKey,
  stepKeyOffset,
  type WizardRoutePlan,
  type WizardStepKey,
} from "./workspaceSetup/wizardFlow";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import {
  computeInstallPct,
  formatByteSize,
  installErrorSummary,
  installTargetLabel,
  parseInstallTarget,
  providerInstallSizeBytes,
} from "../utils/providerInstallUi";
import {
  trackWizardAbandoned,
  trackWizardCompleted,
  trackWizardStarted,
  trackWizardStepViewed,
} from "../utils/analytics";
import { upsertLauncherRecent } from "../state/launcherRecentsStore";
import { upsertProviderInstallProgress } from "../state/providerInstallProgressStore";

type WizardOption = {
  id: string;
  title: string;
  desc: string;
  badge?: string;
  advanced?: boolean;
};

type WizardStep = {
  key: WizardStepKey;
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
  state: InstallInfo["state"];
  pct: number | null;
  errorCode?: InstallInfo["error_code"];
  error?: string;
};

type HarnessInstallProviderRow = {
  providerId: string;
  label: string;
  installed: boolean;
  healthy: boolean;
  installSupported: boolean;
  installRunning: boolean;
  installId?: string;
  installTarget?: InstallTarget;
  installSizeBytes?: number | null;
};

type HarnessInstallRowState = {
  installId: string;
  state: InstallInfo["state"];
  pct: number | null;
  target?: InstallTarget;
  errorCode?: InstallInfo["error_code"];
  error?: string;
};

type HarnessInstallCandidateStatus =
  | "installed"
  | "running"
  | "ready_to_start"
  | "succeeded"
  | "failed"
  | "cancelled";

function resolveHarnessInstallCandidateStatus(
  candidate: HarnessInstallProviderRow,
  installUi?: HarnessInstallRowState,
): HarnessInstallCandidateStatus {
  if (candidate.installed && candidate.healthy) return "installed";
  if (installUi?.state === "running" || candidate.installRunning) return "running";
  if (installUi?.state === "failed") return "failed";
  if (installUi?.state === "cancelled") return "cancelled";
  if (installUi?.state === "succeeded") return "succeeded";
  return "ready_to_start";
}

const looksLikeSshAuthFailure = (message: string): boolean => {
  const lowered = message.toLowerCase();
  return lowered.includes("permission denied")
    || lowered.includes("publickey")
    || lowered.includes("authentication failed")
    || lowered.includes("too many authentication failures");
};

export default function WorkspaceSetupPage() {
  const navigate = useNavigate();
  const [currentStepKey, setCurrentStepKey] = useState<WizardStepKey>("location");
  const [selections, setSelections] = useState<Record<string, string>>({});
  const [sshHosts, setSshHosts] = useState<DesktopSshHost[]>([]);
  const [sshRecents, setSshRecents] = useState<SshRecent[]>(() => loadSshRecents());
  const [remoteHostInput, setRemoteHostInput] = useState("");
  const [remotePasswordInput, setRemotePasswordInput] = useState("");
  const [remotePasswordPromptVisible, setRemotePasswordPromptVisible] = useState(false);
  const [remoteStatus, setRemoteStatus] = useState<"idle" | "connecting" | "connected" | "error">("idle");
  const [remoteError, setRemoteError] = useState<string | null>(null);
  const [remotePortInput, setRemotePortInput] = useState("4399");
  const [remoteDataDirInput, setRemoteDataDirInput] = useState("");
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
  const [harnessInstallCandidates, setHarnessInstallCandidates] = useState<HarnessInstallProviderRow[]>([]);
  const [harnessInstallSelected, setHarnessInstallSelected] = useState<Record<string, boolean>>({});
  const [harnessInstallBusy, setHarnessInstallBusy] = useState(false);
  const [harnessInstallError, setHarnessInstallError] = useState<string | null>(null);
  const [harnessInstallScannedKey, setHarnessInstallScannedKey] = useState<string | null>(null);
  const [harnessInstallRows, setHarnessInstallRows] = useState<Record<string, HarnessInstallRowState>>({});
  const [harnessDownloadsCanScroll, setHarnessDownloadsCanScroll] = useState(false);
  const [harnessDownloadsAtBottom, setHarnessDownloadsAtBottom] = useState(true);
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
  const [routePlan, setRoutePlan] = useState<WizardRoutePlan | null>(null);
  const [routePlanningBusy, setRoutePlanningBusy] = useState(false);
  const importInitResolveRef = useRef<((confirmed: boolean) => void) | null>(null);
  const remoteProfileAutoAppliedKeyRef = useRef<string | null>(null);
  const titlingInstallPollRef = useRef<number | null>(null);
  const titlingInstallPollGenerationRef = useRef(0);
  const harnessInstallPollTimeoutsRef = useRef<Record<string, number>>({});
  const harnessDownloadsScrollRef = useRef<HTMLDivElement | null>(null);
  const selectedDaemonTargetKeyRef = useRef<string | null>(null);
  const authImportScanPromiseRef = useRef<Promise<ProviderAuthImportCandidate[]> | null>(null);
  const authImportScanKeyRef = useRef<string | null>(null);
  const authImportScanRunRef = useRef<FlowRunToken | null>(null);
  const harnessInstallScanPromiseRef = useRef<Promise<HarnessInstallProviderRow[]> | null>(null);
  const harnessInstallScanKeyRef = useRef<string | null>(null);
  const harnessInstallScanRunRef = useRef<FlowRunToken | null>(null);
  const containerPrewarmInFlightRef = useRef<Record<string, true>>({});
  const routePlanRunRef = useRef<FlowRunToken | null>(null);
  const currentStepKeyRef = useRef<WizardStepKey>("location");
  const previousStepIndexRef = useRef(0);
  const wizardStartedRef = useRef(false);
  const wizardCompletedRef = useRef(false);
  const lastWizardStepViewedRef = useRef<{ key: string; index: number } | null>(null);
  const titlingProbePromiseRef = useRef<Promise<boolean | null> | null>(null);
  const titlingProbePromiseTargetKeyRef = useRef<string | null>(null);
  const remoteStatusRef = useRef(remoteStatus);
  const wizardKey = "workspace_setup" as const;
  const harnessByProviderId = useMemo(() => {
    return new Map(HARNESS_CATALOG.map((entry) => [entry.id, entry]));
  }, []);

  const logoClasses = (base: string, invertInDark?: boolean, invertInLight?: boolean): string =>
    [base, invertInDark ? "wb-invert" : "", invertInLight ? "wb-invert-light" : ""]
      .filter(Boolean)
      .join(" ");

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

  const containerMode = selections.container;
  const authImportStepVisible = Boolean(routePlan?.includeAuthImport);
  const harnessInstallStepVisible = Boolean(routePlan?.includeHarnessDownloads);
  const titlingStepVisible = Boolean(routePlan?.includeTitling);

  const stepKeys = useMemo<WizardStepKey[]>(
    () => buildWizardStepPath({
      containerSelection: containerMode,
      routePlan,
      currentStepKey,
    }),
    [containerMode, currentStepKey, routePlan],
  );

  const steps = useMemo<WizardStep[]>(() => {
    const stepMap: Record<WizardStepKey, WizardStep> = {
      "location": {
        key: "location",
        title: "Location",
        note: "Where will this workspace run?",
        options: [
          { id: "local", title: "Local", desc: "Agents run on this machine." },
          { id: "remote", title: "Remote", desc: "Agents run on your existing dev box (remote IDE experience)." },
        ],
      },
      "container": {
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
      "harness-downloads": {
        key: "harness-downloads",
        title: "Harness Downloads",
        note: "Choose which harness providers to download now.",
      },
      "auth-import": {
        key: "auth-import",
        title: "Import Existing Auth",
        note: "Import existing provider credentials or add them later.",
      },
      "session-titling": {
        key: "session-titling",
        title: "Task Titling",
        note: "Choose an LLM source for generating task titles.",
      },
      "source": {
        key: "source",
        title: "Source",
        note: "How should we create the workspace?",
        options: [
          { id: "clone", title: "Clone repo", desc: "Git URL + optional branch." },
          { id: "import", title: "Import folder", desc: "Use an existing git repo folder path." },
          { id: "new", title: "New empty", desc: "Initialize a new git repo." },
        ],
      },
      "network": {
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
      },
      "setup": {
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
      "merge-queue": {
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
      "confirm": {
        key: "confirm",
        title: "Confirm and create",
        note: "Review your choices before provisioning.",
      },
    };

    return stepKeys.map((key) => stepMap[key]);
  }, [stepKeys]);

  const resolvedCurrentStepKey = useMemo<WizardStepKey>(
    () => resolveWizardCurrentStepKey(stepKeys, currentStepKey, previousStepIndexRef.current),
    [currentStepKey, stepKeys],
  );

  useEffect(() => {
    if (resolvedCurrentStepKey === currentStepKey) return;
    setCurrentStepKey(resolvedCurrentStepKey);
  }, [currentStepKey, resolvedCurrentStepKey]);
  useEffect(() => {
    if (!creating || !launchSnapshot || launchSnapshot.state !== "running") return;
    const handle = window.setInterval(() => setLaunchTick((value) => value + 1), 1000);
    return () => window.clearInterval(handle);
  }, [creating, launchSnapshot?.job_id, launchSnapshot?.state]);
  useEffect(() => {
    if (currentStepKey !== "harness-downloads") {
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
    currentStepKey,
    harnessInstallCandidates,
    harnessInstallRows,
    harnessInstallBusy,
    harnessInstallError,
    updateHarnessDownloadsScrollState,
  ]);

  const stepIndex = Math.max(0, stepKeys.indexOf(resolvedCurrentStepKey));
  const step = steps[stepIndex];
  const infoStep = openInfoKey ? steps.find((s) => s.key === openInfoKey) : null;
  const isFirst = stepIndex === 0;
  const isLast = stepIndex === steps.length - 1;
  const goToStepKey = useCallback((key: WizardStepKey) => {
    setCurrentStepKey(key);
  }, []);
  const goRelativeStep = useCallback((delta: number) => {
    setCurrentStepKey((current) => {
      const resolved = resolveWizardCurrentStepKey(stepKeys, current, previousStepIndexRef.current);
      return stepKeyOffset(stepKeys, resolved, delta);
    });
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
  const remotePasswordOnce = remotePasswordInput.length > 0 ? remotePasswordInput : null;
  const desktopApp = isDesktopApp();
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
      ? `ssh:${parsedRemote.user ?? ""}@${parsedRemote.host}:${parsedRemotePort ?? 4399}:${remoteDataDirInput.trim()}`
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
  const selectedHarnessInstallTarget: InstallTarget =
    selections.container && selections.container !== "no-container" ? "container" : "host";
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
        ? `${selectedHarnessBlockedCount} selected download${selectedHarnessBlockedCount === 1 ? "" : "s"} need attention`
        : selectedHarnessReadyToStartCount > 0
          ? `${selectedHarnessReadyToStartCount} selected for download`
          : selectedHarnessCompletedCount > 0
            ? `${selectedHarnessCompletedCount} selected download${selectedHarnessCompletedCount === 1 ? "" : "s"} ready`
            : "Skipped for now";
  const canAdvance = (!requiresSelection || hasSelection)
    && (!isRemoteStep || (
      hasRemoteHost
      && remoteStatus !== "connecting"
      && parsedRemotePort !== null
    ))
    && !(step.key === "container" && routePlanningBusy)
    && hasSourceStepInputs
    && hasTargetBranch
    && hasAllowlist
    && (step.key !== "auth-import" || !authImportBusy)
    && (
      step.key !== "harness-downloads"
      || (
        !harnessInstallBusy
        && selectedHarnessRunningCount === 0
        && selectedHarnessBlockedCount === 0
      )
    )
    && titlingStepCanAdvance
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
  const nextButtonLabel = step.key === "container" && routePlanningBusy
    ? "Working..."
    : step.key === "harness-downloads"
      ? (
        harnessInstallBusy
          ? "Working..."
          : selectedHarnessRunningCount > 0
            ? "Waiting for downloads..."
            : selectedHarnessBlockedCount > 0
              ? "Resolve downloads"
              : selectedHarnessReadyToStartCount > 0
                ? "Download selected"
                : "Continue"
      )
      : "Next";

  function applyConnection(info: DesktopConnectionInfo) {
    applyDaemonDesktopConnection(info);
  }

  const sleepMs = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

  const waitForDaemonReady = async (timeoutMs: number) => {
    const started = Date.now();
    let lastErr: unknown = null;
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
      const info = await desktopConnectSsh({
        host: parsed.host,
        user: parsed.user ?? null,
        password_once: remotePasswordOnce,
        remote_port: parsedRemotePort,
        start_remote: true,
        remote_data_dir: remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null,
      });
      applyConnection(info);
      await waitForDaemonReady(15000);
      return;
    }
    const info = await desktopConnectLocal();
    applyConnection(info);
    await waitForDaemonReady(15000);
  };

  const kickoffContainerPrewarmInBackground = useCallback(async (locationOverride?: "local" | "remote") => {
    const location = locationOverride ?? selections.location;
    if (location !== "local" && location !== "remote") return;
    if (!selections.container || selections.container === "no-container") return;
    if (location === "remote") {
      if (remoteStatusRef.current !== "connected") return;
      if (!parsedRemote?.host) return;
    }
    const remoteKey = location === "remote"
      ? `${parsedRemote?.user ?? ""}@${parsedRemote?.host ?? ""}:${parsedRemotePort ?? ""}:${remoteDataDirInput.trim()}`
      : "local";
    const key = `${location}:${selections.container}:${remoteKey}`;
    if (containerPrewarmInFlightRef.current[key]) return;
    containerPrewarmInFlightRef.current[key] = true;

    try {
      await connectDaemonForImport(location);
      await startExecutionRuntimePrewarm();
    } catch (err: unknown) {
      delete containerPrewarmInFlightRef.current[key];
      console.debug("container prewarm kickoff skipped/failed", messageFromError(err));
      return;
    }
  }, [
    connectDaemonForImport,
    parsedRemote?.host,
    parsedRemote?.user,
    parsedRemotePort,
    remoteDataDirInput,
    selections.container,
    selections.location,
  ]);

  const messageFromError = (error: unknown): string =>
    error instanceof Error && error.message ? error.message : String(error);

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
      }
      upsertProviderInstallProgress(providerId, nextInstallState);
    } catch (error) {
      setHarnessInstallError(messageFromError(error));
    }
  };

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
      : `remote|${parsedRemote?.user ?? ""}@${parsedRemote?.host ?? ""}:${parsedRemotePort ?? 4399}:${remoteDataDirInput.trim()}`;
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

  const scanHarnessInstallCandidatesForTarget = useCallback(async (
    target: "local" | "remote",
    containerSelectionOverride?: string,
  ): Promise<HarnessInstallProviderRow[]> => {
    if (!isDesktopApp()) return [];
    if (target === "remote") {
      if (!parsedRemote?.host) return [];
      if (remoteStatusRef.current !== "connected") return [];
    }

    const containerSelection = containerSelectionOverride ?? selections.container;
    const installTarget: InstallTarget =
      containerSelection && containerSelection !== "no-container" ? "container" : "host";
    const scanKey = target === "local"
      ? `local|@|${installTarget}`
      : `remote|${parsedRemote?.user ?? ""}@${parsedRemote?.host ?? ""}|${installTarget}`;
    if (harnessInstallScannedKey === scanKey) return harnessInstallCandidates;

    if (
      harnessInstallScanPromiseRef.current
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
          )
        );
        const runningRows = rows.filter((row) => row.installRunning && row.installId);
        for (const row of runningRows) {
          await attachHarnessInstall(row.providerId, row.installId!);
        }
        return rows;
      } catch (error) {
        if (!isCurrentFlowRunToken(harnessInstallScanRunRef.current, scanRun)) return [];
        setHarnessInstallCandidates([]);
        setHarnessInstallSelected({});
        setHarnessInstallRows({});
        setHarnessInstallError(messageFromError(error));
        return [];
      } finally {
        if (isCurrentFlowRunToken(harnessInstallScanRunRef.current, scanRun)) {
          setHarnessInstallScannedKey(scanKey);
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
    harnessInstallCandidates,
    harnessInstallScannedKey,
    parsedRemote?.host,
    parsedRemote?.user,
    parsedRemotePort,
    remoteDataDirInput,
    selections.container,
  ]);

  const invalidateRoutePlan = useCallback(() => {
    setRoutePlan(null);
    setRoutePlanningBusy(false);
    routePlanRunRef.current = nextFlowRunToken(routePlanRunRef.current?.runId ?? 0, "reset");
  }, []);

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
  ]);

  const shouldAutoAdvance = (stepKey: string, optionId: string): boolean => {
    if (stepKey === "location") return false;
    if (stepKey === "container") return false;
    if (stepKey === "network") return optionId !== "allowlist";
    return false;
  };

  const nextStepAfterLocation = (): WizardStepKey => "container";

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
      invalidateRoutePlan();
      authImportScanPromiseRef.current = null;
      authImportScanKeyRef.current = null;
      authImportScanRunRef.current = null;
      setAuthImportBusy(false);
      setAuthImportScannedKey(null);
      setAuthImportCandidates([]);
      setAuthImportSelected({});
      setAuthImportError(null);
      setHarnessInstallScannedKey(null);
      clearHarnessInstallPoll();
      setHarnessInstallCandidates([]);
      setHarnessInstallSelected({});
      setHarnessInstallRows({});
      setHarnessInstallError(null);
      clearHarnessInstallPoll();
      harnessInstallScanPromiseRef.current = null;
      harnessInstallScanKeyRef.current = null;
      harnessInstallScanRunRef.current = null;
      setHarnessInstallBusy(false);
      setHarnessInstallScannedKey(null);
      setHarnessInstallCandidates([]);
      setHarnessInstallSelected({});
      setHarnessInstallRows({});
      setHarnessInstallError(null);
      invalidateTitlingPersisted();
      setTitlingProbeError(null);
    }
    if (stepKey === "container") {
      invalidateRoutePlan();
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
      goToStepKey(nextStepAfterLocation());
      return;
    }
    if (stepKey === "container") {
      void (async () => {
        const plan = await ensureRoutePlanForSelection(optionId);
        if (!plan) return;
        if (currentStepKeyRef.current !== "container") return;
        goToStepKey(nextBoundaryStep(plan));
      })();
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
    if (wizardStartedRef.current) return;
    wizardStartedRef.current = true;
    trackWizardStarted({ wizardKey });
    return () => {
      if (wizardCompletedRef.current) return;
      const last = lastWizardStepViewedRef.current ?? {
        key: currentStepKeyRef.current,
        index: previousStepIndexRef.current,
      };
      trackWizardAbandoned({
        wizardKey,
        lastStepKey: last.key,
        lastStepIndex: last.index,
      });
    };
  }, [wizardKey]);

  useEffect(() => {
    if (!wizardStartedRef.current) return;
    lastWizardStepViewedRef.current = { key: step.key, index: stepIndex };
    trackWizardStepViewed({
      wizardKey,
      stepKey: step.key,
      stepIndex,
    });
  }, [step.key, stepIndex, wizardKey]);

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
    if (!isDesktopApp()) return;
    if (!selectedDaemonTargetKey) return;
    if (selections.location !== "local" && selections.location !== "remote") return;
    if (selections.location === "remote") {
      if (!parsedRemote?.host) return;
      if (remoteStatus !== "connected") return;
    }
    void scanHarnessInstallCandidatesForTarget(selections.location).catch(() => {});
  }, [
    harnessInstallScannedKey,
    parsedRemote?.host,
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
    invalidateRoutePlan();
  }, [
    invalidateRoutePlan,
    routePlan,
    selectedDaemonTargetKey,
    selections.container,
  ]);

  useEffect(() => {
    if (selections.container === "no-container" || !selections.container) return;
    if (selections.location !== "local" && selections.location !== "remote") return;
    if (selections.location === "remote") {
      if (!parsedRemote?.host) return;
      if (remoteStatus !== "connected") return;
    }
    void kickoffContainerPrewarmInBackground(selections.location);
  }, [
    kickoffContainerPrewarmInBackground,
    parsedRemote?.host,
    remoteStatus,
    selections.container,
    selections.location,
  ]);

  useEffect(() => {
    if (selections.location) return;
    clearHarnessInstallPoll();
    setHarnessInstallCandidates([]);
    setHarnessInstallSelected({});
    setHarnessInstallRows({});
    setHarnessInstallError(null);
    setHarnessInstallScannedKey(null);
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
        .catch((err: unknown) => {
          setRemotePathStatus("error");
          setRemotePathError(messageFromError(err));
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
        password_once: remotePasswordOnce,
        remote_port: parsedRemotePort,
        start_remote: true,
        remote_data_dir: remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null,
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
            const detail = String(st.error ?? "").trim();
            setImportRepoNote(detail ? `Not a git repo: ${detail}` : "Not a git repo.");
          }
        })
        .catch((e: unknown) => {
          if (cancelled) return;
          setImportRepoStatus("error");
          const detail = messageFromError(e);
          setImportRepoNote(detail ? `Remote repo check failed: ${detail}` : "Remote repo check failed.");
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
    remotePasswordOnce,
    remoteDataDirInput,
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
    setRemotePasswordInput("");
    setRemotePasswordPromptVisible(false);
    setAuthImportScannedKey(null);
    setHarnessInstallScannedKey(null);
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
        const resp = await importProviderAuthCandidates(candidateIds);
        const acceptableStatuses = new Set(["imported", "updated", "already_imported"]);
        const failures = (resp.results ?? [])
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
      } catch (err: unknown) {
        setAuthImportError(messageFromError(err));
        setAuthImportBusy(false);
        return;
      }
      setAuthImportBusy(false);
    }
    const titlingRequired = await ensureTitlingProbeForCurrentTarget();
    if (currentStepKeyRef.current !== "auth-import") return;
    if (titlingRequired === true && routePlan?.includeTitling !== true) {
      setRoutePlan((prev) => prev ? { ...prev, includeTitling: true } : prev);
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
      if (currentStepKeyRef.current !== "harness-downloads") return;
      goToStepKey(nextAfterHarnessDownloads(routePlan));
      return;
    }
    if (blockingRows.length > 0) {
      setHarnessInstallError("Resolve failed or canceled downloads, or skip them for now before continuing.");
      return;
    }
    if (runningRows.length > 0) {
      setHarnessInstallError(null);
      return;
    }

    setHarnessInstallBusy(true);
    setHarnessInstallError(null);
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
        const prefix = failures.length === startableRows.length
          ? "Unable to start selected downloads."
          : "Some downloads failed to start.";
        setHarnessInstallError(`${prefix} ${failures.join(" ; ")}`);
      }

      const startedAny = startResults.some((result) => result.ok);
      if (!startedAny) {
        return;
      }
    } catch (error) {
      setHarnessInstallError(messageFromError(error));
    } finally {
      setHarnessInstallBusy(false);
    }
  };

  const onNext = async () => {
    if (step.key === "location") {
      if (selections.location === "remote") {
        if (!parsedRemote) return;
        if (!isDesktopApp()) {
          setRemoteStatus("error");
          setRemoteError("Remote connections require the desktop app.");
          return;
        }
        if (remoteStatus !== "connected") {
          setRemoteStatus("connecting");
          setRemoteError(null);
          try {
            await desktopTestSsh({
              host: parsedRemote.host,
              user: parsedRemote.user ?? null,
              password_once: remotePasswordOnce,
            });
            remoteStatusRef.current = "connected";
            setRemoteStatus("connected");
            setRemotePasswordInput("");
            setRemotePasswordPromptVisible(false);
            const normalizedDataDir = remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null;
            setRemoteProfiles(upsertRemoteProfile(parsedRemote.host, parsedRemote.user ?? null, {
              remote_port: parsedRemotePort ?? 4399,
              remote_data_dir: normalizedDataDir,
            }));
            remoteProfileAutoAppliedKeyRef.current = remoteProfileKey(parsedRemote.host, parsedRemote.user ?? null);
            void desktopKickoffRemotePrewarm({
              host: parsedRemote.host,
              user: parsedRemote.user ?? null,
              remote_port: parsedRemotePort,
              remote_data_dir: normalizedDataDir,
            }).catch((err: unknown) => {
              console.debug("remote prewarm kickoff skipped/failed", messageFromError(err));
            });
            const nextRecents = upsertSshRecent(parsedRemote.host, parsedRemote.user ?? null);
            setSshRecents(nextRecents);
          } catch (err: unknown) {
            const detail = messageFromError(err);
            if (!remotePasswordPromptVisible && remotePasswordOnce === null && looksLikeSshAuthFailure(detail)) {
              setRemotePasswordPromptVisible(true);
              setRemoteStatus("idle");
              setRemoteError(null);
              return;
            }
            setRemoteStatus("error");
            setRemoteError(detail);
            return;
          }
        }
      }
      if (currentStepKeyRef.current !== "location") return;
      goToStepKey(nextStepAfterLocation());
      return;
    }
    if (step.key === "container") {
      const plan = await ensureRoutePlanForSelection();
      if (!plan) return;
      if (currentStepKeyRef.current !== "container") return;
      goToStepKey(nextBoundaryStep(plan));
      return;
    }
    if (step.key === "auth-import") {
      await advanceFromAuthImportStep();
      return;
    }
    if (step.key === "harness-downloads") {
      await advanceFromHarnessDownloadsStep();
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
          .catch((err: unknown) => {
            settle(new Error(messageFromError(err)));
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
      if (selections.location === "remote" && !isDesktopApp()) {
        throw new Error("Workspace creation from the wizard requires the desktop app.");
      }

      const parsed = selections.location === "remote" ? parseUserHost(remoteHostInput) : null;
      if (selections.location === "remote" && !parsed?.host) {
        throw new Error("Remote host is required (user@host).");
      }

      // 1. Connect to the intended daemon (reuse existing if already running).
      if (isDesktopApp()) {
        const info = selections.location === "remote"
          ? await desktopConnectSsh({
            host: parsed!.host,
            user: parsed!.user ?? null,
            password_once: remotePasswordOnce,
            remote_port: parsedRemotePort,
            start_remote: true,
            remote_data_dir: remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null,
          })
          : await desktopConnectLocal();
        applyConnection(info);
      }

      if (selections.location === "remote" && parsed?.host) {
        const normalizedDataDir = remoteDataDirInput.trim() ? remoteDataDirInput.trim() : null;
        setRemoteProfiles(upsertRemoteProfile(parsed.host, parsed.user ?? null, {
          remote_port: parsedRemotePort ?? 4399,
          remote_data_dir: normalizedDataDir,
        }));
        remoteProfileAutoAppliedKeyRef.current = remoteProfileKey(parsed.host, parsed.user ?? null);
      }

      // Ensure the daemon is reachable before we navigate away from the wizard.
      // This avoids landing on the workbench too early on cold start.
      await waitForDaemonReady(15000);
      const containerEnabled = selections.container !== "no-container";
      if (selectedHarnessRunningCount > 0) {
        throw new Error("Selected harness downloads are still running. Wait for them to finish or skip them before creating the workspace.");
      }
      if (selectedHarnessBlockedCount > 0 || selectedHarnessReadyToStartCount > 0) {
        throw new Error("Resolve selected harness downloads before creating the workspace.");
      }

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
            .map((workspace) => String(workspace.name ?? "").trim())
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
            const detailAfter = String(st.error ?? "").trim();
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
        const hit = all.find((w) => String(w.root_path) === rootPath);
        if (hit) {
          wsId = idToString(hit.id);
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
      const workspaceKind = selections.location === "remote" ? "remote" : "local";
      if (!wsId) {
        const created = await createWorkspace(rootPath, name, workspaceKind, "wizard");
        wsId = idToString(created.id);
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
      navigate(`/workspaces/${wsId}`, { replace: true });
    } catch (e: unknown) {
      const msg = messageFromError(e);
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
                    {remotePasswordPromptVisible ? (
                      <div className="wizard-input">
                        <label>
                          SSH Password
                          <input
                            data-testid="wizard-remote-password-once"
                            type="password"
                            autoComplete="current-password"
                            value={remotePasswordInput}
                            onChange={(e) => {
                              setCreateError(null);
                              setRemotePasswordInput(e.target.value);
                              if (remoteStatus !== "idle") {
                                setRemoteStatus("idle");
                                setRemoteError(null);
                              }
                            }}
                          />
                        </label>
                        <div className="wizard-note">
                          Used to install SSH key auth; never stored
                        </div>
                      </div>
                    ) : null}
                    {/* Temporarily hiding location-step advanced remote fields.
                        Re-enable this block if we need manual remote port/data-dir controls again. */}
                    {/*
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
                    */}
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
                {step.key === "harness-downloads" && (
                  <div className="wizard-input">
                    {harnessInstallBusy ? <div className="wizard-note">Checking/downloading harnesses…</div> : null}
                    {harnessInstallError ? <div className="wizard-error">{harnessInstallError}</div> : null}
                    {!harnessInstallBusy && selectedHarnessRunningCount > 0 ? (
                      <div className="wizard-note">
                        Selected downloads are still running. Wait for them to finish, cancel them, or skip them for now before continuing.
                      </div>
                    ) : null}
                    {!harnessInstallBusy && selectedHarnessBlockedCount > 0 ? (
                      <div className="wizard-note">
                        One or more selected downloads failed or were canceled. Clear them or skip for now before continuing.
                      </div>
                    ) : null}
                    {!harnessInstallBusy && !harnessInstallCandidates.length ? (
                      <div className="wizard-note">No downloadable harness providers detected on this daemon.</div>
                    ) : null}
                    {harnessInstallCandidates.length > 0 && (
                      <div
                        className={`wizard-harness-downloads-scroll-shell${harnessDownloadsCanScroll ? " is-scrollable" : ""}${harnessDownloadsAtBottom ? " is-bottom" : ""}`}
                        data-testid="wizard-harness-downloads-scroll-shell"
                      >
                        <div
                          ref={harnessDownloadsScrollRef}
                          className="wizard-auth-import-list wizard-auth-import-list--scrollable"
                          onScroll={updateHarnessDownloadsScrollState}
                        >
                          {harnessInstallCandidates.map((candidate) => {
                            const checked = Boolean(harnessInstallSelected[candidate.providerId]);
                            const installedReady = candidate.installed && candidate.healthy;
                            const installUi = harnessInstallRows[candidate.providerId];
                            const running = installUi?.state === "running" || candidate.installRunning;
                            const canCancelInstall = Boolean(installUi?.installId ?? candidate.installId);
                            const disabled = !candidate.installSupported || installedReady || running || harnessInstallBusy;
                            const harness = harnessByProviderId.get(candidate.providerId);
                            const installTarget = installUi?.target ?? candidate.installTarget ?? selectedHarnessInstallTarget;
                            const sizeLabel = formatByteSize(candidate.installSizeBytes ?? null);
                            const installContextLabel = `${installTargetLabel(installTarget)}${sizeLabel ? ` · ${sizeLabel}` : ""}`;
                            const installFailureMessage =
                              installUi?.state === "failed" || installUi?.state === "cancelled"
                                ? installErrorSummary(installUi.errorCode, installUi.error)
                                : null;
                            return (
                              <label
                                key={candidate.providerId}
                                className={`wizard-auth-import-row ${disabled ? "wizard-auth-import-row--disabled" : ""}`}
                              >
                                <div className="wizard-auth-import-title">
                                  <input
                                    type="checkbox"
                                    className="wizard-auth-import-checkbox"
                                    data-testid={`wizard-harness-checkbox-${candidate.providerId}`}
                                    checked={checked}
                                    disabled={disabled}
                                    onChange={(e) =>
                                      setHarnessInstallSelected((prev) => ({ ...prev, [candidate.providerId]: e.target.checked }))
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
                                  <span className="wizard-auth-import-name">{candidate.label}</span>
                                </div>
                                <div className="wizard-auth-import-path">
                                  {installedReady
                                    ? `Installed · ${installContextLabel}`
                                    : running
                                      ? `Downloading${typeof installUi?.pct === "number" ? ` (${installUi.pct}%)` : ""} · ${installContextLabel}`
                                      : `Not installed · ${installContextLabel}`}
                                </div>
                                {running ? (
                                  <div className="wizard-auth-import-actions">
                                    <button
                                      type="button"
                                      className="wizard-inline-action"
                                      onClick={(event) => {
                                        event.preventDefault();
                                        event.stopPropagation();
                                        void cancelHarnessInstall(candidate.providerId);
                                      }}
                                      disabled={!canCancelInstall}
                                    >
                                      Cancel install
                                    </button>
                                  </div>
                                ) : null}
                                {installFailureMessage ? (
                                  <div className="wizard-error wizard-note--tight">{installFailureMessage}</div>
                                ) : null}
                              </label>
                            );
                          })}
                        </div>
                      </div>
                    )}
                    <button
                      type="button"
                      className="wizard-skip wizard-skip--left wizard-skip--below"
                      data-testid="wizard-harness-skip"
                      onClick={() => {
                        void advanceFromHarnessDownloadsStep({ clearSelections: true });
                      }}
                      disabled={harnessInstallBusy}
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
                          <span className="wizard-option-title-text">Remote LLM via API Key</span>
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
                        disabled
                        aria-pressed={titlingMode === "local"}
                      >
                        <div className="wizard-option-title">
                          <span className="wizard-option-title-text">Local model</span>
                        </div>
                        <div className="wizard-option-desc">
                          Coming soon: download a small LLM to run locally for generating task titles.
                        </div>
                      </button>
                    </div>
                    {titlingMode === "local" ? (
                      <div className="wizard-note" data-testid="wizard-titling-local-status">
                        {titlingLocalStatus?.ready
                          ? "Local model ready."
                          : titlingLocalInstallBusy
                            ? "Starting local model download…"
                            : titlingLocalInstall?.state === "running"
                              ? `Installing local model${typeof titlingLocalInstall.pct === "number" ? ` (${titlingLocalInstall.pct}%)` : ""}. This continues in background.`
                              : titlingLocalInstall?.state === "cancelled"
                                ? "Local model install cancelled."
                              : titlingLocalInstall?.state === "failed"
                                ? `Local model install failed${titlingLocalInstall.error ? `: ${titlingLocalInstall.error}` : "."}`
                                : "Local model is not ready yet. Titles use fallback until install completes."}
                      </div>
                    ) : null}
                    {titlingMode === "remote" && (
                      <div className="wizard-input">
                        <label>
                          Endpoint base URL
                          <input
                            data-testid="wizard-titling-remote-base-url"
                            placeholder="https://api.your-llm-gateway.example/v1"
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
                            placeholder="model-slug"
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
                        <div className="wizard-summary-k">Harness downloads</div>
                        <div className="wizard-summary-v">{harnessSummaryValue}</div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Task titling</div>
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
                if (key === "harness-downloads") return true;
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
	                {nextButtonLabel}
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
