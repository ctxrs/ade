import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { createWorkspace, getDaemonBaseUrl, idToString, listWorkspaces, setDaemonAuthToken, setDaemonBaseUrl } from "../api/client";
import {
  desktopConnectLocal,
  desktopConnectSsh,
  desktopGetConnection,
  isDesktopApp,
  type DesktopConnectionInfo,
} from "../utils/desktop";
import LauncherBrand from "../components/LauncherBrand";

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
  setDaemonAuthToken(token || null);
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
  const [recents, setRecents] = useState<RecentEntry[]>(() => loadRecents());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isDesktop = isDesktopApp();

  useEffect(() => {
    if (!isDesktop) {
      setConnection({ kind: "none" } as any);
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
    setRecents(loadRecents());
  }, [busy]);

  const connectedLabel = useMemo(() => {
    if (!connection || connection.kind === "none") return "";
    const base = String(connection.base_url ?? getDaemonBaseUrl() ?? "").trim();
    return `${connection.kind.toUpperCase()} · ${base || "(unknown)"}`;
  }, [connection]);

  const connectLocalAndOpen = async (rootPath?: string) => {
    setError(null);
    setBusy(true);
    try {
      const info = await desktopConnectLocal();
      setConnection(info);
      applyConnection(info);
      if (rootPath) {
        const wsId = await createOrOpenWorkspaceByPath(rootPath);
        upsertRecent({ kind: "local", label: lastSegment(rootPath), root_path: rootPath, updated_at_ms: Date.now() });
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

  const onOpenRecent = async (r: RecentEntry) => {
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
      upsertRecent({ ...r, updated_at_ms: Date.now() });
      navigate("/workspaces", { replace: true });
    } catch (e: any) {
      setError(e?.message ?? String(e));
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

          {connectedLabel && (
            <div className="launcher-footer">
              <div className="launcher-connection">{connectedLabel}</div>
            </div>
          )}
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
