import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";
import {
  Archive,
  ArrowUp,
  AtSign,
  ChevronDown,
  Check,
  Copy,
  Ellipsis,
  GitBranch,
  Image,
  Laptop,
  LayersPlus,
  MessageSquare,
  Mic,
  Settings,
} from "lucide-react";
import {
  DictationSettings,
  EditPlanSummary,
  InstallInfo,
  LspStatus,
  MessageAttachment,
  ProviderOptions,
  ProviderStatus,
  Task,
  Track,
  Worktree,
  Workspace,
  archiveTask,
  applyTrackDiffPatch,
  discardEditPlan,
  createSession,
  createTask,
  createTrack,
  deleteTask,
  getDaemonBaseUrl,
  getInstall,
  getLspStatus,
  getProviderOptions,
  getSettings,
  getWorktree,
  getWorkspace,
  idToString,
  installAllProviders,
  installProvider,
  listProviders,
  listEditPlansForTrack,
  listSessionsForTrack,
  listTasks,
  listTracks,
  markTaskRead as markTaskReadApi,
  markTaskUnread as markTaskUnreadApi,
  postMessage,
  trackDiff,
  unarchiveTask,
  updateTaskTitle,
} from "../api/client";
import { useSessionCacheSnapshot, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { loadWorkbenchSelectionV1, saveWorkbenchSelectionV1, type PersistedWorkbenchSelectionV1 } from "../state/uiStateStore";
import { DiffReviewPane } from "../components/DiffReviewPane";
import { EditPlanReviewPane } from "../components/EditPlanReviewPane";
import { SessionView, buildWorkbenchThreadViewModel } from "./SessionPage";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import { WorkbenchComposer, type DraftTrack, type WorkbenchEnvTarget, type WorkbenchModeId } from "../components/WorkbenchComposer";
import type { SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import { startMicPcmStream } from "../utils/micPcmStream";
import { desktopSaveTextFile, isDesktopApp } from "../utils/desktop";
import { parseWsJson } from "../utils/wsJson";
import { registerDropScope } from "../utils/dragDropScopes";
import { pickPreferredSession, pickPreferredSessionId, pickPreferredTrackId } from "../utils/workbenchSelection";
import { imageFilesToInlineAttachments } from "../utils/messageAttachments";
import { parseModelId } from "../utils/modelEffort";
import { formatRelativeAgeShort } from "../utils/relativeTime";
import {
  composerDraftKeyNewTaskV1,
  loadComposerDraftV1,
  removeComposerDraft,
  saveComposerDraftV1,
} from "../utils/composerDraftPersistence";

function deriveTaskTitle(prompt: string): string {
  const line = prompt.trim().split("\n")[0] ?? "";
  const short = line.trim().slice(0, 60);
  return short.length > 0 ? short : "New conversation";
}

function modelIdsFromOptions(opts?: ProviderOptions): string[] {
  const raw = opts?.models;
  if (!raw) return [];
  const list = (raw as any)?.availableModels ?? (raw as any)?.available_models ?? (raw as any)?.models ?? raw;
  if (!Array.isArray(list)) return [];
  return list
    .map((m: any) => String(m?.modelId ?? m?.model_id ?? m?.id ?? "").trim())
    .filter((s: string) => s.length > 0);
}

function workbenchLabelForTrack(dt: DraftTrack): string {
  const name = dt.providerId;
  return dt.label?.trim() ? `${name} — ${dt.label.trim()}` : name;
}

function appendSegment(base: string, addition: string): string {
  const trimmed = addition.trim();
  if (!trimmed) return base;
  if (!base) return trimmed;
  const needsSpace = /\S$/.test(base) && !/^[,.;!?]/.test(trimmed);
  return `${base}${needsSpace ? " " : ""}${trimmed}`;
}

function parseMs(value: string | null | undefined): number | null {
  if (!value) return null;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms : null;
}

function lastPathSegment(path: string | null | undefined): string {
  const raw = String(path ?? "").trim();
  if (!raw) return "";
  const parts = raw.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? "";
}

function taskActivityMs(t: Task): number | null {
  return parseMs(t.last_activity_at ?? null) ?? parseMs(t.updated_at ?? null) ?? parseMs(t.created_at ?? null);
}

function lastAssistantMessageMs(messages: { role: string; created_at: string }[]): number | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role === "assistant") return parseMs(m.created_at);
  }
  return null;
}

function lastRoleMessageMs(messages: { role: string; created_at: string }[], role: string): number | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role === role) return parseMs(m.created_at);
  }
  return null;
}

