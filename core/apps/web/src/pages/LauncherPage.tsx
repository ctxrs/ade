import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  applyDaemonDesktopConnection,
  getHealth,
  idToString,
  listWorkspaces,
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
import { loadLauncherRecents, upsertLauncherRecent, type LauncherRecentEntry } from "../state/launcherRecentsStore";

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

async function existingWorkspaceIdByPath(rootPath: string): Promise<string | null> {
  const all = await listWorkspaces();
  const hit = all.find((w) => String(w.root_path) === rootPath);
  if (!hit) return null;
  return idToString(hit.id ?? "");
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
    loadLauncherRecents()
      .then((next) => {
        if (cancelled) return;
        setRecents(next);
        if (!isDesktop) return;
        const localEntries: DesktopDockRecentLocalWorkspace[] = next
          .filter((entry): entry is Extract<LauncherRecentEntry, { kind: "local" }> => entry.kind === "local")
          .map((entry) => ({
            label: entry.label,
            root_path: entry.root_path,
          }));
        void desktopSetDockRecentLocalWorkspaces(localEntries).catch(() => {});
      })
      .catch(() => {
        if (cancelled) return;
        setRecents([]);
        if (!isDesktop) return;
        void desktopSetDockRecentLocalWorkspaces([]).catch(() => {});
      });
    return () => {
      cancelled = true;
    };
  }, [busy, isDesktop]);

  const connectLocalAndOpen = async (rootPath?: string) => {
    setError(null);
    setBusy(true);
    try {
      const info = await desktopConnectLocal();
      setConnection(info);
      applyConnection(info);
      // Avoid landing on workspaces while the daemon is still booting.
      await waitForDaemonReady(15000);
      if (rootPath) {
        const wsId = await existingWorkspaceIdByPath(rootPath);
        if (!wsId) {
          setError("Workspace not found for this path. Re-create it from New Workspace.");
          navigate("/workspace-setup");
          return;
        }
        try {
          await upsertLauncherRecent({
            kind: "local",
            label: lastSegment(rootPath),
            root_path: rootPath,
            updated_at_ms: Date.now(),
          });
        } catch {
          // best-effort only; do not block workspace open on recents persistence
        }
        navigate(`/workspaces/${wsId}`, { replace: true });
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
        await connectLocalAndOpen(r.root_path);
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
      try {
        await upsertLauncherRecent({ ...r, updated_at_ms: Date.now() });
      } catch {
        // best-effort only; do not block connection flow on recents persistence
      }
      navigate("/", { replace: true });
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
                const key = r.kind === "local" ? `local:${r.root_path}` : `ssh:${r.user ?? ""}@${r.host}:${r.remote_port}`;
                const location = r.kind === "local"
                  ? `Local: ${r.root_path}`
                  : `Remote [${r.label}]: ${r.remote_data_dir ?? "/workspace"}`;
                return (
                  <button
                    type="button"
                    key={key}
                    className="launcher-recent-item"
                    onClick={() => onOpenRecent(r)}
                    disabled={busy}
                  >
                    <span className="launcher-recent-name">{r.label}</span>
                    <span className="launcher-recent-location">{location}</span>
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
