import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  applyDaemonDesktopConnection,
  getHealth,
  getWorkspaceExecutionConfig,
  idToString,
  listWorkspaces,
  repoStatus,
} from "../api/client";
import {
  desktopConnectLocal,
  desktopConnectSsh,
  desktopGetConnection,
  desktopSetDockRecentLocalWorkspaces,
  isDesktopApp,
  type DesktopConnectionInfo,
  type DesktopDockRecentLocalWorkspace,
} from "../utils/desktop";
import { errorMessage } from "../utils/errorMessage";
import LauncherBrand from "../components/LauncherBrand";
import {
  loadLauncherRecents,
  upsertLauncherRecent,
  type LauncherExecutionEnvironment,
  type LauncherRecentEntry,
} from "../state/launcherRecentsStore";

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

type WorkspaceSummary = Awaited<ReturnType<typeof listWorkspaces>>[number];

type ResolvedWorkspace = {
  workspaceId: string;
  rootPath: string;
  label: string;
};

function normalizeWorkspacePathForCompare(path: string): string {
  const normalized = String(path || "").trim().replace(/\\/g, "/");
  if (!normalized) return "";

  const windowsDriveRoot = normalized.match(/^([A-Za-z]):\/?$/);
  if (windowsDriveRoot) {
    return `${windowsDriveRoot[1].toLowerCase()}:/`;
  }

  const withoutTrailing = normalized === "/" ? "/" : normalized.replace(/\/+$/, "");
  const windowsDrivePath = withoutTrailing.match(/^([A-Za-z]):(\/.*)$/);
  if (windowsDrivePath) {
    return `${windowsDrivePath[1].toLowerCase()}:${windowsDrivePath[2]}`;
  }

  return withoutTrailing || "/";
}

function findWorkspaceByPath(
  workspaces: WorkspaceSummary[],
  candidatePath: string,
): ResolvedWorkspace | null {
  const normalizedCandidate = normalizeWorkspacePathForCompare(candidatePath);
  if (!normalizedCandidate) return null;

  for (const workspace of workspaces) {
    const workspaceId = idToString(workspace.id ?? "").trim();
    const workspaceRootPath = String(workspace.root_path ?? "").trim();
    if (!workspaceId || !workspaceRootPath) continue;
    if (normalizeWorkspacePathForCompare(workspaceRootPath) !== normalizedCandidate) continue;
    const workspaceLabel = String(workspace.name ?? "").trim();
    return {
      workspaceId,
      rootPath: workspaceRootPath,
      label: workspaceLabel || lastSegment(workspaceRootPath),
    };
  }

  return null;
}

async function resolveWorkspaceByPath(rootPath: string): Promise<ResolvedWorkspace | null> {
  const all = await listWorkspaces();
  const directMatch = findWorkspaceByPath(all, rootPath);
  if (directMatch) return directMatch;

  const trimmedRootPath = String(rootPath || "").trim();
  if (!trimmedRootPath) return null;

  try {
    const status = await repoStatus({ path: trimmedRootPath });
    const canonicalPath = String(status.canonical_path ?? "").trim();
    if (!canonicalPath) return null;
    return findWorkspaceByPath(all, canonicalPath);
  } catch {
    return null;
  }
}

