import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Virtuoso } from "react-virtuoso";
import { Link, useParams } from "react-router-dom";
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
  InstallInfo,
  MessageAttachment,
  ProviderOptions,
  ProviderStatus,
  Workspace,
  archiveTask,
  applyTrackDiffPatch,
  createSession,
  createTask,
  createTrack,
  deleteTask,
  getDaemonBaseUrl,
  getInstall,
  getProviderOptions,
  getWorktree,
  getWorkspace,
  idToString,
  installAllProviders,
  installProvider,
  listProviders,
  markTaskRead as markTaskReadApi,
  markTaskUnread as markTaskUnreadApi,
  postMessage,
  trackDiff,
  unarchiveTask,
  updateTaskTitle,
  verifyProviderForWorkspace,
} from "../api/client";
import { useSessionCacheSnapshot, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { useSettingsSnapshot } from "../state/settingsStore";
import { loadWorktreeV1, saveWorktreeV1 } from "../state/uiStateStore";
import { DiffReviewPane } from "../components/DiffReviewPane";
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
  NEW_TASK_DRAFT_KEY,
  WorkbenchStoreProvider,
  scrollKey,
  sessionDraftKey,
  useActiveWorkbenchIds,
  useActiveWorkbenchTab,
  useNewTaskDraft,
  useWorkbenchDraft,
  useWorkbenchSnapshot,
  useWorkbenchStore,
} from "../workbench/store";
import {
  WorkspaceCatchupProvider,
  useWorkspaceCatchupSnapshot,
  useWorkspaceCatchupStore,
  type WorkspaceCatchupItem,
} from "../state/workspaceCatchupStore";

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
  if (!workspaceId) return null;
  return (
    <WorkspaceCatchupProvider workspaceId={workspaceId}>
      <WorkbenchStoreProvider workspaceId={workspaceId}>
        <WorkbenchPageInner workspaceId={workspaceId} />
      </WorkbenchStoreProvider>
    </WorkspaceCatchupProvider>
  );
}

