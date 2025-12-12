import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { createTask, getWorkspace, listTasks, Task, Workspace, idToString } from "../api/client";

export default function WorkspacePage() {
  const { id } = useParams<{ id: string }>();
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    if (!id) return;
    setWorkspace(await getWorkspace(id));
    setTasks(await listTasks(id));
  };

  useEffect(() => {
    refresh().catch((e) => setError(e.message));
  }, [id]);

  const onCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!id) return;
    setError(null);
    try {
      await createTask(id, title, description || undefined);
      setTitle("");
      setDescription("");
      refresh();
    } catch (e: any) {
      setError(e.message);
    }
  };

  return (
    <div className="page">
      <Link to="/">← Workspaces</Link>
      {workspace && (
        <>
          <h1>{workspace.name}</h1>
          <div className="muted">{workspace.root_path}</div>
        </>
      )}

      <form onSubmit={onCreate} className="card">
        <label>
          Task title
          <input value={title} onChange={(e) => setTitle(e.target.value)} />
        </label>
        <label>
          Description
          <textarea value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        <button type="submit" disabled={!title.trim()}>
          Create task
        </button>
        {error && <div className="error">{error}</div>}
      </form>

      <ul className="list">
        {tasks.map((t) => {
          const tid = idToString(t.id);
          return (
            <li key={tid}>
              <Link to={`/tasks/${tid}`}>{t.title}</Link>
              <div className="muted">{t.status}</div>
            </li>
          );
        })}
        {tasks.length === 0 && <li className="muted">No tasks yet.</li>}
      </ul>
    </div>
  );
}

