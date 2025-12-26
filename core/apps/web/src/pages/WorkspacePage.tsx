import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  createTask,
  createWorkspaceAttachment,
  getWorkspace,
  listTasks,
  listWorkspaceAttachments,
  syncWorkspaceAttachments,
  Task,
  Workspace,
  WorkspaceAttachment,
  idToString,
} from "../api/client";

export default function WorkspacePage() {
  const { id } = useParams<{ id: string }>();
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [attachments, setAttachments] = useState<WorkspaceAttachment[]>([]);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [attachKind, setAttachKind] = useState<"reference_repo" | "doc_mirror">("reference_repo");
  const [attachName, setAttachName] = useState("");
  const [attachSource, setAttachSource] = useState("");
  const [attachRevision, setAttachRevision] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    if (!id) return;
    setWorkspace(await getWorkspace(id));
    setTasks(await listTasks(id));
    setAttachments(await listWorkspaceAttachments(id));
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

  const onAttachmentCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!id) return;
    setError(null);
    try {
      await createWorkspaceAttachment(id, {
        kind: attachKind,
        name: attachName.trim(),
        source: attachSource.trim(),
        revision: attachRevision.trim() || undefined,
      });
      setAttachName("");
      setAttachSource("");
      setAttachRevision("");
      refresh();
    } catch (e: any) {
      setError(e.message);
    }
  };

  const onAttachmentSync = async () => {
    if (!id) return;
    setError(null);
    try {
      await syncWorkspaceAttachments(id, { refresh: true });
      refresh();
    } catch (e: any) {
      setError(e.message);
    }
  };

  return (
    <div className="page">
      <Link to="/workspaces">← Workspaces</Link>
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

      <div className="card">
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <h2 style={{ margin: 0 }}>Attachments</h2>
          <button type="button" onClick={onAttachmentSync}>
            Sync attachments
          </button>
        </div>
        <form onSubmit={onAttachmentCreate} style={{ marginTop: 12 }}>
          <label>
            Kind
            <select value={attachKind} onChange={(e) => setAttachKind(e.target.value as any)}>
              <option value="reference_repo">Reference repo</option>
              <option value="doc_mirror">Doc mirror</option>
            </select>
          </label>
          <label>
            Name
            <input value={attachName} onChange={(e) => setAttachName(e.target.value)} />
          </label>
          <label>
            Source
            <input value={attachSource} onChange={(e) => setAttachSource(e.target.value)} />
          </label>
          <label>
            Revision (optional)
            <input value={attachRevision} onChange={(e) => setAttachRevision(e.target.value)} />
          </label>
          <button type="submit" disabled={!attachName.trim() || !attachSource.trim()}>
            Add attachment
          </button>
        </form>

        <ul className="list" style={{ marginTop: 12 }}>
          {attachments.map((att) => {
            const aid = idToString(att.id);
            return (
              <li key={aid}>
                <div>
                  <strong>{att.name}</strong> <span className="muted">({att.kind})</span>
                </div>
                <div className="muted">{att.source}</div>
                {att.revision && <div className="muted">Revision: {att.revision}</div>}
                <div className="muted">Mount: {att.mount_relpath}</div>
              </li>
            );
          })}
          {attachments.length === 0 && <li className="muted">No attachments yet.</li>}
        </ul>
      </div>

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
