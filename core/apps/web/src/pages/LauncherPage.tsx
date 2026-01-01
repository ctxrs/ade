import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import {
  createMobileConnectionProfile,
  createWorkspace,
  deleteMobileConnectionProfile,
  getDaemonBaseUrl,
  idToString,
  listMobileConnectionProfiles,
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
  desktopGitClone,
  desktopPickFolder,
  isDesktopApp,
  type DesktopConnectionInfo,
} from "../utils/desktop";

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
      auth_token?: string | null;
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

  const onOpenLocalProject = async () => {
    setError(null);
    try {
      const folder = await desktopPickFolder();
      if (!folder) return;
      await connectLocalAndOpen(folder);
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
      await connectLocalAndOpen(rootPath);
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
  const [sshToken, setSshToken] = useState("");
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
        auth_token: sshToken.trim() || null,
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
        auth_token: String(info.token ?? sshToken).trim() || null,
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
        await connectLocalAndOpen(r.root_path);
        return;
      }
      const info = await desktopConnectSsh({
        host: r.host,
        user: r.user ?? null,
        remote_port: r.remote_port,
        start_remote: Boolean(r.start_remote),
        auth_token: r.auth_token ?? null,
        remote_data_dir: r.remote_data_dir ?? null,
      });
      setConnection(info);
      applyConnection(info);
      upsertRecent({ ...r, auth_token: String(info.token ?? r.auth_token ?? "").trim() || null, updated_at_ms: Date.now() });
      navigate("/workspaces", { replace: true });
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(false);
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

  return (
    <>
    <div className="page">
      <div className="row" style={{ alignItems: "baseline" }}>
        <h1 style={{ marginRight: "auto" }}>ctx</h1>
        <Link to="/app-settings">Settings</Link>
      </div>

      <div className="muted" style={{ marginTop: 6 }}>{connectedLabel}</div>

      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 12, marginTop: 18 }}>
        <button type="button" onClick={onOpenLocalProject} disabled={busy}>
          Open local project
        </button>
        <button type="button" onClick={onCloneRepo} disabled={busy}>
          Clone repo
        </button>
        <button type="button" onClick={() => {
          const el = document.getElementById("ssh-form");
          el?.scrollIntoView({ behavior: "smooth", block: "start" });
        }} disabled={busy}>
          Connect via SSH
        </button>
      </div>

      {error && <div className="error" style={{ marginTop: 12 }}>{error}</div>}

      <div className="card" style={{ marginTop: 18 }}>
        <div className="row" style={{ marginBottom: 6 }}>
          <strong>Recent</strong>
          <div style={{ marginLeft: "auto" }} className="muted">{recents.length ? `${recents.length}` : ""}</div>
        </div>
        <ul className="list">
          {recents.slice(0, 8).map((r) => (
            <li key={r.kind === "local" ? `local:${r.root_path}` : `ssh:${r.user ?? ""}@${r.host}:${r.remote_port}`}>
              <button type="button" className="linklike" onClick={() => onOpenRecent(r)} disabled={busy} style={{ padding: 0 }}>
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
          Generate HTTPS + API-token QR codes for the Expo app. Tokens are only shown once.
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
          <label style={{ gridColumn: "1 / span 2" }}>
            <div className="muted">Auth token (optional)</div>
            <input value={sshToken} onChange={(e) => setSshToken(e.target.value)} placeholder="(leave empty to auto-generate when starting remote)" />
          </label>
        </div>

        <div className="row" style={{ marginTop: 12, alignItems: "center", gap: 12 }}>
          <label className="muted" style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
            <input type="checkbox" checked={sshStartRemote} onChange={(e) => setSshStartRemote(e.target.checked)} />
            Start remote daemon (best-effort)
          </label>
          <button type="submit" disabled={busy || !sshHost.trim()}>
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
            Scan this QR in the Expo app or copy the token below. Store it securely—it's only shown once.
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