function sanitizeFileName(name: string): string {
  const raw = String(name ?? "").trim() || "conversation";
  const noBadChars = raw.replace(/[<>:"/\\|?*\u0000-\u001F]/g, "");
  const collapsed = noBadChars.replace(/\s+/g, "-").replace(/-+/g, "-").replace(/^-+|-+$/g, "");
  return (collapsed || "conversation").slice(0, 80);
}

async function saveMarkdownExport(suggestedName: string, contents: string): Promise<void> {
  const name = suggestedName.toLowerCase().endsWith(".md") ? suggestedName : `${suggestedName}.md`;
  if (isDesktopApp()) {
    await desktopSaveTextFile({ suggested_name: name, contents });
    return;
  }

  const picker = (window as any).showSaveFilePicker as undefined | ((opts: any) => Promise<any>);
  if (typeof picker === "function") {
    const handle = await picker({
      suggestedName: name,
      types: [{ description: "Markdown", accept: { "text/markdown": [".md"] } }],
    });
    const writable = await handle.createWritable();
    await writable.write(contents);
    await writable.close();
    return;
  }

  const blob = new Blob([contents], { type: "text/markdown" });
  const url = URL.createObjectURL(blob);
  try {
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    a.rel = "noopener";
    a.click();
  } finally {
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
}

export default function WorkbenchPage() {
  const { id: workspaceId } = useParams<{ id: string }>();
  const location = useLocation();
  const navigate = useNavigate();
  const supervisor = useSessionSupervisor();
  const sessionSnap = useSessionCacheSnapshot();
  const newComposerRef = useRef<HTMLDivElement | null>(null);

  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(260);
  const [sidebarResizing, setSidebarResizing] = useState(false);
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerInstallsById, setProviderInstallsById] = useState<
    Record<
      string,
      | {
        installId: string;
        state: InstallInfo["state"];
        pct: number | null;
      }
      | undefined
    >
  >({});
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions | undefined>>({});
  const providersById = useMemo(
    () => Object.fromEntries(providers.map((p) => [p.provider_id, p])),
    [providers],
  );
  const defaultProviderId = useMemo(() => {
    const installed = providers
      .filter((p) => p.installed && p.details?.ui_hidden !== "true")
      .map((p) => p.provider_id);
    if (installed.includes("codex")) return "codex";
    if (installed.includes("claude")) return "claude";
    if (installed.includes("gemini")) return "gemini";
    if (installed.includes("qwen")) return "qwen";
    if (installed.includes("opencode")) return "opencode";
    if (installed.includes("mistral")) return "mistral";
    if (installed.includes("goose")) return "goose";
    if (installed.includes("kimi")) return "kimi";
    if (installed.includes("auggie")) return "auggie";
    return installed[0] ?? "codex";
  }, [providers]);

  const [tasks, setTasks] = useState<Task[]>([]);
  const [taskQuery, setTaskQuery] = useState("");
  const [activeTaskId, setActiveTaskId] = useState<string | null>(() => {
    const params = new URLSearchParams(location.search);
    const taskId = params.get("task");
    return taskId ? String(taskId) : null;
  });
  const [archivedCollapsed, setArchivedCollapsed] = useState(true);
  const [taskMenu, setTaskMenu] = useState<{ taskId: string; style: React.CSSProperties } | null>(null);
  const taskMenuRef = useRef<HTMLDivElement | null>(null);
  const [renamingTaskId, setRenamingTaskId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const renameInputRef = useRef<HTMLInputElement | null>(null);
  const [taskProviderIdsByTaskId, setTaskProviderIdsByTaskId] = useState<Record<string, string[]>>({});
  const [convoMenu, setConvoMenu] = useState<{ style: React.CSSProperties } | null>(null);
  const convoMenuRef = useRef<HTMLDivElement | null>(null);

  const [tracks, setTracks] = useState<Track[]>([]);
  const [sessionsByTrack, setSessionsByTrack] = useState<Record<string, any[]>>({});
  const [activeTrackId, setActiveTrackId] = useState<string | null>(() => {
    const params = new URLSearchParams(location.search);
    const trackId = params.get("track");
    return trackId ? String(trackId) : null;
  });
  const [activeWorktree, setActiveWorktree] = useState<Worktree | null>(null);
  const taskDetailAbortRef = useRef<AbortController | null>(null);
  const taskDetailLoadSeqRef = useRef(0);
  const editPlansAbortRef = useRef<AbortController | null>(null);
  const editPlansLoadSeqRef = useRef(0);

  const selectionFromUrl = useMemo(() => {
    const params = new URLSearchParams(location.search);
    const taskId = params.get("task");
    const trackId = params.get("track");
    const sessionId = params.get("session");
    return {
      taskId: taskId ? String(taskId) : null,
      trackId: trackId ? String(trackId) : null,
      sessionId: sessionId ? String(sessionId) : null,
    };
  }, [location.search]);

  const [persistedSelection, setPersistedSelection] = useState<PersistedWorkbenchSelectionV1 | null>(null);
  const [persistedSelectionLoaded, setPersistedSelectionLoaded] = useState(false);
  const [uiStateWarning, setUiStateWarning] = useState<string | null>(null);
  const persistedSelectionLoadSeqRef = useRef(0);
  const didAttemptSelectionRestoreRef = useRef(false);

  useEffect(() => {
    didAttemptSelectionRestoreRef.current = false;
  }, [workspaceId]);

  useEffect(() => {
    if (!workspaceId) {
      setPersistedSelection(null);
      setPersistedSelectionLoaded(false);
      setUiStateWarning(null);
      return;
    }
    setPersistedSelectionLoaded(false);
    const seq = ++persistedSelectionLoadSeqRef.current;
    loadWorkbenchSelectionV1(workspaceId)
      .then((sel) => {
        if (seq !== persistedSelectionLoadSeqRef.current) return;
        setPersistedSelection(sel);
        setPersistedSelectionLoaded(true);
        setUiStateWarning(null);
      })
      .catch((e: unknown) => {
        if (seq !== persistedSelectionLoadSeqRef.current) return;
        const msg = e instanceof Error ? e.message : String(e);
        setPersistedSelection(null);
        setPersistedSelectionLoaded(true);
        setUiStateWarning(`UI state persistence disabled: ${msg}`);
      });
  }, [workspaceId]);

  const updateWorkbenchUrlSelection = useCallback(
    (sel: { taskId: string | null; trackId: string | null; sessionId: string | null }, replace: boolean) => {
      const params = new URLSearchParams(location.search);
      const update = (key: string, value: string | null) => {
        if (!value) params.delete(key);
        else params.set(key, value);
      };
      update("task", sel.taskId);
      update("track", sel.trackId);
      update("session", sel.sessionId);

      const search = params.toString();
      const nextSearch = search ? `?${search}` : "";
      if (nextSearch === location.search) return;
      navigate({ pathname: location.pathname, search: nextSearch }, { replace });
    },
    [location.pathname, location.search, navigate],
  );

  const normalizeWorkbenchSelection = useCallback(
    (sel: { taskId: string | null; trackId: string | null; sessionId: string | null }) => {
      if (!sel.taskId) return { taskId: null, trackId: null, sessionId: null };
      if (!sel.trackId) return { taskId: sel.taskId, trackId: null, sessionId: null };
      if (!sel.sessionId) return { taskId: sel.taskId, trackId: sel.trackId, sessionId: null };
      return sel;
    },
    [],
  );

  const applyWorkbenchSelection = useCallback(
    (sel: { taskId: string | null; trackId: string | null; sessionId: string | null }, replace: boolean) => {
      const next = normalizeWorkbenchSelection(sel);
      didAttemptSelectionRestoreRef.current = true;
      setActiveTaskId(next.taskId);
      setActiveTrackId(next.trackId);
      updateWorkbenchUrlSelection(next, replace);
    },
    [normalizeWorkbenchSelection, updateWorkbenchUrlSelection],
  );

  // Keep tracks cached per task so we can show best-effort provider badges.
  const [tracksByTaskId, setTracksByTaskId] = useState<Record<string, Track[]>>({});

  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
    { key: "t1", label: "", providerId: "codex", modelId: "" },
  ]);
  const [draftMode, setDraftMode] = useState<WorkbenchModeId>("default");
  const [execTarget, setExecTarget] = useState<WorkbenchEnvTarget>("worktree");
  const [startBusy, setStartBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const [installAllBusy, setInstallAllBusy] = useState(false);
  const [useMultipleAgents, setUseMultipleAgents] = useState(false);
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const [dropActive, setDropActive] = useState(false);
  const dropHideTimerRef = useRef<number | null>(null);

  const [dictationSettings, setDictationSettings] = useState<DictationSettings | null>(null);
  const [dictationRecording, setDictationRecording] = useState(false);
  const [dictationError, setDictationError] = useState<string | null>(null);
  const dictationWsRef = useRef<WebSocket | null>(null);
  const dictationMicRef = useRef<{ stop: () => Promise<void> } | null>(null);
  const dictationBaseRef = useRef<string>("");
  const dictationCommittedRef = useRef<string>("");
  const dictationInterimRef = useRef<string>("");
  const dictationDebugEnabled = useMemo(() => {
    try {
      return new URLSearchParams(window.location.search).get("dictation_debug") === "1";
    } catch {
      return false;
    }
  }, []);
  const [dictationDebugText, setDictationDebugText] = useState<string | null>(null);
  const dictationAudioBytesRef = useRef(0);
  const dictationAudioChunksRef = useRef(0);
  const dictationReadyRef = useRef(false);
  const dictationAudioStartedRef = useRef(false);
  const dictationTranscriptMsgsRef = useRef(0);

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
    if (!workspaceId) return;
    const key = composerDraftKeyNewTaskV1(workspaceId);
    const draft = loadComposerDraftV1(key);
    if (!draft) return;
    setDraftPrompt(draft.text);
    setDraftMode(draft.modeId ?? "default");
  }, [workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    const key = composerDraftKeyNewTaskV1(workspaceId);
    const timer = window.setTimeout(() => {
      const text = draftPrompt;
      const modeId = draftMode;
      if (text.trim().length === 0 && modeId === "default") {
        removeComposerDraft(key);
        return;
      }
      saveComposerDraftV1(key, { v: 1, text, modeId });
    }, 200);
    return () => window.clearTimeout(timer);
  }, [draftMode, draftPrompt, workspaceId]);

  useEffect(() => {
    const onResize = () => {
      const max = Math.max(170, window.innerWidth - 240);
      setSidebarWidth((w) => Math.min(max, Math.max(170, Math.round(w))));
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    if (!workspaceId) return;
    const key = `wb.archivedCollapsed.${workspaceId}`;
    const v = localStorage.getItem(key);
    if (v === "0") setArchivedCollapsed(false);
    else setArchivedCollapsed(true);
  }, [workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    localStorage.setItem(`wb.archivedCollapsed.${workspaceId}`, archivedCollapsed ? "1" : "0");
  }, [archivedCollapsed, workspaceId]);

  useLayoutEffect(() => {
    if (!workspaceId) return;
    const key = `wb.sidebarWidth.${workspaceId}`;
    try {
      const raw = localStorage.getItem(key);
      const parsed = raw ? Number(raw) : NaN;
      if (!Number.isFinite(parsed)) return;
      const max = Math.max(170, window.innerWidth - 240);
      setSidebarWidth(Math.min(max, Math.max(170, Math.round(parsed))));
    } catch {
      // ignore
    }
  }, [workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    try {
      const max = Math.max(170, window.innerWidth - 240);
      const clamped = Math.min(max, Math.max(170, Math.round(sidebarWidth)));
      localStorage.setItem(`wb.sidebarWidth.${workspaceId}`, String(clamped));
    } catch {
      // ignore
    }
  }, [sidebarWidth, workspaceId]);

  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      if (!taskMenu) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.closest(".wb-task-menu") || el.closest(".wb-task-menu-trigger"))) return;
      setTaskMenu(null);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setTaskMenu(null);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [taskMenu]);

  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      if (!convoMenu) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.closest(".wb-convo-menu") || el.closest(".wb-convo-menu-trigger"))) return;
      setConvoMenu(null);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setConvoMenu(null);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [convoMenu]);

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (cancelled) return;
        setDictationSettings(s.dictation ?? null);
      })
      .catch(() => { });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (useMultipleAgents) return;
    if (draftTracks.length <= 1) return;
    setDraftTracks((prev) => (prev.length > 0 ? [prev[0]] : prev));
  }, [useMultipleAgents, draftTracks.length]);

  const refreshTasks = async () => {
    if (!workspaceId) return;
    setTasks(await listTasks(workspaceId));
  };

  const refreshTaskDetail = async (
    taskId: string,
    preferredTrackId: string | null,
    preferredSessionId: string | null,
    signal?: AbortSignal,
    seq?: number,
  ) => {
    const trs = await listTracks(taskId);
    if (signal?.aborted) return;
    if (seq !== undefined && seq !== taskDetailLoadSeqRef.current) return;
    setTracks(trs);
    const map: Record<string, any[]> = {};
    await Promise.allSettled(
      trs.map(async (tr) => {
        const trid = idToString(tr.id);
        try {
          map[trid] = await listSessionsForTrack(trid);
        } catch {
          map[trid] = [];
        }
      }),
    );
    if (signal?.aborted) return;
    if (seq !== undefined && seq !== taskDetailLoadSeqRef.current) return;
    setSessionsByTrack(map);
    const trackIds = trs.map((tr) => idToString(tr.id)).filter(Boolean);
    setActiveTrackId((prev) => {
      const wantedFromSession =
        preferredSessionId && trackIds.length > 0
          ? trackIds.find((trid) => (map[trid] ?? []).some((s: any) => idToString(s?.id) === preferredSessionId))
          : null;
      const wanted = wantedFromSession ?? preferredTrackId ?? prev;
      return pickPreferredTrackId(trackIds, map, wanted);
    });
  };

  useEffect(() => {
    if (!workspaceId) return;
    getWorkspace(workspaceId).then(setWorkspace).catch(() => setWorkspace(null));
    refreshTasks().catch(() => { });
    listProviders().then(setProviders).catch(() => setProviders([]));
    getLspStatus().then(setLspStatus).catch(() => setLspStatus(null));
  }, [workspaceId]);

  const attachProviderInstall = useCallback((providerId: string, installId: string) => {
    setProviderInstallsById((prev) => {
      if (prev[providerId]?.installId === installId) return prev;
      return {
        ...prev,
        [providerId]: {
          installId,
          state: "running",
          pct: prev[providerId]?.pct ?? 0,
        },
      };
    });
  }, []);

  useEffect(() => {
    for (const p of providers) {
      const installId = p.details?.install_id;
      const running = p.details?.install_running === "true";
      if (running && installId && !providerInstallsById[p.provider_id]) {
        attachProviderInstall(p.provider_id, installId);
      }
    }
  }, [providers, providerInstallsById, attachProviderInstall]);

  useEffect(() => {
    const running = Object.entries(providerInstallsById).flatMap(([providerId, s]) =>
      s && s.state === "running" ? ([[providerId, s]] as const) : [],
    );
    if (running.length === 0) return;

    const stagePct: Record<string, number> = {
      start: 2,
      download: 10,
      node: 15,
      node_download: 18,
      node_extract: 22,
      prepare: 25,
      venv: 35,
      npm_install: 65,
      pip_install: 70,
      extract: 78,
      entrypoint: 80,
      inspect: 90,
      refresh: 95,
      registry: 98,
    };

    let cancelled = false;
    const tick = async () => {
      if (cancelled) return;
      let needsProviderRefresh = false;
      await Promise.all(
        running.map(async ([providerId, s]) => {
          try {
            const info = await getInstall(s.installId);
            const last = info.last_event;
            const pct =
              typeof last?.bytes === "number" &&
                typeof last?.total_bytes === "number" &&
                last.total_bytes > 0
                ? (() => {
                  const raw = Math.max(0, Math.min(100, Math.round((last.bytes / last.total_bytes) * 100)));
                  const stage = typeof last?.stage === "string" ? last.stage : "";
                  if (stage.includes("download")) {
                    // Keep room for non-download stages so the UI doesn't hit 100% early.
                    return Math.round((raw / 100) * 75);
                  }
                  return raw;
                })()
                : typeof last?.stage === "string"
                  ? (stagePct[last.stage] ?? s.pct ?? 0)
                  : (s.pct ?? 0);

            setProviderInstallsById((prev) => {
              const existing = prev[providerId];
              if (!existing || existing.installId !== s.installId) return prev;
              const stablePct =
                info.state === "succeeded"
                  ? 100
                  : typeof pct === "number" && Number.isFinite(pct)
                    ? Math.max(existing.pct ?? 0, pct)
                    : (existing.pct ?? 0);
              return {
                ...prev,
                [providerId]: { installId: s.installId, state: info.state, pct: stablePct },
              };
            });

            if (info.state !== "running") {
              needsProviderRefresh = true;
            }
          } catch {
            // ignore poll errors
          }
        }),
      );

      if (needsProviderRefresh) {
        try {
          const next = await listProviders();
          if (!cancelled) setProviders(next);
        } catch {
          // ignore
        }
      }
    };

    tick();
    const t = window.setInterval(tick, 1000);
    return () => {
      cancelled = true;
      window.clearInterval(t);
    };
  }, [providerInstallsById, getInstall, listProviders]);

  const installProviderFromMenu = useCallback(
    async (providerId: string) => {
      setStartError(null);
      try {
        const { install_id } = await installProvider(providerId);
        attachProviderInstall(providerId, install_id);
      } catch (e: any) {
        setStartError(e?.message ? String(e.message) : String(e));
      }
    },
    [setStartError, installProvider, attachProviderInstall],
  );

  const installAllProvidersFromMenu = useCallback(async () => {
    setStartError(null);
    setInstallAllBusy(true);
    try {
      const installs = await installAllProviders();
      for (const { provider_id, install_id } of installs) {
        attachProviderInstall(provider_id, install_id);
      }
    } catch (e: any) {
      setStartError(e?.message ? String(e.message) : String(e));
    } finally {
      setInstallAllBusy(false);
    }
  }, [attachProviderInstall, installAllProviders, setStartError]);

  useEffect(() => {
    if (Object.keys(providerInstallsById).length === 0) return;
    setProviderInstallsById((prev) => {
      let changed = false;
      const next: typeof prev = { ...prev };
      for (const [providerId, s] of Object.entries(prev)) {
        const st = providersById[providerId];
        const stillRunning = st?.details?.install_running === "true";
        if (s?.state === "succeeded" && st?.installed && !stillRunning) {
          delete next[providerId];
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [providerInstallsById, providersById]);

  // Load tracks for all tasks (for expansion UI)
  useEffect(() => {
    if (!tasks.length) return;
    const loadAllTracks = async () => {
      const map: Record<string, Track[]> = {};
      await Promise.all(
        tasks.map(async (task) => {
          try {
            const taskId = idToString(task.id);
            const tracks = await listTracks(taskId);
            map[taskId] = tracks;
          } catch {
            // Ignore errors for individual tasks
          }
        })
      );
      setTracksByTaskId(map);
    };
    loadAllTracks().catch(() => { });
  }, [tasks]);

  useEffect(() => {
    if (!workspaceId) return;

    if (selectionFromUrl.taskId) {
      // Treat the URL as the source of truth for selection, even before tasks/tracks have loaded.
      // Validation happens once tasks are hydrated.
      didAttemptSelectionRestoreRef.current = true;
      setActiveTaskId(selectionFromUrl.taskId);
      setActiveTrackId(selectionFromUrl.trackId);
      if (tasks.length > 0) {
        const exists = tasks.some((t) => idToString(t.id) === selectionFromUrl.taskId);
        if (!exists) updateWorkbenchUrlSelection({ taskId: null, trackId: null, sessionId: null }, true);
      }
      return;
    }

    // URL cleared selection: clear local selection too.
    if (activeTaskId !== null) setActiveTaskId(null);
    if (activeTrackId !== null) setActiveTrackId(null);
  }, [
    workspaceId,
    selectionFromUrl.taskId,
    selectionFromUrl.trackId,
    tasks,
    updateWorkbenchUrlSelection,
    activeTaskId,
    activeTrackId,
  ]);

  useEffect(() => {
    if (!workspaceId) return;
    if (selectionFromUrl.taskId) return;
    if (!persistedSelectionLoaded) return;
    if (didAttemptSelectionRestoreRef.current) return;
    didAttemptSelectionRestoreRef.current = true;

    const stored = persistedSelection;
    if (!stored?.taskId) return;
    updateWorkbenchUrlSelection(
      { taskId: stored.taskId, trackId: stored.trackId, sessionId: stored.sessionId },
      true,
    );
  }, [workspaceId, selectionFromUrl.taskId, persistedSelectionLoaded, persistedSelection, updateWorkbenchUrlSelection]);

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
    taskDetailAbortRef.current?.abort();
    const controller = new AbortController();
    taskDetailAbortRef.current = controller;
    const seq = ++taskDetailLoadSeqRef.current;

    if (!activeTaskId) {
      setTracks([]);
      setSessionsByTrack({});
      setEditPlans([]);
      setActiveEditPlanId(null);
      controller.abort();
      return;
    }
    const useUrl = selectionFromUrl.taskId === activeTaskId;
    const preferredTrackId = useUrl ? selectionFromUrl.trackId : null;
    const preferredSessionId = useUrl ? selectionFromUrl.sessionId : null;
    // Prime selection from URL to avoid clobbering deep-links while task detail hydrates.
    setActiveTrackId(preferredTrackId ?? null);
    setTracks([]);
    setSessionsByTrack({});
    setEditPlans([]);
    setActiveEditPlanId(null);
    refreshTaskDetail(activeTaskId, preferredTrackId, preferredSessionId, controller.signal, seq).catch(() => { });
    return () => controller.abort();
  }, [
    activeTaskId,
    selectionFromUrl.sessionId,
    selectionFromUrl.taskId,
    selectionFromUrl.trackId,
  ]);

  useEffect(() => {
    editPlansAbortRef.current?.abort();
    const controller = new AbortController();
    editPlansAbortRef.current = controller;
    const seq = ++editPlansLoadSeqRef.current;

    if (!activeTrackId) {
      setEditPlans([]);
      setActiveEditPlanId(null);
      controller.abort();
      return;
    }
    setEditPlans([]);
    setActiveEditPlanId(null);
    listEditPlansForTrack(activeTrackId, controller.signal)
      .then((plans) => {
        if (controller.signal.aborted) return;
        if (seq !== editPlansLoadSeqRef.current) return;
        setEditPlans(plans);
        const first = plans[0] ? idToString(plans[0].id) : null;
        setActiveEditPlanId((prev) => (prev && plans.some((p) => idToString(p.id) === prev) ? prev : first));
      })
      .catch((e: any) => {
        if (e?.name === "AbortError") return;
        setEditPlans([]);
        setActiveEditPlanId(null);
      });

    return () => controller.abort();
  }, [activeTrackId]);

  const sortedTasks = useMemo(() => {
    return [...tasks].sort((a, b) => {
      const ta = taskActivityMs(a) ?? 0;
      const tb = taskActivityMs(b) ?? 0;
      return tb - ta;
    });
  }, [tasks]);

  const filteredTasks = useMemo(() => {
    const q = taskQuery.trim().toLowerCase();
    if (!q) return sortedTasks;
    return sortedTasks.filter((t) => (t.title ?? "").toLowerCase().includes(q));
  }, [sortedTasks, taskQuery]);

  // Helper to check if agent is still working (hasn't finished responding)
  const isAgentStillWorking = useCallback((entry: any): boolean => {
    const doneLike = new Set(["done", "assistant_complete", "turn_interrupted"]);
    const ignored = new Set(["input_queued", "notice", "interrupt_requested"]);

    const events = Array.isArray(entry?.events) ? entry.events : [];
    const messages = Array.isArray(entry?.messages) ? entry.messages : [];

    const lastRelevantEventType = (() => {
      for (let i = events.length - 1; i >= 0; i--) {
        const t = String(events[i]?.event_type ?? "").trim();
        if (!t) continue;
        if (ignored.has(t)) continue;
        return t;
      }
      return "";
    })();

    if (lastRelevantEventType) {
      return !doneLike.has(lastRelevantEventType);
    }

    const lastUser = lastRoleMessageMs(messages, "user");
    if (lastUser === null) return false;
    const lastAssistant = lastRoleMessageMs(messages, "assistant");
    return lastAssistant === null || lastUser > lastAssistant;
  }, []);

  const taskLiveInfo = useMemo(() => {
    const workingByTask = new Set<string>();
    const errorByTask = new Set<string>();
    const lastAssistantMsByTask: Record<string, number> = {};
    for (const entry of Object.values(sessionSnap.sessions)) {
      const taskId = entry.session ? idToString(entry.session.task_id) : "";
      if (!taskId) continue;

      // Check if agent is still working on this session
      if (isAgentStillWorking(entry)) {
        workingByTask.add(taskId);
      }

      // Error state - check multiple sources
      const hasErrorStatus =
        entry.session?.status === "failed" ||
        entry.session?.status === "cancelled";
      const hasErrorInSupervisor = !!entry.error;
      const hasErrorEvent = entry.events.some(ev => ev.event_type === "error");

      if (hasErrorStatus || hasErrorInSupervisor || hasErrorEvent) {
        errorByTask.add(taskId);
      }

      const ms = lastAssistantMessageMs(entry.messages);
      if (ms !== null) lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, ms);
    }
    return { workingByTask, errorByTask, lastAssistantMsByTask };
  }, [sessionSnap.sessions, isAgentStillWorking]);

  const providerIdsByTaskFromSessions = useMemo(() => {
    const byTask: Record<string, Array<{ providerId: string; updatedAt: number }>> = {};
    for (const entry of Object.values(sessionSnap.sessions)) {
      const sess: any = entry.session;
      const taskId = sess ? idToString(sess.task_id) : "";
      const providerId = String(sess?.provider_id ?? "").trim();
      if (!taskId || !providerId) continue;
      (byTask[taskId] ??= []).push({ providerId, updatedAt: entry.updatedAtMs ?? 0 });
    }
    const out: Record<string, string[]> = {};
    for (const [taskId, list] of Object.entries(byTask)) {
      const seen = new Set<string>();
      const ordered = list
        .slice()
        .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0))
        .map((x) => x.providerId)
        .filter((p) => {
          if (seen.has(p)) return false;
          seen.add(p);
          return true;
        });
      out[taskId] = ordered;
    }
    return out;
  }, [sessionSnap.sessions]);

  const taskProviderFetchInFlightRef = useRef<Record<string, Promise<void>>>({});
  useEffect(() => {
    if (tasks.length === 0) return;
    const maxFetch = 24;
    const candidates = tasks
      .map((t) => idToString(t.id))
      .filter(Boolean)
      .filter((tid) => {
        if ((providerIdsByTaskFromSessions[tid] ?? []).length > 0) return false;
        if ((taskProviderIdsByTaskId[tid] ?? []).length > 0) return false;
        const trs = tracksByTaskId[tid] ?? [];
        return trs.length > 0;
      })
      .slice(0, maxFetch);

    for (const tid of candidates) {
      if (taskProviderFetchInFlightRef.current[tid]) continue;
      const p = (async () => {
        const trs = tracksByTaskId[tid] ?? [];
        const providerIds: string[] = [];
        const seen = new Set<string>();
        for (const tr of trs.slice(0, 3)) {
          const trid = idToString(tr.id);
          if (!trid) continue;
          let sessions: any[] = [];
          try {
            sessions = await listSessionsForTrack(trid);
          } catch {
            continue;
          }
          for (const s of sessions) {
            const pid = String((s as any)?.provider_id ?? "").trim();
            if (!pid || seen.has(pid)) continue;
            seen.add(pid);
            providerIds.push(pid);
            if (providerIds.length >= 3) break;
          }
          if (providerIds.length >= 3) break;
        }
        if (providerIds.length > 0) {
          setTaskProviderIdsByTaskId((prev) => {
            if ((prev[tid] ?? []).length > 0) return prev;
            return { ...prev, [tid]: providerIds };
          });
        }
      })().finally(() => {
        delete taskProviderFetchInFlightRef.current[tid];
      });
      taskProviderFetchInFlightRef.current[tid] = p;
    }
  }, [providerIdsByTaskFromSessions, taskProviderIdsByTaskId, tasks, tracksByTaskId]);

  const activeTasks = useMemo(() => filteredTasks.filter((t) => !t.archived_at), [filteredTasks]);
  const archivedTasks = useMemo(() => filteredTasks.filter((t) => !!t.archived_at), [filteredTasks]);

  const markTaskReadInFlightRef = useRef<Record<string, Promise<void>>>({});

  const markTaskRead = useCallback(async (taskId: string) => {
    if (markTaskReadInFlightRef.current[taskId]) return;
    const p = (async () => {
      try {
        const updated = await markTaskReadApi(taskId);
        setTasks((prev) => prev.map((t) => (idToString(t.id) === taskId ? { ...t, ...updated } : t)));
      } catch {
        // ignore
      }
    })().finally(() => {
      delete markTaskReadInFlightRef.current[taskId];
    });
    markTaskReadInFlightRef.current[taskId] = p;
    await p;
  }, []);

  const markTaskUnread = useCallback(async (taskId: string) => {
    try {
      const updated = await markTaskUnreadApi(taskId);
      setTasks((prev) => prev.map((t) => (idToString(t.id) === taskId ? { ...t, ...updated } : t)));
    } catch {
      // ignore
    }
  }, []);

  useEffect(() => {
    if (!activeTaskId) return;
    const tid = activeTaskId;
    const t = tasks.find((x) => idToString(x.id) === tid);
    if (!t) return;
    const working = taskLiveInfo.workingByTask.has(tid);
    const serverLastAssistantMs = parseMs(t.last_assistant_message_at ?? null);
    const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[tid] ?? null;
    const lastAssistantMs =
      liveLastAssistantMs !== null && serverLastAssistantMs !== null
        ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
        : liveLastAssistantMs ?? serverLastAssistantMs;
    const seenMs = parseMs(t.assistant_seen_at ?? null);
    const unread = !working && lastAssistantMs !== null && (seenMs === null || lastAssistantMs > seenMs);
    if (!unread) return;
    void markTaskRead(tid);
  }, [activeTaskId, markTaskRead, taskLiveInfo.lastAssistantMsByTask, taskLiveInfo.workingByTask, tasks]);

  useEffect(() => {
    if (!renamingTaskId) return;
    requestAnimationFrame(() => {
      const el = renameInputRef.current;
      if (!el) return;
      el.focus();
      el.select();
    });
  }, [renamingTaskId]);

  const onToggleArchive = useCallback(
    async (taskId: string, nextArchived: boolean) => {
      const updated = nextArchived ? await archiveTask(taskId) : await unarchiveTask(taskId);
      setTasks((prev) => prev.map((t) => (idToString(t.id) === taskId ? { ...t, ...updated } : t)));
      if (nextArchived && activeTaskId === taskId) setArchivedCollapsed(false);
    },
    [activeTaskId],
  );

  const openTaskMenu = useCallback((taskId: string, opts: { triggerEl: HTMLElement } | { x: number; y: number }) => {
    const baseLeft =
      "triggerEl" in opts ? opts.triggerEl.getBoundingClientRect().left : Math.max(8, Math.min(opts.x, window.innerWidth - 8));
    const baseTop =
      "triggerEl" in opts
        ? opts.triggerEl.getBoundingClientRect().bottom + 6
        : Math.max(8, Math.min(opts.y, window.innerHeight - 8));
    const left = Math.min(baseLeft, window.innerWidth - 240);
    const top = Math.min(baseTop, window.innerHeight - 260);
    setTaskMenu((prev) => (prev?.taskId === taskId ? null : { taskId, style: { left, top } }));
  }, []);

  const beginRenameTask = useCallback(
    (taskId: string) => {
      const t = tasks.find((x) => idToString(x.id) === taskId);
      setRenamingTaskId(taskId);
      setRenameDraft(String(t?.title ?? "").trim());
    },
    [tasks],
  );

  const cancelRenameTask = useCallback(() => {
    setRenamingTaskId(null);
    setRenameDraft("");
  }, []);

  const commitRenameTask = useCallback(
    async (taskId: string) => {
      const next = renameDraft.trim();
      if (!next) {
        window.alert("Task title is required.");
        return;
      }
      const current = String(tasks.find((t) => idToString(t.id) === taskId)?.title ?? "").trim();
      if (current && current === next) {
        cancelRenameTask();
        return;
      }
      try {
        const updated = await updateTaskTitle(taskId, next);
        setTasks((prev) => prev.map((t) => (idToString(t.id) === taskId ? { ...t, ...updated } : t)));
        cancelRenameTask();
      } catch (e: any) {
        window.alert(e?.message ?? "Failed to rename.");
      }
    },
    [cancelRenameTask, renameDraft, tasks],
  );

  const onDeleteTask = useCallback(
    async (taskId: string) => {
      const t = tasks.find((x) => idToString(x.id) === taskId);
      const title = String(t?.title ?? "this task");
      if (!window.confirm(`Delete “${title}”? This deletes all sessions and messages in the task.`)) return;
      try {
        await deleteTask(taskId);
        setTasks((prev) => prev.filter((x) => idToString(x.id) !== taskId));
        setTracksByTaskId((prev) => {
          const next = { ...prev };
          delete next[taskId];
          return next;
        });
        setTaskProviderIdsByTaskId((prev) => {
          const next = { ...prev };
          delete next[taskId];
          return next;
        });
        if (activeTaskId === taskId) {
          applyWorkbenchSelection({ taskId: null, trackId: null, sessionId: null }, true);
        }
      } catch (e: any) {
        window.alert(e?.message ?? "Failed to delete task.");
      }
    },
    [activeTaskId, tasks, applyWorkbenchSelection],
  );

  const openConvoMenu = useCallback((triggerEl: HTMLElement) => {
    const rect = triggerEl.getBoundingClientRect();
    const left = Math.min(rect.left, window.innerWidth - 240);
    const top = Math.min(rect.bottom + 6, window.innerHeight - 220);
    setConvoMenu((prev) => (prev ? null : { style: { left, top } }));
  }, []);

  const activeSessionId = useMemo(() => {
    if (!activeTrackId) return null;
    const sessions = sessionsByTrack[activeTrackId] ?? [];
    if (selectionFromUrl.sessionId) {
      const trackMatches = !selectionFromUrl.trackId || selectionFromUrl.trackId === activeTrackId;
      if (trackMatches) {
        // If the URL specifies a session, prefer it even before sessions are loaded for the track.
        // This makes refresh + deep-links deterministic and avoids transient “empty” UI states.
        if (sessions.length === 0) return selectionFromUrl.sessionId;
        const matches = sessions.some((s: any) => idToString(s?.id) === selectionFromUrl.sessionId);
        if (matches) return selectionFromUrl.sessionId;
      }
    }
    return pickPreferredSessionId(sessions);
  }, [activeTrackId, sessionsByTrack, selectionFromUrl.sessionId, selectionFromUrl.trackId]);

  useEffect(() => {
    if (!workspaceId) return;
    // If the URL already specifies a Task but state hasn't hydrated yet, don't clobber it.
    if (!activeTaskId && selectionFromUrl.taskId) return;

    const shouldDeferUrlSync =
      selectionFromUrl.taskId === activeTaskId &&
      tracks.length === 0 &&
      ((selectionFromUrl.trackId && selectionFromUrl.trackId !== activeTrackId) ||
        (selectionFromUrl.sessionId && !activeSessionId));
    if (shouldDeferUrlSync) return;

    const next = { taskId: activeTaskId, trackId: activeTrackId, sessionId: activeSessionId };
    const persisted: PersistedWorkbenchSelectionV1 = {
      v: 1,
      taskId: next.taskId,
      trackId: next.taskId ? next.trackId : null,
      sessionId: next.taskId && next.trackId ? next.sessionId : null,
    };
    setPersistedSelection(persisted);
    saveWorkbenchSelectionV1(workspaceId, persisted).catch((e: unknown) => {
      const msg = e instanceof Error ? e.message : String(e);
      setUiStateWarning(`UI state persistence disabled: ${msg}`);
    });

    const matchesUrl =
      selectionFromUrl.taskId === next.taskId &&
      selectionFromUrl.trackId === next.trackId &&
      selectionFromUrl.sessionId === next.sessionId;
    if (matchesUrl) return;
    updateWorkbenchUrlSelection(next, true);
  }, [
    workspaceId,
    activeTaskId,
    activeTrackId,
    activeSessionId,
    selectionFromUrl.sessionId,
    selectionFromUrl.taskId,
    selectionFromUrl.trackId,
    updateWorkbenchUrlSelection,
  ]);

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

  const sessionCache = useSessionCacheSnapshot();
  const activeEntry = useSessionEntry(activeSessionId ?? "");
  const activeTrackDiff = activeEntry?.diff ?? "";
  const activeTrackIdFromSession = activeEntry?.session ? idToString(activeEntry.session.track_id) : "";
  const activeWorktreeId = activeEntry?.session ? idToString(activeEntry.session.worktree_id) : "";

  useEffect(() => {
    if (!activeWorktreeId) {
      setActiveWorktree(null);
      return;
    }
    let cancelled = false;
    getWorktree(activeWorktreeId)
      .then((wt) => {
        if (cancelled) return;
        setActiveWorktree(wt);
      })
      .catch(() => {
        if (cancelled) return;
        setActiveWorktree(null);
      });
    return () => {
      cancelled = true;
    };
  }, [activeWorktreeId]);

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

  const providerOptionsInFlightRef = useRef<Record<string, Promise<ProviderOptions | undefined>>>({});

  const ensureProviderOptions = useCallback(
    async (providerId: string): Promise<ProviderOptions | undefined> => {
      if (!workspaceId) return;
      const installed = providersById[providerId]?.installed ?? false;
      if (!installed) return;
      if (providerOptions[providerId]) return providerOptions[providerId];

      const existing = providerOptionsInFlightRef.current[providerId];
      if (existing) return existing;

      const p = getProviderOptions(workspaceId, providerId)
        .then((opts) => {
          setProviderOptions((prev) => ({ ...prev, [providerId]: opts }));
          return opts;
        })
        .finally(() => {
          delete providerOptionsInFlightRef.current[providerId];
        });

      providerOptionsInFlightRef.current[providerId] = p;
      return p;
    },
    [providerOptions, providersById, workspaceId],
  );

  const slashCommands = useMemo<SlashCommandDescriptor[]>(() => {
    // New sessions don't have ACP "available_commands_update" yet, so use a safe fallback.
    return [
      { name: "review", description: "Review my current changes and find issues" },
      { name: "review-branch", description: "Review a branch" },
      { name: "review-commit", description: "Review a commit" },
      { name: "init", description: "Create an AGENTS.md file" },
      { name: "compact", description: "Summarize conversation to save context" },
      { name: "logout", description: "Log out" },
      { name: "help", description: "Show help" },
    ];
  }, []);

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

  const stopDictation = useCallback(async (): Promise<string> => {
    setDictationRecording(false);

    const ws = dictationWsRef.current;

    const mic = dictationMicRef.current;
    dictationMicRef.current = null;

    try {
      await mic?.stop();
    } catch { }

    try {
      ws?.send(JSON.stringify({ type: "stop" }));
    } catch { }

    const base = dictationBaseRef.current;
    const committed = dictationCommittedRef.current;
    const interim = dictationInterimRef.current;
    const next = appendSegment(appendSegment(base, committed), interim);
    setDraftPrompt(next);
    dictationInterimRef.current = "";
    dictationReadyRef.current = false;
    dictationAudioStartedRef.current = false;

    return next;
  }, []);

  const startDictation = useCallback(async () => {
    setDictationError(null);

    const enabled =
      Boolean(dictationSettings?.enabled) && dictationSettings?.provider === "livekit_inference";
    if (!enabled) {
      setDictationError("Dictation is disabled. Configure it in Settings.");
      return;
    }

    const existing = dictationWsRef.current;
    if (existing && existing.readyState !== WebSocket.CLOSED) return;
    if (dictationRecording) return;

    const token = (() => {
      try {
        return sessionStorage.getItem("contextAuthToken");
      } catch {
        return null;
      }
    })();
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";
    const base = getDaemonBaseUrl();
    const wsBase = base
      ? base.startsWith("https://")
        ? base.replace(/^https:\/\//, "wss://")
        : base.replace(/^http:\/\//, "ws://")
      : `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}`;
    const url = `${wsBase}/api/dictation/livekit/stream${qs}`;

    const ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    dictationWsRef.current = ws;

    dictationBaseRef.current = draftPrompt;
    dictationCommittedRef.current = "";
    dictationInterimRef.current = "";
    dictationAudioBytesRef.current = 0;
    dictationAudioChunksRef.current = 0;
    dictationReadyRef.current = false;
    dictationAudioStartedRef.current = false;
    dictationTranscriptMsgsRef.current = 0;

    const openPromise = new Promise<void>((resolve, reject) => {
      ws.addEventListener("open", () => resolve(), { once: true });
      ws.addEventListener("error", () => reject(new Error("Failed to connect to dictation stream.")), { once: true });
    });

    ws.addEventListener("message", (ev) => {
      void parseWsJson((ev as MessageEvent).data).then((data) => {
        if (!data) return;
        const t = String(data.type ?? "");
        if (t === "ready") {
          dictationReadyRef.current = true;
          return;
        } else if (t === "audio_started") {
          dictationAudioStartedRef.current = true;
          return;
        } else if (t === "interim") {
          dictationTranscriptMsgsRef.current += 1;
          dictationInterimRef.current = String(data.text ?? "");
        } else if (t === "final") {
          dictationTranscriptMsgsRef.current += 1;
          dictationCommittedRef.current = appendSegment(dictationCommittedRef.current, String(data.text ?? ""));
          dictationInterimRef.current = "";
        } else if (t === "done") {
          try {
            ws.close();
          } catch { }
          return;
        } else if (t === "error") {
          setDictationError(String(data.message ?? "Dictation error"));
          stopDictation().catch(() => { });
          return;
        } else {
          return;
        }

        const base = dictationBaseRef.current;
        const committed = dictationCommittedRef.current;
        const interim = dictationInterimRef.current;
        setDraftPrompt(appendSegment(appendSegment(base, committed), interim));
      });
    });

    ws.addEventListener("close", () => {
      dictationWsRef.current = null;
      setDictationRecording(false);
    });

    try {
      await openPromise;
      setDictationRecording(true);
      dictationMicRef.current = await startMicPcmStream({
        onPcmChunk: (pcm16) => {
          dictationAudioChunksRef.current += 1;
          dictationAudioBytesRef.current += pcm16.byteLength;
          if (ws.readyState === WebSocket.OPEN) ws.send(pcm16);
        },
        onError: (err) => {
          setDictationError(err.message);
          stopDictation().catch(() => { });
        },
      });
    } catch (e: any) {
      setDictationError(e?.message ?? String(e));
      try {
        ws.close();
      } catch { }
      dictationWsRef.current = null;
      setDictationRecording(false);
    }
  }, [dictationSettings, dictationRecording, draftPrompt, stopDictation]);

  useEffect(() => {
    return () => {
      stopDictation().catch(() => { });
    };
  }, [stopDictation]);

  useEffect(() => {
    if (!dictationDebugEnabled) return;
    if (!dictationRecording) {
      setDictationDebugText(null);
      return;
    }
    const timer = window.setInterval(() => {
      const ws = dictationWsRef.current;
      const wsState = ws ? ws.readyState : -1;
      setDictationDebugText(
        `Dictation debug\nws_state=${wsState} ready=${dictationReadyRef.current} audio_started=${dictationAudioStartedRef.current}\naudio_chunks=${dictationAudioChunksRef.current} audio_bytes=${dictationAudioBytesRef.current} transcript_msgs=${dictationTranscriptMsgsRef.current}`,
      );
    }, 500);
    return () => window.clearInterval(timer);
  }, [dictationDebugEnabled, dictationRecording]);

  const startNewTask = async () => {
    if (!workspaceId) return;
    const prompt = (dictationRecording ? await stopDictation() : draftPrompt).trim();
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
	      let firstTrackId: string | null = null;
	      let firstSessionId: string | null = null;
	      setActiveTaskId(taskId);
	      setActiveTrackId(null);
	      updateWorkbenchUrlSelection({ taskId, trackId: null, sessionId: null }, true);

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
        const env_target = execTarget === "local" ? "local" : "worktree";
        const tr = await createTrack(taskId, label, { env_target });
	        const trackId = idToString(tr.id);
	        const opts = await ensureProviderOptions(dt.providerId).catch(() => undefined);
	        const modelIds = modelIdsFromOptions(opts ?? providerOptions[dt.providerId]);
	        const modelId = dt.modelId || modelIds[0] || (dt.providerId === "fake" ? "fake-model" : "default");
	        const session = await createSession(trackId, dt.providerId, modelId);
	        const sessionId = idToString(session.id);
	        if (!firstTrackId) firstTrackId = trackId;
        if (!firstSessionId) {
	          firstSessionId = sessionId;
	          setActiveTrackId(trackId);
	          updateWorkbenchUrlSelection({ taskId, trackId, sessionId }, true);
	        }
	        supervisor.refreshSession(sessionId, { watchDiff: true });
	        supervisor.refreshQueue(sessionId);
	        await postMessage(sessionId, prompt, "immediate", draftAttachments);
        // Ensure the workbench view can render the just-posted user message (and any streamed events)
        // without waiting for a `done` event to trigger a refresh.
        supervisor.refreshQueue(sessionId);
        supervisor.refreshSession(sessionId, { watchDiff: true });
	      }

	      if (firstTrackId) {
	        await refreshTaskDetail(taskId, firstTrackId, firstSessionId);
	      }
	      await refreshTasks();
	      setDraftPrompt("");
	      setDraftAttachments([]);
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

  const onSidebarResizerMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (sidebarCollapsed) return;
    setSidebarResizing(true);
    const startX = e.clientX;
    const startW = sidebarWidth;
    const onMove = (ev: MouseEvent) => {
      const dx = ev.clientX - startX;
      const max = Math.max(170, window.innerWidth - 240);
      const next = Math.min(max, Math.max(170, Math.round(startW + dx)));
      setSidebarWidth(next);
    };
    const onUp = () => {
      setSidebarResizing(false);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const onDropFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      const next = await imageFilesToInlineAttachments(files);
      if (next.length === 0) return;
      setDraftAttachments((prev) => [...prev, ...next]);
    },
    [],
  );

  const showDropOverlay = useCallback(() => {
    setDropActive(true);
    if (dropHideTimerRef.current) window.clearTimeout(dropHideTimerRef.current);
    dropHideTimerRef.current = window.setTimeout(() => setDropActive(false), 140);
  }, []);

  const hideDropOverlay = useCallback(() => {
    if (dropHideTimerRef.current) window.clearTimeout(dropHideTimerRef.current);
    dropHideTimerRef.current = null;
    setDropActive(false);
  }, []);

  const extractFilesFromTransfer = useCallback(
    (dt: DataTransfer | null): File[] => {
      if (!dt) return [];
      const out: File[] = [];
      const files = dt.files ? Array.from(dt.files) : [];
      out.push(...files);
      const items = dt.items;
      if (out.length === 0 && items && items.length > 0) {
        for (const item of Array.from(items as any) as DataTransferItem[]) {
          if (item.kind !== "file") continue;
          const f = item.getAsFile?.();
          if (f) out.push(f);
        }
      }
      return out;
    },
    [],
  );

  const extractFirstUrlFromTransfer = useCallback((dt: DataTransfer | null): string | null => {
    if (!dt) return null;
    const uriRaw = (dt.getData?.("text/uri-list") ?? "").trim();
    if (uriRaw) {
      for (const line of uriRaw.split("\n")) {
        const v = line.trim();
        if (!v || v.startsWith("#")) continue;
        return v;
      }
    }
    const html = (dt.getData?.("text/html") ?? "").trim();
    if (html) {
      const m = html.match(/<img[^>]*\ssrc=("([^"]+)"|'([^']+)'|([^\s>]+))/i);
      const src = (m?.[2] ?? m?.[3] ?? m?.[4] ?? "").trim();
      if (src) return src;
    }
    const text = (dt.getData?.("text/plain") ?? "").trim();
    if (text && /^(https?:|data:image\/|blob:)/i.test(text)) return text;
    return null;
  }, []);

  const urlToImageFile = useCallback(async (url: string): Promise<File | null> => {
    try {
      const res = await fetch(url);
      if (!res.ok) return null;
      const blob = await res.blob();
      const type = blob.type || "";
      if (!type.startsWith("image/")) return null;
      const baseName = (() => {
        try {
          const u = new URL(url, window.location.href);
          const last = u.pathname.split("/").filter(Boolean).pop() || "image";
          return last.replace(/[?#].*$/, "") || "image";
        } catch {
          return "image";
        }
      })();
      const ext = type.split("/")[1] || "";
      const name = ext && !baseName.toLowerCase().endsWith(`.${ext.toLowerCase()}`) ? `${baseName}.${ext}` : baseName;
      return new File([blob], name, { type });
    } catch {
      return null;
    }
  }, []);

  const activeTask = activeTaskId ? tasks.find((t) => idToString(t.id) === activeTaskId) : null;
  const singleTrackHeader = useMemo(() => {
    if (tracks.length !== 1) return null;
    const sess = activeEntry?.session ?? null;
    const parsedModel = parseModelId(sess?.model_id ?? "");
    const harness =
      HARNESS_CATALOG.find((h) => h.id === (sess?.provider_id ?? ""))?.label ??
      (sess?.provider_id ?? "Provider");

    const lastIso = (() => {
      const entry = activeEntry;
      if (!entry) return null;
      let bestIso: string | null = entry.session?.updated_at ?? null;
      let bestMs = bestIso ? parseMs(bestIso) ?? -1 : -1;
      for (const m of entry.messages ?? []) {
        const ms = parseMs(m.created_at);
        if (ms !== null && ms >= bestMs) {
          bestMs = ms;
          bestIso = m.created_at;
        }
      }
      for (const e of entry.events ?? []) {
        const ms = parseMs(e.created_at);
        if (ms !== null && ms >= bestMs) {
          bestMs = ms;
          bestIso = e.created_at;
        }
      }
      return bestIso;
    })();

    const age = formatRelativeAgeShort(lastIso) || "Now";
    const worktreePath = sess?.env_target === "worktree" ? String(activeWorktree?.root_path ?? "") : "";
    const worktreeSlug = worktreePath ? lastPathSegment(worktreePath) : "";

    return {
      title: activeTask?.title ?? "Conversation",
      age,
      harness,
      modelBase: parsedModel.base || String(sess?.model_id ?? ""),
      effort: parsedModel.effort,
      worktreeSlug,
      worktreePath,
      canCopyWorktree: Boolean(worktreePath),
    };
  }, [activeEntry, activeTask?.title, activeWorktree?.root_path, tracks.length]);

  const formatWorktreePathForCopy = useCallback((raw: string): string => {
    const path = String(raw ?? "").trim();
    if (!path) return "";

    // Prefer "~" for user home directories.
    // This is a UI convenience for paths that typically get pasted into shells.
    const posixMatch = path.match(/^\/(?:home|Users)\/[^/]+(\/.*)?$/);
    if (posixMatch) return `~${posixMatch[1] ?? ""}`;

    const windowsMatch = path.match(/^[A-Za-z]:\\Users\\[^\\]+(\\.*)?$/);
    if (windowsMatch) return `~${windowsMatch[1] ?? ""}`;

    return path;
  }, []);

  const [worktreeCopied, setWorktreeCopied] = useState(false);
  const worktreeCopiedTimerRef = useRef<ReturnType<typeof window.setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (worktreeCopiedTimerRef.current) {
        window.clearTimeout(worktreeCopiedTimerRef.current);
        worktreeCopiedTimerRef.current = null;
      }
    };
  }, []);

  const copyWorktreeLocation = useCallback(async () => {
    const path = String(singleTrackHeader?.worktreePath ?? "").trim();
    if (!path) return;
    try {
      await navigator.clipboard.writeText(formatWorktreePathForCopy(path));
      setWorktreeCopied(true);
      if (worktreeCopiedTimerRef.current) {
        window.clearTimeout(worktreeCopiedTimerRef.current);
      }
      worktreeCopiedTimerRef.current = window.setTimeout(() => {
        setWorktreeCopied(false);
        worktreeCopiedTimerRef.current = null;
      }, 1100);
    } catch (e: any) {
      window.alert(e?.message ?? "Failed to copy worktree location.");
    }
  }, [formatWorktreePathForCopy, singleTrackHeader?.worktreePath]);

  const exportConversation = useCallback(async () => {
    if (!activeEntry?.session) return;
    const sess = activeEntry.session;

    const harness =
      HARNESS_CATALOG.find((h) => h.id === (sess?.provider_id ?? ""))?.label ??
      (sess?.provider_id ?? "Provider");
    const parsedModel = parseModelId(sess?.model_id ?? "");

    const thread = buildWorkbenchThreadViewModel(activeEntry.events ?? [], activeEntry.messages ?? []);
    const exportedAt = new Date().toISOString();

    const lines: string[] = [];
    const title = singleTrackHeader?.title ?? "Conversation";
    lines.push(`# ${title}`);
    lines.push("");
    lines.push(`- Exported: ${exportedAt}`);
    lines.push(`- Harness: ${harness}`);
    lines.push(`- Model: ${parsedModel.base || String(sess.model_id ?? "")}`);
    if (parsedModel.effort) lines.push(`- Effort: ${parsedModel.effort}`);
    if (singleTrackHeader?.worktreePath) lines.push(`- Worktree: ${formatWorktreePathForCopy(singleTrackHeader.worktreePath)}`);
    lines.push(`- Session ID: ${idToString(sess.id)}`);
    lines.push("");
    lines.push("---");
    lines.push("");

    const attachmentLine = (atts: any[]): string => {
      const names = (atts ?? [])
        .map((a) => String(a?.name ?? a?.blob_id ?? a?.kind ?? "").trim())
        .filter(Boolean);
      if (names.length === 0) return "";
      return `Attachments: ${names.join(", ")}`;
    };

    for (let i = 0; i < (thread.groups ?? []).length; i++) {
      const g: any = (thread.groups ?? [])[i];
      lines.push(`## Turn ${i + 1}`);
      lines.push("");

      if (g?.header) {
        lines.push(`### User (${String(g.header.created_at ?? "").trim() || "unknown time"})`);
        lines.push("");
        const attsLine = attachmentLine(g.header.attachments ?? []);
        if (attsLine) {
          lines.push(`_${attsLine}_`);
          lines.push("");
        }
        lines.push(String(g.header.content ?? ""));
        lines.push("");
      }

      const items: any[] = Array.isArray(g?.items) ? g.items : [];
      for (const item of items) {
        if (!item || item.kind === "spacer") continue;
        if (item.kind === "tool") {
          const title = String(item.title ?? "Tool");
          const status = String(item.status ?? "").trim();
          lines.push(`### Tool: ${title}${status ? ` (${status})` : ""}`);
          lines.push("");
          const kind = String(item.tool_kind ?? "").trim();
          if (kind) {
            lines.push(`**Kind:** \`${kind}\``);
            lines.push("");
          }
          if (item.input != null) {
            lines.push("**Input:**");
            lines.push("");
            lines.push("```json");
            try {
              lines.push(JSON.stringify(item.input, null, 2));
            } catch {
              lines.push(String(item.input));
            }
            lines.push("```");
            lines.push("");
          }
          const out = String(item.output_text ?? "").trim();
          if (out) {
            lines.push("**Output:**");
            lines.push("");
            lines.push("```");
            lines.push(out);
            lines.push("```");
            lines.push("");
          }
          continue;
        }
        if (item.kind === "assistant") {
          lines.push(`### Assistant (${String(item.created_at ?? "").trim() || "unknown time"})`);
          lines.push("");
          lines.push(String(item.content ?? ""));
          lines.push("");
          continue;
        }
      }

      lines.push("---");
      lines.push("");
    }

    try {
      const fileBase = sanitizeFileName(title);
      await saveMarkdownExport(fileBase, lines.join("\n"));
    } catch (e: any) {
      window.alert(e?.message ?? "Failed to export conversation.");
    }
  }, [activeEntry, singleTrackHeader?.title, singleTrackHeader?.worktreePath]);
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

  useEffect(() => {
    const el = newComposerRef.current;
    if (!el) return;
    return registerDropScope({
      element: el,
      onDragOver: () => showDropOverlay(),
      onDrop: (dt) => {
        hideDropOverlay();
        void (async () => {
          const files = extractFilesFromTransfer(dt);
          if (files.length > 0) {
            await onDropFiles(files);
            return;
          }
          const url = extractFirstUrlFromTransfer(dt);
          if (!url) return;
          const asFile = await urlToImageFile(url);
          if (!asFile) return;
          await onDropFiles([asFile]);
        })();
      },
    });
  }, [extractFilesFromTransfer, extractFirstUrlFromTransfer, hideDropOverlay, onDropFiles, showDropOverlay, urlToImageFile]);

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

  const rootStyle = useMemo(() => {
    const max = Math.max(170, window.innerWidth - 240);
    const clamped = Math.min(max, Math.max(170, Math.round(sidebarWidth)));
    return { ["--wb-sidebar-width" as any]: `${clamped}px` } as React.CSSProperties;
  }, [sidebarWidth]);

  return (
    <div
      className={`wb-root ${sidebarCollapsed ? "wb-root-collapsed" : ""} ${sidebarResizing ? "wb-root-resizing" : ""}`}
      style={rootStyle}
    >
      <div className="wb-topbar">
        {sidebarCollapsed && (
          <button
            type="button"
            className="wb-topbar-expand"
            aria-label="Show sidebar"
            title="Show sidebar"
            onClick={() => setSidebarCollapsed(false)}
          >
            ›
          </button>
        )}
        <div className="wb-topbar-title">{workspace?.name ?? "Workspace"}</div>
        {activeTask && <div className="wb-topbar-sub">{activeTask.title}</div>}
        <div className="wb-topbar-right">
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
          <Link className="wb-topbar-icon" to="/settings" title="Settings" aria-label="Settings">
            <Settings size={14} />
          </Link>
        </div>
      </div>

      {uiStateWarning && (
        <div className="banner" style={{ margin: "8px 12px 0" }}>
          {uiStateWarning}
        </div>
      )}

      <div className="wb-sidebar" aria-hidden={sidebarCollapsed}>
        <div className="wb-sidebar-top">
          <div className="wb-sidebar-header">
            <button
              type="button"
              className="wb-new-agent"
              onClick={() => applyWorkbenchSelection({ taskId: null, trackId: null, sessionId: null }, false)}
            >
              New Task
            </button>
            <button
              type="button"
              className="wb-sidebar-collapse"
              aria-label="Collapse sidebar"
              title="Collapse"
              onClick={() => setSidebarCollapsed(true)}
            >
              ‹
            </button>
          </div>

          <input
            className="wb-search"
            placeholder="Search Tasks"
            value={taskQuery}
            onChange={(e) => setTaskQuery(e.target.value)}
          />
        </div>

        <div className="wb-sidebar-section wb-sidebar-grow">
          <div className="wb-task-scroll">
            <div className="wb-section-header">
              <div className="wb-section-title">Active</div>
            </div>

            <div className="wb-task-list" role="list" aria-label="Active tasks">
              {activeTasks.map((t) => {
                const tid = idToString(t.id);
                const selected = tid === activeTaskId;
                const title = t.title ?? "New conversation";
                const working = taskLiveInfo.workingByTask.has(tid);
                const hasError = taskLiveInfo.errorByTask.has(tid);
                const serverLastAssistantMs = parseMs(t.last_assistant_message_at ?? null);
                const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[tid] ?? null;
                const lastAssistantMs =
                  liveLastAssistantMs !== null && serverLastAssistantMs !== null
                    ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
                    : liveLastAssistantMs ?? serverLastAssistantMs;
                const seenMs = parseMs(t.assistant_seen_at ?? null);
                const unread = !working && lastAssistantMs !== null && (seenMs === null || lastAssistantMs > seenMs);
                const age = formatRelativeAgeShort(t.last_activity_at ?? t.updated_at ?? t.created_at) || "Now";
                const dotKind = hasError ? "error" : unread ? "unread" : null;
                const providerIds =
                  (providerIdsByTaskFromSessions[tid] ?? []).length > 0
                    ? providerIdsByTaskFromSessions[tid]
                    : (taskProviderIdsByTaskId[tid] ?? []);
                const providerCount = new Set(providerIds).size;
                const harnesses = providerIds
                  .map((pid) => HARNESS_CATALOG.find((h) => h.id === pid))
                  .filter(Boolean)
                  .slice(0, 3) as Array<(typeof HARNESS_CATALOG)[number]>;

                return (
                  <React.Fragment key={tid}>
                    <div
                      className={`wb-task-row ${selected ? "wb-task-row-active" : ""}`}
                      role="listitem"
                      onClick={() => applyWorkbenchSelection({ taskId: tid, trackId: null, sessionId: null }, false)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        openTaskMenu(tid, { x: e.clientX, y: e.clientY });
                      }}
                      onKeyDown={(e) => {
                        const target = e.target as HTMLElement | null;
                        if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
                          return;
                        }
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          applyWorkbenchSelection({ taskId: tid, trackId: null, sessionId: null }, false);
                        }
                      }}
                      tabIndex={0}
                      title={title}
                    >
                      <div className="wb-task-leading" aria-hidden="true">
                        {/* TODO: multi-track indicator/dropdown (design TBD). */}
                        {providerCount > 1 ? (
                          <LayersPlus className="wb-task-harness-multi" size={16} />
                        ) : harnesses.length > 0 ? (
                          <img
                            className={`wb-task-harness-logo ${harnesses[0].invertInDark ? "wb-invert" : ""}`}
                            src={harnesses[0].logoSrc}
                            alt=""
                          />
                        ) : (
                          <span className="wb-task-harness-fallback" aria-hidden="true" />
                        )}
                      </div>
                      <div className="wb-task-body">
                        {renamingTaskId === tid ? (
                          <input
                            ref={renameInputRef}
                            className="wb-task-rename"
                            value={renameDraft}
                            onChange={(e) => setRenameDraft(e.target.value)}
                            onClick={(e) => e.stopPropagation()}
                            onKeyDown={(e) => {
                              if (e.key === "Escape") {
                                e.preventDefault();
                                e.stopPropagation();
                                cancelRenameTask();
                              }
                              if (e.key === "Enter") {
                                e.preventDefault();
                                e.stopPropagation();
                                void commitRenameTask(tid);
                              }
                            }}
                            onBlur={() => void commitRenameTask(tid)}
                            aria-label="Rename task"
                          />
                        ) : (
                          <div className="wb-task-title">{title}</div>
                        )}
                      </div>
                      <div className="wb-task-meta">
                        <div className="wb-task-meta-status" aria-hidden="true">
                          <div className="wb-task-age">{age}</div>
                          {working && <span className="wb-task-spinner" />}
                          {dotKind === "unread" && <span className="wb-task-status-dot wb-task-status-dot-unread" />}
                          {dotKind === "error" && <span className="wb-task-status-dot wb-task-status-dot-error" />}
                        </div>
                        <div className="wb-task-actions" aria-label="Task actions">
                          <button
                            type="button"
                            className="wb-icon wb-task-action wb-task-menu-trigger"
                            onClick={(e) => {
                              e.stopPropagation();
                              openTaskMenu(tid, { triggerEl: e.currentTarget });
                            }}
                            aria-label="More actions"
                            title="More actions"
                          >
                            <Ellipsis size={14} />
                          </button>
                          <button
                            type="button"
                            className="wb-icon wb-task-action"
                            onClick={(e) => {
                              e.stopPropagation();
                              onToggleArchive(tid, true).catch(() => { });
                            }}
                            aria-label="Archive"
                            title="Archive"
                          >
                            <Archive size={14} />
                          </button>
                        </div>
                      </div>
                    </div>
                  </React.Fragment>
                );
              })}
              {activeTasks.length === 0 && <div className="wb-muted">No active tasks.</div>}
            </div>

            <div className="wb-section-header">
              <button
                type="button"
                className="wb-section-toggle"
                onClick={() => setArchivedCollapsed((v) => !v)}
                aria-expanded={!archivedCollapsed}
                aria-controls="wb-archived-list"
              >
                <span className="wb-section-title">Archived</span>
                <span className={`wb-section-chev ${archivedCollapsed ? "wb-section-chev-collapsed" : ""}`}>
                  <ChevronDown size={14} />
                </span>
              </button>
            </div>

            {!archivedCollapsed && (
              <div
                id="wb-archived-list"
                className="wb-task-list wb-task-list-archived"
                role="list"
                aria-label="Archived tasks"
              >
                {archivedTasks.map((t) => {
                  const tid = idToString(t.id);
                  const selected = tid === activeTaskId;
                  const title = t.title ?? "New conversation";
                  const working = taskLiveInfo.workingByTask.has(tid);
                  const hasError = taskLiveInfo.errorByTask.has(tid);
                  const serverLastAssistantMs = parseMs(t.last_assistant_message_at ?? null);
                  const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[tid] ?? null;
                  const lastAssistantMs =
                    liveLastAssistantMs !== null && serverLastAssistantMs !== null
                      ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
                      : liveLastAssistantMs ?? serverLastAssistantMs;
                  const seenMs = parseMs(t.assistant_seen_at ?? null);
                  const unread = !working && lastAssistantMs !== null && (seenMs === null || lastAssistantMs > seenMs);
                  const age = formatRelativeAgeShort(t.last_activity_at ?? t.updated_at ?? t.created_at) || "Now";
                  const dotKind = hasError ? "error" : unread ? "unread" : null;
                  const providerIds =
                    (providerIdsByTaskFromSessions[tid] ?? []).length > 0
                      ? providerIdsByTaskFromSessions[tid]
                      : (taskProviderIdsByTaskId[tid] ?? []);
                  const providerCount = new Set(providerIds).size;
                  const harnesses = providerIds
                    .map((pid) => HARNESS_CATALOG.find((h) => h.id === pid))
                    .filter(Boolean)
                    .slice(0, 3) as Array<(typeof HARNESS_CATALOG)[number]>;

                  return (
                    <React.Fragment key={tid}>
                      <div
                        className={`wb-task-row wb-task-row-archived ${selected ? "wb-task-row-active" : ""}`}
                      role="listitem"
                      onClick={() => applyWorkbenchSelection({ taskId: tid, trackId: null, sessionId: null }, false)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        openTaskMenu(tid, { x: e.clientX, y: e.clientY });
                      }}
                      onKeyDown={(e) => {
                          const target = e.target as HTMLElement | null;
                          if (
                            target &&
                            (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
                          ) {
                            return;
                        }
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          applyWorkbenchSelection({ taskId: tid, trackId: null, sessionId: null }, false);
                        }
                      }}
                      tabIndex={0}
                      title={title}
                    >
                        <div className="wb-task-leading" aria-hidden="true">
                          {/* TODO: multi-track indicator/dropdown (design TBD). */}
                          {providerCount > 1 ? (
                            <LayersPlus className="wb-task-harness-multi" size={16} />
                          ) : harnesses.length > 0 ? (
                            <img
                              className={`wb-task-harness-logo ${harnesses[0].invertInDark ? "wb-invert" : ""}`}
                              src={harnesses[0].logoSrc}
                              alt=""
                            />
                          ) : (
                            <span className="wb-task-harness-fallback" aria-hidden="true" />
                          )}
                        </div>
                        <div className="wb-task-body">
                          {renamingTaskId === tid ? (
                            <input
                              ref={renameInputRef}
                              className="wb-task-rename"
                              value={renameDraft}
                              onChange={(e) => setRenameDraft(e.target.value)}
                              onClick={(e) => e.stopPropagation()}
                              onKeyDown={(e) => {
                                if (e.key === "Escape") {
                                  e.preventDefault();
                                  e.stopPropagation();
                                  cancelRenameTask();
                                }
                                if (e.key === "Enter") {
                                  e.preventDefault();
                                  e.stopPropagation();
                                  void commitRenameTask(tid);
                                }
                              }}
                              onBlur={() => void commitRenameTask(tid)}
                              aria-label="Rename task"
                            />
                          ) : (
                            <div className="wb-task-title">{title}</div>
                          )}
                        </div>
                        <div className="wb-task-meta">
                          <div className="wb-task-meta-status" aria-hidden="true">
                            <div className="wb-task-age">{age}</div>
                            {working && <span className="wb-task-spinner" />}
                            {dotKind === "unread" && <span className="wb-task-status-dot wb-task-status-dot-unread" />}
                            {dotKind === "error" && <span className="wb-task-status-dot wb-task-status-dot-error" />}
                          </div>
                          <div className="wb-task-actions" aria-label="Task actions">
                            <button
                              type="button"
                              className="wb-icon wb-task-action wb-task-menu-trigger"
                              onClick={(e) => {
                                e.stopPropagation();
                                openTaskMenu(tid, { triggerEl: e.currentTarget });
                              }}
                              aria-label="More actions"
                              title="More actions"
                            >
                              <Ellipsis size={14} />
                            </button>
                            <button
                              type="button"
                              className="wb-icon wb-task-action"
                              onClick={(e) => {
                                e.stopPropagation();
                                onToggleArchive(tid, false).catch(() => { });
                              }}
                              aria-label="Unarchive"
                              title="Unarchive"
                            >
                              <Archive size={14} />
                            </button>
                          </div>
                        </div>
                      </div>
                    </React.Fragment>
                  );
                })}
                {archivedTasks.length === 0 && <div className="wb-muted">No archived tasks.</div>}
              </div>
            )}
          </div>
        </div>

        <div className="wb-sidebar-bottom">
          <Link className="wb-link" to="/providers" title="Providers">
            Providers
          </Link>
          <Link className="wb-link" to="/diagnostics" title="Diagnostics">
            Diagnostics
          </Link>
        </div>
      </div>

      {!sidebarCollapsed && (
        <div
          className="wb-sidebar-resizer"
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize sidebar"
          onMouseDown={onSidebarResizerMouseDown}
          onDoubleClick={() => setSidebarWidth(260)}
        />
      )}

      <div className="wb-main">
        {!activeTaskId ? (
          <div className="wb-center">
            <div className="wb-new-composer-stack ctx-drop-scope" ref={newComposerRef}>
              {dropActive && (
                <div className="ctx-drop-overlay" aria-hidden="true">
                  <div className="ctx-drop-overlay-text">Drop image to attach</div>
                </div>
              )}
              <WorkbenchComposer
                variant="newSession"
                value={draftPrompt}
                setValue={setDraftPrompt}
                placeholder="@ for context, / for commands"
                inputDisabled={dictationRecording}
                recording={dictationRecording}
                onToggleRecording={() => {
                  if (dictationRecording) stopDictation().catch(() => { });
                  else startDictation().catch(() => { });
                }}
                sessionIdForAutocomplete={null}
                workspaceIdForAutocomplete={workspaceId}
                slashCommands={slashCommands}
                attachments={draftAttachments}
                setAttachments={setDraftAttachments}
                onSend={startNewTask}
                sendDisabled={!!startBlockedReason}
                sendDisabledReason={startBlockedReason}
                onInterrupt={null}
                modeId={draftMode}
                setModeId={setDraftMode}
                harnessCatalog={HARNESS_CATALOG}
                providersById={providersById}
                providerInstallsById={providerInstallsById}
                onInstallProvider={installProviderFromMenu}
                onInstallAllProviders={installAllProvidersFromMenu}
                installAllBusy={installAllBusy}
                providerOptions={providerOptions}
                ensureProviderOptions={ensureProviderOptions}
                draftTracks={draftTracks}
                setDraftTracks={setDraftTracks}
                defaultProviderId={defaultProviderId}
                useMultipleAgents={useMultipleAgents}
                setUseMultipleAgents={setUseMultipleAgents}
                envTarget={execTarget}
                setEnvTarget={setExecTarget}
              />

              {false && (
                <>
                  <div className="wb-composer-card wb-new-composer-card">
                    <textarea
                      className="wb-composer-textarea"
                      placeholder="@ for context, / for command"
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
                            ref={modeTriggerRef}
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
                            <ChevronDown size={14} />
                          </button>
                          {openMenu === "mode" && (
                            <div className="wb-menu" role="menu" ref={activeMenuRef} style={menuStyle ?? undefined}>
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
                            ref={harnessTriggerRef}
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
                            <ChevronDown size={14} />
                          </button>

                          {openMenu === "harness" && (
                            <div
                              className="wb-menu wb-harness-menu"
                              role="menu"
                              ref={activeMenuRef}
                              style={menuStyle ?? undefined}
                            >
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
                                const all = HARNESS_CATALOG;
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
                                  const installSupported = providersById[id]?.details?.install_supported === "true";
                                  const installRunning = providersById[id]?.details?.install_running === "true";
                                  const installBusy = providerInstallsById[id]?.state === "running";
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

                                      <div className="wb-harness-actions">
                                        {!installed ? (
                                          installSupported ? (
                                            <button
                                              type="button"
                                              className="wb-harness-install"
                                              onClick={(e) => {
                                                e.stopPropagation();
                                                installProviderFromMenu(id);
                                              }}
                                              disabled={installRunning || installBusy}
                                              title="Install this harness"
                                            >
                                              {installRunning || installBusy ? "Installing…" : "Install"}
                                            </button>
                                          ) : (
                                            <button
                                              type="button"
                                              className="wb-harness-install"
                                              disabled
                                              title="Install not supported yet"
                                            >
                                              Install
                                            </button>
                                          )
                                        ) : (
                                          checked && (
                                            <button
                                              type="button"
                                              className="wb-harness-expand wb-menu-trigger"
                                              onClick={(e) => {
                                                e.stopPropagation();
                                                if (!canConfigureModels) return;
                                                setExpandedHarnessId((prev) => (prev === id ? null : id));
                                                ensureProviderOptions(id).catch(() => { });
                                              }}
                                              disabled={!canConfigureModels}
                                              title={canConfigureModels ? "Configure models" : "Enable multi-agent to configure"}
                                            >
                                              <ChevronDown size={14} />
                                            </button>
                                          )
                                        )}
                                      </div>

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
                                                    onFocus={() => ensureProviderOptions(id).catch(() => { })}
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
                                                    onFocus={() => ensureProviderOptions(id).catch(() => { })}
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
                              ref={modelTriggerRef}
                              onClick={() => {
                                setContextMenuOpen(false);
                                setExpandedHarnessId(null);
                                setModelSearch("");
                                setOpenMenu((v) => (v === "model" ? null : "model"));
                                ensureProviderOptions(primaryHarnessId).catch(() => { });
                              }}
                              aria-haspopup="menu"
                              aria-expanded={openMenu === "model"}
                              title="Model"
                            >
                              <span className="wb-switcher-label">
                                {(primaryTrack?.modelId ?? "").trim() || "Model"}
                              </span>
                              <ChevronDown size={14} />
                            </button>
                            {openMenu === "model" && (
                              <div
                                className="wb-menu wb-model-menu"
                                role="menu"
                                ref={activeMenuRef}
                                style={menuStyle ?? undefined}
                              >
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

                        <div className="wb-switcher-wrap">
                          <button
                            type="button"
                            className="wb-switcher wb-menu-trigger"
                            ref={execTriggerRef}
                            onClick={() => {
                              setContextMenuOpen(false);
                              setExpandedHarnessId(null);
                              setOpenMenu((v) => (v === "exec" ? null : "exec"));
                            }}
                            aria-haspopup="menu"
                            aria-expanded={openMenu === "exec"}
                            title="Execution target"
                          >
                            <span className="wb-switcher-icon">
                              <Laptop size={14} />
                            </span>
                            <span className="wb-switcher-label">
                              {execTarget === "worktree" ? "Worktree" : execTarget === "local" ? "Local" : "Container"}
                            </span>
                            <ChevronDown size={14} />
                          </button>
                          {openMenu === "exec" && (
                            <div
                              className="wb-menu wb-exec-menu"
                              role="menu"
                              ref={activeMenuRef}
                              style={menuStyle ?? undefined}
                            >
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
                                Container (coming soon)
                              </button>
                            </div>
                          )}
                        </div>
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
                          <AtSign size={14} />
                        </button>
                        <button
                          type="button"
                          className="wb-icon"
                          title="Attach image (coming soon)"
                          disabled
                          aria-label="Attach image"
                        >
                          <Image size={14} />
                        </button>
                        <button
                          type="button"
                          className="wb-icon"
                          title="Record (coming soon)"
                          disabled
                          aria-label="Record"
                        >
                          <Mic size={14} />
                        </button>
                        <button
                          type="button"
                          className="wb-send"
                          onClick={startNewTask}
                          disabled={!!startBlockedReason}
                          title={startBlockedReason ?? "Start"}
                          aria-label="Start"
                        >
                          <ArrowUp size={14} />
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

                </>
              )}
              {dictationDebugText && <div className="wb-banner">{dictationDebugText}</div>}
              {dictationError && <div className="wb-banner">{dictationError}</div>}
              {startError && <div className="wb-banner">{startError}</div>}
            </div>
          </div>
        ) : null}

        {activeTaskId && (
          <div className="wb-body">
            <div className="wb-convo">
              {tracks.length === 1 && singleTrackHeader ? (
                <div className="wb-single-track-header">
                  <div className="wb-single-track-title">{singleTrackHeader.title}</div>
                  <div className="wb-single-track-meta">
                    <div className="wb-single-track-meta-left">
                      <span>{singleTrackHeader.age}</span>
                      <span className="wb-single-track-dot" aria-hidden="true">
                        ·
                      </span>
                      <span>{singleTrackHeader.harness}</span>
                      {singleTrackHeader.modelBase && (
                        <>
                          <span className="wb-single-track-dot" aria-hidden="true">
                            ·
                          </span>
                          <span>{singleTrackHeader.modelBase}</span>
                        </>
                      )}
                      {singleTrackHeader.effort && (
                        <>
                          <span className="wb-single-track-dot" aria-hidden="true">
                            ·
                          </span>
                          <span>{singleTrackHeader.effort}</span>
                        </>
                      )}
                      {singleTrackHeader.worktreeSlug && (
                        <>
                          <span className="wb-single-track-dot" aria-hidden="true">
                            ·
                          </span>
                          <button
                            type="button"
                            className={`wb-worktree-chip ${worktreeCopied ? "wb-worktree-chip-copied" : ""}`}
                            disabled={!singleTrackHeader.canCopyWorktree}
                            onClick={() => void copyWorktreeLocation()}
                            title="Copy worktree location"
                            aria-label="Copy worktree location"
                          >
                            <GitBranch size={13} />
                            <span className="wb-worktree-chip-slug">{singleTrackHeader.worktreeSlug}</span>
                            <span className="wb-worktree-chip-copy" aria-hidden="true">
                              {worktreeCopied ? <Check size={13} /> : <Copy size={13} />}
                            </span>
                          </button>
                          {worktreeCopied && (
                            <span className="sr-only" aria-live="polite">
                              Copied worktree location to clipboard.
                            </span>
                          )}
                        </>
                      )}
                    </div>
                    <button
                      type="button"
                      className="wb-icon wb-convo-menu-trigger"
                      aria-label="Conversation options"
                      title="Conversation options"
                      onClick={(e) => openConvoMenu(e.currentTarget)}
                    >
                      <Ellipsis size={14} />
                    </button>
                  </div>
                </div>
              ) : (
                <div className="wb-trackbar">
                  {tracks.map((tr) => {
                    const trid = idToString(tr.id);
                    const selected = trid === activeTrackId;
                    const sessions = sessionsByTrack[trid] ?? [];
                    const s = pickPreferredSession(sessions) as any;
                    const sessionId = s ? idToString((s as any).id) : "";
                    const liveSession = sessionId ? sessionCache.sessions[sessionId]?.session : null;
                    const displaySession = (liveSession ?? s) as any;
                    const model = displaySession ? `${displaySession.provider_id} ${displaySession.model_id}` : "No session";
                    const status = tr.status === "running" ? "Running…" : tr.status === "completed" ? "Task completed" : tr.status;
                    return (
                      <button
                        key={trid}
                        type="button"
                        className={`wb-trackcard ${selected ? "wb-trackcard-active" : ""}`}
                        onClick={() =>
                          activeTaskId && applyWorkbenchSelection({ taskId: activeTaskId, trackId: trid, sessionId: null }, false)
                        }
                      >
                        <div className="wb-trackcard-title">{model}</div>
                        <div className="wb-trackcard-sub">{status}</div>
                      </button>
                    );
                  })}
                </div>
              )}

              <div className="wb-session">
                {activeSessionId ? (
                  <SessionView key={activeSessionId ?? "empty"} sessionId={activeSessionId} variant="workbench" showDiffPane={false} />
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

      {taskMenu && (
        <div className="wb-menu wb-task-menu" role="menu" ref={taskMenuRef} style={taskMenu.style}>
          <button
            type="button"
            className="wb-menu-item"
            onClick={() => {
              const tid = taskMenu.taskId;
              setTaskMenu(null);
              beginRenameTask(tid);
            }}
            role="menuitem"
          >
            Rename Task
          </button>
          <button
            type="button"
            className="wb-menu-item"
            onClick={() => {
              const tid = taskMenu.taskId;
              const t = tasks.find((x) => idToString(x.id) === tid);
              const nextArchived = !t?.archived_at;
              onToggleArchive(tid, nextArchived).catch(() => { });
              setTaskMenu(null);
            }}
            role="menuitem"
          >
            {(() => {
              const t = tasks.find((x) => idToString(x.id) === taskMenu.taskId);
              return t?.archived_at ? "Unarchive" : "Archive";
            })()}
          </button>
          <button
            type="button"
            className="wb-menu-item"
            disabled={(() => {
              const tid = taskMenu.taskId;
              const t = tasks.find((x) => idToString(x.id) === tid);
              return !t?.last_assistant_message_at;
            })()}
            onClick={() => {
              const tid = taskMenu.taskId;
              const t = tasks.find((x) => idToString(x.id) === tid);
              const serverLastAssistantMs = parseMs(t?.last_assistant_message_at ?? null);
              const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[tid] ?? null;
              const lastAssistantMs =
                liveLastAssistantMs !== null && serverLastAssistantMs !== null
                  ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
                  : liveLastAssistantMs ?? serverLastAssistantMs;
              const seenMs = parseMs(t?.assistant_seen_at ?? null);
              const unread =
                lastAssistantMs !== null && (seenMs === null || lastAssistantMs > seenMs);
              setTaskMenu(null);
              if (unread) markTaskRead(tid);
              else markTaskUnread(tid);
            }}
            role="menuitem"
          >
            {(() => {
              const tid = taskMenu.taskId;
              const t = tasks.find((x) => idToString(x.id) === tid);
              const serverLastAssistantMs = parseMs(t?.last_assistant_message_at ?? null);
              const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[tid] ?? null;
              const lastAssistantMs =
                liveLastAssistantMs !== null && serverLastAssistantMs !== null
                  ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
                  : liveLastAssistantMs ?? serverLastAssistantMs;
              const seenMs = parseMs(t?.assistant_seen_at ?? null);
              const unread =
                lastAssistantMs !== null && (seenMs === null || lastAssistantMs > seenMs);
              return unread ? "Mark as Read" : "Mark as Unread";
            })()}
          </button>
          <button
            type="button"
            className="wb-menu-item wb-menu-item-danger"
            onClick={() => {
              const tid = taskMenu.taskId;
              setTaskMenu(null);
              void onDeleteTask(tid);
            }}
            role="menuitem"
          >
            Delete Task
          </button>
        </div>
      )}

      {convoMenu && (
        <div className="wb-menu wb-convo-menu" role="menu" ref={convoMenuRef} style={convoMenu.style}>
          <button
            type="button"
            className="wb-menu-item"
            disabled={!activeSessionId}
            onClick={() => {
              setConvoMenu(null);
              void exportConversation();
            }}
            role="menuitem"
          >
            Export Conversation
          </button>
          <button
            type="button"
            className="wb-menu-item"
            disabled={!singleTrackHeader?.canCopyWorktree}
            onClick={() => {
              setConvoMenu(null);
              void copyWorktreeLocation();
            }}
            role="menuitem"
          >
            Copy Worktree Location
          </button>
          <button
            type="button"
            className="wb-menu-item"
            disabled={!activeTaskId || !!activeTask?.archived_at}
            onClick={() => {
              if (!activeTaskId) return;
              setConvoMenu(null);
              onToggleArchive(activeTaskId, true).catch(() => { });
            }}
            role="menuitem"
          >
            Archive Conversation
          </button>
        </div>
      )}
    </div>
  );
}
