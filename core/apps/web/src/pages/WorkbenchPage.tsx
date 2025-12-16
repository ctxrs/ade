import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation, useParams } from "react-router-dom";
import {
  DictationSettings,
  EditPlanSummary,
  LspStatus,
  MessageAttachment,
  ProviderOptions,
  ProviderStatus,
  Task,
  Track,
  Workspace,
  archiveTask,
  applyTrackDiffPatch,
  discardEditPlan,
  createSession,
  createTask,
  createTrack,
  getDaemonBaseUrl,
  getLspStatus,
  getProviderOptions,
  getSettings,
  getWorkspace,
  idToString,
  listProviders,
  listEditPlansForTrack,
  listSessionsForTrack,
  listTasks,
  listTracks,
  postMessage,
  trackDiff,
  unarchiveTask,
} from "../api/client";
import { useSessionCacheSnapshot, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { DiffReviewPane } from "../components/DiffReviewPane";
import { EditPlanReviewPane } from "../components/EditPlanReviewPane";
import { SessionView } from "./SessionPage";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import { WorkbenchComposer, type DraftTrack, type WorkbenchEnvTarget, type WorkbenchModeId } from "../components/WorkbenchComposer";
import type { SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import { IconArchive, IconArrowUp, IconAt, IconChat, IconChevronDown, IconDots, IconGear, IconImage, IconLaptop, IconMic } from "../components/workbenchIcons";
import { startMicPcmStream } from "../utils/micPcmStream";
import { parseWsJson } from "../utils/wsJson";

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

function taskActivityMs(t: Task): number | null {
  return parseMs(t.last_activity_at ?? null) ?? parseMs(t.updated_at ?? null) ?? parseMs(t.created_at ?? null);
}

function formatAgeShort(ms: number | null): string {
  if (ms === null) return "";
  const diffMs = Math.max(0, Date.now() - ms);
  const diffMin = Math.floor(diffMs / 60000);
  if (diffMin < 1) return "now";
  if (diffMin < 60) return `${diffMin}m`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h`;
  const diffDay = Math.floor(diffHr / 24);
  if (diffDay < 14) return `${diffDay}d`;
  return new Date(ms).toLocaleDateString([], { month: "short", day: "numeric" });
}

function lastAssistantMessageMs(messages: { role: string; created_at: string }[]): number | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role === "assistant") return parseMs(m.created_at);
  }
  return null;
}

export default function WorkbenchPage() {
  const { id: workspaceId } = useParams<{ id: string }>();
  const location = useLocation();
  const supervisor = useSessionSupervisor();
  const sessionSnap = useSessionCacheSnapshot();
  const newComposerRef = useRef<HTMLDivElement | null>(null);

  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions | undefined>>({});
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
  const [archivedCollapsed, setArchivedCollapsed] = useState(true);
  const [taskSeenAssistantAtById, setTaskSeenAssistantAtById] = useState<Record<string, string>>({});
  const [taskMenu, setTaskMenu] = useState<{ taskId: string; style: React.CSSProperties } | null>(null);
  const taskMenuRef = useRef<HTMLDivElement | null>(null);

  const [tracks, setTracks] = useState<Track[]>([]);
  const [sessionsByTrack, setSessionsByTrack] = useState<Record<string, any[]>>({});
  const [activeTrackId, setActiveTrackId] = useState<string | null>(null);

  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
    { key: "t1", label: "", providerId: "codex", modelId: "" },
  ]);
  const [draftMode, setDraftMode] = useState<WorkbenchModeId>("default");
  const [execTarget, setExecTarget] = useState<WorkbenchEnvTarget>("worktree");
  const [startBusy, setStartBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const [useMultipleAgents, setUseMultipleAgents] = useState(false);
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);

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
    const key = `wb.archivedCollapsed.${workspaceId}`;
    const v = localStorage.getItem(key);
    if (v === "0") setArchivedCollapsed(false);
    else setArchivedCollapsed(true);
  }, [workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    localStorage.setItem(`wb.archivedCollapsed.${workspaceId}`, archivedCollapsed ? "1" : "0");
  }, [archivedCollapsed, workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    const key = `wb.taskSeenAssistantAtById.${workspaceId}`;
    try {
      const parsed = JSON.parse(localStorage.getItem(key) ?? "{}");
      if (parsed && typeof parsed === "object") setTaskSeenAssistantAtById(parsed);
      else setTaskSeenAssistantAtById({});
    } catch {
      setTaskSeenAssistantAtById({});
    }
  }, [workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    localStorage.setItem(`wb.taskSeenAssistantAtById.${workspaceId}`, JSON.stringify(taskSeenAssistantAtById));
  }, [taskSeenAssistantAtById, workspaceId]);

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
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (cancelled) return;
        setDictationSettings(s.dictation ?? null);
      })
      .catch(() => {});
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

  const taskLiveInfo = useMemo(() => {
    const workingByTask = new Set<string>();
    const lastAssistantMsByTask: Record<string, number> = {};
    for (const entry of Object.values(sessionSnap.sessions)) {
      const taskId = entry.session ? idToString(entry.session.task_id) : "";
      if (!taskId) continue;
      if (entry.session?.status === "active") workingByTask.add(taskId);
      const ms = lastAssistantMessageMs(entry.messages);
      if (ms !== null) lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, ms);
    }
    return { workingByTask, lastAssistantMsByTask };
  }, [sessionSnap.sessions]);

  const activeTasks = useMemo(() => filteredTasks.filter((t) => !t.archived_at), [filteredTasks]);
  const archivedTasks = useMemo(() => filteredTasks.filter((t) => !!t.archived_at), [filteredTasks]);

  const markTaskSeen = useCallback(
    (taskId: string) => {
      const t = tasks.find((x) => idToString(x.id) === taskId);
      const last = t?.last_assistant_message_at ?? null;
      if (!last) return;
      setTaskSeenAssistantAtById((prev) => ({ ...prev, [taskId]: last }));
    },
    [tasks],
  );

  useEffect(() => {
    if (!activeTaskId) return;
    markTaskSeen(activeTaskId);
  }, [activeTaskId, markTaskSeen]);

  const onToggleArchive = useCallback(
    async (taskId: string, nextArchived: boolean) => {
      const updated = nextArchived ? await archiveTask(taskId) : await unarchiveTask(taskId);
      setTasks((prev) => prev.map((t) => (idToString(t.id) === taskId ? { ...t, ...updated } : t)));
      if (nextArchived && activeTaskId === taskId) setArchivedCollapsed(false);
    },
    [activeTaskId],
  );

  const openTaskMenu = useCallback((taskId: string, triggerEl: HTMLElement) => {
    const rect = triggerEl.getBoundingClientRect();
    const left = Math.min(rect.left, window.innerWidth - 240);
    const top = Math.min(rect.bottom + 6, window.innerHeight - 220);
    setTaskMenu((prev) => (prev?.taskId === taskId ? null : { taskId, style: { left, top } }));
  }, []);

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

  const sessionCache = useSessionCacheSnapshot();
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
    } catch {}

    try {
      ws?.send(JSON.stringify({ type: "stop" }));
    } catch {}

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
          } catch {}
          return;
        } else if (t === "error") {
          setDictationError(String(data.message ?? "Dictation error"));
          stopDictation().catch(() => {});
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
          stopDictation().catch(() => {});
        },
      });
    } catch (e: any) {
      setDictationError(e?.message ?? String(e));
      try {
        ws.close();
      } catch {}
      dictationWsRef.current = null;
      setDictationRecording(false);
    }
  }, [dictationSettings, dictationRecording, draftPrompt, stopDictation]);

  useEffect(() => {
    return () => {
      stopDictation().catch(() => {});
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
        supervisor.openSession(sessionId, { watchDiff: true });
        supervisor.refreshSession(sessionId, { watchDiff: true });
        supervisor.refreshQueue(sessionId);
        await postMessage(sessionId, prompt, "immediate", draftAttachments);
        // Ensure the workbench view can render the just-posted user message (and any streamed events)
        // without waiting for a `done` event to trigger a refresh.
        supervisor.refreshQueue(sessionId);
        supervisor.refreshSession(sessionId, { watchDiff: true });
      }

      await refreshTasks();
      setActiveTaskId(taskId);
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
      <div className="wb-sidebar" aria-hidden={sidebarCollapsed}>
        <div className="wb-sidebar-top">
          <div className="wb-sidebar-header">
            <button type="button" className="wb-new-agent" onClick={() => setActiveTaskId(null)}>
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
                const working = taskLiveInfo.workingByTask.has(tid) || t.has_active_session === true;
                const serverLastAssistantMs = parseMs(t.last_assistant_message_at ?? null);
                const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[tid] ?? null;
                const lastAssistantMs =
                  liveLastAssistantMs !== null && serverLastAssistantMs !== null
                    ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
                    : liveLastAssistantMs ?? serverLastAssistantMs;
                const seenMs = parseMs(taskSeenAssistantAtById[tid] ?? null);
                const unread = !working && lastAssistantMs !== null && (seenMs === null || lastAssistantMs > seenMs);
                const age = formatAgeShort(taskActivityMs(t));
                return (
                  <div
                    key={tid}
                    className={`wb-task-row ${selected ? "wb-task-row-active" : ""}`}
                    role="listitem"
                    onClick={() => setActiveTaskId(tid)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        setActiveTaskId(tid);
                      }
                    }}
                    tabIndex={0}
                    title={title}
                  >
                    <div className="wb-task-leading" aria-hidden="true">
                      {working ? <span className="wb-task-spinner" /> : <IconChat size={14} />}
                      {unread && <span className="wb-task-unread" />}
                    </div>
                    <div className="wb-task-body">
                      <div className="wb-task-title">{title}</div>
                    </div>
                    <div className="wb-task-meta">
                      {age && <div className="wb-task-age">{age}</div>}
                      <div className="wb-task-actions" aria-label="Task actions">
                        <button
                          type="button"
                          className="wb-icon wb-task-action"
                          onClick={(e) => {
                            e.stopPropagation();
                            onToggleArchive(tid, true).catch(() => {});
                          }}
                          aria-label="Archive"
                          title="Archive"
                        >
                          <IconArchive size={14} />
                        </button>
                        <button
                          type="button"
                          className="wb-icon wb-task-action wb-task-menu-trigger"
                          onClick={(e) => {
                            e.stopPropagation();
                            openTaskMenu(tid, e.currentTarget);
                          }}
                          aria-label="More actions"
                          title="More actions"
                        >
                          <IconDots size={14} />
                        </button>
                      </div>
                    </div>
                  </div>
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
                  <IconChevronDown size={14} />
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
                  const working = taskLiveInfo.workingByTask.has(tid) || t.has_active_session === true;
                  const serverLastAssistantMs = parseMs(t.last_assistant_message_at ?? null);
                  const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[tid] ?? null;
                  const lastAssistantMs =
                    liveLastAssistantMs !== null && serverLastAssistantMs !== null
                      ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
                      : liveLastAssistantMs ?? serverLastAssistantMs;
                  const seenMs = parseMs(taskSeenAssistantAtById[tid] ?? null);
                  const unread = !working && lastAssistantMs !== null && (seenMs === null || lastAssistantMs > seenMs);
                  const age = formatAgeShort(taskActivityMs(t));
                  return (
                    <div
                      key={tid}
                      className={`wb-task-row wb-task-row-archived ${selected ? "wb-task-row-active" : ""}`}
                      role="listitem"
                      onClick={() => setActiveTaskId(tid)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          setActiveTaskId(tid);
                        }
                      }}
                      tabIndex={0}
                      title={title}
                    >
                      <div className="wb-task-leading" aria-hidden="true">
                        {working ? <span className="wb-task-spinner" /> : <IconChat size={14} />}
                        {unread && <span className="wb-task-unread" />}
                      </div>
                      <div className="wb-task-body">
                        <div className="wb-task-title">{title}</div>
                      </div>
                      <div className="wb-task-meta">
                        {age && <div className="wb-task-age">{age}</div>}
                        <div className="wb-task-actions" aria-label="Task actions">
                          <button
                            type="button"
                            className="wb-icon wb-task-action"
                            onClick={(e) => {
                              e.stopPropagation();
                              onToggleArchive(tid, false).catch(() => {});
                            }}
                            aria-label="Unarchive"
                            title="Unarchive"
                          >
                            <IconArchive size={14} />
                          </button>
                          <button
                            type="button"
                            className="wb-icon wb-task-action wb-task-menu-trigger"
                            onClick={(e) => {
                              e.stopPropagation();
                              openTaskMenu(tid, e.currentTarget);
                            }}
                            aria-label="More actions"
                            title="More actions"
                          >
                            <IconDots size={14} />
                          </button>
                        </div>
                      </div>
                    </div>
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

      <div className="wb-main">
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
              <IconGear size={14} />
            </Link>
          </div>
        </div>

        {!activeTaskId ? (
          <div className="wb-center">
            <div className="wb-new-composer-stack" ref={newComposerRef}>
              <WorkbenchComposer
                variant="newSession"
                value={draftPrompt}
                setValue={setDraftPrompt}
                placeholder="Plan, @ for context, / for commands"
                inputDisabled={dictationRecording}
                recording={dictationRecording}
                onToggleRecording={() => {
                  if (dictationRecording) stopDictation().catch(() => {});
                  else startDictation().catch(() => {});
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
                        <IconChevronDown size={14} />
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
                        <IconChevronDown size={14} />
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
                          ref={modelTriggerRef}
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
                          <span className="wb-switcher-label">
                            {(primaryTrack?.modelId ?? "").trim() || "Model"}
                          </span>
                          <IconChevronDown size={14} />
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
                          <IconLaptop size={14} />
                        </span>
                        <span className="wb-switcher-label">
                          {execTarget === "worktree" ? "Worktree" : execTarget === "local" ? "Local" : "Container"}
                        </span>
                        <IconChevronDown size={14} />
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
                            Container (soon)
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
              <div className="wb-trackbar">
                {tracks.map((tr) => {
                  const trid = idToString(tr.id);
                  const selected = trid === activeTrackId;
                  const sessions = sessionsByTrack[trid] ?? [];
                  const s = sessions[0] as any;
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

      {taskMenu && (
        <div className="wb-menu wb-task-menu" role="menu" ref={taskMenuRef} style={taskMenu.style}>
          <button
            type="button"
            className="wb-menu-item"
            onClick={() => {
              const tid = taskMenu.taskId;
              const t = tasks.find((x) => idToString(x.id) === tid);
              const nextArchived = !t?.archived_at;
              onToggleArchive(tid, nextArchived).catch(() => {});
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
            onClick={() => {
              navigator.clipboard.writeText(taskMenu.taskId).catch(() => {});
              setTaskMenu(null);
            }}
            role="menuitem"
          >
            Copy task ID
          </button>
        </div>
      )}
    </div>
  );
}
