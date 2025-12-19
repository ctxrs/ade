import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  createSession,
  getTask,
  listProviders,
  listSessionsForTrack,
  listTracks,
  ProviderStatus,
  Session,
  Track,
  Task,
  idToString,
} from "../api/client";

export default function TaskPage() {
  const { id } = useParams<{ id: string }>();
  const [task, setTask] = useState<Task | null>(null);
  const [tracks, setTracks] = useState<Track[]>([]);
  const [sessionsByTrack, setSessionsByTrack] = useState<Record<string, Session[]>>({});
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerId, setProviderId] = useState("codex");
  const [modelId, setModelId] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    if (!id) return;
    setTask(await getTask(id));
    const trs = await listTracks(id);
    setTracks(trs);
    const map: Record<string, Session[]> = {};
    await Promise.all(
      trs.map(async (tr) => {
        const trid = idToString(tr.id);
        map[trid] = await listSessionsForTrack(trid);
      }),
    );
    setSessionsByTrack(map);
  };

  useEffect(() => {
    refresh().catch((e) => setError(e.message));
  }, [id]);

  useEffect(() => {
    listProviders()
      .then(setProviders)
      .catch(() => undefined);
  }, []);

  const onCreateSession = async (trackId: string) => {
    setError(null);
    try {
      const s = await createSession(trackId, providerId, modelId);
      const sid = idToString(s.id);
      window.location.href = `/sessions/${sid}`;
    } catch (e: any) {
      setError(e.message);
    }
  };

  return (
    <div className="page">
      <Link to={`/workspaces/${task ? idToString(task.workspace_id) : ""}`}>← Workspace</Link>
      {task && (
        <>
          <h1>{task.title}</h1>
          {task.description && <div>{task.description}</div>}
        </>
      )}

      <div className="card">
        <h2>New session</h2>
        <label>
          Provider
          {providers.length > 0 ? (
            <select value={providerId} onChange={(e) => setProviderId(e.target.value)}>
              {providers
                .filter((p) => p.details?.ui_hidden !== "true")
                .map((p) => (
                  <option
                    key={p.provider_id}
                    value={p.provider_id}
                    disabled={!p.installed || p.health !== "ok"}
                  >
                    {p.provider_id} {p.installed && p.health === "ok" ? "" : "(missing)"}
                  </option>
                ))}
            </select>
          ) : (
            <input value={providerId} onChange={(e) => setProviderId(e.target.value)} />
          )}
        </label>
        <label>
          Model
          <input value={modelId} onChange={(e) => setModelId(e.target.value)} />
        </label>
        {error && <div className="error">{error}</div>}
      </div>

      <h2>Tracks</h2>
      <ul className="list">
        {tracks.map((tr) => {
          const trid = idToString(tr.id);
          const sessions = sessionsByTrack[trid] || [];
          return (
            <li key={trid}>
              <div>
                {tr.label} – {tr.status}
              </div>
              <button onClick={() => onCreateSession(trid)}>Create session</button>
              {sessions.length > 0 && (
                <ul className="sublist">
                  {sessions.map((s) => {
                    const sid = idToString(s.id);
                    return (
                      <li key={sid}>
                        <Link to={`/sessions/${sid}`}>
                          {s.provider_id}/{s.model_id} – {s.status}
                        </Link>
                      </li>
                    );
                  })}
                </ul>
              )}
            </li>
          );
        })}
        {tracks.length === 0 && <li className="muted">No tracks yet.</li>}
      </ul>
    </div>
  );
}
