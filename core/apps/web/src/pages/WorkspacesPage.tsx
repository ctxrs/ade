import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { createWorkspace, listWorkspaces, Workspace } from "../api/client";

export default function WorkspacesPage() {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [rootPath, setRootPath] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    listWorkspaces()
      .then(setWorkspaces)
      .catch((e) => setError(e.message));

  useEffect(() => {
    refresh();
  }, []);

  const onCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      await createWorkspace(rootPath, name || undefined);
      setRootPath("");
      setName("");
      refresh();
    } catch (e: any) {
      setError(e.message);
    }
  };

  return (
    <div className="page">
      <h1>Workspaces</h1>

      <form onSubmit={onCreate} className="card">
        <label>
          Root path
          <input value={rootPath} onChange={(e) => setRootPath(e.target.value)} />
        </label>
        <label>
          Name (optional)
          <input value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <button type="submit">Add workspace</button>
        {error && <div className="error">{error}</div>}
      </form>

      <ul className="list">
        {workspaces.map((ws) => {
          const id = (ws as any).id?.["0"] ?? (ws as any).id;
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

