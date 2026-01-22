import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import {
  createMobileConnectionProfile,
  createWorkspace,
  deleteMobileConnectionProfile,
  getDaemonBaseUrl,
  idToString,
  listMobileConnectionProfiles,
  listProviders,
  listWorkspaces,
  setDaemonBaseUrl,
  type CreateMobileProfileResponse,
  type MobileConnectionProfile,
} from "../api/client";
import { QRCodeSVG } from "qrcode.react";
import {
  desktopConnectLocal,
  desktopConnectSsh,
  desktopGetConnection,
  desktopGetDaemonSettings,
  desktopGitClone,
  desktopPickFolder,
  isDesktopApp,
  type DesktopDaemonSettings,
  type DesktopConnectionInfo,
} from "../utils/desktop";
import { loadLauncherPrefsV1, saveLauncherPrefsV1 } from "../state/uiStateStore";

type RecentEntry =
  | {
      kind: "local";
      label: string;
      root_path: string;
      updated_at_ms: number;
    }
  | {
      kind: "ssh";
      label: string;
      host: string;
      user?: string | null;
      remote_port: number;
      start_remote?: boolean;
      remote_data_dir?: string | null;
      updated_at_ms: number;
    };

type ExecutionMode = "container" | "host";
type WizardStep = "host" | "mode" | "workspace" | "review" | "progress";
type ProgressStatus = "pending" | "running" | "done" | "error" | "skipped";

type ProgressItem = {
  id: "daemon" | "workspace" | "worktree" | "harness";
  label: string;
  status: ProgressStatus;
  detail?: string;
};

const WIZARD_STEPS: Array<{ id: WizardStep; label: string }> = [
  { id: "host", label: "Host" },
  { id: "mode", label: "Mode" },
  { id: "workspace", label: "Workspace" },
  { id: "review", label: "Review" },
  { id: "progress", label: "Progress" },
];

const defaultProgressItems = (): ProgressItem[] => [
  { id: "daemon", label: "Start daemon", status: "pending" },
  { id: "workspace", label: "Register workspace", status: "pending" },
  { id: "worktree", label: "Prepare worktree", status: "pending" },
  { id: "harness", label: "Check harnesses", status: "pending" },
];

const isLinuxPlatform = (): boolean => {
  if (typeof navigator === "undefined") return false;
  return /Linux/i.test(navigator.userAgent || "");
};

const progressStatusLabel = (status: ProgressStatus): string => {
  switch (status) {
    case "running":
      return "Working";
    case "done":
      return "Done";
    case "error":
      return "Failed";
    case "skipped":
      return "Skipped";
    default:
      return "Pending";
  }
};

const progressPillClass = (status: ProgressStatus): string => {
  if (status === "running") return "pill run";
  if (status === "done") return "pill ok";
  if (status === "error") return "pill err";
  return "pill";
};

const RECENTS_KEY = "contextDesktopRecentsV1";

const loadRecents = (): RecentEntry[] => {
  try {
    const raw = localStorage.getItem(RECENTS_KEY);
    const parsed = raw ? JSON.parse(raw) : null;
    return Array.isArray(parsed) ? (parsed as RecentEntry[]) : [];
  } catch {
    return [];
  }
};

const saveRecents = (recents: RecentEntry[]) => {
  try {
    localStorage.setItem(RECENTS_KEY, JSON.stringify(recents.slice(0, 50)));
  } catch {
    // ignore
  }
};

const upsertRecent = (entry: RecentEntry) => {
  const recents = loadRecents();
  const key = (() => {
    if (entry.kind === "local") return `local:${entry.root_path}`;
    return `ssh:${entry.user ?? ""}@${entry.host}:${entry.remote_port}`;
  })();
  const next = [entry, ...recents.filter((r) => {
    const k = r.kind === "local" ? `local:${r.root_path}` : `ssh:${r.user ?? ""}@${r.host}:${r.remote_port}`;
    return k !== key;
  })];
  saveRecents(next);
};

function applyConnection(info: DesktopConnectionInfo) {
  const baseUrl = String(info.base_url ?? "").trim();
  const token = String(info.token ?? "").trim();
  if (baseUrl) setDaemonBaseUrl(baseUrl, true);
  else setDaemonBaseUrl(null, false);
  if (token) {
    try {
      sessionStorage.setItem("ctxAuthToken", token);
    } catch {
      // ignore
    }
  } else {
    try {
      sessionStorage.removeItem("ctxAuthToken");
    } catch {
      // ignore
    }
  }
}

