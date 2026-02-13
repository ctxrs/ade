import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { createWorkspace, idToString, listProviders, listWorkspaces, ProviderStatus, Workspace } from "../api/client";
import { errorMessage } from "../utils/errorMessage";
import UpdateNoticeBanner from "../components/UpdateNoticeBanner";

export default function WorkspacesPage() {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [rootPath, setRootPath] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    listWorkspaces()
      .then(setWorkspaces)
      .catch((e) => setError(e.message));

  useEffect(() => {
    refresh();
    listProviders().then(setProviders).catch(() => {});
  }, []);

  const onCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      await createWorkspace(rootPath, name || undefined);
      setRootPath("");
      setName("");
      refresh();
    } catch (e: unknown) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="page">
      <div className="row">
        <h1 style={{ marginRight: "auto" }}>Workspaces</h1>
        <Link to="/">Launcher</Link>
        <Link to="/app-settings" style={{ marginLeft: 12 }}>Settings</Link>
        <Link to="/settings#agent_harnesses" style={{ marginLeft: 12 }}>Agent Harnesses</Link>
      </div>

      {providers.some(
        (p) =>
          p.details?.install_supported === "true" && (!p.installed || p.health !== "ok"),
      ) && (
        <div className="banner">
          Some harnesses are not ready. <Link to="/settings#agent_harnesses">Install or update harnesses</Link>.
        </div>
      )}
      <UpdateNoticeBanner />

      <form onSubmit={onCreate} className="card">
        <label>
          Root path
          <input
            value={rootPath}
            onChange={(e) => setRootPath(e.target.value)}
          />
          <div className="muted" style={{ marginTop: 6 }}>
            Must be a git repo root (contains <code>.git</code>). Tilde (<code>~</code>) is supported.
          </div>
        </label>
        <label>
          Name (optional)
          <input value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <button type="submit">Add workspace</button>
        {error && (
          <div className="error">
            <div>{error}</div>
            {(error === "400 Bad Request" || error.startsWith("400 ")) && (
              <div className="muted" style={{ marginTop: 6 }}>
                Tip: use an absolute path (no <code>~</code>) and ensure the daemon is rebuilt/restarted.
              </div>
            )}
          </div>
        )}
      </form>

      {error?.toLowerCase().includes("not connected") && (
        <div className="banner" style={{ marginTop: 12 }}>
          Not connected. Go to <Link to="/">Launcher</Link> to connect to a host.
        </div>
      )}

      <ul className="list">
        {workspaces.map((ws) => {
          const id = idToString(ws.id ?? "");
          return (
            <li key={id}>
              <Link to={`/workspaces/${id}`}>{ws.name}</Link>
              <div className="muted">{ws.root_path}</div>
            </li>
          );
        })}
        {workspaces.length === 0 && <li className="muted">No workspaces yet.</li>}
      </ul>
    </div>
  );
}