function WorkbenchPageInner({ workspaceId }: { workspaceId: string }) {
  const supervisor = useSessionSupervisor();
  const sessionSnap = useSessionCacheSnapshot();
  const workbenchStore = useWorkbenchStore();
  const workspaceCatchupStore = useWorkspaceCatchupStore();
  const workspaceCatchup = useWorkspaceCatchupSnapshot();
  const tasksById = workspaceCatchup.tasksById;
  const workbenchSnap = useWorkbenchSnapshot();
  const activeTab = useActiveWorkbenchTab();
  const { taskId: activeTaskId, trackId: activeTrackId } = useActiveWorkbenchIds();
  const { value: newTaskDraft, setValue: setNewTaskDraft } = useNewTaskDraft();
  const draftPrompt = newTaskDraft.text;
  const draftMode = newTaskDraft.modeId;

  useEffect(() => {
    supervisor.bindWorkspaceCatchupStore(workspaceCatchupStore);
    return () => supervisor.bindWorkspaceCatchupStore(null);
  }, [supervisor, workspaceCatchupStore]);
  const setDraftPrompt = useCallback(
    (text: string) => setNewTaskDraft({ text, modeId: newTaskDraft.modeId }),
    [newTaskDraft.modeId, setNewTaskDraft],
  );
  const setDraftMode = useCallback(
    (modeId: WorkbenchModeId) => setNewTaskDraft({ text: newTaskDraft.text, modeId }),
    [newTaskDraft.text, setNewTaskDraft],
  );
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
  const postInstallHandledRef = useRef<Set<string>>(new Set());
  const postInstallInFlightRef = useRef<Set<string>>(new Set());
  const providersById = useMemo(
    () => Object.fromEntries(providers.map((p) => [p.provider_id, p])),
    [providers],
  );
  const defaultProviderId = useMemo(() => {
    const installed = providers
      .filter((p) => p.installed && p.health === "ok" && p.details?.ui_hidden !== "true")
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

  const [taskQuery, setTaskQuery] = useState("");
  const [archivedCollapsed, setArchivedCollapsed] = useState(true);
  const [taskMenu, setTaskMenu] = useState<{ taskId: string; style: React.CSSProperties } | null>(null);
  const taskMenuRef = useRef<HTMLDivElement | null>(null);
  const [renamingTaskId, setRenamingTaskId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const renameInputRef = useRef<HTMLInputElement | null>(null);
  const [convoMenu, setConvoMenu] = useState<{ style: React.CSSProperties } | null>(null);
  const convoMenuRef = useRef<HTMLDivElement | null>(null);


  const focusNewTask = useCallback(() => {
    workbenchStore.focusNewTask();
  }, [workbenchStore]);

  const focusTask = useCallback(
    (taskId: string, trackId?: string | null, sessionId?: string | null) => {
      workbenchStore.focusTask(taskId, trackId, sessionId);
    },
    [workbenchStore],
  );

  const [draftTracks, setDraftTracks] = useState<DraftTrack[]>([
    { key: "t1", label: "", providerId: "codex", modelId: "" },
  ]);
  const [execTarget, setExecTarget] = useState<WorkbenchEnvTarget>("worktree");
  const [startBusy, setStartBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const [installAllBusy, setInstallAllBusy] = useState(false);
  const [useMultipleAgents, setUseMultipleAgents] = useState(false);
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const [dropActive, setDropActive] = useState(false);
  const dropHideTimerRef = useRef<number | null>(null);

  const settingsSnapshot = useSettingsSnapshot();
  const dictationSettings = settingsSnapshot.settings?.dictation ?? null;
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

  useEffect(() => {
    document.documentElement.classList.add("wb-no-scroll");
    document.body.classList.add("wb-no-scroll");
    return () => {
      document.body.classList.remove("wb-no-scroll");
      document.documentElement.classList.remove("wb-no-scroll");
    };
  }, []);

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
    if (useMultipleAgents) return;
    if (draftTracks.length <= 1) return;
    setDraftTracks((prev) => (prev.length > 0 ? [prev[0]] : prev));
  }, [useMultipleAgents, draftTracks.length]);


  const ensureActiveTrackSelection = useCallback(
    (taskId: string, trackIds: string[], sessionsMap: Record<string, any[]>) => {
      const activeTab = workbenchStore.getActiveTab();
      const prevTrackId = activeTab?.kind === "track" && activeTab.ref.taskId === taskId ? activeTab.ref.trackId : null;
      const wanted = prevTrackId ?? null;
      const nextTrackId = pickPreferredTrackId(trackIds, sessionsMap, wanted);
      if (activeTab?.kind === "track" && activeTab.ref.taskId === taskId && nextTrackId !== prevTrackId) {
        workbenchStore.setActiveTrackForActiveTask(nextTrackId);
      }
    },
    [workbenchStore],
  );

  useEffect(() => {
    if (!workspaceId) return;
    getWorkspace(workspaceId).then(setWorkspace).catch(() => setWorkspace(null));
    listProviders().then(setProviders).catch(() => setProviders([]));
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

  const refreshProviderOptions = useCallback(
    async (providerId: string): Promise<ProviderOptions | undefined> => {
      if (!workspaceId) return;
      try {
        const opts = await getProviderOptions(workspaceId, providerId);
        setProviderOptions((prev) => ({ ...prev, [providerId]: opts }));
        return opts;
      } catch {
        return;
      }
    },
    [workspaceId],
  );

  const runPostInstallAuthVerify = useCallback(
    async (providerId: string) => {
      if (!workspaceId) return;

      // First: probe (no prompt) so we can learn if auth is required.
      const opts = await refreshProviderOptions(providerId);
      if (!opts) return;

      if (opts.auth_required) {
        // Don't automatically kick off long-running auth flows; surface the Authenticate button instead.
        return;
      }

      // Second: explicit verify (tiny prompt) to catch BYO-key / endpoint issues that don't show up on probe.
      try {
        await verifyProviderForWorkspace(workspaceId, providerId);
      } catch {
        // ignore; options refresh will surface status details
      }

      await refreshProviderOptions(providerId);
    },
    [refreshProviderOptions, workspaceId],
  );

  useEffect(() => {
    // Keep polling after success so we can refresh provider status/options and transition the UI
    // away from the "100%" install pill promptly.
    const active = Object.entries(providerInstallsById).flatMap(([providerId, s]) =>
      s && (s.state === "running" || s.state === "succeeded") ? ([[providerId, s]] as const) : [],
    );
    if (active.length === 0) return;

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
      const completedProviders: string[] = [];
      await Promise.all(
        active.map(async ([providerId, s]) => {
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

            if (info.state === "succeeded" && !postInstallHandledRef.current.has(s.installId)) {
              postInstallHandledRef.current.add(s.installId);
              completedProviders.push(providerId);
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

          if (!cancelled && completedProviders.length > 0) {
            for (const providerId of completedProviders) {
              if (postInstallInFlightRef.current.has(providerId)) continue;
              postInstallInFlightRef.current.add(providerId);
              runPostInstallAuthVerify(providerId).finally(() => {
                postInstallInFlightRef.current.delete(providerId);
              });
            }
          }
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
  }, [providerInstallsById, getInstall, listProviders, runPostInstallAuthVerify]);

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
        if (s?.state === "succeeded" && st?.installed && st.health === "ok" && !stillRunning) {
          delete next[providerId];
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [providerInstallsById, providersById]);

  useEffect(() => {
    if (!providers.length) return;
    const codexInstalled = providersById["codex"]?.installed === true && providersById["codex"]?.health === "ok";
    if (codexInstalled) return;
    if (defaultProviderId === "codex") return;
    setDraftTracks((prev) => {
      const isDefault = prev.every((t) => t.providerId === "codex" && !t.label.trim() && !t.modelId.trim());
      if (!isDefault) return prev;
      return prev.map((t) => ({ ...t, providerId: defaultProviderId }));
    });
  }, [providers.length, providersById, defaultProviderId]);

  const activeTaskSummary = activeTaskId ? tasksById[activeTaskId] : null;
  const trackSummaries = useMemo(() => activeTaskSummary?.tracks ?? [], [activeTaskSummary]);
  const tracks = useMemo(() => trackSummaries.map((t) => t.track), [trackSummaries]);
  const primarySessionByTrackId = useMemo(() => {
    const out: Record<string, string> = {};
    for (const summary of trackSummaries) {
      const trackId = idToString(summary.track.id);
      const primary = idToString(summary.primary_session_id ?? "");
      if (trackId && primary) out[trackId] = primary;
    }
    return out;
  }, [trackSummaries]);
  const sessionsByTrack = useMemo(() => {
    const out: Record<string, any[]> = {};
    for (const summary of trackSummaries) {
      const trid = idToString(summary.track.id);
      if (!trid) continue;
      out[trid] = summary.sessions.map((s) => s.session);
    }
    return out;
  }, [trackSummaries]);
  const activeTaskSessionIds = useMemo(() => {
    const ids: string[] = [];
    for (const summary of trackSummaries) {
      for (const session of summary.sessions) {
        const sid = idToString(session.session.id);
        if (sid) ids.push(sid);
      }
    }
    return ids;
  }, [trackSummaries]);

  const warmSessionIds = useMemo(() => {
    const ids: { id: string; updatedAt: number; running: boolean }[] = [];
    const activeSet = new Set(activeTaskSessionIds);
    for (const taskId of workspaceCatchup.activeIds) {
      const task = tasksById[taskId];
      if (!task) continue;
      for (const summary of task.tracks) {
        for (const sess of summary.sessions) {
          const sid = idToString(sess.session.id);
          if (!sid || activeSet.has(sid)) continue;
          const last = parseMs(sess.last_message_at) ?? parseMs(sess.session.updated_at) ?? 0;
          const running = sess.session.status === "active" || sess.session.status === "running";
          ids.push({ id: sid, updatedAt: last, running });
        }
      }
    }
    ids.sort((a, b) => {
      if (a.running !== b.running) return a.running ? -1 : 1;
      return b.updatedAt - a.updatedAt;
    });
    return ids.map((s) => s.id).slice(0, 20);
  }, [activeTaskSessionIds, tasksById, workspaceCatchup.activeIds]);

  const diffPrefetchTrackIds = useMemo(() => {
    const out = new Set<string>();
    for (const summary of trackSummaries) {
      const trackId = idToString(summary.track.id);
      if (trackId) out.add(trackId);
    }
    const warmSet = new Set(warmSessionIds);
    for (const taskId of workspaceCatchup.activeIds) {
      const task = tasksById[taskId];
      if (!task) continue;
      for (const summary of task.tracks) {
        const trackId = idToString(summary.track.id);
        if (!trackId) continue;
        for (const sess of summary.sessions) {
          const sid = idToString(sess.session.id);
          if (!sid || !warmSet.has(sid)) continue;
          out.add(trackId);
        }
      }
    }
    return Array.from(out);
  }, [trackSummaries, warmSessionIds, tasksById, workspaceCatchup.activeIds]);

  useEffect(() => {
    if (!activeTaskId) {
      return;
    }
    const trackIds = tracks.map((tr) => idToString(tr.id)).filter(Boolean);
    if (trackIds.length === 0) {
      workbenchStore.setActiveTrackForActiveTask(null);
      return;
    }
    ensureActiveTrackSelection(activeTaskId, trackIds, sessionsByTrack);
  }, [activeTaskId, ensureActiveTrackSelection, sessionsByTrack, tracks, workbenchStore]);

  useEffect(() => {
    supervisor.setActiveTaskSessionIds(activeTaskSessionIds);
  }, [supervisor, activeTaskSessionIds]);

  useEffect(() => {
    supervisor.setWarmSessionIds(warmSessionIds);
  }, [supervisor, warmSessionIds]);

  useEffect(() => {
    if (diffPrefetchTrackIds.length === 0) return;
    supervisor.prefetchDiffs(diffPrefetchTrackIds);
  }, [supervisor, diffPrefetchTrackIds]);


  const normalizedTaskQuery = taskQuery.trim().toLowerCase();
  const filteredActiveIds = useMemo(() => {
    return workspaceCatchup.activeIds.filter((id) => {
      const summary = tasksById[id];
      if (!summary) return false;
      if (!normalizedTaskQuery) return true;
      return (summary.task.title ?? "").toLowerCase().includes(normalizedTaskQuery);
    });
  }, [workspaceCatchup.activeIds, tasksById, normalizedTaskQuery]);

  const filteredArchivedIds = useMemo(() => {
    return workspaceCatchup.archivedIds.filter((id) => {
      const summary = tasksById[id];
      if (!summary) return false;
      if (!normalizedTaskQuery) return true;
      return (summary.task.title ?? "").toLowerCase().includes(normalizedTaskQuery);
    });
  }, [workspaceCatchup.archivedIds, tasksById, normalizedTaskQuery]);

  const activeTaskSummaries = useMemo(
    () => filteredActiveIds.map((id) => tasksById[id]).filter((v): v is WorkspaceCatchupItem => Boolean(v)),
    [filteredActiveIds, tasksById],
  );
  const archivedTaskSummaries = useMemo(
    () => filteredArchivedIds.map((id) => tasksById[id]).filter((v): v is WorkspaceCatchupItem => Boolean(v)),
    [filteredArchivedIds, tasksById],
  );

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

  const markTaskReadInFlightRef = useRef<Record<string, Promise<void>>>({});

  const markTaskRead = useCallback(async (taskId: string) => {
    if (markTaskReadInFlightRef.current[taskId]) return;
    const p = (async () => {
      try {
        const updated = await markTaskReadApi(taskId);
        workspaceCatchupStore.applyTaskUpdate(updated);
      } catch {
        // ignore
      }
    })().finally(() => {
      delete markTaskReadInFlightRef.current[taskId];
    });
    markTaskReadInFlightRef.current[taskId] = p;
    await p;
  }, [workspaceCatchupStore]);

  const markTaskUnread = useCallback(async (taskId: string) => {
    try {
      const updated = await markTaskUnreadApi(taskId);
      workspaceCatchupStore.applyTaskUpdate(updated);
    } catch {
      // ignore
    }
  }, [workspaceCatchupStore]);

  useEffect(() => {
    if (!activeTaskId) return;
    const tid = activeTaskId;
    const taskSummary = tasksById[tid];
    const t = taskSummary?.task;
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
  }, [activeTaskId, markTaskRead, taskLiveInfo.lastAssistantMsByTask, taskLiveInfo.workingByTask, tasksById]);

  useEffect(() => {
    if (!archivedCollapsed) {
      workspaceCatchupStore.ensureArchivedLoaded();
    }
  }, [archivedCollapsed, workspaceCatchupStore]);

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
      workspaceCatchupStore.applyTaskUpdate(updated);
      if (nextArchived && activeTaskId === taskId) setArchivedCollapsed(false);
    },
    [activeTaskId, workspaceCatchupStore],
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
      const summary = tasksById[taskId];
      setRenamingTaskId(taskId);
      setRenameDraft(String(summary?.task.title ?? "").trim());
    },
    [tasksById],
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
      const current = String(tasksById[taskId]?.task.title ?? "").trim();
      if (current && current === next) {
        cancelRenameTask();
        return;
      }
      try {
        const updated = await updateTaskTitle(taskId, next);
        workspaceCatchupStore.applyTaskUpdate(updated);
        cancelRenameTask();
      } catch (e: any) {
        window.alert(e?.message ?? "Failed to rename.");
      }
    },
    [cancelRenameTask, renameDraft, tasksById, workspaceCatchupStore],
  );

  const renderTaskRow = useCallback(
    (summary: WorkspaceCatchupItem, opts?: { archived?: boolean }) => {
      const tid = summary.id;
      const t = summary.task;
      const selected = tid === activeTaskId;
      const archived = !!opts?.archived;
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
      const summaryProviders = summary.tracks.flatMap((tr) =>
        tr.sessions.map((s) => String(s.session.provider_id ?? "").trim()).filter(Boolean),
      );
      const providerIds =
        (providerIdsByTaskFromSessions[tid] ?? []).length > 0 ? providerIdsByTaskFromSessions[tid] : summaryProviders;
      const providerCount = new Set(providerIds).size;
      const harnesses = providerIds
        .map((pid) => HARNESS_CATALOG.find((h) => h.id === pid))
        .filter(Boolean)
        .slice(0, 3) as Array<(typeof HARNESS_CATALOG)[number]>;

      return (
        <React.Fragment key={tid}>
          <div
            className={`wb-task-row ${archived ? "wb-task-row-archived" : ""} ${selected ? "wb-task-row-active" : ""}`}
            role="listitem"
            onClick={() => focusTask(tid)}
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
                focusTask(tid);
              }
            }}
            tabIndex={0}
            title={title}
          >
            <div className="wb-task-leading" aria-hidden="true">
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
                    onToggleArchive(tid, !archived).catch(() => {});
                  }}
                  aria-label={archived ? "Unarchive" : "Archive"}
                  title={archived ? "Unarchive" : "Archive"}
                >
                  <Archive size={14} />
                </button>
              </div>
            </div>
          </div>
        </React.Fragment>
      );
    },
    [
      activeTaskId,
      cancelRenameTask,
      commitRenameTask,
      focusTask,
      onToggleArchive,
      openTaskMenu,
      providerIdsByTaskFromSessions,
      renameDraft,
      renamingTaskId,
      taskLiveInfo.errorByTask,
      taskLiveInfo.lastAssistantMsByTask,
      taskLiveInfo.workingByTask,
    ],
  );

  const onDeleteTask = useCallback(
    async (taskId: string) => {
      const summary = tasksById[taskId];
      const title = String(summary?.task.title ?? "this task");
      if (!window.confirm(`Delete “${title}”? This deletes all sessions and messages in the task.`)) return;
      try {
        await deleteTask(taskId);
        if (activeTaskId === taskId) {
          focusNewTask();
        }
      } catch (e: any) {
        window.alert(e?.message ?? "Failed to delete task.");
      }
    },
    [activeTaskId, focusNewTask, tasksById],
  );

  const openConvoMenu = useCallback((triggerEl: HTMLElement) => {
    const rect = triggerEl.getBoundingClientRect();
    const left = Math.min(rect.left, window.innerWidth - 240);
    const top = Math.min(rect.bottom + 6, window.innerHeight - 220);
    setConvoMenu((prev) => (prev ? null : { style: { left, top } }));
  }, []);

  const activeSessionId = useMemo(() => {
    if (!activeTrackId) return null;
    const override =
      activeTab?.kind === "track" && activeTab.ref.trackId === activeTrackId ? (activeTab.ref.sessionId ?? null) : null;
    if (override) return override;
    const sessions = sessionsByTrack[activeTrackId] ?? [];
    return pickPreferredSessionId(sessions, primarySessionByTrackId[activeTrackId]);
  }, [activeTab, activeTrackId, sessionsByTrack, primarySessionByTrackId]);

  const activeSessionDraft = useWorkbenchDraft(activeSessionId ? sessionDraftKey(activeSessionId) : "", {
    text: "",
    modeId: "default",
  });

  const activeScrollKey = useMemo(() => {
    if (!activeSessionId) return null;
    if (!activeTab) return null;
    return scrollKey(activeTab.id, activeSessionId);
  }, [activeSessionId, activeTab]);

  const activeScrollState = useMemo(() => {
    if (!activeScrollKey) return null;
    return (
      workbenchSnap.window.scrollByKey[activeScrollKey] ?? {
        stickToBottom: true,
        anchorItemId: null,
        updatedAtMs: 0,
      }
    );
  }, [activeScrollKey, workbenchSnap.window.scrollByKey]);

  const showDebugIds = useMemo(() => {
    const params = new URLSearchParams(window.location.search);
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
  }, []);

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
  const [worktreePaths, setWorktreePaths] = useState<Record<string, string>>({});
  const worktreeRequestsRef = useRef<Record<string, Promise<string | null>>>({});
  const activeWorktreePath = activeWorktreeId ? worktreePaths[activeWorktreeId] ?? "" : "";

  const rememberWorktreePath = useCallback((worktreeId: string, rootPath: string) => {
    if (!worktreeId || !rootPath) return;
    setWorktreePaths((prev) => (prev[worktreeId] === rootPath ? prev : { ...prev, [worktreeId]: rootPath }));
  }, []);

  const ensureWorktreePath = useCallback(
    async (worktreeId: string) => {
      if (!worktreeId) return null;
      if (worktreePaths[worktreeId]) return worktreePaths[worktreeId];
      const existing = worktreeRequestsRef.current[worktreeId];
      if (existing) return existing;

      const task = (async () => {
        const cached = await loadWorktreeV1(worktreeId);
        if (cached?.rootPath) {
          rememberWorktreePath(worktreeId, cached.rootPath);
          return cached.rootPath;
        }
        const worktree = await getWorktree(worktreeId);
        const rootPath = String(worktree?.root_path ?? "");
        if (rootPath) {
          rememberWorktreePath(worktreeId, rootPath);
          await saveWorktreeV1(worktreeId, rootPath);
          return rootPath;
        }
        return null;
      })().finally(() => {
        delete worktreeRequestsRef.current[worktreeId];
      });

      worktreeRequestsRef.current[worktreeId] = task;
      return task;
    },
    [rememberWorktreePath, worktreePaths],
  );

  useEffect(() => {
    if (!activeWorktreeId) return;
    void ensureWorktreePath(activeWorktreeId);
  }, [activeWorktreeId, ensureWorktreePath]);

  const worktreePrefetchIds = useMemo(() => {
    const ids = new Set<string>();
    for (const taskId of workspaceCatchup.activeIds) {
      const task = tasksById[taskId];
      if (!task) continue;
      for (const summary of task.tracks) {
        const worktreeId = idToString(summary.track.worktree_id);
        if (worktreeId) ids.add(worktreeId);
      }
    }
    return Array.from(ids);
  }, [tasksById, workspaceCatchup.activeIds]);

  useEffect(() => {
    if (worktreePrefetchIds.length === 0) return;
    for (const worktreeId of worktreePrefetchIds) {
      void ensureWorktreePath(worktreeId);
    }
  }, [ensureWorktreePath, worktreePrefetchIds]);

  const hasDiff = activeTrackDiff.trim().length > 0;
  const showReviewPane = hasDiff;
  const diffFileCount = useMemo(() => {
    if (!hasDiff) return 0;
    const m = activeTrackDiff.match(/^diff --git /gm);
    return m ? m.length : 1;
  }, [activeTrackDiff, hasDiff]);



  const providerOptionsInFlightRef = useRef<Record<string, Promise<ProviderOptions | undefined>>>({});

  const ensureProviderOptions = useCallback(
    async (providerId: string, opts?: { force?: boolean }): Promise<ProviderOptions | undefined> => {
      if (!workspaceId) return;
      const installed = providersById[providerId]?.installed === true && providersById[providerId]?.health === "ok";
      if (!installed) return;

      const force = opts?.force ?? false;
      const existing = providerOptionsInFlightRef.current[providerId];
      if (existing && !force) return existing;
      if (!force && providerOptions[providerId]) return providerOptions[providerId];

      const p = getProviderOptions(workspaceId, providerId)
        .then((opts) => {
          setProviderOptions((prev) => ({ ...prev, [providerId]: opts }));
          return opts;
        })
        .finally(() => {
          if (providerOptionsInFlightRef.current[providerId] === p) {
            delete providerOptionsInFlightRef.current[providerId];
          }
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
    const missing = draftTracks.find(
      (t) => !(providersById[t.providerId]?.installed === true && providersById[t.providerId]?.health === "ok"),
    );
    if (missing) {
      const diag = providersById[missing.providerId]?.diagnostics?.[0];
      return diag
        ? `Harness “${missing.providerId}” unavailable: ${diag}`
        : `Harness “${missing.providerId}” unavailable.`;
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

      const toStart =
        draftTracks.length > 0 ? draftTracks : [{ key: "t1", label: "", providerId: "codex", modelId: "" }];

      for (let i = 0; i < toStart.length; i++) {
        const dt = toStart[i];
        const installed = providersById[dt.providerId]?.installed === true && providersById[dt.providerId]?.health === "ok";
        if (!installed) {
          const diag = providersById[dt.providerId]?.diagnostics?.[0];
          throw new Error(
            diag ? `Harness “${dt.providerId}” unavailable: ${diag}` : `Harness “${dt.providerId}” unavailable.`,
          );
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
          focusTask(taskId, trackId, sessionId);
        }
        supervisor.refreshSession(sessionId, { watchDiff: false });
        await postMessage(sessionId, prompt, "immediate", draftAttachments);
        supervisor.refreshSession(sessionId, { watchDiff: false });
      }

      setNewTaskDraft({ text: "", modeId: "default" });
      await workbenchStore.flushDraft(NEW_TASK_DRAFT_KEY);
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

  const activeTask = activeTaskSummary?.task ?? null;
  const expectedActiveTrackCount = activeTaskSummary ? activeTaskSummary.tracks.length : null;
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
    const worktreePath = sess?.env_target === "worktree" ? String(activeWorktreePath ?? "") : "";
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
  }, [activeEntry, activeTask?.title, activeWorktreePath, tracks.length]);

  const singleTrackHeaderForRender = useMemo(() => {
    if (singleTrackHeader) return singleTrackHeader;
    if (!activeTaskId) return null;
    return {
      title: activeTask?.title ?? "Conversation",
      age: "Loading…",
      harness: "",
      modelBase: "",
      effort: "",
      worktreeSlug: "",
      worktreePath: "",
      canCopyWorktree: false,
    };
  }, [activeTask?.title, activeTaskId, singleTrackHeader]);

  const showSingleTrackHeader = Boolean(
    activeTaskId &&
      singleTrackHeaderForRender &&
      (tracks.length === 1 || (tracks.length === 0 && (expectedActiveTrackCount === 1 || expectedActiveTrackCount === null))),
  );

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

    const thread = buildWorkbenchThreadViewModel(
      activeEntry.turns ?? [],
      activeEntry.messages ?? [],
      activeEntry.turnToolsByTurnId ?? {},
      activeEntry.events ?? [],
    );
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
  }, [
    activeTaskId,
    extractFilesFromTransfer,
    extractFirstUrlFromTransfer,
    hideDropOverlay,
    onDropFiles,
    showDropOverlay,
    urlToImageFile,
  ]);

  const rootStyle = useMemo(() => {
    const max = Math.max(170, window.innerWidth - 240);
    const clamped = Math.min(max, Math.max(170, Math.round(sidebarWidth)));
    return { ["--wb-sidebar-width" as any]: `${clamped}px` } as React.CSSProperties;
  }, [sidebarWidth]);

  if (!workbenchSnap.hydrated) {
    return (
      <div
        className={`wb-root ${sidebarCollapsed ? "wb-root-collapsed" : ""} ${sidebarResizing ? "wb-root-resizing" : ""}`}
        style={rootStyle}
      >
        <div className="wb-topbar">
          <div className="wb-topbar-title">{workspace?.name ?? "Workspace"}</div>
        </div>
        <div className="wb-main">
          <div className="wb-center">
            <div className="wb-muted" style={{ padding: 16 }}>
              Loading workspace layout…
            </div>
          </div>
        </div>
      </div>
    );
  }

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
          <Link
            className="wb-topbar-icon"
            to={`/settings?ws=${encodeURIComponent(workspaceId)}`}
            title="Settings"
            aria-label="Settings"
          >
            <Settings size={14} />
          </Link>
        </div>
      </div>

      {workbenchSnap.warnings.length > 0 && (
        <div className="banner" style={{ margin: "8px 12px 0" }}>
          {workbenchSnap.warnings[0]}
        </div>
      )}

      <div className="wb-sidebar" aria-hidden={sidebarCollapsed}>
        <div className="wb-sidebar-top">
          <div className="wb-sidebar-header">
            <button
              type="button"
              className="wb-new-agent"
              onClick={focusNewTask}
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

        <div className="wb-sidebar-section wb-sidebar-grow" style={{ minHeight: 0, display: "flex" }}>
          <Virtuoso
            style={{ height: "100%" }}
            data={activeTaskSummaries}
            overscan={8}
            computeItemKey={(_, summary) => summary.id}
            itemContent={(_, summary) => renderTaskRow(summary)}
            endReached={() => {
              if (workspaceCatchup.hasMoreActive) {
                workspaceCatchupStore.loadMoreActive();
              }
            }}
            components={{
              Scroller: React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
                <div {...props} ref={ref} className="wb-task-scroll" />
              )),
              List: React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
                <div {...props} ref={ref} className="wb-task-list" role="list" aria-label="Active tasks" />
              )),
              Header: () => (
                <div className="wb-section-header">
                  <div className="wb-section-title">Active</div>
                </div>
              ),
              Footer: () => (
                <>
                  {activeTaskSummaries.length === 0 &&
                    workspaceCatchup.initialized &&
                    workspaceCatchup.fetchState.active !== "loading" && (
                      <div className="wb-task-list">
                        <div className="wb-muted">No active tasks.</div>
                      </div>
                    )}
                  {workspaceCatchup.fetchState.active === "loading" && (
                    <div className="wb-task-list">
                      <div className="wb-muted">Loading tasks…</div>
                    </div>
                  )}
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
                      {workspaceCatchup.fetchState.archived === "loading" && (
                        <div className="wb-muted">Loading archived tasks…</div>
                      )}
                      {workspaceCatchup.fetchState.archived === "error" && (
                        <div className="wb-muted">Failed to load archived tasks. Retry.</div>
                      )}
                      {archivedTaskSummaries.map((summary) => renderTaskRow(summary, { archived: true }))}
                      {archivedTaskSummaries.length === 0 &&
                        workspaceCatchup.archivedLoaded &&
                        workspaceCatchup.fetchState.archived !== "loading" && <div className="wb-muted">No archived tasks.</div>}
                      {workspaceCatchup.hasMoreArchived && (
                        <button
                          type="button"
                          className="wb-archived-more"
                          onClick={() => workspaceCatchupStore.loadMoreArchived()}
                        >
                          Load more
                        </button>
                      )}
                    </div>
                  )}
                </>
              ),
            }}
          />
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

                              <div className="wb-harness-list">
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
                                  const providerStatus = providersById[id];
                                  const installed = providerStatus?.installed === true && providerStatus?.health === "ok";
                                  const installSupported = providerStatus?.details?.install_supported === "true";
                                  const installRunning = providerStatus?.details?.install_running === "true";
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
                                              title={providerStatus?.installed ? "Update this harness" : "Install this harness"}
                                            >
                                              {installRunning || installBusy ? "Installing…" : providerStatus?.installed ? "Update" : "Install"}
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
              {showSingleTrackHeader ? (
                <div className="wb-single-track-header" aria-busy={tracks.length === 0 ? "true" : undefined}>
                  <div className="wb-single-track-title">{singleTrackHeaderForRender?.title ?? "Conversation"}</div>
                  <div className="wb-single-track-meta">
                    <div className="wb-single-track-meta-left">
                      <span>{singleTrackHeaderForRender?.age ?? "Now"}</span>
                      {singleTrackHeaderForRender?.harness ? (
                        <>
                          <span className="wb-single-track-dot" aria-hidden="true">
                            ·
                          </span>
                          <span>{singleTrackHeaderForRender.harness}</span>
                        </>
                      ) : null}
                      {singleTrackHeaderForRender?.modelBase && (
                        <>
                          <span className="wb-single-track-dot" aria-hidden="true">
                            ·
                          </span>
                          <span>{singleTrackHeaderForRender.modelBase}</span>
                        </>
                      )}
                      {singleTrackHeaderForRender?.effort && (
                        <>
                          <span className="wb-single-track-dot" aria-hidden="true">
                            ·
                          </span>
                          <span>{singleTrackHeaderForRender.effort}</span>
                        </>
                      )}
                      {singleTrackHeaderForRender?.worktreeSlug && (
                        <>
                          <span className="wb-single-track-dot" aria-hidden="true">
                            ·
                          </span>
                          <button
                            type="button"
                            className={`wb-worktree-chip ${worktreeCopied ? "wb-worktree-chip-copied" : ""}`}
                            disabled={!singleTrackHeaderForRender.canCopyWorktree}
                            onClick={() => void copyWorktreeLocation()}
                            title="Copy worktree location"
                            aria-label="Copy worktree location"
                          >
                            <GitBranch size={13} />
                            <span className="wb-worktree-chip-slug">{singleTrackHeaderForRender.worktreeSlug}</span>
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
                <div className="wb-trackbar" aria-busy={tracks.length === 0 ? "true" : undefined}>
                  {tracks.length === 0 ? (
                    <>
                      <div className="wb-trackcard wb-trackcard-skeleton" aria-hidden="true" />
                      <div className="wb-trackcard wb-trackcard-skeleton" aria-hidden="true" />
                    </>
                  ) : (
                    tracks.map((tr) => {
                      const trid = idToString(tr.id);
                      const selected = trid === activeTrackId;
                      const sessions = sessionsByTrack[trid] ?? [];
                      const preferredSession = pickPreferredSession(sessions, primarySessionByTrackId[trid]) as any;
                      const sessionId = preferredSession ? idToString((preferredSession as any).id) : "";
                      const liveSession = sessionId ? sessionCache.sessions[sessionId]?.session : null;
                      const displaySession = (liveSession ?? preferredSession) as any;
                      const model = displaySession ? `${displaySession.provider_id} ${displaySession.model_id}` : "No session";
                      const status =
                        tr.status === "running" ? "Running…" : tr.status === "completed" ? "Task completed" : tr.status;
                      return (
                        <button
                          key={trid}
                          type="button"
                          className={`wb-trackcard ${selected ? "wb-trackcard-active" : ""}`}
                          onClick={() => workbenchStore.setActiveTrackForActiveTask(trid)}
                        >
                          <div className="wb-trackcard-title">{model}</div>
                          <div className="wb-trackcard-sub">{status}</div>
                        </button>
                      );
                    })
                  )}
                </div>
              )}

              <div className="wb-session">
                {activeSessionId ? (
                  <SessionView
                    key={activeSessionId ?? "empty"}
                    sessionId={activeSessionId}
                    variant="workbench"
                    showDiffPane={false}
                    draft={activeSessionDraft.value}
                    draftUpdatedAtMs={activeSessionDraft.updatedAtMs}
                    onDraftChange={(text) =>
                      activeSessionDraft.setValue({ text, modeId: activeSessionDraft.value.modeId })
                    }
                    onDraftPersistNow={() => workbenchStore.flushDraft(sessionDraftKey(activeSessionId))}
                    onModeChange={(modeId) =>
                      activeSessionDraft.setValue({ text: activeSessionDraft.value.text, modeId })
                    }
                    scrollState={activeScrollState ? { stickToBottom: activeScrollState.stickToBottom, anchorItemId: activeScrollState.anchorItemId } : null}
                    onScrollStateChange={(next) => {
                      if (!activeScrollKey) return;
                      workbenchStore.setScrollState(activeScrollKey, next);
                    }}
                  />
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
                      <div className="wb-diff-tab wb-diff-tab-active">All Changes</div>
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

                  {hasDiff && (
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
              const summary = tasksById[tid];
              const nextArchived = !summary?.task.archived_at;
              onToggleArchive(tid, nextArchived).catch(() => { });
              setTaskMenu(null);
            }}
            role="menuitem"
          >
            {(() => {
              const summary = tasksById[taskMenu.taskId];
              return summary?.task.archived_at ? "Unarchive" : "Archive";
            })()}
          </button>
          <button
            type="button"
            className="wb-menu-item"
            disabled={(() => {
              const tid = taskMenu.taskId;
              const summary = tasksById[tid];
              return !summary?.task.last_assistant_message_at;
            })()}
            onClick={() => {
              const tid = taskMenu.taskId;
              const summary = tasksById[tid];
              const t = summary?.task;
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
              const summary = tasksById[tid];
              const t = summary?.task;
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
