import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useLocation, useParams } from "react-router-dom";
import {
  EditPlanSummary,
  LspStatus,
  ProviderOptions,
  ProviderStatus,
  Task,
  Track,
  Workspace,
  applyTrackDiffPatch,
  discardEditPlan,
  createSession,
  createTask,
  createTrack,
  getLspStatus,
  getProviderOptions,
  getWorkspace,
  idToString,
  listProviders,
  listEditPlansForTrack,
  listSessionsForTrack,
  listTasks,
  listTracks,
  postMessage,
  trackDiff,
} from "../api/client";
import { useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { DiffReviewPane } from "../components/DiffReviewPane";
import { EditPlanReviewPane } from "../components/EditPlanReviewPane";
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
  const location = useLocation();
  const supervisor = useSessionSupervisor();

  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions>>({});
  const providersById = useMemo(
    () => Object.fromEntries(providers.map((p) => [p.provider_id, p])),
    [providers],
  );
  const defaultProviderId = useMemo(() => {
    const installed = providers.filter((p) => p.installed).map((p) => p.provider_id);
    if (installed.includes("codex")) return "codex";
    if (installed.includes("claude")) return "claude";
    if (installed.includes("fake")) return "fake";
    if (installed.includes("gemini")) return "gemini";
    return installed[0] ?? "codex";
  }, [providers]);

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
  const [startBusy, setStartBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);

  const [harnessPickerOpen, setHarnessPickerOpen] = useState(false);
  const [contextMenuOpen, setContextMenuOpen] = useState(false);

  const [diffWidth, setDiffWidth] = useState(480);
  const [reviewTab, setReviewTab] = useState<"git" | "lsp">("git");
  const [editPlans, setEditPlans] = useState<EditPlanSummary[]>([]);
  const [activeEditPlanId, setActiveEditPlanId] = useState<string | null>(null);
  const [lspStatus, setLspStatus] = useState<LspStatus | null>(null);

  useEffect(() => {
    document.documentElement.classList.add("wb-no-scroll");
    document.body.classList.add("wb-no-scroll");
    return () => {
      document.body.classList.remove("wb-no-scroll");
      document.documentElement.classList.remove("wb-no-scroll");
    };
  }, []);

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
    getLspStatus().then(setLspStatus).catch(() => setLspStatus(null));
  }, [workspaceId]);

  useEffect(() => {
    if (!providers.length) return;
    const codexInstalled = providersById["codex"]?.installed ?? true;
    if (codexInstalled) return;
    if (defaultProviderId === "codex") return;
    setDraftTracks((prev) => {
      const isDefault = prev.every((t) => t.providerId === "codex" && !t.label.trim() && !t.modelId.trim());
      if (!isDefault) return prev;
      return prev.map((t) => ({ ...t, providerId: defaultProviderId }));
    });
  }, [providers.length, providersById, defaultProviderId]);

  useEffect(() => {
    if (!activeTaskId) {
      setTracks([]);
      setSessionsByTrack({});
      setActiveTrackId(null);
      setEditPlans([]);
      setActiveEditPlanId(null);
      return;
    }
    refreshTaskDetail(activeTaskId).catch(() => {});
  }, [activeTaskId]);

  useEffect(() => {
    if (!activeTrackId) {
      setEditPlans([]);
      setActiveEditPlanId(null);
      return;
    }
    listEditPlansForTrack(activeTrackId)
      .then((plans) => {
        setEditPlans(plans);
        const first = plans[0] ? idToString(plans[0].id) : null;
        setActiveEditPlanId((prev) => (prev && plans.some((p) => idToString(p.id) === prev) ? prev : first));
      })
      .catch(() => {
        setEditPlans([]);
        setActiveEditPlanId(null);
      });
  }, [activeTrackId]);

  const sortedTasks = useMemo(() => {
    return [...tasks].sort((a, b) => {
      const ta = Date.parse(a.updated_at);
      const tb = Date.parse(b.updated_at);
      if (Number.isFinite(ta) && Number.isFinite(tb)) return tb - ta;
      return String(b.updated_at).localeCompare(String(a.updated_at));
    });
  }, [tasks]);

  const filteredTasks = useMemo(() => {
    const q = taskQuery.trim().toLowerCase();
    if (!q) return sortedTasks;
    return sortedTasks.filter((t) => (t.title ?? "").toLowerCase().includes(q));
  }, [sortedTasks, taskQuery]);

  const activeSessionId = useMemo(() => {
    if (!activeTrackId) return null;
    const ss = sessionsByTrack[activeTrackId] ?? [];
    const s = ss[0];
    return s ? idToString((s as any).id) : null;
  }, [activeTrackId, sessionsByTrack]);

  const showDebugIds = useMemo(() => {
    const params = new URLSearchParams(location.search);
    const ids = params.get("ids");
    const debug = params.get("debug");
    if (ids === "1" || debug === "1") {
      localStorage.setItem("contextDebugIds", "1");
      return true;
    }
    if (ids === "0" || debug === "0") {
      localStorage.removeItem("contextDebugIds");
      return false;
    }
    return localStorage.getItem("contextDebugIds") === "1";
  }, [location.search]);

  const debugIdLabel = useMemo(() => {
    const short = (v: string | null) => {
      const s = String(v ?? "");
      return s ? s.slice(0, 8) : "-";
    };
    return `task:${short(activeTaskId)} track:${short(activeTrackId)} session:${short(activeSessionId)}`;
  }, [activeTaskId, activeTrackId, activeSessionId]);

  const activeEntry = useSessionEntry(activeSessionId ?? "");
  const activeTrackDiff = activeEntry?.diff ?? "";
  const activeTrackIdFromSession = activeEntry?.session ? idToString(activeEntry.session.track_id) : "";

  const hasDiff = activeTrackDiff.trim().length > 0;
  const hasEditPlans = editPlans.some((p) => (p.diff ?? "").trim().length > 0);
  const showReviewPane = hasDiff || hasEditPlans;
  const diffFileCount = useMemo(() => {
    if (!hasDiff) return 0;
    const m = activeTrackDiff.match(/^diff --git /gm);
    return m ? m.length : 1;
  }, [activeTrackDiff, hasDiff]);

  useEffect(() => {
    setReviewTab((prev) => {
      if (prev === "git" && !hasDiff && hasEditPlans) return "lsp";
      if (prev === "lsp" && !hasEditPlans && hasDiff) return "git";
      return prev;
    });
  }, [hasDiff, hasEditPlans]);

  const ensureProviderOptions = async (providerId: string): Promise<ProviderOptions | undefined> => {
    if (!workspaceId) return;
    const installed = providers.find((p) => p.provider_id === providerId)?.installed ?? true;
    if (!installed) return;
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

  const startBlockedReason = useMemo(() => {
    if (draftPrompt.trim().length === 0) return "Enter a prompt to start.";
    if (startBusy) return "Starting…";
    const missing = draftTracks.find((t) => (providersById[t.providerId]?.installed ?? true) === false);
    if (missing) {
      const diag = providersById[missing.providerId]?.diagnostics?.[0];
      return diag ? `Harness “${missing.providerId}” not installed: ${diag}` : `Harness “${missing.providerId}” not installed.`;
    }
    return null;
  }, [draftPrompt, startBusy, draftTracks, providersById]);

  const startNewTask = async () => {
    if (!workspaceId) return;
    const prompt = draftPrompt.trim();
    if (!prompt) return;
    if (startBusy) return;
    if (startBlockedReason && !startBlockedReason.startsWith("Starting")) {
      setStartError(startBlockedReason);
      return;
    }
    setStartBusy(true);
    setStartError(null);

    try {
      const title = deriveTaskTitle(prompt);
      const task = await createTask(workspaceId, title, undefined, { create_default_track: false });
      const taskId = idToString(task.id);

      const toStart =
        draftTracks.length > 0
          ? draftTracks
          : [{ key: "t1", label: "", providerId: "codex", modelId: "" }];

      for (let i = 0; i < toStart.length; i++) {
        const dt = toStart[i];
        const installed = providersById[dt.providerId]?.installed ?? true;
        if (!installed) {
          const diag = providersById[dt.providerId]?.diagnostics?.[0];
          throw new Error(diag ? `Harness “${dt.providerId}” not installed: ${diag}` : `Harness “${dt.providerId}” not installed.`);
        }
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
    } catch (e: any) {
      setStartError(e?.message ?? String(e));
    } finally {
      setStartBusy(false);
    }
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

  const refreshActiveDiff = useCallback(async () => {
    if (!activeTrackIdFromSession || !activeSessionId) return;
    const d = await trackDiff(activeTrackIdFromSession);
    supervisor.setDiff(activeSessionId, d.diff ?? "");
  }, [activeTrackIdFromSession, activeSessionId, supervisor]);

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
  const activeEditPlan = useMemo(() => {
    if (!activeEditPlanId) return null;
    return editPlans.find((p) => idToString(p.id) === activeEditPlanId) ?? null;
  }, [activeEditPlanId, editPlans]);

  const lspMissing = useMemo(() => {
    const servers = lspStatus?.servers ?? [];
    return servers.filter((s) => !s.found);
  }, [lspStatus]);

  useEffect(() => {
    if (!activeEditPlanId) {
      setActiveEditPlanId(editPlans[0] ? idToString(editPlans[0].id) : null);
      return;
    }
    if (!editPlans.some((p) => idToString(p.id) === activeEditPlanId)) {
      setActiveEditPlanId(editPlans[0] ? idToString(editPlans[0].id) : null);
    }
  }, [activeEditPlanId, editPlans]);

  const onEditPlanUpdated = (updated: EditPlanSummary) => {
    const pid = idToString(updated.id);
    setEditPlans((prev) => {
      const has = prev.some((p) => idToString(p.id) === pid);
      const next = (has ? prev.map((p) => (idToString(p.id) === pid ? updated : p)) : [updated, ...prev]).filter(
        (p) => (p.diff ?? "").trim().length > 0,
      );
      return next;
    });
  };

  const discardActiveEditPlan = async () => {
    if (!activeEditPlan) return;
    const pid = idToString(activeEditPlan.id);
    await discardEditPlan(pid);
    setEditPlans((prev) => prev.filter((p) => idToString(p.id) !== pid));
  };

  return (
    <div className={`wb-root ${sidebarCollapsed ? "wb-root-collapsed" : ""}`}>
      <div className={`wb-sidebar ${sidebarCollapsed ? "wb-sidebar-collapsed" : ""}`}>
        <div className="wb-sidebar-top">
          <div className="wb-sidebar-header">
            {!sidebarCollapsed && <div className="wb-sidebar-title">Agents</div>}
            <button
              type="button"
              className="wb-sidebar-collapse"
              aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              title={sidebarCollapsed ? "Expand" : "Collapse"}
              onClick={() => setSidebarCollapsed((v) => !v)}
            >
              {sidebarCollapsed ? "›" : "‹"}
            </button>
          </div>

          {!sidebarCollapsed && (
            <>
              <input
                className="wb-search"
                placeholder="Search Agents…"
                value={taskQuery}
                onChange={(e) => setTaskQuery(e.target.value)}
              />
              <button type="button" className="wb-new-agent" onClick={() => setActiveTaskId(null)}>
                New Agent
              </button>
            </>
          )}
        </div>

        {!sidebarCollapsed && (
          <div className="wb-sidebar-section">
            <div className="wb-section-title">Pinned</div>
            <div className="wb-muted">No pinned agents yet.</div>
          </div>
        )}

        <div className="wb-sidebar-section wb-sidebar-grow">
          {!sidebarCollapsed && <div className="wb-section-title">Agents</div>}
          <div className="wb-task-list">
            {filteredTasks.map((t) => {
              const tid = idToString(t.id);
              const selected = tid === activeTaskId;
              const title = t.title ?? "New conversation";
              const icon = title.trim().slice(0, 1).toUpperCase() || "•";
              return (
                <button
                  key={tid}
                  type="button"
                  className={`wb-task ${selected ? "wb-task-active" : ""} ${
                    sidebarCollapsed ? "wb-task-collapsed" : ""
                  }`}
                  onClick={() => setActiveTaskId(tid)}
                  title={title}
                >
                  {sidebarCollapsed ? (
                    <div className="wb-task-icon">{icon}</div>
                  ) : (
                    <>
                      <div className="wb-task-title">{title}</div>
                      <div className="wb-task-sub">
                        {new Date(t.updated_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                      </div>
                    </>
                  )}
                </button>
              );
            })}
            {filteredTasks.length === 0 && <div className="wb-muted">No agents yet.</div>}
          </div>
        </div>

        <div className="wb-sidebar-bottom">
          <Link className="wb-link" to="/providers" title="Providers">
            {sidebarCollapsed ? "P" : "Providers"}
          </Link>
          <Link className="wb-link" to="/diagnostics" title="Diagnostics">
            {sidebarCollapsed ? "D" : "Diagnostics"}
          </Link>
        </div>
      </div>

      <div className="wb-main">
        <div className="wb-topbar">
          <div className="wb-topbar-title">{workspace?.name ?? "Workspace"}</div>
          {activeTask && <div className="wb-topbar-sub">{activeTask.title}</div>}
          {showDebugIds && (
            <button
              type="button"
              className="wb-topbar-ids"
              title="Click to copy workspace/task/track/session IDs"
              onClick={() =>
                navigator.clipboard.writeText(
                  JSON.stringify(
                    {
                      workspaceId,
                      taskId: activeTaskId,
                      trackId: activeTrackId,
                      sessionId: activeSessionId,
                    },
                    null,
                    2,
                  ),
                )
              }
            >
              {debugIdLabel}
            </button>
          )}
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

              {startError && <div className="wb-banner">{startError}</div>}

              <div className="wb-composer-actions">
                <button
                  type="button"
                  className="wb-primary"
                  onClick={startNewTask}
                  disabled={!!startBlockedReason}
                >
                  {startBusy ? "Starting…" : "Start"}
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

            {showReviewPane && (
              <>
                <div className="wb-splitter" onMouseDown={onSplitterMouseDown} />
                <div className="wb-diff" style={{ width: diffWidth }}>
                  <div className="wb-diff-top">
                    <div className="wb-diff-tabs">
                      {hasDiff && (
                        <button
                          type="button"
                          className={`wb-diff-tab wb-diff-tab-button ${reviewTab === "git" ? "wb-diff-tab-active" : ""}`}
                          onClick={() => setReviewTab("git")}
                        >
                          All Changes
                        </button>
                      )}
                      {hasEditPlans && (
                        <button
                          type="button"
                          className={`wb-diff-tab wb-diff-tab-button ${reviewTab === "lsp" ? "wb-diff-tab-active" : ""}`}
                          onClick={() => setReviewTab("lsp")}
                        >
                          LSP Plans
                        </button>
                      )}
                      {reviewTab === "git" && hasDiff && (
                        <div className="wb-diff-pill">
                          {diffFileCount} Pending Change{diffFileCount === 1 ? "" : "s"}
                        </div>
                      )}
                      {reviewTab === "lsp" && hasEditPlans && (
                        <div className="wb-diff-pill">
                          {editPlans.length} Plan{editPlans.length === 1 ? "" : "s"}
                        </div>
                      )}
                    </div>
                    <div className="wb-diff-actions">
                      {reviewTab === "git" && hasDiff && (
                        <>
                          <button type="button" className="wb-primary" onClick={approveAll}>
                            Approve
                          </button>
                          <button type="button" className="wb-small" onClick={rejectAll}>
                            Reject
                          </button>
                        </>
                      )}
                      {reviewTab === "lsp" && hasEditPlans && activeEditPlan && (
                        <button type="button" className="wb-small" onClick={discardActiveEditPlan}>
                          Discard Plan
                        </button>
                      )}
                    </div>
                  </div>

                  {reviewTab === "git" && hasDiff && (
                    <DiffReviewPane
                      diff={activeTrackDiff}
                      trackId={activeTrackIdFromSession}
                      sessionId={activeSessionId || undefined}
                      onDiffUpdated={(d) => activeSessionId && supervisor.setDiff(activeSessionId, d)}
                      onFileSaved={refreshActiveDiff}
                      labels={{
                        title: "Pending Changes",
                        acceptAll: "Approve all",
                        rejectAll: "Reject all",
                        accept: "Approve",
                        reject: "Reject",
                      }}
                    />
                  )}

                  {reviewTab === "lsp" && hasEditPlans && (
                    <div className="wb-editplans">
                      {lspStatus && (!lspStatus.enabled || !lspStatus.edit_plans_enabled || lspMissing.length > 0) && (
                        <div className="banner" style={{ margin: "12px 12px 0" }}>
                          {!lspStatus.enabled && (
                            <div>
                              LSP is disabled (set <code>CONTEXT_LSP_ENABLED=1</code>).
                            </div>
                          )}
                          {lspStatus.enabled && !lspStatus.edit_plans_enabled && (
                            <div>
                              LSP edit plans are disabled (set{" "}
                              <code>CONTEXT_LSP_EDITPLANS_ENABLED=1</code>).
                            </div>
                          )}
                          {lspMissing.length > 0 && (
                            <div>
                              Missing language servers:{" "}
                              <span className="muted">{lspMissing.map((s) => s.language).join(", ")}</span>. See{" "}
                              <Link to="/diagnostics">Diagnostics</Link> for install hints.
                            </div>
                          )}
                        </div>
                      )}
                      <div className="wb-editplans-list">
                        {editPlans.map((p) => {
                          const pid = idToString(p.id);
                          const selected = pid === activeEditPlanId;
                          return (
                            <button
                              key={pid}
                              type="button"
                              className={`wb-editplan-item ${selected ? "wb-editplan-item-active" : ""}`}
                              onClick={() => setActiveEditPlanId(pid)}
                            >
                              <div className="wb-editplan-title">{p.title}</div>
                              <div className="wb-editplan-sub">{p.remaining_hunks} hunk(s)</div>
                            </button>
                          );
                        })}
                      </div>
                      {activeEditPlan ? (
                        <EditPlanReviewPane
                          plan={activeEditPlan}
                          sessionId={activeSessionId || undefined}
                          onPlanUpdated={onEditPlanUpdated}
                          onFileSaved={refreshActiveDiff}
                          labels={{
                            title: activeEditPlan.title || "Pending LSP Changes",
                            acceptAll: "Approve all",
                            rejectAll: "Reject all",
                            accept: "Approve",
                            reject: "Reject",
                          }}
                        />
                      ) : (
                        <div className="wb-muted" style={{ padding: 12 }}>
                          No pending LSP changes.
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