export default function LauncherPage() {
  const navigate = useNavigate();
  const [connection, setConnection] = useState<DesktopConnectionInfo | null>(null);
  const [recents, setRecents] = useState<LauncherRecentEntry[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isDesktop = isDesktopApp();

  useEffect(() => {
    if (!isDesktop) {
      setConnection({ kind: "none" });
      return;
    }
    desktopGetConnection()
      .then((info) => {
        setConnection(info);
        applyConnection(info);
      })
      .catch(() => setConnection({ kind: "none" }));
  }, [isDesktop, navigate]);

  useEffect(() => {
    let cancelled = false;
    const syncDockRecents = (entries: LauncherRecentEntry[]) => {
      if (!isDesktop) return;
      const localEntries: DesktopDockRecentLocalWorkspace[] = entries
        .filter((entry): entry is Extract<LauncherRecentEntry, { kind: "local" }> => entry.kind === "local")
        .map((entry) => ({
          label: entry.label,
          root_path: entry.root_path,
        }));
      void desktopSetDockRecentLocalWorkspaces(localEntries).catch(() => {});
    };

    const loadRecents = async () => {
      try {
        const persisted = await loadLauncherRecents();
        if (cancelled) return;
        if (persisted.length > 0) {
          setRecents(persisted);
          syncDockRecents(persisted);
          return;
        }

        const workspaces = await listWorkspaces();
        if (cancelled) return;
        const inferred = await recentsFromWorkspaces(workspaces);
        setRecents(inferred);
        syncDockRecents(inferred);
      } catch {
        if (cancelled) return;
        setRecents([]);
        syncDockRecents([]);
      }
    };

    void loadRecents();
    return () => {
      cancelled = true;
    };
  }, [busy, isDesktop]);

  const connectLocalAndOpen = async (rootPath?: string, executionEnvironment?: LauncherExecutionEnvironment) => {
    setError(null);
    setBusy(true);
    try {
      const info = await desktopConnectLocal();
      setConnection(info);
      applyConnection(info);
      // Avoid landing on workspaces while the daemon is still booting.
      await waitForDaemonReady(15000);
      if (rootPath) {
        const resolvedWorkspace = await resolveWorkspaceByPath(rootPath);
        if (!resolvedWorkspace) {
          setError("Workspace not found for this path. Re-create it from New Workspace.");
          navigate("/workspace-setup");
          return;
        }
        try {
          await upsertLauncherRecent({
            kind: "local",
            label: resolvedWorkspace.label,
            root_path: resolvedWorkspace.rootPath,
            execution_environment: executionEnvironment ?? inferLocalExecutionEnvironment(resolvedWorkspace.rootPath),
            updated_at_ms: Date.now(),
          });
        } catch {
          // best-effort only; do not block workspace open on recents persistence
        }
        navigate(`/workspaces/${resolvedWorkspace.workspaceId}`, { replace: true });
      } else {
        navigate("/", { replace: true });
      }
    } catch (e: unknown) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const onOpenRecent = async (r: LauncherRecentEntry) => {
    setError(null);
    setBusy(true);
    try {
      if (r.kind === "local") {
        await connectLocalAndOpen(r.root_path, r.execution_environment);
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
      // Avoid landing on workspaces while the daemon is still booting / tunnel is coming up.
      await waitForDaemonReady(15000);
      const targetWorkspaceRootPath = String(r.workspace_root_path ?? "").trim();
      const resolvedWorkspace = targetWorkspaceRootPath
        ? await resolveWorkspaceByPath(targetWorkspaceRootPath)
        : null;
      if (targetWorkspaceRootPath && !resolvedWorkspace) {
        setError("Workspace not found on the connected host for this path. Re-create it from New Workspace.");
        navigate("/workspace-setup");
        return;
      }
      try {
        await upsertLauncherRecent({
          ...r,
          ...(resolvedWorkspace
            ? {
                label: resolvedWorkspace.label,
                workspace_root_path: resolvedWorkspace.rootPath,
              }
            : {}),
          updated_at_ms: Date.now(),
        });
      } catch {
        // best-effort only; do not block connection flow on recents persistence
      }
      navigate(resolvedWorkspace ? `/workspaces/${resolvedWorkspace.workspaceId}` : "/", { replace: true });
    } catch (e: unknown) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const onNewWorkspace = () => {
    navigate("/workspace-setup");
  };

  return (
    <div className="launcher-shell launcher-shell--crt">
      <LauncherBrand fullScreen>
        <div className="launcher-panel">
          <div className="launcher-actions">
            <button type="button" className="launcher-action" onClick={onNewWorkspace} disabled={busy}>
              <span className="launcher-action-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2">
                  <path d="M12 5v14" />
                  <path d="M5 12h14" />
                </svg>
              </span>
              <span className="launcher-action-label">New Workspace</span>
            </button>
          </div>

          {error && <div className="launcher-error">{error}</div>}

          <section className="launcher-recents">
            <div className="launcher-recents-header">
              <strong>Recent Workspaces</strong>
            </div>
            <div className="launcher-recents-list">
              {recents.slice(0, 8).map((r) => {
                const key = recentRenderKey(r);
                const location = recentLocationDisplay(r);
                return (
                  <button
                    type="button"
                    key={key}
                    className="launcher-recent-item"
                    onClick={() => onOpenRecent(r)}
                    disabled={busy}
                  >
                    <span className="launcher-recent-name">{r.label}</span>
                    <span className="launcher-recent-location" title={location.title}>{location.label}</span>
                  </button>
                );
              })}
              {recents.length === 0 && <div className="launcher-empty">No recent workspaces yet.</div>}
            </div>
          </section>
        </div>
      </LauncherBrand>
    </div>
  );
}

function lastSegment(path: string): string {
  const s = String(path || "").trim().replace(/\/+$/, "");
  const idx = s.lastIndexOf("/");
  return idx >= 0 ? s.slice(idx + 1) : s;
}

function isDaemonManagedLocalContainerPath(path: string): boolean {
  const normalized = String(path || "")
    .trim()
    .replace(/\\/g, "/")
    .toLowerCase();
  return normalized.includes("/.ctx/workspaces/") || normalized.includes("/workspaces/staging/");
}

function normalizeExecutionEnvironment(value: unknown): LauncherExecutionEnvironment | undefined {
  if (value === "host" || value === "sandbox") {
    return value;
  }
  return undefined;
}

function inferLocalExecutionEnvironment(path: string): LauncherExecutionEnvironment {
  return isDaemonManagedLocalContainerPath(path) ? "sandbox" : "host";
}

function pathForDisplay(path: string): string {
  const normalized = String(path || "").trim().replace(/\\/g, "/");
  if (!normalized) return normalized;
  if (normalized.startsWith("~")) return normalized;

  const macosHome = normalized.match(/^\/Users\/[^/]+(\/.*)?$/);
  if (macosHome) return `~${macosHome[1] ?? ""}`;

  const linuxHome = normalized.match(/^\/home\/[^/]+(\/.*)?$/);
  if (linuxHome) return `~${linuxHome[1] ?? ""}`;

  const windowsHome = normalized.match(/^[A-Za-z]:\/Users\/[^/]+(\/.*)?$/);
  if (windowsHome) return `~${windowsHome[1] ?? ""}`;

  return normalized;
}

function recentLocationDisplay(recent: LauncherRecentEntry): { label: string; title: string } {
  if (recent.kind === "local") {
    const env = normalizeExecutionEnvironment(recent.execution_environment) ?? inferLocalExecutionEnvironment(recent.root_path);
    if (env === "sandbox") {
      return {
        label: "Local sandbox",
        title: recent.root_path,
      };
    }
    const displayPath = pathForDisplay(recent.root_path);
    return {
      label: `${displayPath} (Host)`,
      title: `${recent.root_path} (Host)`,
    };
  }

  const target = sshTarget(recent);
  const env = normalizeExecutionEnvironment(recent.execution_environment);
  if (env === "sandbox") {
    return {
      label: `${target} (Remote sandbox)`,
      title: `Remote sandbox on ${target}`,
    };
  }

  const workspaceRootPath = String(recent.workspace_root_path ?? "").trim();
  if (workspaceRootPath) {
    const displayPath = pathForDisplay(workspaceRootPath);
    return {
      label: `${target}:${displayPath} (Host)`,
      title: `${target}:${workspaceRootPath} (Host)`,
    };
  }

  const remoteDir = recent.remote_data_dir?.trim() || "/workspace";
  return {
    label: `Remote daemon (${target})`,
    title: `Host ${target} (data dir: ${remoteDir})`,
  };
}

function sshTarget(recent: Extract<LauncherRecentEntry, { kind: "ssh" }>): string {
  const user = recent.user?.trim();
  return user ? `${user}@${recent.host}` : recent.host;
}

async function recentsFromWorkspaces(workspaces: Awaited<ReturnType<typeof listWorkspaces>>): Promise<LauncherRecentEntry[]> {
  const entries = await Promise.all(workspaces
    .filter((workspace) => workspace.root_path.trim().length > 0)
    .map(async (workspace) => {
      const workspaceId = idToString(workspace.id ?? "").trim();
      let executionEnvironment = inferLocalExecutionEnvironment(workspace.root_path);
      if (workspaceId) {
        try {
          const config = await getWorkspaceExecutionConfig(workspaceId);
          executionEnvironment = normalizeExecutionEnvironment(config.environment) ?? executionEnvironment;
        } catch {
          // keep inferred fallback
        }
      }
      return {
        kind: "local" as const,
        label: workspace.name.trim() || lastSegment(workspace.root_path),
        root_path: workspace.root_path,
        execution_environment: executionEnvironment,
        updated_at_ms: workspaceCreatedAtMs(workspace.created_at),
      };
    }));
  return entries.sort((a, b) => b.updated_at_ms - a.updated_at_ms);
}

function workspaceCreatedAtMs(createdAt: string): number {
  const parsed = Date.parse(createdAt);
  if (!Number.isFinite(parsed)) return 0;
  return parsed;
}

function recentRenderKey(recent: LauncherRecentEntry): string {
  if (recent.kind === "local") return `local:${recent.root_path}`;
  return `ssh:${recent.user ?? ""}@${recent.host}:${recent.remote_port}:${recent.workspace_root_path ?? ""}:${recent.execution_environment ?? ""}`;
}