async function createOrOpenWorkspaceByPath(rootPath: string): Promise<string> {
  const all = await listWorkspaces();
  const hit = all.find((w) => String(w.root_path) === rootPath);
  if (hit) return idToString((hit as any).id);
  const created = await createWorkspace(rootPath);
  return idToString((created as any).id);
}

export default function LauncherPage() {
  const navigate = useNavigate();
  const [connection, setConnection] = useState<DesktopConnectionInfo | null>(null);
  const [daemonSettings, setDaemonSettings] = useState<DesktopDaemonSettings | null>(null);
  const [recents, setRecents] = useState<RecentEntry[]>(() => loadRecents());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mobileProfiles, setMobileProfiles] = useState<MobileConnectionProfile[]>([]);
  const [mobileProfilesLoading, setMobileProfilesLoading] = useState(true);
  const [mobileProfilesError, setMobileProfilesError] = useState<string | null>(null);
  const [mobileCreateOpen, setMobileCreateOpen] = useState(false);
  const [mobileCreateBusy, setMobileCreateBusy] = useState(false);
  const [mobileCreateError, setMobileCreateError] = useState<string | null>(null);
  const [mobileLabel, setMobileLabel] = useState("");
  const [mobileBaseUrl, setMobileBaseUrl] = useState("");
  const [qrModal, setQrModal] = useState<CreateMobileProfileResponse | null>(null);

  const isDesktop = isDesktopApp();
  const supportsContainer = isDesktop && isLinuxPlatform();
  const defaultExecutionMode: ExecutionMode = supportsContainer ? "container" : "host";
  const [wizardStep, setWizardStep] = useState<WizardStep>("host");
  const [hostKind, setHostKind] = useState<"local" | "ssh">("local");
  const [executionMode, setExecutionMode] = useState<ExecutionMode>(defaultExecutionMode);
  const [workspacePath, setWorkspacePath] = useState("");
  const [wizardBusy, setWizardBusy] = useState(false);
  const [wizardError, setWizardError] = useState<string | null>(null);
  const [wizardErrorStep, setWizardErrorStep] = useState<WizardStep | null>(null);
  const [progressItems, setProgressItems] = useState<ProgressItem[]>(() => defaultProgressItems());
  const [wizardWorkspaceId, setWizardWorkspaceId] = useState<string | null>(null);
  const [prefsLoaded, setPrefsLoaded] = useState(false);
  const autoOpenRef = useRef(false);
  const connectionRetryRef = useRef(false);

  useEffect(() => {
    if (!isDesktop) {
      navigate("/workspaces", { replace: true });
      return;
    }
    desktopGetConnection()
      .then((info) => {
        setConnection(info);
        applyConnection(info);
      })
      .catch(() => setConnection({ kind: "none" } as any));
  }, [isDesktop, navigate]);

  useEffect(() => {
    if (!isDesktop) return;
    let mounted = true;
    desktopGetDaemonSettings()
      .then((settings) => {
        if (mounted) setDaemonSettings(settings);
      })
      .catch(() => {
        if (mounted) setDaemonSettings(null);
      });
    return () => {
      mounted = false;
    };
  }, [isDesktop]);

  useEffect(() => {
    if (!isDesktop) return;
    if (!daemonSettings || !connection || connection.kind === "none") return;
    if (autoOpenRef.current) return;
    const lastWorkspaceId = daemonSettings.last_workspace_id?.trim();
    if (!lastWorkspaceId) return;
    autoOpenRef.current = true;
    navigate(`/workspaces/${encodeURIComponent(lastWorkspaceId)}`, { replace: true });
  }, [connection, daemonSettings, isDesktop, navigate]);

  useEffect(() => {
    if (!isDesktop) return;
    if (!daemonSettings?.last_connection) return;
    if (connection && connection.kind !== "none") return;
    if (connectionRetryRef.current) return;
    connectionRetryRef.current = true;
    const t = window.setTimeout(() => {
      desktopGetConnection()
        .then((info) => {
          setConnection(info);
          applyConnection(info);
        })
        .catch(() => {});
    }, 800);
    return () => window.clearTimeout(t);
  }, [connection, daemonSettings, isDesktop]);

  useEffect(() => {
    let mounted = true;
    loadLauncherPrefsV1()
      .then((prefs) => {
        if (!mounted || !prefs) return;
        if (prefs.executionMode) setExecutionMode(prefs.executionMode);
        if (prefs.hostKind) setHostKind(prefs.hostKind);
        if (prefs.workspacePath) setWorkspacePath(prefs.workspacePath);
      })
      .finally(() => {
        if (mounted) setPrefsLoaded(true);
      });
    return () => {
      mounted = false;
    };
  }, []);

  useEffect(() => {
    if (!prefsLoaded) return;
    saveLauncherPrefsV1({
      executionMode,
      hostKind,
      workspacePath: workspacePath.trim() ? workspacePath.trim() : null,
    }).catch(() => {});
  }, [executionMode, hostKind, prefsLoaded, workspacePath]);

  useEffect(() => {
    if (!supportsContainer && executionMode === "container") {
      setExecutionMode("host");
    }
  }, [executionMode, supportsContainer]);

  useEffect(() => {
    setRecents(loadRecents());
  }, [busy]);

  const loadMobileProfiles = useCallback(async () => {
    setMobileProfilesLoading(true);
    setMobileProfilesError(null);
    try {
      const profiles = await listMobileConnectionProfiles();
      setMobileProfiles(profiles);
    } catch (e: any) {
      setMobileProfilesError(e?.message ?? String(e));
    } finally {
      setMobileProfilesLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadMobileProfiles();
  }, [loadMobileProfiles]);

  const connectedLabel = useMemo(() => {
    if (!connection || connection.kind === "none") return "Not connected";
    const base = String(connection.base_url ?? getDaemonBaseUrl() ?? "").trim();
    return `${connection.kind.toUpperCase()} · ${base || "(unknown)"}`;
  }, [connection]);

  const wizardStepIndex = Math.max(0, WIZARD_STEPS.findIndex((step) => step.id === wizardStep));

  const updateProgressItem = useCallback((id: ProgressItem["id"], patch: Partial<ProgressItem>) => {
    setProgressItems((prev) =>
      prev.map((item) => (item.id === id ? { ...item, ...patch } : item)),
    );
  }, []);

  const resetWizardErrors = useCallback(() => {
    setWizardError(null);
    setWizardErrorStep(null);
  }, []);

  const goToWizardStep = useCallback((step: WizardStep) => {
    resetWizardErrors();
    setWizardStep(step);
  }, [resetWizardErrors]);

  const connectLocalAndOpen = async (rootPath?: string, launchMode?: ExecutionMode) => {
    setError(null);
    setBusy(true);
    try {
      const trimmedRoot = rootPath?.trim();
      const workspaceRoots = trimmedRoot ? [trimmedRoot] : undefined;
      const opts: { launch_mode?: ExecutionMode; workspace_roots?: string[] } = {};
      if (launchMode) opts.launch_mode = launchMode;
      if (workspaceRoots) opts.workspace_roots = workspaceRoots;
      const info = await desktopConnectLocal(Object.keys(opts).length ? opts : undefined);
      setConnection(info);
      applyConnection(info);
      if (trimmedRoot) {
        setWorkspacePath(trimmedRoot);
        const wsId = await createOrOpenWorkspaceByPath(trimmedRoot);
        upsertRecent({
          kind: "local",
          label: lastSegment(trimmedRoot),
          root_path: trimmedRoot,
          updated_at_ms: Date.now(),
        });
        navigate(`/workspaces/${wsId}`, { replace: true });
      } else {
        navigate("/workspaces", { replace: true });
      }
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  };

  const onOpenLocalProject = async () => {
    setError(null);
    try {
      const folder = await desktopPickFolder();
      if (!folder) return;
      await connectLocalAndOpen(folder, executionMode);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    }
  };

  const onCloneRepo = async () => {
    setError(null);
    const repoUrl = window.prompt("Git repo URL to clone:");
    if (!repoUrl) return;
    setBusy(true);
    try {
      const destParent = await desktopPickFolder();
      if (!destParent) return;
      const rootPath = await desktopGitClone(repoUrl, destParent);
      await connectLocalAndOpen(rootPath, executionMode);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  };

  const [sshHost, setSshHost] = useState("");
  const [sshUser, setSshUser] = useState("");
  const [sshRemotePort, setSshRemotePort] = useState(4399);
  const [sshStartRemote, setSshStartRemote] = useState(true);
  const [sshDataDir, setSshDataDir] = useState("~/.ctx");

  const onConnectSsh = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const info = await desktopConnectSsh({
        host: sshHost.trim(),
        user: sshUser.trim() || null,
        remote_port: sshRemotePort,
        start_remote: sshStartRemote,
        remote_data_dir: sshDataDir.trim() || null,
      });
      setConnection(info);
      applyConnection(info);
      upsertRecent({
        kind: "ssh",
        label: `${sshHost.trim()}${sshUser.trim() ? ` (${sshUser.trim()})` : ""}`,
        host: sshHost.trim(),
        user: sshUser.trim() || null,
        remote_port: sshRemotePort,
        start_remote: sshStartRemote,
        remote_data_dir: sshDataDir.trim() || null,
        updated_at_ms: Date.now(),
      });
      navigate("/workspaces", { replace: true });
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  };

  const onOpenRecent = async (r: RecentEntry) => {
    setError(null);
    setBusy(true);
    try {
      if (r.kind === "local") {
        await connectLocalAndOpen(r.root_path, executionMode);
        return;
      }
      const info = await desktopConnectSsh({
        host: r.host,
        user: r.user ?? null,
        remote_port: r.remote_port,
        start_remote: Boolean(r.start_remote),
        remote_data_dir: r.remote_data_dir ?? null,
      });
      setConnection(info);
      applyConnection(info);
      upsertRecent({ ...r, updated_at_ms: Date.now() });
      navigate("/workspaces", { replace: true });
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  };

  const onPickWorkspaceFolder = async () => {
    resetWizardErrors();
    try {
      const folder = await desktopPickFolder();
      if (folder) setWorkspacePath(folder);
    } catch (e: any) {
      setWizardError(e?.message ?? String(e));
    }
  };

  const onWizardNext = () => {
    resetWizardErrors();
    if (wizardStep === "host") {
      if (hostKind !== "local") {
        setWizardError("Remote hosts are not available yet.");
        setWizardErrorStep("host");
        return;
      }
      setWizardStep("mode");
      return;
    }
    if (wizardStep === "mode") {
      if (executionMode === "container" && !supportsContainer) {
        setWizardError("Container mode is currently supported only on Linux.");
        setWizardErrorStep("mode");
        return;
      }
      setWizardStep("workspace");
      return;
    }
    if (wizardStep === "workspace") {
      if (!workspacePath.trim()) {
        setWizardError("Select a workspace folder to continue.");
        setWizardErrorStep("workspace");
        return;
      }
      setWizardStep("review");
    }
  };

  const onWizardBack = () => {
    resetWizardErrors();
    const prev = WIZARD_STEPS[wizardStepIndex - 1]?.id;
    if (prev) setWizardStep(prev);
  };

  const onSwitchToHostMode = () => {
    setExecutionMode("host");
    goToWizardStep("mode");
  };

  const startWizard = async () => {
    if (wizardBusy) return;
    resetWizardErrors();
    if (hostKind !== "local") {
      setWizardError("Remote hosts are not available yet.");
      setWizardErrorStep("host");
      setWizardStep("host");
      return;
    }
    const rootPath = workspacePath.trim();
    if (!rootPath) {
      setWizardError("Select a workspace folder to continue.");
      setWizardErrorStep("workspace");
      setWizardStep("workspace");
      return;
    }
    setProgressItems(defaultProgressItems());
    setWizardWorkspaceId(null);
    setWizardStep("progress");
    setWizardBusy(true);

    let activeId: ProgressItem["id"] = "daemon";
    try {
      updateProgressItem("daemon", {
        status: "running",
        detail: executionMode === "container" ? "Launching containerized daemon" : "Launching daemon",
      });
      const info = await desktopConnectLocal({
        launch_mode: executionMode,
        workspace_roots: [rootPath],
      });
      setConnection(info);
      applyConnection(info);
      updateProgressItem("daemon", {
        status: "done",
        detail: info.base_url ? `Connected to ${info.base_url}` : "Connected",
      });

      activeId = "workspace";
      updateProgressItem("workspace", { status: "running" });
      const workspaceId = await createOrOpenWorkspaceByPath(rootPath);
      setWizardWorkspaceId(workspaceId);
      upsertRecent({
        kind: "local",
        label: lastSegment(rootPath),
        root_path: rootPath,
        updated_at_ms: Date.now(),
      });
      setRecents(loadRecents());
      updateProgressItem("workspace", { status: "done", detail: rootPath });

      activeId = "worktree";
      updateProgressItem("worktree", {
        status: "done",
        detail: "Created when you start your first task.",
      });

      activeId = "harness";
      updateProgressItem("harness", { status: "running" });
      try {
        const providers = await listProviders();
        const needsSetup = providers.filter(
          (p) =>
            p.details?.install_supported === "true" && (!p.installed || p.health !== "ok"),
        );
        const detail = needsSetup.length
          ? `${needsSetup.length} harness${needsSetup.length === 1 ? "" : "es"} need setup in Settings.`
          : "All harnesses ready.";
        updateProgressItem("harness", { status: "done", detail });
      } catch {
        updateProgressItem("harness", {
          status: "skipped",
          detail: "Check harnesses later in Settings.",
        });
      }
    } catch (e: any) {
      const message = e?.message ?? String(e);
      updateProgressItem(activeId, { status: "error", detail: message });
      setWizardError(message);
      setWizardErrorStep(activeId === "daemon" ? "mode" : "workspace");
    } finally {
      setWizardBusy(false);
    }
  };

  const openCreateMobileModal = () => {
    setMobileCreateError(null);
    setMobileLabel("");
    setMobileBaseUrl(connection?.base_url ?? getDaemonBaseUrl() ?? "");
    setMobileCreateOpen(true);
  };

  const closeCreateMobileModal = () => {
    if (!mobileCreateBusy) setMobileCreateOpen(false);
  };

  const submitMobileProfile = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!mobileLabel.trim() || !mobileBaseUrl.trim()) {
      setMobileCreateError("Label and base URL are required.");
      return;
    }
    setMobileCreateBusy(true);
    setMobileCreateError(null);
    try {
      const resp = await createMobileConnectionProfile({
        label: mobileLabel.trim(),
        base_url: mobileBaseUrl.trim(),
      });
      setQrModal(resp);
      setMobileCreateOpen(false);
      setMobileLabel("");
      setMobileBaseUrl("");
      await loadMobileProfiles();
    } catch (e: any) {
      setMobileCreateError(e?.message ?? String(e));
    } finally {
      setMobileCreateBusy(false);
    }
  };

  const onRevokeMobileProfile = async (profile: MobileConnectionProfile) => {
    if (!window.confirm(`Revoke mobile access for “${profile.label}”?`)) {
      return;
    }
    try {
      await deleteMobileConnectionProfile(profile.id);
      await loadMobileProfiles();
    } catch (e: any) {
      setMobileProfilesError(e?.message ?? String(e));
    }
  };

  const closeQrModal = () => setQrModal(null);

  const wizardContent = (() => {
    switch (wizardStep) {
      case "host":
        return (
          <div style={{ display: "grid", gap: 12, marginTop: 12 }}>
            <label className="card" style={{ margin: 0 }}>
              <div className="row" style={{ alignItems: "center" }}>
                <input
                  type="radio"
                  name="wizard-host"
                  checked={hostKind === "local"}
                  data-testid="launcher-host-local"
                  onChange={() => setHostKind("local")}
                />
                <div>
                  <div><strong>This machine</strong></div>
                  <div className="muted">Start the daemon locally on this computer.</div>
                </div>
              </div>
            </label>
            <label className="card" style={{ margin: 0, opacity: 0.6 }}>
              <div className="row" style={{ alignItems: "center" }}>
                <input
                  type="radio"
                  name="wizard-host"
                  checked={hostKind === "ssh"}
                  disabled
                  data-testid="launcher-host-ssh"
                  onChange={() => setHostKind("ssh")}
                />
                <div>
                  <div>
                    <strong>Remote via SSH</strong> <span className="muted">(coming soon)</span>
                  </div>
                  <div className="muted">Connect to a devbox over SSH.</div>
                </div>
              </div>
            </label>
          </div>
        );
      case "mode":
        return (
          <div style={{ display: "grid", gap: 12, marginTop: 12 }}>
            <label className="card" style={{ margin: 0, opacity: supportsContainer ? 1 : 0.6 }}>
              <div className="row" style={{ alignItems: "center" }}>
                <input
                  type="radio"
                  name="wizard-mode"
                  checked={executionMode === "container"}
                  disabled={!supportsContainer}
                  data-testid="launcher-mode-container"
                  onChange={() => setExecutionMode("container")}
                />
                <div>
                  <div><strong>Container (recommended)</strong></div>
                  <div className="muted">Runs ctx in a host-mounted container for isolation.</div>
                </div>
              </div>
            </label>
            <label className="card" style={{ margin: 0 }}>
              <div className="row" style={{ alignItems: "center" }}>
                <input
                  type="radio"
                  name="wizard-mode"
                  checked={executionMode === "host"}
                  data-testid="launcher-mode-host"
                  onChange={() => setExecutionMode("host")}
                />
                <div>
                  <div><strong>Host mode</strong></div>
                  <div className="muted">Run directly on the host (escape hatch).</div>
                </div>
              </div>
            </label>
            {!supportsContainer && (
              <div className="banner">
                Container mode is available on Linux only. Host mode will be used here.
              </div>
            )}
          </div>
        );
      case "workspace":
        return (
          <div style={{ display: "grid", gap: 12, marginTop: 12 }}>
            <label>
              <div className="muted">Workspace folder</div>
              <div className="row" style={{ alignItems: "center" }}>
                <input
                  style={{ flex: 1 }}
                  value={workspacePath}
                  data-testid="launcher-workspace-path"
                  onChange={(e) => setWorkspacePath(e.target.value)}
                  placeholder="~/code/my-repo"
                />
                <button
                  type="button"
                  onClick={onPickWorkspaceFolder}
                  disabled={wizardBusy}
                  data-testid="launcher-workspace-pick"
                >
                  Pick folder
                </button>
              </div>
            </label>
            <div className="muted">
              Must be a git repo root. Tilde (<code>~</code>) is supported.
            </div>
          </div>
        );
      case "review":
        return (
          <div style={{ display: "grid", gap: 12, marginTop: 12 }}>
            <div className="card" style={{ margin: 0 }}>
              <div className="row" style={{ alignItems: "center" }}>
                <strong>Host</strong>
                <div className="muted" style={{ marginLeft: "auto" }}>This machine</div>
              </div>
              <div className="row" style={{ alignItems: "center" }}>
                <strong>Execution</strong>
                <div className="muted" style={{ marginLeft: "auto" }}>
                  {executionMode === "container" ? "Container (host-mounted)" : "Host mode"}
                </div>
              </div>
              <div className="row" style={{ alignItems: "center" }}>
                <strong>Workspace</strong>
                <div className="muted" style={{ marginLeft: "auto" }}>
                  {workspacePath || "Not set"}
                </div>
              </div>
            </div>
            <div className="card" style={{ margin: 0 }}>
              <strong>Mounts</strong>
              {executionMode === "container" ? (
                <ul className="sublist">
                  <li>RW: {workspacePath || "workspace root"}</li>
                  <li>RW: ~/.ctx (daemon data)</li>
                </ul>
              ) : (
                <div className="muted">Host mode runs directly on your machine (no container mounts).</div>
              )}
            </div>
            <div className="card" style={{ margin: 0 }}>
              <strong>Network policy</strong>
              <div className="muted">
                Outbound traffic is routed through ctx (policy: full). You can tune this later.
              </div>
            </div>
            {executionMode === "container" && (
              <button type="button" className="linklike" onClick={onSwitchToHostMode}>
                Switch to host mode instead
              </button>
            )}
          </div>
        );
      case "progress":
        return (
          <div style={{ display: "grid", gap: 12, marginTop: 12 }}>
            <div style={{ display: "grid", gap: 10 }}>
              {progressItems.map((item) => (
                <div key={item.id} className="row" style={{ alignItems: "center", gap: 12 }}>
                  <span className={progressPillClass(item.status)}>{progressStatusLabel(item.status)}</span>
                  <div>
                    <div>{item.label}</div>
                    {item.detail && <div className="muted">{item.detail}</div>}
                  </div>
                </div>
              ))}
            </div>
            {wizardWorkspaceId && !wizardBusy && !wizardError && (
              <div className="banner">
                Workspace is ready. Open the workbench to start your first task.
              </div>
            )}
          </div>
        );
      default:
        return null;
    }
  })();

  const wizardActions = (() => {
    if (wizardStep === "progress") {
      const canGoBack = Boolean(wizardError);
      return (
        <div className="row" style={{ justifyContent: "space-between", marginTop: 12 }}>
          <div className="row" style={{ gap: 8 }}>
            {canGoBack && (
              <button
                type="button"
                onClick={() => goToWizardStep(wizardErrorStep ?? "review")}
                disabled={wizardBusy}
                data-testid="launcher-wizard-back"
              >
                Back
              </button>
            )}
            {wizardError && executionMode === "container" && (
              <button type="button" onClick={onSwitchToHostMode} disabled={wizardBusy}>
                Switch to host mode
              </button>
            )}
          </div>
          <div className="row" style={{ gap: 8, marginLeft: "auto" }}>
            {wizardWorkspaceId && !wizardError && (
              <button
                type="button"
                onClick={() => navigate(`/workspaces/${wizardWorkspaceId}`)}
                disabled={wizardBusy}
                data-testid="launcher-open-workbench"
              >
                Open workbench
              </button>
            )}
          </div>
        </div>
      );
    }

    const backDisabled = wizardStepIndex === 0;
    const nextDisabled = wizardStep === "workspace" && !workspacePath.trim();
    const nextLabel = wizardStep === "review" ? "Start" : "Continue";
    const nextAction = wizardStep === "review" ? startWizard : onWizardNext;

    return (
      <div className="row" style={{ justifyContent: "space-between", marginTop: 12 }}>
        <button
          type="button"
          onClick={onWizardBack}
          disabled={backDisabled}
          data-testid="launcher-wizard-back"
        >
          Back
        </button>
        <button
          type="button"
          onClick={nextAction}
          disabled={wizardBusy || nextDisabled}
          data-testid="launcher-wizard-next"
        >
          {wizardBusy && wizardStep === "review" ? "Starting..." : nextLabel}
        </button>
      </div>
    );
  })();

  return (
    <>
    <div className="page">
      <div className="row" style={{ alignItems: "baseline" }}>
        <h1 style={{ marginRight: "auto" }}>ctx</h1>
        <Link to="/app-settings">Settings</Link>
      </div>

      <div className="muted" style={{ marginTop: 6 }}>{connectedLabel}</div>

      <div className="card" style={{ marginTop: 18 }}>
        <div className="row" style={{ alignItems: "center" }}>
          <strong>Get started</strong>
          <div className="muted" style={{ marginLeft: "auto" }}>
            Step {wizardStepIndex + 1} of {WIZARD_STEPS.length}
          </div>
        </div>
        <div className="row" style={{ flexWrap: "wrap", marginTop: 8 }}>
          {WIZARD_STEPS.map((step, idx) => (
            <div
              key={step.id}
              className={`pill ${idx === wizardStepIndex ? "run" : idx < wizardStepIndex ? "ok" : ""}`}
            >
              {step.label}
            </div>
          ))}
        </div>
        {wizardContent}
        {wizardError && <div className="error" style={{ marginTop: 10 }}>{wizardError}</div>}
        {wizardActions}
      </div>

      <div className="card" style={{ marginTop: 18 }}>
        <div className="row" style={{ alignItems: "center" }}>
          <strong>Quick actions</strong>
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 12, marginTop: 12 }}>
          <button type="button" onClick={onOpenLocalProject} disabled={busy || wizardBusy}>
            Open local project
          </button>
          <button type="button" onClick={onCloneRepo} disabled={busy || wizardBusy}>
            Clone repo
          </button>
          <button type="button" onClick={() => {
            const el = document.getElementById("ssh-form");
            el?.scrollIntoView({ behavior: "smooth", block: "start" });
          }} disabled={busy || wizardBusy}>
            Connect via SSH
          </button>
        </div>
        {error && <div className="error" style={{ marginTop: 12 }}>{error}</div>}
      </div>

      <div className="card" style={{ marginTop: 18 }}>
        <div className="row" style={{ marginBottom: 6 }}>
          <strong>Recent</strong>
          <div style={{ marginLeft: "auto" }} className="muted">{recents.length ? `${recents.length}` : ""}</div>
        </div>
        <ul className="list">
          {recents.slice(0, 8).map((r) => (
            <li key={r.kind === "local" ? `local:${r.root_path}` : `ssh:${r.user ?? ""}@${r.host}:${r.remote_port}`}>
              <button type="button" className="linklike" onClick={() => onOpenRecent(r)} disabled={busy || wizardBusy} style={{ padding: 0 }}>
                {r.label}
              </button>
              <div className="muted">
                {r.kind === "local"
                  ? r.root_path
                  : `${r.user ? `${r.user}@` : ""}${r.host}:${r.remote_port}`}
              </div>
            </li>
          ))}
          {recents.length === 0 && <li className="muted">No recent projects yet.</li>}
        </ul>
      </div>

      <div className="card" style={{ marginTop: 18 }}>
        <div className="row" style={{ alignItems: "center" }}>
          <strong>Mobile connections</strong>
          <button type="button" onClick={openCreateMobileModal} disabled={mobileCreateBusy}>
            Enable mobile
          </button>
        </div>
        <p className="muted" style={{ marginTop: 4 }}>
          Generate HTTPS + API-token QR codes for the ctx mobile app. Tokens are only shown once.
        </p>
        {mobileProfilesError && <div className="error" style={{ marginTop: 6 }}>{mobileProfilesError}</div>}
        <div style={{ marginTop: 12 }}>
          {mobileProfilesLoading ? (
            <div className="muted">Loading profiles…</div>
          ) : mobileProfiles.length === 0 ? (
            <div className="muted">No mobile profiles yet.</div>
          ) : (
            <div style={{ display: "grid", gap: 12 }}>
              {mobileProfiles.map((profile) => (
                <div key={profile.id} className="mobile-profile-row">
                  <div>
                    <div style={{ fontWeight: 600 }}>{profile.label}</div>
                    <div className="muted">{profile.base_url}</div>
                    <div className="muted">Token prefix: {profile.token_prefix}</div>
                    <div className="muted">Created {formatDate(profile.created_at)}</div>
                    {profile.last_used_at && (
                      <div className="muted">Last used {formatDate(profile.last_used_at)}</div>
                    )}
                  </div>
                  <button type="button" onClick={() => onRevokeMobileProfile(profile)}>
                    Revoke
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <form id="ssh-form" onSubmit={onConnectSsh} className="card" style={{ marginTop: 18 }}>
        <div className="row">
          <strong>Connect via SSH</strong>
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10, marginTop: 12 }}>
          <label>
            <div className="muted">Host</div>
            <input value={sshHost} onChange={(e) => setSshHost(e.target.value)} placeholder="devbox.example.com" />
          </label>
          <label>
            <div className="muted">User (optional)</div>
            <input value={sshUser} onChange={(e) => setSshUser(e.target.value)} placeholder="ubuntu" />
          </label>
          <label>
            <div className="muted">Remote daemon port</div>
            <input
              value={String(sshRemotePort)}
              onChange={(e) => setSshRemotePort(Number(e.target.value) || 4399)}
              inputMode="numeric"
            />
          </label>
          <label>
            <div className="muted">Remote data dir</div>
            <input value={sshDataDir} onChange={(e) => setSshDataDir(e.target.value)} />
          </label>
        </div>

        <div className="row" style={{ marginTop: 12, alignItems: "center", gap: 12 }}>
          <label className="muted" style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
            <input type="checkbox" checked={sshStartRemote} onChange={(e) => setSshStartRemote(e.target.checked)} />
            Start remote daemon (best-effort)
          </label>
          <button type="submit" disabled={busy || wizardBusy || !sshHost.trim()}>
            {busy ? "Connecting…" : "Connect"}
          </button>
        </div>

      <div className="muted" style={{ marginTop: 10 }}>
        Uses SSH port forwarding to connect to a daemon bound on the remote host’s <code>127.0.0.1</code>.
      </div>
    </form>

    {mobileCreateOpen && (
      <div className="modal-overlay">
        <div className="modal">
          <h3>Enable mobile connection</h3>
          <form onSubmit={submitMobileProfile} style={{ display: "grid", gap: 12 }}>
            <label>
              <div className="muted">Label</div>
              <input value={mobileLabel} onChange={(e) => setMobileLabel(e.target.value)} placeholder="Home devbox" />
            </label>
            <label>
              <div className="muted">HTTPS base URL</div>
              <input
                value={mobileBaseUrl}
                onChange={(e) => setMobileBaseUrl(e.target.value)}
                placeholder="https://devbox.example.com"
              />
            </label>
            {mobileCreateError && <div className="error">{mobileCreateError}</div>}
            <div className="modal-actions">
              <button type="button" onClick={closeCreateMobileModal} disabled={mobileCreateBusy}>
                Cancel
              </button>
              <button type="submit" disabled={mobileCreateBusy}>
                {mobileCreateBusy ? "Creating…" : "Create"}
              </button>
            </div>
          </form>
        </div>
      </div>
    )}

    {qrModal && (
      <div className="modal-overlay">
        <div className="modal">
          <h3>Scan with ctx mobile</h3>
          <p className="muted">
            Scan this QR in the ctx mobile app or copy the token below. Store it securely—it's only shown once.
          </p>
          <div style={{ display: "flex", justifyContent: "center", marginBottom: 12 }}>
            <QRCodeSVG value={JSON.stringify(qrModal.qr_payload)} size={220} bgColor="transparent" fgColor="#f5f7ff" />
          </div>
          <div className="mobile-token">{qrModal.token}</div>
          <div className="modal-actions" style={{ marginTop: 16 }}>
            <button type="button" onClick={closeQrModal}>
              Close
            </button>
          </div>
        </div>
      </div>
    )}
    </div>
    </>
  );
}

function lastSegment(path: string): string {
  const s = String(path || "").trim().replace(/\/+$/, "");
  const idx = s.lastIndexOf("/");
  return idx >= 0 ? s.slice(idx + 1) : s;
}

function formatDate(value?: string | null): string {
  if (!value) return "never";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
