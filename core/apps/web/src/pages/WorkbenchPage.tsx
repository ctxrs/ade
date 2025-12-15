import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { shouldSendOnEnter } from "../utils/keyboard";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";

type DraftTrack = {
  key: string;
  label: string;
  providerId: string;
  modelId: string;
};

type DraftModeId = "default" | "research" | "plan" | "review";

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

function workbenchLabelForTrack(d: DraftTrack): string {
  if (d.label.trim().length > 0) return d.label.trim();
  return `${d.providerId} · ${d.modelId}`;
}

function IconChevronDown({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M6 9l6 6 6-6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function IconArrowUp({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M12 19V5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      <path d="M5 12l7-7 7 7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function IconAt({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M16.5 12a4.5 4.5 0 10-1.3 3.2"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <path
        d="M16.5 12V9.5a2.5 2.5 0 015 0V12a9.5 9.5 0 11-3.2-7.1"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}

function IconImage({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M4 5a2 2 0 012-2h12a2 2 0 012 2v14a2 2 0 01-2 2H6a2 2 0 01-2-2V5z"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
      <path
        d="M8 11a2 2 0 100-4 2 2 0 000 4z"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
      <path
        d="M21 16l-5-5-4 4-2-2-5 5"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}

function IconMic({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M12 14a3 3 0 003-3V7a3 3 0 10-6 0v4a3 3 0 003 3z"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M19 11a7 7 0 01-14 0"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <path
        d="M12 21v-3"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}

function IconLaptop({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M4 5h16v10H4V5z"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
      <path
        d="M2 19h20"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
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
  const [draftMode, setDraftMode] = useState<DraftModeId>("default");
  const [execTarget, setExecTarget] = useState<"local" | "worktree" | "container">("worktree");
  const [startBusy, setStartBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);

  const [openMenu, setOpenMenu] = useState<null | "mode" | "harness" | "model" | "exec">(null);
  const [expandedHarnessId, setExpandedHarnessId] = useState<string | null>(null);
  const [contextMenuOpen, setContextMenuOpen] = useState(false);
  const [useMultipleAgents, setUseMultipleAgents] = useState(false);
  const [harnessSearch, setHarnessSearch] = useState("");
  const [modelSearch, setModelSearch] = useState("");

  const newComposerRef = useRef<HTMLDivElement | null>(null);

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

  useEffect(() => {
    if (!openMenu && !contextMenuOpen) return;
    const onPointerDown = (e: PointerEvent) => {
      const root = newComposerRef.current;
      if (!root) return;
      const target = e.target as Node;
      if (!root.contains(target)) {
        setOpenMenu(null);
        setExpandedHarnessId(null);
        setContextMenuOpen(false);
        return;
      }
      // Click within the composer but outside any open popover should close popovers (Cursor-style).
      const el = e.target as Element | null;
      if (el && (el.closest(".wb-menu") || el.closest(".wb-context-popover") || el.closest(".wb-menu-trigger"))) {
        return;
      }
      setOpenMenu(null);
      setExpandedHarnessId(null);
      setContextMenuOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [openMenu, contextMenuOpen]);

  useEffect(() => {
    if (useMultipleAgents) return;
    if (draftTracks.length <= 1) return;
    setDraftTracks((prev) => (prev.length > 0 ? [prev[0]] : prev));
    setExpandedHarnessId(null);
  }, [useMultipleAgents, draftTracks.length]);

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
    const codexInstalled = providersById["codex"]?.installed ?? false;
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
    const installed = providersById[providerId]?.installed ?? false;
    if (!installed) return;
    if (providerOptions[providerId]) return providerOptions[providerId];
    const opts = await getProviderOptions(workspaceId, providerId);
    setProviderOptions((prev) => ({ ...prev, [providerId]: opts }));
    return opts;
  };

  const harnessInfoById = useMemo(() => {
    const map: Record<string, { label: string; logoSrc?: string; invertInDark?: boolean }> = {};
    for (const h of HARNESS_CATALOG) {
      map[h.id] = { label: h.label, logoSrc: h.logoSrc, invertInDark: h.invertInDark };
    }
    // Development/CI provider (not in Emdash list)
    if (!map["fake"]) {
      map["fake"] = { label: "Fake" };
    }
    return map;
  }, []);

  const harnessCounts = useMemo(() => {
    const out: Record<string, number> = {};
    for (const t of draftTracks) {
      out[t.providerId] = (out[t.providerId] ?? 0) + 1;
    }
    return out;
  }, [draftTracks]);

  const primaryTrack = draftTracks[0] ?? null;
  const primaryHarnessId = primaryTrack?.providerId ?? "codex";
  const primaryHarnessLabel = harnessInfoById[primaryHarnessId]?.label ?? primaryHarnessId;

  const startBlockedReason = useMemo(() => {
    if (draftPrompt.trim().length === 0) return "Enter a prompt to start.";
    if (startBusy) return "Starting…";
    const missing = draftTracks.find((t) => (providersById[t.providerId]?.installed ?? false) === false);
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
        const installed = providersById[dt.providerId]?.installed ?? false;
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
        // Ensure the workbench view can render the just-posted user message (and any streamed events)
        // without waiting for a `done` event to trigger a refresh.
        supervisor.refreshQueue(sessionId);
        supervisor.refreshSession(sessionId, { watchDiff: true });
      }

      await refreshTasks();
      setActiveTaskId(taskId);
      setDraftPrompt("");
      setOpenMenu(null);
      setExpandedHarnessId(null);
      setContextMenuOpen(false);
    } catch (e: any) {
      setStartError(e?.message ?? String(e));
    } finally {
      setStartBusy(false);
    }
  };

  const toggleHarness = (providerId: string) => {
    const installed = providersById[providerId]?.installed ?? false;
    // Don’t allow selecting unknown/uninstalled harnesses yet (UI-only list mirrors Emdash).
    if (!installed) return;
    setDraftTracks((prev) => {
      const has = prev.some((t) => t.providerId === providerId);
      if (!useMultipleAgents) {
        if (has) return prev;
        return [{ key: `t${Date.now()}`, label: "", providerId, modelId: "" }];
      }
      if (has) {
        const next = prev.filter((t) => t.providerId !== providerId);
        return next.length > 0
          ? next
          : [{ key: `t${Date.now()}`, label: "", providerId: defaultProviderId, modelId: "" }];
      }
      return [...prev, { key: `t${Date.now()}`, label: "", providerId, modelId: "" }];
    });
    ensureProviderOptions(providerId).catch(() => {});
    if (!useMultipleAgents) {
      setOpenMenu(null);
      setExpandedHarnessId(null);
    }
  };

  const updateTrackModel = (key: string, modelId: string) => {
    setDraftTracks((prev) => prev.map((t) => (t.key === key ? { ...t, modelId } : t)));
  };

  const addTrackForProvider = (providerId: string) => {
    setDraftTracks((prev) => [...prev, { key: `t${Date.now()}`, label: "", providerId, modelId: "" }]);
  };

  const removeTrackByKey = (key: string) => {
    setDraftTracks((prev) => {
      const next = prev.filter((t) => t.key !== key);
      return next.length > 0 ? next : prev;
    });
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

        {!activeTaskId ? (
          <div className="wb-center">
            <div className="wb-new-composer-stack" ref={newComposerRef}>
              <div className="wb-composer-card wb-new-composer-card">
                <textarea
                  className="wb-composer-textarea"
                  placeholder="Plan, @ for context, / for commands"
                  value={draftPrompt}
                  onChange={(e) => setDraftPrompt(e.target.value)}
                  onKeyDown={(e) => {
                    if (!shouldSendOnEnter(e)) return;
                    e.preventDefault();
                    startNewTask();
                  }}
                />

                <div className="wb-composer-bottom">
                  <div className="wb-switcher-row">
                    <div className="wb-switcher-wrap">
                      <button
                        type="button"
                        className="wb-switcher wb-menu-trigger"
                        onClick={() => {
                          setContextMenuOpen(false);
                          setExpandedHarnessId(null);
                          setOpenMenu((v) => (v === "mode" ? null : "mode"));
                        }}
                        aria-haspopup="menu"
                        aria-expanded={openMenu === "mode"}
                        title="Mode"
                      >
                        <span className="wb-switcher-icon">∞</span>
                        <span className="wb-switcher-label">
                          {draftMode === "default"
                            ? "Default"
                            : draftMode === "research"
                              ? "Research"
                              : draftMode === "plan"
                                ? "Plan"
                                : "Review"}
                        </span>
                        <IconChevronDown size={14} />
                      </button>
                      {openMenu === "mode" && (
                        <div className="wb-menu" role="menu">
                          {(["default", "research", "plan", "review"] as DraftModeId[]).map((m) => (
                            <button
                              key={m}
                              type="button"
                              className={`wb-menu-item ${draftMode === m ? "wb-menu-item-active" : ""}`}
                              onClick={() => {
                                setDraftMode(m);
                                setOpenMenu(null);
                              }}
                              role="menuitem"
                            >
                              {m === "default"
                                ? "Default"
                                : m === "research"
                                  ? "Research"
                                  : m === "plan"
                                    ? "Plan"
                                    : "Review"}
                            </button>
                          ))}
                        </div>
                      )}
                    </div>

                    <div className="wb-switcher-wrap">
                      <button
                        type="button"
                        className="wb-switcher wb-menu-trigger"
                        onClick={() => {
                          setContextMenuOpen(false);
                          setExpandedHarnessId(null);
                          setHarnessSearch("");
                          setOpenMenu((v) => (v === "harness" ? null : "harness"));
                        }}
                        aria-haspopup="menu"
                        aria-expanded={openMenu === "harness"}
                        title="Harness"
                      >
                        {harnessInfoById[primaryHarnessId]?.logoSrc ? (
                          <img
                            className={`wb-switcher-logo ${harnessInfoById[primaryHarnessId]?.invertInDark ? "wb-invert" : ""}`}
                            src={harnessInfoById[primaryHarnessId].logoSrc!}
                            alt=""
                          />
                        ) : (
                          <span className="wb-switcher-logo-fallback" />
                        )}
                        <span className="wb-switcher-label">
                          {draftTracks.length === 1 ? primaryHarnessLabel : `${draftTracks.length} tracks`}
                        </span>
                        <IconChevronDown size={14} />
                      </button>

                      {openMenu === "harness" && (
                        <div className="wb-menu wb-harness-menu" role="menu">
                          <div className="wb-menu-top">
                            <input
                              className="wb-menu-search"
                              value={harnessSearch}
                              onChange={(e) => setHarnessSearch(e.target.value)}
                              placeholder="Search agents"
                              aria-label="Search agents"
                              autoFocus
                            />
                            <label className="wb-menu-toggle">
                              <span>Use Multiple Agents</span>
                              <input
                                type="checkbox"
                                checked={useMultipleAgents}
                                onChange={(e) => {
                                  setExpandedHarnessId(null);
                                  setUseMultipleAgents(e.target.checked);
                                }}
                              />
                              <span className="wb-toggle" aria-hidden="true" />
                            </label>
                          </div>
                          {(() => {
                            const q = harnessSearch.trim().toLowerCase();
                            const all = HARNESS_CATALOG.concat(
                              providersById["fake"]
                                ? [{ id: "fake", label: "Fake", logoSrc: "", invertInDark: false } as any]
                                : [],
                            );
                            const filtered = q
                              ? all.filter((h: any) => {
                                  const id = String(h.id).toLowerCase();
                                  const label = String(h.label).toLowerCase();
                                  return id.includes(q) || label.includes(q);
                                })
                              : all;
                            if (filtered.length === 0) {
                              return <div className="wb-menu-empty">No matching agents.</div>;
                            }
                            return filtered.map((h: any) => {
                              const id = String(h.id);
                              const label = String(h.label);
                              const count = harnessCounts[id] ?? 0;
                              const checked = count > 0;
                              const installed = providersById[id]?.installed ?? false;
                              const expanded = expandedHarnessId === id;
                              const canConfigureModels = useMultipleAgents && draftTracks.length > 1;
                              const rows = draftTracks.filter((t) => t.providerId === id);
                              const opts = providerOptions[id];
                              const modelIds = modelIdsFromOptions(opts);

                              return (
                                <div key={id} className={`wb-harness-row ${installed ? "" : "wb-disabled"}`}>
                                  <button
                                    type="button"
                                    className="wb-harness-row-main"
                                    onClick={() => toggleHarness(id)}
                                    disabled={!installed}
                                  >
                                    <span className={`wb-check ${checked ? "wb-check-on" : ""}`} aria-hidden="true">
                                      {checked ? "✓" : ""}
                                    </span>
                                    {h.logoSrc ? (
                                      <img
                                        className={`wb-harness-logo ${h.invertInDark ? "wb-invert" : ""}`}
                                        src={h.logoSrc}
                                        alt=""
                                      />
                                    ) : (
                                      <span className="wb-harness-logo-fallback" aria-hidden="true" />
                                    )}
                                    <span className="wb-harness-name">{label}</span>
                                    <span className="wb-harness-right">
                                      <span className="wb-harness-count">{count > 0 ? `${count}x` : ""}</span>
                                    </span>
                                  </button>

                                  {checked && (
                                    <button
                                      type="button"
                                      className="wb-harness-expand wb-menu-trigger"
                                      onClick={() => {
                                        if (!canConfigureModels) return;
                                        setExpandedHarnessId((prev) => (prev === id ? null : id));
                                        ensureProviderOptions(id).catch(() => {});
                                      }}
                                      disabled={!canConfigureModels}
                                      title={canConfigureModels ? "Configure models" : "Enable multi-agent to configure"}
                                    >
                                      <IconChevronDown size={14} />
                                    </button>
                                  )}

                                  {expanded && canConfigureModels && (
                                    <div className="wb-harness-config">
                                      {rows.map((t) => (
                                        <div key={t.key} className="wb-harness-track">
                                          <div className="wb-harness-track-left">
                                            <div className="wb-harness-track-title">Track</div>
                                            {modelIds.length > 0 ? (
                                              <select
                                                className="wb-harness-model-select"
                                                value={t.modelId}
                                                onChange={(e) => updateTrackModel(t.key, e.target.value)}
                                                onFocus={() => ensureProviderOptions(id).catch(() => {})}
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
                                                className="wb-harness-model-input"
                                                value={t.modelId}
                                                placeholder="model_id"
                                                onFocus={() => ensureProviderOptions(id).catch(() => {})}
                                                onChange={(e) => updateTrackModel(t.key, e.target.value)}
                                              />
                                            )}
                                          </div>
                                          <div className="wb-harness-track-right">
                                            <button
                                              type="button"
                                              className="wb-harness-mini"
                                              onClick={() => addTrackForProvider(id)}
                                              title="Add another track"
                                            >
                                              +
                                            </button>
                                            <button
                                              type="button"
                                              className="wb-harness-mini"
                                              onClick={() => removeTrackByKey(t.key)}
                                              title="Remove track"
                                              disabled={rows.length <= 1}
                                            >
                                              −
                                            </button>
                                          </div>
                                        </div>
                                      ))}
                                    </div>
                                  )}

                                  {!installed && <div className="wb-harness-note">Not installed</div>}
                                </div>
                              );
                            });
                          })()}
                        </div>
                      )}
                    </div>

                    {draftTracks.length === 1 && (
                      <div className="wb-switcher-wrap">
                        <button
                          type="button"
                          className="wb-switcher wb-menu-trigger"
                          onClick={() => {
                            setContextMenuOpen(false);
                            setExpandedHarnessId(null);
                            setModelSearch("");
                            setOpenMenu((v) => (v === "model" ? null : "model"));
                            ensureProviderOptions(primaryHarnessId).catch(() => {});
                          }}
                          aria-haspopup="menu"
                          aria-expanded={openMenu === "model"}
                          title="Model"
                        >
                          <span className="wb-switcher-label wb-mono">
                            {(primaryTrack?.modelId ?? "").trim() || "Model"}
                          </span>
                          <IconChevronDown size={14} />
                        </button>
                        {openMenu === "model" && (
                          <div className="wb-menu wb-model-menu" role="menu">
                            <div className="wb-menu-top">
                              <input
                                className="wb-menu-search"
                                value={modelSearch}
                                onChange={(e) => setModelSearch(e.target.value)}
                                placeholder="Search models"
                                aria-label="Search models"
                                autoFocus
                              />
                            </div>
                            {(() => {
                              const opts = providerOptions[primaryHarnessId];
                              const modelIds = modelIdsFromOptions(opts);
                              if (modelIds.length === 0) {
                                return <div className="wb-menu-empty">No model list yet.</div>;
                              }
                              const q = modelSearch.trim().toLowerCase();
                              const filtered = q ? modelIds.filter((m) => m.toLowerCase().includes(q)) : modelIds;
                              if (filtered.length === 0) {
                                return <div className="wb-menu-empty">No matching models.</div>;
                              }
                              return filtered.map((m) => (
                                <button
                                  key={m}
                                  type="button"
                                  className={`wb-menu-item ${primaryTrack?.modelId === m ? "wb-menu-item-active" : ""}`}
                                  onClick={() => {
                                    if (!primaryTrack) return;
                                    updateTrackModel(primaryTrack.key, m);
                                    setOpenMenu(null);
                                  }}
                                  role="menuitem"
                                >
                                  {m}
                                </button>
                              ));
                            })()}
                          </div>
                        )}
                      </div>
                    )}
                  </div>

                  <div className="wb-action-row">
                    <button
                      type="button"
                      className="wb-icon wb-menu-trigger"
                      onClick={() => {
                        setOpenMenu(null);
                        setExpandedHarnessId(null);
                        setContextMenuOpen((v) => !v);
                      }}
                      aria-label="Add context"
                    >
                      <IconAt size={14} />
                    </button>
                    <button
                      type="button"
                      className="wb-icon"
                      title="Attach image (coming soon)"
                      disabled
                      aria-label="Attach image"
                    >
                      <IconImage size={14} />
                    </button>
                    <button
                      type="button"
                      className="wb-icon"
                      title="Record (coming soon)"
                      disabled
                      aria-label="Record"
                    >
                      <IconMic size={14} />
                    </button>
                    <button
                      type="button"
                      className="wb-send"
                      onClick={startNewTask}
                      disabled={!!startBlockedReason}
                      title={startBlockedReason ?? "Start"}
                      aria-label="Start"
                    >
                      <IconArrowUp size={14} />
                    </button>
                  </div>
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

              <div className="wb-exec-row" title="Execution target">
                <div className="wb-switcher-wrap">
                  <button
                    type="button"
                    className="wb-exec-switcher wb-menu-trigger"
                    onClick={() => {
                      setContextMenuOpen(false);
                      setExpandedHarnessId(null);
                      setOpenMenu((v) => (v === "exec" ? null : "exec"));
                    }}
                    aria-haspopup="menu"
                    aria-expanded={openMenu === "exec"}
                  >
                    <IconLaptop size={14} />
                    <span className="wb-exec-label">
                      {execTarget === "worktree" ? "Worktree" : execTarget === "local" ? "Local" : "Container"}
                    </span>
                    <IconChevronDown size={14} />
                  </button>
                  {openMenu === "exec" && (
                    <div className="wb-menu wb-exec-menu" role="menu">
                      <button
                        type="button"
                        className={`wb-menu-item ${execTarget === "worktree" ? "wb-menu-item-active" : ""}`}
                        onClick={() => {
                          setExecTarget("worktree");
                          setOpenMenu(null);
                        }}
                      >
                        Worktree
                      </button>
                      <button type="button" className="wb-menu-item" disabled>
                        Local (disabled)
                      </button>
                      <button type="button" className="wb-menu-item" disabled>
                        Container (soon)
                      </button>
                    </div>
                  )}
                </div>
              </div>
            </div>
          </div>
        ) : null}

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
