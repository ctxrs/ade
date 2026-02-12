import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, useNavigate } from "react-router-dom";
import { ChevronRight, Info, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import LauncherBrand from "../components/LauncherBrand";
import {
  applyDaemonDesktopConnection,
  buildExecutionLaunchWsUrl,
  createWorkspace,
  getExecutionLaunchStatus,
  getHealth,
  importProviderAuthCandidates,
  idToString,
  listProviderAuthImportCandidates,
  listWorkspaces,
  startExecutionLaunch,
  type ExecutionLaunchLogLine,
  type ExecutionLaunchPhase,
  type ExecutionLaunchPhaseStatus,
  type ExecutionLaunchSnapshot,
  type ExecutionLaunchStreamEvent,
  type ProviderAuthImportCandidate,
  repoClone,
  repoInit,
  repoStatus,
  repoStagingPath,
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
  deriveRepoNameFromUrl,
  getSourceStepValidation,
  parseCloneDestPath,
  resolveWorkspaceName,
} from "./WorkspaceSetupPage.logic";

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

type SshRecent = {
  host: string;
  user?: string | null;
  updated_at_ms: number;
};

type RemoteProfile = {
  host: string;
  user?: string | null;
  remote_port?: number | null;
  remote_data_dir?: string | null;
  remote_ctx_bin?: string | null;
  updated_at_ms: number;
};

type ImportInitDialogState = {
  path: string;
};

const SSH_RECENTS_KEY = "contextDesktopSshRecentsV1";
const REMOTE_PROFILES_KEY = "contextDesktopRemoteProfilesV1";

const loadSshRecents = (): SshRecent[] => {
  try {
    const raw = localStorage.getItem(SSH_RECENTS_KEY);
    const parsed = raw ? JSON.parse(raw) : null;
    return Array.isArray(parsed) ? (parsed as SshRecent[]) : [];
  } catch {
    return [];
  }
};

const saveSshRecents = (recents: SshRecent[]) => {
  try {
    localStorage.setItem(SSH_RECENTS_KEY, JSON.stringify(recents.slice(0, 50)));
  } catch {
    // ignore
  }
};

const upsertSshRecent = (host: string, user?: string | null): SshRecent[] => {
  const recents = loadSshRecents();
  const key = `${user ?? ""}@${host}`;
  const next = [
    { host, user: user ?? null, updated_at_ms: Date.now() },
    ...recents.filter((r) => `${r.user ?? ""}@${r.host}` !== key),
  ];
  saveSshRecents(next);
  return next;
};

const remoteProfileKey = (host: string, user?: string | null) => `${user ?? ""}@${host}`;

const loadRemoteProfiles = (): RemoteProfile[] => {
  try {
    const raw = localStorage.getItem(REMOTE_PROFILES_KEY);
    const parsed = raw ? JSON.parse(raw) : null;
    return Array.isArray(parsed) ? (parsed as RemoteProfile[]) : [];
  } catch {
    return [];
  }
};

const saveRemoteProfiles = (profiles: RemoteProfile[]) => {
  try {
    localStorage.setItem(REMOTE_PROFILES_KEY, JSON.stringify(profiles.slice(0, 100)));
  } catch {
    // ignore
  }
};

const upsertRemoteProfile = (
  host: string,
  user: string | null | undefined,
  fields: {
    remote_port?: number | null;
    remote_data_dir?: string | null;
    remote_ctx_bin?: string | null;
  },
): RemoteProfile[] => {
  const profiles = loadRemoteProfiles();
  const key = remoteProfileKey(host, user ?? null);
  const nextEntry: RemoteProfile = {
    host,
    user: user ?? null,
    remote_port: fields.remote_port ?? null,
    remote_data_dir: fields.remote_data_dir ?? null,
    remote_ctx_bin: fields.remote_ctx_bin ?? null,
    updated_at_ms: Date.now(),
  };
  const next = [nextEntry, ...profiles.filter((entry) => remoteProfileKey(entry.host, entry.user) !== key)];
  saveRemoteProfiles(next);
  return next;
};

const parseUserHost = (raw: string): { host: string; user?: string | null } | null => {
  const trimmed = String(raw || "").trim();
  if (!trimmed) return null;
  const at = trimmed.lastIndexOf("@");
  if (at > 0) {
    const user = trimmed.slice(0, at).trim();
    const host = trimmed.slice(at + 1).trim();
    if (!host) return null;
    return { host, user: user || null };
  }
  return { host: trimmed };
};

const LAUNCH_LOG_MAX = 400;

const launchPhaseLabel = (phase?: ExecutionLaunchPhase | null): string => {
  if (!phase) return "Preparing";
  switch (phase) {
    case "machine_check":
      return "Machine check";
    case "machine_start_or_init":
      return "Machine start/init";
    case "image_check":
      return "Image check";
    case "image_load":
      return "Image load";
    case "container_check":
      return "Container check";
    case "container_start_or_create":
      return "Container start/create";
    case "runtime_network_setup":
      return "Network setup";
    case "ready":
      return "Ready";
    default:
      return phase;
  }
};

const parseUtcMs = (value?: string | null): number | null => {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
};

const phaseEntryForCurrent = (snapshot: ExecutionLaunchSnapshot): ExecutionLaunchPhaseStatus | null => {
  if (!snapshot.current_phase) return null;
  for (let i = snapshot.phases.length - 1; i >= 0; i -= 1) {
    if (snapshot.phases[i].phase === snapshot.current_phase) {
      return snapshot.phases[i];
    }
  }
  return null;
};

const mergeLaunchLogs = (
  current: ExecutionLaunchLogLine[],
  incoming: ExecutionLaunchLogLine[],
): ExecutionLaunchLogLine[] => {
  if (!incoming.length) return current.slice(-LAUNCH_LOG_MAX);
  const bySeq = new Map<number, ExecutionLaunchLogLine>();
  for (const line of current) bySeq.set(line.seq, line);
  for (const line of incoming) bySeq.set(line.seq, line);
  const merged = Array.from(bySeq.values()).sort((a, b) => a.seq - b.seq);
  return merged.slice(-LAUNCH_LOG_MAX);
};

const launchErrorFromSnapshot = (snapshot: ExecutionLaunchSnapshot): string => {
  const phase = launchPhaseLabel(snapshot.current_phase);
  const message = String(snapshot.error ?? "").trim();
  if (!message) return `Workspace launch failed during ${phase}.`;
  return `${phase}: ${message}`;
};

const formatLaunchElapsed = (ms: number | null): string => {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return "0s";
  const rounded = Math.floor(ms / 1000);
  const minutes = Math.floor(rounded / 60);
  const seconds = rounded % 60;
  if (minutes <= 0) return `${seconds}s`;
  return `${minutes}m ${seconds}s`;
};

const formatLaunchTime = (ts: string): string => {
  const value = parseUtcMs(ts);
  if (value === null) return ts;
  const date = new Date(value);
  return date.toLocaleTimeString([], { hour12: false });
};

export default function WorkspaceSetupPage() {
  const navigate = useNavigate();
  const [stepIndex, setStepIndex] = useState(0);
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
  const [authImportResults, setAuthImportResults] = useState<Array<{ provider: string; status: string; message?: string | null }>>([]);
  const [authImportScannedKey, setAuthImportScannedKey] = useState<string | null>(null);
  const importInitResolveRef = useRef<((confirmed: boolean) => void) | null>(null);
  const remoteProfileAutoAppliedKeyRef = useRef<string | null>(null);

  const containerMode = selections.container;
  const authImportStepVisible = authImportCandidates.length > 0;

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
  }, [containerMode, authImportStepVisible]);

  useEffect(() => {
    setStepIndex((idx) => Math.min(idx, Math.max(0, steps.length - 1)));
  }, [steps.length]);
  useEffect(() => {
    if (!creating || !launchSnapshot || launchSnapshot.state !== "running") return;
    const handle = window.setInterval(() => setLaunchTick((value) => value + 1), 1000);
    return () => window.clearInterval(handle);
  }, [creating, launchSnapshot?.job_id, launchSnapshot?.state]);

  const step = steps[stepIndex];
  const infoStep = openInfoKey ? steps.find((s) => s.key === openInfoKey) : null;
  const isFirst = stepIndex === 0;
  const isLast = stepIndex === steps.length - 1;
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
  const authScanKey = `${selections.location ?? ""}|${parsedRemote?.user ?? ""}@${parsedRemote?.host ?? ""}`;
  const hasRemoteHost = Boolean(parsedRemote?.host);
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

  const connectDaemonForImport = async () => {
    if (!isDesktopApp()) {
      throw new Error("Auth import requires the desktop app.");
    }
    if (selections.location === "remote") {
      const parsed = parseUserHost(remoteHostInput);
      if (!parsed?.host) {
        throw new Error("Remote host is required before scanning auth.");
      }
      if (remoteStatus !== "connected") {
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

  const shouldAutoAdvance = (stepKey: string, optionId: string): boolean => {
    if (stepKey === "location") return optionId === "local";
    if (stepKey === "container") return true;
    if (stepKey === "network") return optionId !== "allowlist";
    return false;
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
      setRemoteStatus("idle");
      setRemoteError(null);
      setImportRepoStatus("idle");
      setImportRepoNote(null);
    }
    if (stepKey === "location") {
      setAuthImportScannedKey(null);
      setAuthImportCandidates([]);
      setAuthImportSelected({});
      setAuthImportError(null);
      setAuthImportResults([]);
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
    if (shouldAutoAdvance(stepKey, optionId)) {
      setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
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
    if (!selections.location) {
      setAuthImportCandidates([]);
      setAuthImportSelected({});
      setAuthImportError(null);
      setAuthImportResults([]);
      setAuthImportScannedKey(null);
      return;
    }
    if (selections.location === "remote") {
      if (!parsedRemote?.host) return;
      if (remoteStatus !== "connected") return;
    }
    if (authImportScannedKey === authScanKey) return;

    let cancelled = false;
    setAuthImportBusy(true);
    setAuthImportError(null);
    setAuthImportResults([]);

    connectDaemonForImport()
      .then(() => listProviderAuthImportCandidates())
      .then((resp) => {
        if (cancelled) return;
        const candidates = resp.candidates ?? [];
        setAuthImportCandidates(candidates);
        setAuthImportSelected(
          Object.fromEntries(candidates.map((candidate) => [candidate.id, candidate.parse_status === "parsed"])),
        );
        setAuthImportScannedKey(authScanKey);
      })
      .catch((err: any) => {
        if (cancelled) return;
        setAuthImportCandidates([]);
        setAuthImportSelected({});
        setAuthImportError(err?.message ?? String(err));
      })
      .finally(() => {
        if (!cancelled) {
          setAuthImportBusy(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    authImportScannedKey,
    authScanKey,
    parsedRemote?.host,
    remoteStatus,
    selections.location,
  ]);

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

  const onNext = async () => {
    if (step.key === "location" && selections.location === "remote") {
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
      if (remoteStatus === "connected") {
        setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
        return;
      }
      setRemoteStatus("connecting");
      setRemoteError(null);
      try {
        await desktopTestSsh({
          host: parsedRemote.host,
          user: parsedRemote.user ?? null,
        });
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
        setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
      } catch (err: any) {
        setRemoteStatus("error");
        setRemoteError(err?.message ?? String(err));
      }
      return;
    }
    if (step.key === "auth-import") {
      if (authImportBusy) return;
      const candidateIds = authImportCandidates
        .filter((candidate) => authImportSelected[candidate.id])
        .map((candidate) => candidate.id);
      if (!candidateIds.length) {
        setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
        return;
      }
      setAuthImportBusy(true);
      setAuthImportError(null);
      try {
        await connectDaemonForImport();
        const resp = await importProviderAuthCandidates(candidateIds);
        const results = (resp.results ?? []).map((result) => ({
          provider: result.provider_id,
          status: result.status,
          message: result.message ?? null,
        }));
        setAuthImportResults(results);
      } catch (err: any) {
        setAuthImportError(err?.message ?? String(err));
        setAuthImportBusy(false);
        return;
      }
      setAuthImportBusy(false);
      setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
      return;
    }
    setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
  };

  const applyLaunchSnapshot = (snapshot: ExecutionLaunchSnapshot) => {
    setLaunchSnapshot(snapshot);
    setLaunchLogs((prev) => mergeLaunchLogs(prev, snapshot.logs ?? []));
  };

  const appendLaunchLine = (line: ExecutionLaunchLogLine) => {
    setLaunchLogs((prev) => mergeLaunchLogs(prev, [line]));
  };

  const waitForLaunchCompletion = async (workspaceId: string) => {
    const initial = await startExecutionLaunch(workspaceId);
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
      const containerEnabled = environment !== "host";
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
      navigate(`/workspaces/${wsId}`, { replace: true });
    } catch (e: any) {
      const msg = e?.message ?? String(e);
      setCreateError(msg);

      const stepKeyForError = (m: string): WizardStep["key"] | null => {
        const s = String(m || "");
        if (s.includes("Remote host is required")) return "location";
        if (s.includes("repo_url") || s.includes("Destination") || s.includes("Folder") || s.includes("git clone") || s.includes("git init") || s.includes("root_path") || s.includes("not a repo")) {
          return "source";
        }
        return null;
      };
      const key = stepKeyForError(msg);
      if (key) {
        const idx = steps.findIndex((st) => st.key === key);
        if (idx >= 0) setStepIndex(idx);
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
                      <div className="wizard-option-grid">
                        {authImportCandidates.map((candidate) => {
                          const checked = Boolean(authImportSelected[candidate.id]);
                          const importable = candidate.parse_status === "parsed";
                          return (
                            <label key={candidate.id} className="wizard-option" style={{ cursor: importable ? "pointer" : "not-allowed" }}>
                              <div className="wizard-option-title">
                                <input
                                  type="checkbox"
                                  checked={checked}
                                  disabled={!importable || authImportBusy}
                                  onChange={(e) =>
                                    setAuthImportSelected((prev) => ({ ...prev, [candidate.id]: e.target.checked }))
                                  }
                                />
                                <span className="wizard-option-title-text">{candidate.provider_label}</span>
                                <span className="wizard-option-badge">{candidate.parse_status}</span>
                              </div>
                              <div className="wizard-option-desc">{candidate.path}</div>
                              {candidate.summary ? <div className="wizard-note wizard-note--tight">{candidate.summary}</div> : null}
                              {candidate.unsupported_reason ? (
                                <div className="wizard-note wizard-note--tight">{candidate.unsupported_reason}</div>
                              ) : null}
                            </label>
                          );
                        })}
                      </div>
                    )}
                    {authImportResults.length > 0 && (
                      <div className="wizard-note">
                        {authImportResults.map((entry, idx) => (
                          <div key={`${entry.provider}:${entry.status}:${idx}`}>
                            {entry.provider}: {entry.status}
                            {entry.message ? ` (${entry.message})` : ""}
                          </div>
                        ))}
                      </div>
                    )}
                    <button
                      type="button"
                      className="wizard-skip wizard-skip--left wizard-skip--below"
                      onClick={() => {
                        setAuthImportSelected({});
                        setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
                      }}
                      disabled={authImportBusy}
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
		                      placeholder="pnpm install"
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
	                        placeholder="pnpm test"
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
                        setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
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
                      if (!disabled) setStepIndex(idx);
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
	                onClick={() => setStepIndex((idx) => Math.max(0, idx - 1))}
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
	                {creating ? "Creating…" : "Create workspace"}
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
