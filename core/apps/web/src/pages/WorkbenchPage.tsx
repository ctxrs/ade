import { useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  ProviderOptions,
  ProviderStatus,
  Task,
  Track,
  Workspace,
  applyTrackDiffPatch,
  createSession,
  createTask,
  createTrack,
  getProviderOptions,
  getWorkspace,
  idToString,
  listProviders,
  listSessionsForTrack,
  listTasks,
  listTracks,
  postMessage,
} from "../api/client";
import { useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { DiffReviewPane } from "../components/DiffReviewPane";
import { SessionView } from "./SessionPage";

type DraftTrack = {
  key: string;
  label: string;
  providerId: string;
  modelId: string;
};

const CODEX_EFFORTS = ["none", "minimal", "low", "medium", "high", "xhigh"] as const;

function deriveTaskTitle(prompt: string): string {
  const line = prompt.trim().split("\n")[0] ?? "";
  const short = line.trim().slice(0, 60);
  return short.length > 0 ? short : "New conversation";
}

function modelIdsFromOptions(opts?: ProviderOptions): string[] {
  const raw = opts?.models;
  if (!raw) return [];
  const arr = Array.isArray(raw) ? raw : raw?.models;
  if (!Array.isArray(arr)) return [];
  const ids: string[] = [];
  for (const it of arr) {
    if (typeof it === "string") ids.push(it);
    else if (it && typeof it === "object") {
      const id = (it as any).id ?? (it as any).modelId ?? (it as any).model_id;
      if (typeof id === "string") ids.push(id);
    }
  }
  return [...new Set(ids)].slice(0, 200);
}

function codexBaseFromModelId(modelId: string): { base: string; effort?: string } {
  const parts = String(modelId ?? "").split("/");
  if (parts.length >= 2) return { base: parts[0], effort: parts[1] };
  return { base: String(modelId ?? ""), effort: undefined };
}

function workbenchLabelForTrack(d: DraftTrack): string {
  if (d.label.trim().length > 0) return d.label.trim();
  return `${d.providerId} · ${d.modelId}`;
}

export default function WorkbenchPage() {
  const { id: workspaceId } = useParams<{ id: string }>();
  const supervisor = useSessionSupervisor();

  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions>>({});

  const [tasks, setTasks] = useState<Task[]>([]);
  const [taskQuery, setTaskQuery] = useState("");
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);

  const [tracks, setTracks] = useState<Track[]>([]);
  const [sessionsByTrack, setSessionsByTrack] = useState<Record<string, any[]>>({});
  const [activeTrackId, setActiveTrackId] = useState<string | null>(null);

  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
    { key: "t1", label: "", providerId: "codex", modelId: "" },
  ]);
  const [execTarget, setExecTarget] = useState<"local" | "worktree" | "container">("worktree");

  const [harnessPickerOpen, setHarnessPickerOpen] = useState(false);
  const [contextMenuOpen, setContextMenuOpen] = useState(false);

  const [diffWidth, setDiffWidth] = useState(480);

  const refreshTasks = async () => {
    if (!workspaceId) return;
    setTasks(await listTasks(workspaceId));
  };

  const refreshTaskDetail = async (taskId: string) => {
    const trs = await listTracks(taskId);
    setTracks(trs);
    const map: Record<string, any[]> = {};
    await Promise.all(
      trs.map(async (tr) => {
        const trid = idToString(tr.id);
        map[trid] = await listSessionsForTrack(trid);
      }),
    );
    setSessionsByTrack(map);
    const firstTrackId = trs[0] ? idToString(trs[0].id) : null;
    setActiveTrackId((prev) => prev ?? firstTrackId);
  };

  useEffect(() => {
    if (!workspaceId) return;
    getWorkspace(workspaceId).then(setWorkspace).catch(() => setWorkspace(null));
    refreshTasks().catch(() => {});
    listProviders().then(setProviders).catch(() => setProviders([]));
  }, [workspaceId]);

  useEffect(() => {
    if (!activeTaskId) {
      setTracks([]);
      setSessionsByTrack({});
      setActiveTrackId(null);
      return;
    }
    refreshTaskDetail(activeTaskId).catch(() => {});
  }, [activeTaskId]);

  const filteredTasks = useMemo(() => {
    const q = taskQuery.trim().toLowerCase();
    if (!q) return tasks;
    return tasks.filter((t) => (t.title ?? "").toLowerCase().includes(q));
  }, [tasks, taskQuery]);

  const activeSessionId = useMemo(() => {
    if (!activeTrackId) return null;
    const ss = sessionsByTrack[activeTrackId] ?? [];
    const s = ss[0];
    return s ? idToString((s as any).id) : null;
  }, [activeTrackId, sessionsByTrack]);

  const activeEntry = useSessionEntry(activeSessionId ?? "");
  const activeTrackDiff = activeEntry?.diff ?? "";
  const activeTrackIdFromSession = activeEntry?.session ? idToString(activeEntry.session.track_id) : "";

  const hasDiff = activeTrackDiff.trim().length > 0;
  const diffFileCount = useMemo(() => {
    if (!hasDiff) return 0;
    const m = activeTrackDiff.match(/^diff --git /gm);
    return m ? m.length : 1;
  }, [activeTrackDiff, hasDiff]);

  const ensureProviderOptions = async (providerId: string): Promise<ProviderOptions | undefined> => {
    if (!workspaceId) return;
    if (providerOptions[providerId]) return providerOptions[providerId];
    const opts = await getProviderOptions(workspaceId, providerId);
    setProviderOptions((prev) => ({ ...prev, [providerId]: opts }));
    return opts;
  };

  const summarizeDraftHarnesses = (): string => {
    const counts = new Map<string, number>();
    for (const t of draftTracks) counts.set(t.providerId, (counts.get(t.providerId) ?? 0) + 1);
    const parts = [...counts.entries()].map(([p, n]) => `${n}× ${p}`);
    return parts.length > 0 ? parts.join(", ") : "Pick harnesses";
  };

  const startNewTask = async () => {
    if (!workspaceId) return;
    const prompt = draftPrompt.trim();
    if (!prompt) return;

    const title = deriveTaskTitle(prompt);
    const task = await createTask(workspaceId, title, undefined, { create_default_track: false });
    const taskId = idToString(task.id);

    const toStart = draftTracks.length > 0 ? draftTracks : [{ key: "t1", label: "", providerId: "codex", modelId: "" }];

    for (let i = 0; i < toStart.length; i++) {
      const dt = toStart[i];
      const label = workbenchLabelForTrack(dt);
      const tr = await createTrack(taskId, label);
      const trackId = idToString(tr.id);
      const opts = await ensureProviderOptions(dt.providerId).catch(() => undefined);
      const modelIds = modelIdsFromOptions(opts ?? providerOptions[dt.providerId]);
      const defaultModel = dt.modelId || modelIds[0] || (dt.providerId === "fake" ? "fake-model" : "default");
      const session = await createSession(trackId, dt.providerId, defaultModel);
      const sessionId = idToString(session.id);
      supervisor.openSession(sessionId, { watchDiff: true });
      supervisor.refreshSession(sessionId, { watchDiff: true });
      supervisor.refreshQueue(sessionId);
      await postMessage(sessionId, prompt, "immediate");
    }

    await refreshTasks();
    setActiveTaskId(taskId);
    setDraftPrompt("");
    setHarnessPickerOpen(false);
  };

  const approveAll = async () => {
    if (!activeTrackIdFromSession || !hasDiff) return;
    const resp = await applyTrackDiffPatch(activeTrackIdFromSession, "accept", activeTrackDiff);
    if (activeSessionId) supervisor.setDiff(activeSessionId, resp.diff ?? "");
  };

  const rejectAll = async () => {
    if (!activeTrackIdFromSession || !hasDiff) return;
    const resp = await applyTrackDiffPatch(activeTrackIdFromSession, "reject", activeTrackDiff);
    if (activeSessionId) supervisor.setDiff(activeSessionId, resp.diff ?? "");
  };

  const onSplitterMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = diffWidth;
    const onMove = (ev: MouseEvent) => {
      const dx = startX - ev.clientX;
      const next = Math.min(900, Math.max(320, startW + dx));
      setDiffWidth(next);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const setCodexModel = (idx: number, base: string, effort: string) => {
    const modelId = effort ? `${base}/${effort}` : base;
    setDraftTracks((prev) => prev.map((t, i) => (i === idx ? { ...t, modelId } : t)));
  };

  const activeTask = activeTaskId ? tasks.find((t) => idToString(t.id) === activeTaskId) : null;

  return (
    <div className="wb-root">
      <div className="wb-sidebar">
        <div className="wb-sidebar-top">
          <div className="wb-sidebar-tabs">
            <div className="wb-tab wb-tab-active">Agents</div>
            <div className="wb-tab wb-tab-disabled" title="Coming soon">
              Editor
            </div>
          </div>
          <input
            className="wb-search"
            placeholder="Search Agents…"
            value={taskQuery}
            onChange={(e) => setTaskQuery(e.target.value)}
          />
          <button type="button" className="wb-new-agent" onClick={() => setActiveTaskId(null)}>
            New Agent
          </button>
        </div>

        <div className="wb-sidebar-section">
          <div className="wb-section-title">Pinned</div>
          <div className="wb-muted">No pinned agents yet.</div>
        </div>

        <div className="wb-sidebar-section wb-sidebar-grow">
          <div className="wb-section-title">Agents</div>
          <div className="wb-task-list">
            {filteredTasks.map((t) => {
              const tid = idToString(t.id);
              const selected = tid === activeTaskId;
              return (
                <button
                  key={tid}
                  type="button"
                  className={`wb-task ${selected ? "wb-task-active" : ""}`}
                  onClick={() => setActiveTaskId(tid)}
                >
                  <div className="wb-task-title">{t.title}</div>
                  <div className="wb-task-sub">
                    {new Date(t.updated_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                  </div>
                </button>
              );
            })}
            {filteredTasks.length === 0 && <div className="wb-muted">No agents yet.</div>}
          </div>
        </div>

        <div className="wb-sidebar-bottom">
          <Link className="wb-link" to="/providers">
            Providers
          </Link>
          <Link className="wb-link" to="/diagnostics">
            Diagnostics
          </Link>
        </div>
      </div>

      <div className="wb-main">
        <div className="wb-topbar">
          <div className="wb-topbar-title">{workspace?.name ?? "Workspace"}</div>
          {activeTask && <div className="wb-topbar-sub">{activeTask.title}</div>}
        </div>

        {!activeTaskId && (
          <div className="wb-center">
            <div className="wb-composer-card">
              <textarea
                className="wb-composer-textarea"
                placeholder="Plan, @ for context, / for commands"
                value={draftPrompt}
                onChange={(e) => setDraftPrompt(e.target.value)}
              />
              <div className="wb-composer-row">
                <div className="wb-pill-row">
                  <div className="wb-pill" title="Execution target">
                    <select
                      value={execTarget}
                      onChange={(e) => setExecTarget(e.target.value as any)}
                      className="wb-select"
                    >
                      <option value="worktree">Worktree</option>
                      <option value="local" disabled>
                        Local (disabled)
                      </option>
                      <option value="container" disabled>
                        Container (soon)
                      </option>
                    </select>
                  </div>

                  <div className="wb-pill wb-pill-button" style={{ position: "relative" }}>
                    <button type="button" className="wb-pill-button-inner" onClick={() => setHarnessPickerOpen((v) => !v)}>
                      {summarizeDraftHarnesses()}
                    </button>
                    {harnessPickerOpen && (
                      <div className="wb-popover">
                        <div className="wb-popover-title">Harnesses</div>
                        <div className="wb-popover-body">
                          {draftTracks.map((t, idx) => {
                            const installed = providers.find((p) => p.provider_id === t.providerId)?.installed ?? true;
                            const opts = providerOptions[t.providerId];
                            const modelIds = modelIdsFromOptions(opts);
                            const codex = t.providerId === "codex";
                            const parsed = codexBaseFromModelId(t.modelId);
                            const baseChoices = codex
                              ? [...new Set(modelIds.map((m) => codexBaseFromModelId(m).base).filter(Boolean))]
                              : [];
                            const base = parsed.base || baseChoices[0] || modelIds[0] || "";
                            const effort =
                              parsed.effort && CODEX_EFFORTS.includes(parsed.effort as any) ? parsed.effort : "high";
                            return (
                              <div key={t.key} className="wb-track-row">
                                <input
                                  className="wb-track-label"
                                  placeholder="Track label"
                                  value={t.label}
                                  onChange={(e) =>
                                    setDraftTracks((prev) => prev.map((x, i) => (i === idx ? { ...x, label: e.target.value } : x)))
                                  }
                                />

                                <div className="wb-row">
                                  <select
                                    className="wb-select"
                                    value={t.providerId}
                                    onChange={(e) => {
                                      const providerId = e.target.value;
                                      setDraftTracks((prev) =>
                                        prev.map((x, i) => (i === idx ? { ...x, providerId, modelId: "" } : x)),
                                      );
                                      ensureProviderOptions(providerId).catch(() => {});
                                    }}
                                  >
                                    {providers
                                      .filter((p) => p.provider_id !== "fake")
                                      .map((p) => (
                                        <option key={p.provider_id} value={p.provider_id}>
                                          {p.provider_id}
                                        </option>
                                      ))}
                                    <option value="fake">fake</option>
                                  </select>

                                  {codex && baseChoices.length > 0 ? (
                                    <>
                                      <select
                                        className="wb-select"
                                        value={base}
                                        onChange={(e) => setCodexModel(idx, e.target.value, effort)}
                                      >
                                        {baseChoices.map((b) => (
                                          <option key={b} value={b}>
                                            {b}
                                          </option>
                                        ))}
                                      </select>
                                      <select
                                        className="wb-select"
                                        value={effort}
                                        onChange={(e) => setCodexModel(idx, base, e.target.value)}
                                      >
                                        {CODEX_EFFORTS.map((e) => (
                                          <option key={e} value={e}>
                                            {e}
                                          </option>
                                        ))}
                                      </select>
                                    </>
                                  ) : modelIds.length > 0 ? (
                                    <select
                                      className="wb-select"
                                      value={t.modelId}
                                      onChange={(e) =>
                                        setDraftTracks((prev) => prev.map((x, i) => (i === idx ? { ...x, modelId: e.target.value } : x)))
                                      }
                                      onFocus={() => ensureProviderOptions(t.providerId).catch(() => {})}
                                    >
                                      <option value="">Select model…</option>
                                      {modelIds.map((m) => (
                                        <option key={m} value={m}>
                                          {m}
                                        </option>
                                      ))}
                                    </select>
                                  ) : (
                                    <input
                                      className="wb-input"
                                      value={t.modelId}
                                      placeholder="model_id"
                                      onFocus={() => ensureProviderOptions(t.providerId).catch(() => {})}
                                      onChange={(e) =>
                                        setDraftTracks((prev) => prev.map((x, i) => (i === idx ? { ...x, modelId: e.target.value } : x)))
                                      }
                                    />
                                  )}
                                </div>

                                {!installed && <div className="wb-warn">Not installed (install flow out of scope)</div>}

                                <div className="wb-row wb-row-right">
                                  <button
                                    type="button"
                                    className="wb-small"
                                    onClick={() => setDraftTracks((prev) => [...prev, { ...t, key: `t${Date.now()}` }])}
                                  >
                                    Duplicate
                                  </button>
                                  <button
                                    type="button"
                                    className="wb-small"
                                    onClick={() => setDraftTracks((prev) => prev.filter((_, i) => i !== idx))}
                                    disabled={draftTracks.length <= 1}
                                  >
                                    Remove
                                  </button>
                                </div>
                              </div>
                            );
                          })}
                          <div className="wb-row">
                            <button
                              type="button"
                              className="wb-small"
                              onClick={() => setDraftTracks((prev) => [...prev, { key: `t${Date.now()}`, label: "", providerId: "codex", modelId: "" }])}
                            >
                              + Add track
                            </button>
                          </div>
                        </div>
                      </div>
                    )}
                  </div>
                </div>

                <div className="wb-icon-row">
                  <button type="button" className="wb-icon" onClick={() => setContextMenuOpen((v) => !v)}>
                    @
                  </button>
                  <button type="button" className="wb-icon" title="Attach image (coming soon)" disabled>
                    ☐
                  </button>
                  <button type="button" className="wb-icon" title="Microphone (coming soon)" disabled>
                    ⏺
                  </button>
                </div>
              </div>

              {contextMenuOpen && (
                <div className="wb-context-popover" onMouseLeave={() => setContextMenuOpen(false)}>
                  <div className="wb-context-title">Add files, folders, docs…</div>
                  <div className="wb-context-item">Files &amp; Folders</div>
                  <div className="wb-context-item">Docs</div>
                  <div className="wb-context-item">Terminals</div>
                  <div className="wb-context-item">Past Chats</div>
                  <div className="wb-context-item">Branch (Diff with Main)</div>
                </div>
              )}

              <div className="wb-composer-actions">
                <button type="button" className="wb-primary" onClick={startNewTask}>
                  Start
                </button>
              </div>
              <div className="wb-under-row">
                <span className="wb-under-pill">Worktree</span>
                <span className="wb-under-pill">main</span>
              </div>
            </div>
          </div>
        )}

        {activeTaskId && (
          <div className="wb-body">
            <div className="wb-convo">
              <div className="wb-trackbar">
                {tracks.map((tr) => {
                  const trid = idToString(tr.id);
                  const selected = trid === activeTrackId;
                  const sessions = sessionsByTrack[trid] ?? [];
                  const s = sessions[0] as any;
                  const model = s ? `${s.provider_id} ${s.model_id}` : "No session";
                  const status = tr.status === "running" ? "Running…" : tr.status === "completed" ? "Task completed" : tr.status;
                  return (
                    <button
                      key={trid}
                      type="button"
                      className={`wb-trackcard ${selected ? "wb-trackcard-active" : ""}`}
                      onClick={() => setActiveTrackId(trid)}
                    >
                      <div className="wb-trackcard-title">{model}</div>
                      <div className="wb-trackcard-sub">{status}</div>
                    </button>
                  );
                })}
              </div>

              <div className="wb-session">
                {activeSessionId ? (
                  <SessionView sessionId={activeSessionId} variant="workbench" showDiffPane={false} />
                ) : (
                  <div className="wb-muted" style={{ padding: 16 }}>
                    Select a track with a session.
                  </div>
                )}
              </div>
            </div>

            {hasDiff && (
              <>
                <div className="wb-splitter" onMouseDown={onSplitterMouseDown} />
                <div className="wb-diff" style={{ width: diffWidth }}>
                  <div className="wb-diff-top">
                    <div className="wb-diff-tabs">
                      <div className="wb-diff-tab">All Changes</div>
                      <div className="wb-diff-pill">
                        {diffFileCount} Pending Change{diffFileCount === 1 ? "" : "s"}
                      </div>
                    </div>
                    <div className="wb-diff-actions">
                      <button type="button" className="wb-primary" onClick={approveAll}>
                        Approve
                      </button>
                      <button type="button" className="wb-small" onClick={rejectAll}>
                        Reject
                      </button>
                    </div>
                  </div>

                  <DiffReviewPane
                    diff={activeTrackDiff}
                    trackId={activeTrackIdFromSession}
                    onDiffUpdated={(d) => activeSessionId && supervisor.setDiff(activeSessionId, d)}
                    labels={{
                      title: "Pending Changes",
                      acceptAll: "Approve all",
                      rejectAll: "Reject all",
                      accept: "Approve",
                      reject: "Reject",
                    }}
                  />
                </div>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
