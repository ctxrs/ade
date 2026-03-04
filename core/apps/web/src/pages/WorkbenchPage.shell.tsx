import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Virtuoso } from "react-virtuoso";
import { flushSync } from "react-dom";
import { Link, useNavigate } from "react-router-dom";
import {
  ChevronDown,
  ChevronsLeft,
  ChevronsRight,
  SquarePen,
  Settings,
  X,
} from "lucide-react";
import {
  type ArchiveTaskResponse,
  type Message,
  MessageAttachment,
  type Session,
  type SessionTurn,
  type SessionSnapshotSummary,
  type Task,
  WebSessionInfo,
  Worktree,
  Workspace,
  archiveTask,
  createSession,
  createTask,
  deleteTask,
  daemonFetchRaw,
  getHealth,
  getSessionDiff,
  getWorktree,
  idToString,
  interruptSession,
  listWebSessions,
  markTaskRead as markTaskReadApi,
  markTaskUnread as markTaskUnreadApi,
  postMessage,
  unarchiveTask,
  updateTaskTitle,
} from "../api/client";
import {
  useOpenSession,
  useSessionCacheSnapshot,
  useSessionEntry,
  useSessionSupervisor,
  type SessionCacheEntry,
} from "../state/sessionSupervisor";
import { artifactPrefetcher } from "../state/artifactPrefetch";
import { ArtifactsPane } from "../components/ArtifactsPane";
import { DiffReviewPane } from "../components/DiffReviewPane";
import { SessionsPane } from "../components/SessionsPane";
import { TerminalPanel, type TerminalPanelHandle } from "../components/TerminalPanel";
import { TitleGenerationInstallBanner } from "../components/TitleGenerationInstallBanner";
import { WorktreeBootstrapSnackbar } from "../components/WorktreeBootstrapSnackbar";
import { DictationOnboardingModal } from "../components/dictation/DictationOnboardingModal";
import { buildWorkbenchThreadViewModel } from "./SessionPage";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import { WorkbenchComposer, type DraftHarness, type WorkbenchModeId } from "../components/WorkbenchComposer";
import type { SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import {
  desktopRecordWorkspaceVisit,
  desktopSetTitlebarColor,
  desktopSetWindowTitle,
  desktopStorageConsumeNotice,
  getDesktopPlatform,
  isDesktopApp,
  type DesktopPlatform,
  type DesktopStorageNotice,
} from "../utils/desktop";
import { copyTextToClipboard } from "../utils/clipboard";
import { errorMessage } from "../utils/errorMessage";
import { pickPreferredSessionId } from "../utils/workbenchSelection";
import { parseModelId } from "../utils/modelEffort";
import { getLoadTestTelemetry } from "../utils/loadTestTelemetry";
import { useDictationController } from "../utils/useDictationController";
import { randomUuid } from "../utils/randomUuid";
import { trackWorkbenchPanelToggled } from "../utils/analytics";
import {
  WEB_MENU_COMMAND_EVENT,
  WEB_MENU_STATE_EVENT,
  WEB_MENU_TRACE_EVENT,
  type DesktopMenuItemState,
  type WebMenuTraceDetail,
  type WebMenuCommandDetail,
  type WebMenuStateDetail,
} from "../utils/desktopMenuCommands";
import {
  NEW_TASK_DRAFT_KEY,
  scrollKey,
  useActiveWorkbenchIds,
  useNewTaskDraft,
  useWorkbenchShellSnapshot,
  useWorkbenchStore,
} from "../workbench/store";
import {
  loadWorkbenchTerminalPanelOpenV1,
  saveWorkbenchArtifactsPaneOpenV1,
  saveWorkbenchDiffPaneOpenV1,
  saveWorkbenchSessionsPaneOpenV1,
  saveWorkbenchTerminalPanelOpenV1,
} from "../workbench/persistence";
import {
  useWorkspaceActiveSnapshotSnapshot,
  useWorkspaceActiveSnapshotStore,
  type WorkspaceActiveSnapshotItem,
} from "../state/workspaceActiveSnapshotStore";
import { useDaemonBaseUrl } from "../api/useDaemonConnection";
import { useEnsureArchivedLoaded } from "../state/useEnsureArchivedLoaded";
import { hasConfiguredHarnessAuth } from "../utils/providerAuthStatus";
import { HarnessAuthenticationSection } from "./settings/sections/HarnessAuthenticationSection";
import { TaskRow } from "./WorkbenchPage.taskRow";
import { TASK_LIST_COMPONENTS } from "./WorkbenchPage.taskList";
import { WorkbenchSessionSlot } from "./WorkbenchPage.sessionSlot";
import { useWorkbenchDragDropAttachments } from "./workbenchShell/useWorkbenchDragDropAttachments";
import { getDiffSummaryStats, isDiffSummaryTooLarge } from "./workbenchShell/useWorkbenchDiffPane";
import { WORKBENCH_TASK_IDLE_EVENT, type WorkbenchTaskIdleDetail } from "../utils/updaterEvents";
import {
  collectSelectableHarnessProviderIds,
  getHarnessMruStorageKey,
  resolveInitialHarnessSelection,
  shouldFinalizeInitialHarnessSelection,
} from "./workbenchShell/harnessSelection";
import { useWorkbenchOptimisticTasks } from "./workbenchShell/useWorkbenchOptimisticTasks";
import { useWorkbenchProviders } from "./workbenchShell/useWorkbenchProviders";
import { WorkbenchSessionHeader } from "./workbenchShell/WorkbenchSessionHeader";
import { useWorkbenchTaskScrollbar } from "./workbenchShell/useWorkbenchTaskScrollbar";
import type {
  AnchorRect,
  ArchiveConfirmState,
  OptimisticFocus,
  OptimisticTaskSummary,
  TaskListContext,
  TaskListItem,
} from "./WorkbenchPage.types";
import {
  ARCHIVE_CONFIRM_STORAGE_KEY,
  SESSION_VIEW_POOL_LIMIT,
  appendSegment,
  clampNum,
  deriveManagedWorktreeRoot,
  deriveTaskTitle,
  formatWorktreeLabel,
  formatWorktreePath,
  isOptimisticTask,
  lastAssistantMessageMs,
  lastRoleMessageMs,
  modelIdsFromOptions,
  normalizeAnchorRect,
  parseMs,
  sanitizeFileName,
  saveMarkdownExport,
  spinnerDelayForNow,
} from "./WorkbenchPage.utils";
import { buildOptimisticUserMessage } from "./SessionPage.optimisticMessage";

export function WorkbenchPageInner({ workspaceId }: { workspaceId: string }) {
  const navigate = useNavigate();
  const supervisor = useSessionSupervisor();
  const sessionSnap = useSessionCacheSnapshot();
  const workbenchStore = useWorkbenchStore();
  const workspaceSnapshotStore = useWorkspaceActiveSnapshotStore();
  const workspaceSnapshot = useWorkspaceActiveSnapshotSnapshot();
  const tasksById = workspaceSnapshot.tasksById;
  const workbenchSnap = useWorkbenchShellSnapshot();
	const [optimisticFocus, setOptimisticFocus] = useState<OptimisticFocus | null>(null);
	const { taskId: activeTaskIdFromTab, sessionId: activeSessionIdFromTab } = useActiveWorkbenchIds();
	const navToken = workbenchStore.getNavToken();
	const optimisticFocusActive = Boolean(optimisticFocus && navToken === optimisticFocus.navToken);
	const activeTaskId =
	  activeTaskIdFromTab ?? (optimisticFocusActive && optimisticFocus ? optimisticFocus.taskId : null);
	const activeSessionIdFromTabResolved =
	  activeSessionIdFromTab ?? (optimisticFocusActive && optimisticFocus ? optimisticFocus.sessionId : null);
  const { value: newTaskDraft, setValue: setNewTaskDraft } = useNewTaskDraft();
  const draftPrompt = newTaskDraft.text;
  const draftMode = newTaskDraft.modeId;
  const [workspace, setWorkspace] = useState<Workspace | null>(null);

  useEffect(() => {
    supervisor.bindWorkspaceActiveSnapshotStore(workspaceSnapshotStore);
    return () => supervisor.bindWorkspaceActiveSnapshotStore(null);
  }, [supervisor, workspaceSnapshotStore]);
  useEffect(() => {
    if (!optimisticFocus) return;
    if (activeTaskIdFromTab) {
      setOptimisticFocus(null);
      return;
    }
    if (!optimisticFocusActive) {
      setOptimisticFocus(null);
    }
  }, [activeTaskIdFromTab, optimisticFocus, optimisticFocusActive]);
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
  const [daemonDataRoot, setDaemonDataRoot] = useState<string | null>(null);

  const [taskQuery, setTaskQuery] = useState("");
  const taskSearchRef = useRef<HTMLInputElement | null>(null);
  const {
    optimisticTasks,
    setOptimisticTasks,
    optimisticStartingTaskRef,
    optimisticTasksById,
    optimisticSessionIdSet,
    optimisticFailureBySessionId,
    activeTaskSummary,
  } = useWorkbenchOptimisticTasks({
    activeTaskId,
    activeTaskIdFromTab,
    tasksById,
  });
  const [archivedCollapsed, setArchivedCollapsed] = useState(true);
  const [archiveConfirm, setArchiveConfirm] = useState<ArchiveConfirmState | null>(null);
  const [archiveConfirmDontRemind, setArchiveConfirmDontRemind] = useState(false);
  const [archiveConfirmDismissed, setArchiveConfirmDismissed] = useState(false);
  const [archivePendingById, setArchivePendingById] = useState<Record<string, "archive" | "unarchive">>({});
  const [archiveCleanupNotice, setArchiveCleanupNotice] = useState(false);
  const archiveConfirmRef = useRef<HTMLDivElement | null>(null);
  const [taskMenu, setTaskMenu] = useState<{ taskId: string; style: React.CSSProperties } | null>(null);
  const taskMenuRef = useRef<HTMLDivElement | null>(null);
  const [hoveredTaskId, setHoveredTaskId] = useState<string | null>(null);
  const [renamingTaskId, setRenamingTaskId] = useState<string | null>(null);
  const renameDraftsRef = useRef<Map<string, string>>(new Map());
  const [convoMenu, setConvoMenu] = useState<{ style: React.CSSProperties } | null>(null);
  const convoMenuRef = useRef<HTMLDivElement | null>(null);
  const [copyTranscriptBusy, setCopyTranscriptBusy] = useState(false);
  const copyTranscriptBusyRef = useRef(false);
  const transcriptSpinnerDelayRef = useRef<number>(spinnerDelayForNow());
  const [transcriptNotice, setTranscriptNotice] = useState<string | null>(null);
  const [desktopStorageNotice, setDesktopStorageNotice] = useState<DesktopStorageNotice | null>(null);

  const getRenameDraft = useCallback((taskId: string, fallback: string) => {
    return renameDraftsRef.current.get(taskId) ?? fallback;
  }, []);

  const setRenameDraft = useCallback((taskId: string, nextValue: string) => {
    renameDraftsRef.current.set(taskId, nextValue);
  }, []);

  const clearRenameDraft = useCallback((taskId: string) => {
    renameDraftsRef.current.delete(taskId);
  }, []);

  const [activeWorktree, setActiveWorktree] = useState<Worktree | null>(null);
  const worktreeCacheRef = useRef<Map<string, Worktree>>(new Map());
  const worktreeFetchRef = useRef<Map<string, Promise<Worktree | null>>>(new Map());

  const focusNewTask = useCallback(() => {
    workbenchStore.focusNewTask();
  }, [workbenchStore]);

  const focusTask = useCallback(
    (taskId: string, sessionId?: string | null) => {
      workbenchStore.focusTask(taskId, sessionId);
    },
    [workbenchStore],
  );

  const [draftHarness, setDraftHarness] = useState<DraftHarness | null>(null);
  const [startBusy, setStartBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const [harnessAuthModalProviderId, setHarnessAuthModalProviderId] = useState<string | null>(null);
  const [pendingHarnessSelectionProviderId, setPendingHarnessSelectionProviderId] = useState<string | null>(null);
  const prefetchedProviderOptionsRef = useRef<Set<string>>(new Set());
  const initialHarnessSelectionResolvedRef = useRef(false);

  const {
    dictationRecording,
    dictationError,
    dictationDebugText,
    dictationOnboarding,
    dismissDictationOnboarding,
    backDictationOnboarding,
    chooseDictationOnboardingLocal,
    chooseDictationOnboardingCloud,
    updateDictationOnboardingCloud,
    submitDictationOnboardingLocal,
    submitDictationOnboardingCloud,
    startDictation,
    stopDictation,
  } = useDictationController({
    text: draftPrompt,
    setText: setDraftPrompt,
    appendSegment,
  });

  const {
    providersById,
    defaultProviderId,
    providerInstallsById,
    providerOptions,
    installAllBusy,
    installProviderFromMenu,
    cancelProviderInstallFromMenu,
    installAllProvidersFromMenu,
    ensureProviderAuthSummary,
  } = useWorkbenchProviders({
    workspaceId,
    setDraftHarness,
    onStartError: setStartError,
  });

  const selectableHarnessProviderIds = useMemo(
    () => collectSelectableHarnessProviderIds(providersById),
    [providersById],
  );

  const setSingleDraftHarness = useCallback((providerId: string) => {
    setDraftHarness((prev) => {
      if (prev?.providerId === providerId) return prev;
      return { providerId, modelId: "" };
    });
  }, []);

  const requestHarnessAuthFromComposer = useCallback((providerId: string) => {
    setPendingHarnessSelectionProviderId(providerId);
    setHarnessAuthModalProviderId(providerId);
  }, []);

  const onComposerHarnessAuthModalClosed = useCallback(
    (providerId: string | null) => {
      setHarnessAuthModalProviderId(null);
      if (!providerId) return;
      if (pendingHarnessSelectionProviderId !== providerId) return;
      setPendingHarnessSelectionProviderId(null);
      void ensureProviderAuthSummary(providerId, { force: true })
        .then((opts) => {
          const resolved = opts ?? providerOptions[providerId];
          if (!hasConfiguredHarnessAuth(providerId, resolved)) return;
          setSingleDraftHarness(providerId);
        })
        .catch(() => {});
    },
    [ensureProviderAuthSummary, pendingHarnessSelectionProviderId, providerOptions, setSingleDraftHarness],
  );

  const { dropActive } = useWorkbenchDragDropAttachments({
    scopeRef: newComposerRef,
    activeTaskId,
    setDraftAttachments,
  });

  const [rightPaneMode, setRightPaneMode] = useState<"diff" | "artifacts" | "sessions" | null>(null);
  const clampDiffWidth = useCallback((value: number) => {
    const gutterPx = 8;
    const containerWidth =
      (document.querySelector(".wb-body") as HTMLElement | null)?.clientWidth ?? window.innerWidth;
    const min = Math.min(320, containerWidth);
    const max = Math.max(min, containerWidth - gutterPx);
    return Math.min(max, Math.max(min, Math.round(value)));
  }, []);

  const [diffWidth, setDiffWidth] = useState(() => clampDiffWidth(480));
  const [diffResizing, setDiffResizing] = useState(false);
  const [diffOpenHydrated, setDiffOpenHydrated] = useState(false);
  const [diffContentLoading, setDiffContentLoading] = useState(false);
  const [diffContentErrorBySessionId, setDiffContentErrorBySessionId] = useState<
    Record<string, string | undefined>
  >({});
  const [artifactsOpenHydrated, setArtifactsOpenHydrated] = useState(false);
  const [artifactsOpenSeeded, setArtifactsOpenSeeded] = useState(false);
  const [, setArtifactsAutoOpenPending] = useState(false);
  const artifactsPaneScopeRef = useRef<string | null>(null);
  const [sessionsOpenHydrated, setSessionsOpenHydrated] = useState(false);
  const sessionsPaneScopeRef = useRef<string | null>(null);
  const reviewTab: "git" = "git";
  const terminalPanelRef = useRef<TerminalPanelHandle | null>(null);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [terminalHeight, setTerminalHeight] = useState(260);
  const [terminalResizing, setTerminalResizing] = useState(false);
  const [terminalOpenHydrated, setTerminalOpenHydrated] = useState(false);
  const rightPaneModeRef = useRef<"diff" | "artifacts" | "sessions" | null>(rightPaneMode);
  const terminalOpenRef = useRef<boolean>(terminalOpen);
  const diffOpen = rightPaneMode === "diff";
  const artifactsOpen = rightPaneMode === "artifacts";
  const sessionsOpen = rightPaneMode === "sessions";

  useEffect(() => {
    rightPaneModeRef.current = rightPaneMode;
  }, [rightPaneMode]);

  useEffect(() => {
    terminalOpenRef.current = terminalOpen;
  }, [terminalOpen]);

  const toggleTerminalPanel = useCallback((source: "header_button" | "menu_command" | "unknown" = "unknown") => {
    const nextOpen = !terminalOpenRef.current;
    terminalOpenRef.current = nextOpen;
    setTerminalOpen(nextOpen);
    trackWorkbenchPanelToggled({
      panelKey: "terminal",
      open: nextOpen,
      source,
    });
    if (nextOpen) {
      terminalPanelRef.current?.setScope("workspace");
    }
  }, []);

  useEffect(() => {
    document.documentElement.classList.add("wb-no-scroll");
    document.body.classList.add("wb-no-scroll");
    return () => {
      document.body.classList.remove("wb-no-scroll");
      document.documentElement.classList.remove("wb-no-scroll");
    };
  }, []);

  const clampTerminalHeight = useCallback((value: number) => {
    const min = 160;
    const max = Math.max(min, window.innerHeight - 160);
    return Math.min(max, Math.max(min, Math.round(value)));
  }, []);

  useLayoutEffect(() => {
    setDiffWidth((w) => {
      const clamped = clampDiffWidth(w);
      return clamped === w ? w : clamped;
    });
  }, [clampDiffWidth, sidebarCollapsed, sidebarWidth]);

  useEffect(() => {
    const onResize = () => {
      const max = Math.max(170, window.innerWidth - 240);
      setSidebarWidth((w) => Math.min(max, Math.max(170, Math.round(w))));
      setTerminalHeight((h) => clampTerminalHeight(h));
      setDiffWidth((w) => clampDiffWidth(w));
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [clampDiffWidth, clampTerminalHeight]);

  useEffect(() => {
    if (!workspaceId) return;
    const key = `wb.archivedCollapsed.${workspaceId}`;
    const v = localStorage.getItem(key);
    if (v === "0") setArchivedCollapsed(false);
    else setArchivedCollapsed(true);
  }, [workspaceId]);

  useEffect(() => {
    try {
      const v = localStorage.getItem(ARCHIVE_CONFIRM_STORAGE_KEY);
      setArchiveConfirmDismissed(v === "1");
    } catch {
      // ignore
    }
  }, []);

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
    if (!archiveConfirm) return;
    const onPointerDown = (e: PointerEvent) => {
      const el = e.target as HTMLElement | null;
      if (!el) return;
      if (el.closest(".wb-archive-confirm")) return;
      if (el.closest(".wb-archive-confirm-trigger")) return;
      setArchiveConfirm(null);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setArchiveConfirm(null);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [archiveConfirm]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const isToggleKey = e.code === "Backquote" || e.key === "`";
      if (e.ctrlKey && !e.metaKey && !e.altKey && isToggleKey) {
        e.preventDefault();
        toggleTerminalPanel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggleTerminalPanel]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (e.altKey || e.shiftKey) return;
      const hasModifier = e.metaKey || e.ctrlKey;
      if (!hasModifier) return;
      const key = e.key.toLowerCase();
      if (key === "b") {
        e.preventDefault();
        setSidebarCollapsed((prev) => !prev);
      } else if (key === "n") {
        e.preventDefault();
        focusNewTask();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [focusNewTask]);

  useEffect(() => {
    if (activeTaskId) return;
    for (const providerId of selectableHarnessProviderIds) {
      if (providerOptions[providerId]) continue;
      if (prefetchedProviderOptionsRef.current.has(providerId)) continue;
      prefetchedProviderOptionsRef.current.add(providerId);
      ensureProviderAuthSummary(providerId).catch(() => {});
    }
  }, [activeTaskId, ensureProviderAuthSummary, providerOptions, selectableHarnessProviderIds]);

  useEffect(() => {
    prefetchedProviderOptionsRef.current.clear();
    initialHarnessSelectionResolvedRef.current = false;
  }, [workspaceId]);

  useEffect(() => {
    if (activeTaskId) return;
    if (!workspaceId) return;
    if (draftHarness) return;
    if (initialHarnessSelectionResolvedRef.current) return;
    if (selectableHarnessProviderIds.length === 0) return;

    let mruProviderId: string | null = null;
    try {
      mruProviderId = localStorage.getItem(getHarnessMruStorageKey(workspaceId));
    } catch {
      // ignore
    }

    const selectedProviderId = resolveInitialHarnessSelection({
      providerIds: selectableHarnessProviderIds,
      providerOptions,
      mruProviderId,
    });
    if (!shouldFinalizeInitialHarnessSelection(selectedProviderId)) return;
    initialHarnessSelectionResolvedRef.current = true;
    setSingleDraftHarness(selectedProviderId);
  }, [
    activeTaskId,
    draftHarness,
    providerOptions,
    selectableHarnessProviderIds,
    setSingleDraftHarness,
    workspaceId,
  ]);

  const selectedDraftProviderId = draftHarness?.providerId ?? null;

  useEffect(() => {
    if (activeTaskId) return;
    if (!workspaceId) return;
    if (!selectedDraftProviderId) return;
    if (!hasConfiguredHarnessAuth(selectedDraftProviderId, providerOptions[selectedDraftProviderId])) return;
    try {
      localStorage.setItem(getHarnessMruStorageKey(workspaceId), selectedDraftProviderId);
    } catch {
      // ignore
    }
  }, [activeTaskId, providerOptions, selectedDraftProviderId, workspaceId]);

  useEffect(() => {
    if (!activeTaskId) return;
    setHarnessAuthModalProviderId(null);
    setPendingHarnessSelectionProviderId(null);
  }, [activeTaskId]);


  const ensureActiveSessionSelection = useCallback(
    (taskId: string, sessions: Array<{ session?: Session | null }>, preferredSessionId?: string | null) => {
      const activeTab = workbenchStore.getActiveTab();
      const prevSessionId =
        activeTab?.kind === "task" && activeTab.ref.taskId === taskId ? (activeTab.ref.sessionId ?? null) : null;
      const sessionList = sessions
        .map((s) => s.session)
        .filter((session): session is Session => Boolean(session));
      const nextSessionId = preferredSessionId
        ? preferredSessionId
        : pickPreferredSessionId(sessionList, prevSessionId ?? null);
      if (activeTab?.kind === "task" && activeTab.ref.taskId === taskId && nextSessionId !== prevSessionId) {
        workbenchStore.setActiveSessionForActiveTask(nextSessionId, { source: "system" });
      }
    },
    [workbenchStore],
  );

  useEffect(() => {
    if (!workspaceId) return;
    let cancelled = false;
    const loadWorkspace = async () => {
      const resp = await daemonFetchRaw(`/api/workspaces/${workspaceId}`);
      if (cancelled) return;
      if (resp.status === 404 || resp.status === 400) {
        navigate("/", { replace: true });
        return;
      }
      if (resp.status >= 200 && resp.status < 300 && resp.body) {
        try {
          setWorkspace(JSON.parse(resp.body) as Workspace);
          return;
        } catch {
          // ignore parse errors and fall through to null
        }
      }
      setWorkspace(null);
    };
    loadWorkspace().catch(() => setWorkspace(null));
    return () => {
      cancelled = true;
    };
  }, [navigate, workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    let cancelled = false;
    getHealth()
      .then((health) => {
        if (cancelled) return;
        const root = String(health.data_root ?? "").trim();
        setDaemonDataRoot(root || null);
      })
      .catch(() => {
        if (cancelled) return;
        setDaemonDataRoot(null);
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  const sessionSummaries = useMemo(() => activeTaskSummary?.sessions ?? [], [activeTaskSummary]);
  const sessions = useMemo(() => sessionSummaries.map((s) => s.session), [sessionSummaries]);
  const sessionIds = useMemo(
    () => sessions.map((session) => idToString(session.id)).filter(Boolean),
    [sessions],
  );
  const primarySessionId = useMemo(
    () => idToString(activeTaskSummary?.task.primary_session_id ?? ""),
    [activeTaskSummary?.task.primary_session_id],
  );
  const activeTaskSessionIds = useMemo(() => {
    if (primarySessionId) return [primarySessionId];
    return sessionIds;
  }, [primarySessionId, sessionIds]);

  const warmSessionIds = useMemo(() => {
    const ids: { id: string; updatedAt: number; running: boolean }[] = [];
    const activeSet = new Set(activeTaskSessionIds);
    for (const taskId of workspaceSnapshot.activeIds) {
      const task = tasksById[taskId];
      if (!task) continue;
      for (const sess of task.sessions) {
        const sid = idToString(sess.session.id);
        if (!sid || activeSet.has(sid)) continue;
        const last = parseMs(sess.last_message_at) ?? parseMs(sess.session.updated_at) ?? 0;
        const running = sess.session.status === "active" || sess.session.status === "running";
        ids.push({ id: sid, updatedAt: last, running });
      }
    }
    ids.sort((a, b) => {
      if (a.running !== b.running) return a.running ? -1 : 1;
      return b.updatedAt - a.updatedAt;
    });
    return ids.map((s) => s.id).slice(0, 20);
  }, [activeTaskSessionIds, tasksById, workspaceSnapshot.activeIds]);

  useEffect(() => {
    if (!activeTaskId) {
      return;
    }
    const snapshotReady = workspaceSnapshot.initialized && workspaceSnapshot.fetchState.active === "idle";
    if (!activeTaskSummary) {
      if (!snapshotReady) return;
      workbenchStore.setActiveSessionForActiveTask(null, { source: "system" });
      return;
    }
    if (sessionIds.length === 0 && !primarySessionId) {
      if (!snapshotReady) return;
      workbenchStore.setActiveSessionForActiveTask(null, { source: "system" });
      return;
    }
    ensureActiveSessionSelection(activeTaskId, sessionSummaries, primarySessionId || null);
  }, [
    activeTaskId,
    activeTaskSummary,
    ensureActiveSessionSelection,
    primarySessionId,
    sessionIds.length,
    sessionSummaries,
    workbenchStore,
    workspaceSnapshot.initialized,
    workspaceSnapshot.fetchState.active,
  ]);

  useEffect(() => {
    supervisor.setActiveTaskSessionIds(activeTaskSessionIds);
  }, [supervisor, activeTaskSessionIds]);

  useEffect(() => {
    workspaceSnapshotStore.setForegroundTaskId?.(activeTaskId ?? null);
  }, [workspaceSnapshotStore, activeTaskId]);

  useEffect(() => {
    supervisor.setWarmSessionIds(warmSessionIds);
  }, [supervisor, warmSessionIds]);

  const normalizedTaskQuery = taskQuery.trim().toLowerCase();
  const optimisticActiveSummaries = useMemo(() => {
    const activeIdSet = new Set(workspaceSnapshot.activeIds);
    return optimisticTasks.filter((item) => {
      const serverHasItem = !!tasksById[item.id];
      const activeHasItem = activeIdSet.has(item.id);
      if (item.localStatus === "synced" && serverHasItem && activeHasItem) return false;
      if (!normalizedTaskQuery) return true;
      return (item.task.title ?? "").toLowerCase().includes(normalizedTaskQuery);
    });
  }, [optimisticTasks, normalizedTaskQuery, tasksById, workspaceSnapshot.activeIds]);
  const filteredActiveIds = useMemo(() => {
    return workspaceSnapshot.activeIds.filter((id) => {
      const summary = tasksById[id];
      if (!summary) return false;
      if (!normalizedTaskQuery) return true;
      return (summary.task.title ?? "").toLowerCase().includes(normalizedTaskQuery);
    });
  }, [workspaceSnapshot.activeIds, tasksById, normalizedTaskQuery]);

  const filteredArchivedIds = useMemo(() => {
    return workspaceSnapshot.archivedIds.filter((id) => {
      const summary = tasksById[id];
      if (!summary) return false;
      if (!normalizedTaskQuery) return true;
      return (summary.task.title ?? "").toLowerCase().includes(normalizedTaskQuery);
    });
  }, [workspaceSnapshot.archivedIds, tasksById, normalizedTaskQuery]);

  const activeTaskSummaries = useMemo(() => {
    const optimisticIds = new Set(optimisticActiveSummaries.map((item) => item.id));
    const serverSummaries = filteredActiveIds
      .filter((id) => !optimisticIds.has(id))
      .map((id) => tasksById[id])
      .filter((v): v is WorkspaceActiveSnapshotItem => Boolean(v));
    if (optimisticActiveSummaries.length === 0) return serverSummaries;
    return [...optimisticActiveSummaries, ...serverSummaries];
  }, [filteredActiveIds, optimisticActiveSummaries, tasksById]);
  const archivedTaskSummaries = useMemo(
    () => filteredArchivedIds.map((id) => tasksById[id]).filter((v): v is WorkspaceActiveSnapshotItem => Boolean(v)),
    [filteredArchivedIds, tasksById],
  );
  const tasksForLiveInfo = useMemo(() => {
    const merged: Record<string, WorkspaceActiveSnapshotItem> = { ...tasksById };
    for (const item of optimisticTasks) {
      if (item.localStatus === "failed" || !merged[item.id]) {
        merged[item.id] = item;
      }
    }
    return merged;
  }, [optimisticTasks, tasksById]);

  const taskLiveInfo = useMemo(() => {
    const workingByTask = new Set<string>();
    const errorByTask = new Set<string>();
    const lastAssistantMsByTask: Record<string, number> = {};
    const entryBySessionId = new Map<string, SessionCacheEntry>();
    for (const entry of Object.values(sessionSnap.sessions)) {
      const sessionId = entry.session ? idToString(entry.session.id) : "";
      if (sessionId) entryBySessionId.set(sessionId, entry);
    }

    for (const summary of Object.values(tasksForLiveInfo)) {
      if (!summary) continue;
      const taskId = summary.id;
      const primarySessionId = summary.task.primary_session_id
        ? idToString(summary.task.primary_session_id)
        : "";
      // Left nav status must reflect the primary session only (subagents are ignored).
      const primarySessionSummary = primarySessionId
        ? summary.sessions.find((sessionSummary) => idToString(sessionSummary.session.id) === primarySessionId)
        : undefined;
      const primaryEntry = primarySessionId ? entryBySessionId.get(primarySessionId) : undefined;

      if (primarySessionSummary) {
        const isWorking = primarySessionSummary.activity?.is_working === true;
        if (isWorking) workingByTask.add(taskId);

        const status = primaryEntry?.session?.status ?? primarySessionSummary.session.status;
        if (status === "failed" || status === "cancelled") {
          errorByTask.add(taskId);
        }

        const liveMs = primaryEntry ? lastAssistantMessageMs(primaryEntry.messages) : null;
        const summaryMs = parseMs(primarySessionSummary.last_message_at ?? null);
        const ms =
          liveMs !== null && summaryMs !== null ? Math.max(liveMs, summaryMs) : liveMs ?? summaryMs;
        if (ms !== null) lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, ms);
      } else if (primaryEntry?.session) {
        const status = primaryEntry.session.status;
        if (status === "failed" || status === "cancelled") {
          errorByTask.add(taskId);
        }
        const ms = lastAssistantMessageMs(primaryEntry.messages);
        if (ms !== null) lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, ms);
      }
    }

    for (const entry of Object.values(sessionSnap.sessions)) {
      const session = entry.session;
      const taskId = session ? idToString(session.task_id) : "";
      if (!taskId || tasksForLiveInfo[taskId]) continue;
      if (session?.parent_session_id || session?.relationship === "sub_agent") continue;
      const status = session?.status;
      if (status === "failed" || status === "cancelled") errorByTask.add(taskId);
      const ms = lastAssistantMessageMs(entry.messages);
      if (ms !== null) lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, ms);
    }
    return { workingByTask, errorByTask, lastAssistantMsByTask };
  }, [sessionSnap.sessions, tasksForLiveInfo]);

  useEffect(() => {
    const detail: WorkbenchTaskIdleDetail = {
      allTasksIdle: taskLiveInfo.workingByTask.size === 0,
    };
    window.dispatchEvent(
      new CustomEvent<WorkbenchTaskIdleDetail>(WORKBENCH_TASK_IDLE_EVENT, {
        detail,
      }),
    );
  }, [taskLiveInfo.workingByTask.size]);

  useEffect(() => {
    if (optimisticTasks.length === 0) return;
    const shouldTrim = optimisticTasks.some((item) => {
      if (item.localStatus !== "synced") return false;
      const serverItem = tasksById[item.id];
      if (!serverItem) return false;
      const hasSession =
        (serverItem.sessions?.length ?? 0) > 0 || Boolean(serverItem.task.primary_session_id);
      return hasSession;
    });
    if (!shouldTrim) return;
    setOptimisticTasks((prev) => {
      const next = prev.filter((item) => {
        if (item.localStatus === "failed") return true;
        const serverItem = tasksById[item.id];
        if (!serverItem) return true;
        const hasSession =
          (serverItem.sessions?.length ?? 0) > 0 || Boolean(serverItem.task.primary_session_id);
        if (!hasSession) return true;
        return item.localStatus !== "synced";
      });
      return next.length === prev.length ? prev : next;
    });
  }, [optimisticTasks, tasksById]);

  const providerIdsByTaskFromSessions = useMemo(() => {
    const byTask: Record<string, Array<{ providerId: string; updatedAt: number }>> = {};
    for (const entry of Object.values(sessionSnap.sessions)) {
      const sess = entry.session;
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

  const markTaskReadInFlightRef = useRef<Record<string, Promise<void> | undefined>>({});

  const markTaskRead = useCallback(async (taskId: string) => {
    if (markTaskReadInFlightRef.current[taskId]) return;
    const p = (async () => {
      try {
        const updated = await markTaskReadApi(taskId);
        workspaceSnapshotStore.applyTaskUpdate(updated);
      } catch {
        // ignore
      }
    })().finally(() => {
      delete markTaskReadInFlightRef.current[taskId];
    });
    markTaskReadInFlightRef.current[taskId] = p;
    await p;
  }, [workspaceSnapshotStore]);

  const markTaskUnread = useCallback(async (taskId: string) => {
    try {
      const updated = await markTaskUnreadApi(taskId);
      workspaceSnapshotStore.applyTaskUpdate(updated);
    } catch {
      // ignore
    }
  }, [workspaceSnapshotStore]);

  const isTaskUnread = useCallback(
    (taskId: string): boolean => {
      const summary = tasksById[taskId];
      const t = summary?.task;
      if (!t) return false;
      const serverLastAssistantMs = parseMs(t.last_assistant_message_at ?? null);
      const liveLastAssistantMs = taskLiveInfo.lastAssistantMsByTask[taskId] ?? null;
      const lastAssistantMs =
        liveLastAssistantMs !== null && serverLastAssistantMs !== null
          ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
          : liveLastAssistantMs ?? serverLastAssistantMs;
      if (lastAssistantMs === null) return false;
      const seenMs = parseMs(t.assistant_seen_at ?? null);
      return seenMs === null || lastAssistantMs > seenMs;
    },
    [taskLiveInfo.lastAssistantMsByTask, tasksById],
  );

  useEffect(() => {
    if (!activeTaskId) return;
    const tid = activeTaskId;
    const taskSummary = tasksById[tid];
    const t = taskSummary?.task;
    if (!t) return;
    if (optimisticTasksById[tid]) return;
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
  }, [
    activeTaskId,
    markTaskRead,
    taskLiveInfo.lastAssistantMsByTask,
    taskLiveInfo.workingByTask,
    tasksById,
    optimisticTasksById,
  ]);

  useEnsureArchivedLoaded({
    archivedCollapsed,
    archivedLoaded: workspaceSnapshot.archivedLoaded,
    fetchState: workspaceSnapshot.fetchState.archived,
    activeInitialized: workspaceSnapshot.initialized,
    activeFetchState: workspaceSnapshot.fetchState.active,
    prefetchAfterActive: true,
    ensureArchivedLoaded: workspaceSnapshotStore.ensureArchivedLoaded,
  });

  const applyArchiveToggle = useCallback(
    async (taskId: string, nextArchived: boolean) => {
      const startNavToken = workbenchStore.getNavToken();
      setArchivePendingById((prev) => ({
        ...prev,
        [taskId]: nextArchived ? "archive" : "unarchive",
      }));
      try {
        const updated = nextArchived ? await archiveTask(taskId) : await unarchiveTask(taskId);
        workspaceSnapshotStore.applyTaskUpdate(updated);
        if (nextArchived) {
          const cleanupFailed = (updated as ArchiveTaskResponse).cleanup_failed;
          if (cleanupFailed) {
            setArchiveCleanupNotice(true);
          }
          const activeTab = workbenchStore.getActiveTab();
          const activeTaskIdNow = activeTab?.kind === "task" ? activeTab.ref.taskId : null;
          if (activeTaskIdNow === taskId && workbenchStore.getNavToken() === startNavToken) {
            workbenchStore.focusNewTask({ navToken: startNavToken, source: "system" });
          }
        }
      } finally {
        setArchivePendingById((prev) => {
          if (!(taskId in prev)) return prev;
          const next = { ...prev };
          delete next[taskId];
          return next;
        });
      }
    },
    [workbenchStore, workspaceSnapshotStore],
  );

  const onToggleArchive = useCallback(
    async (taskId: string, nextArchived: boolean, anchor?: AnchorRect | null) => {
      if (taskId in archivePendingById) return;
      if (!nextArchived || archiveConfirmDismissed) {
        if (!nextArchived) {
          setArchiveConfirm(null);
        }
        await applyArchiveToggle(taskId, nextArchived);
        return;
      }
      const normalized = normalizeAnchorRect(anchor);
      setArchiveConfirm({ taskId, anchor: normalized });
      setArchiveConfirmDontRemind(false);
    },
    [applyArchiveToggle, archiveConfirmDismissed, archivePendingById],
  );

  const confirmArchive = useCallback(async () => {
    if (!archiveConfirm) return;
    const taskId = archiveConfirm.taskId;
    if (taskId in archivePendingById) {
      setArchiveConfirm(null);
      return;
    }
    setArchiveConfirm(null);
    if (archiveConfirmDontRemind) {
      try {
        localStorage.setItem(ARCHIVE_CONFIRM_STORAGE_KEY, "1");
      } catch {
        // ignore
      }
      setArchiveConfirmDismissed(true);
    }
    await applyArchiveToggle(taskId, true);
  }, [applyArchiveToggle, archiveConfirm, archiveConfirmDontRemind, archivePendingById]);

  const cancelArchiveConfirm = useCallback(() => {
    setArchiveConfirm(null);
  }, []);

  const dismissArchiveCleanupNotice = useCallback(() => {
    setArchiveCleanupNotice(false);
  }, []);

  const dismissTranscriptNotice = useCallback(() => {
    setTranscriptNotice(null);
  }, []);

  const dismissDesktopStorageNotice = useCallback(() => {
    setDesktopStorageNotice(null);
  }, []);

  const archiveConfirmStyle = useMemo(() => {
    if (!archiveConfirm) return null;
    const rect = archiveConfirm.anchor;
    const margin = 12;
    const viewportW = typeof window === "undefined" ? 1200 : window.innerWidth;
    const viewportH = typeof window === "undefined" ? 800 : window.innerHeight;
    const width = Math.min(360, viewportW - margin * 2);
    const left = clampNum(rect.left + rect.width / 2 - width / 2, margin, viewportW - width - margin);
    const top = clampNum(rect.bottom + 10, margin, viewportH - 180);
    return { left, top, width };
  }, [archiveConfirm]);

  const archiveCleanupSnackbar = archiveCleanupNotice ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">Archived task, but some cleanup failed.</div>
        <div className="wb-snackbar-subtitle">
          Some worktree files were likely root-owned and could not be removed. Fix permissions and delete them manually if
          needed.
        </div>
      </div>
      <button type="button" className="wb-snackbar-close" onClick={dismissArchiveCleanupNotice} aria-label="Dismiss">
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

  const transcriptNoticeSnackbar = transcriptNotice ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">{transcriptNotice}</div>
      </div>
      <button type="button" className="wb-snackbar-close" onClick={dismissTranscriptNotice} aria-label="Dismiss">
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

  const desktopStorageNoticeSubtitle =
    desktopStorageNotice?.reason === "schema_mismatch"
      ? "Desktop detected an outdated local UI state format and reset local UI state."
      : "Desktop detected invalid local UI state data and reset local UI state.";
  const desktopStorageNoticeSnackbar = desktopStorageNotice ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">Local UI state was reset.</div>
        <div className="wb-snackbar-subtitle">{desktopStorageNoticeSubtitle}</div>
      </div>
      <button type="button" className="wb-snackbar-close" onClick={dismissDesktopStorageNotice} aria-label="Dismiss">
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

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

  const dismissOptimisticTask = useCallback(
    (taskId: string) => {
      const summary = optimisticTasksById[taskId];
      if (!summary) return;
      if (activeTaskId === taskId) {
        focusNewTask();
      }
      const sessionId = summary.primarySessionId ? String(summary.primarySessionId) : "";
      if (sessionId) {
        supervisor.dropSessionEntry(sessionId);
      }
      setOptimisticTasks((prev) => prev.filter((item) => item.id !== taskId));
    },
    [activeTaskId, focusNewTask, optimisticTasksById, supervisor],
  );

  const beginRenameTask = useCallback(
    (taskId: string) => {
      if (renamingTaskId && renamingTaskId !== taskId) {
        clearRenameDraft(renamingTaskId);
      }
      setRenamingTaskId(taskId);
    },
    [clearRenameDraft, renamingTaskId],
  );

  const cancelRenameTask = useCallback(() => {
    if (renamingTaskId) {
      clearRenameDraft(renamingTaskId);
    }
    setRenamingTaskId(null);
  }, [clearRenameDraft, renamingTaskId]);

  const commitRenameTask = useCallback(
    async (taskId: string, nextValue: string) => {
      const next = nextValue.trim();
      if (!next) {
        window.alert("Task title is required.");
        return;
      }
      const current = String(tasksById[taskId]?.task.title ?? "").trim();
      if (current && current === next) {
        clearRenameDraft(taskId);
        cancelRenameTask();
        return;
      }
      try {
        const updated = await updateTaskTitle(taskId, next);
        workspaceSnapshotStore.applyTaskUpdate(updated);
        clearRenameDraft(taskId);
        cancelRenameTask();
      } catch (e: unknown) {
        window.alert(errorMessage(e) || "Failed to rename.");
      }
    },
    [cancelRenameTask, clearRenameDraft, tasksById, workspaceSnapshotStore],
  );

  const renderTaskRow = useCallback(
    (summary: WorkspaceActiveSnapshotItem, opts?: { archived?: boolean }) => {
      const tid = summary.id;
      const t = summary.task;
      const optimistic = isOptimisticTask(summary) ? summary : null;
      const localStatus = optimistic?.localStatus ?? null;
      const selected = tid === activeTaskId;
      const hovered = tid === hoveredTaskId;
      const archived = !!opts?.archived;
      const pendingAction = archivePendingById[tid];
      const archivePending = typeof pendingAction !== "undefined";
      const title = t.title ?? "New Task";
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
      const ageIso = t.last_activity_at ?? t.updated_at ?? t.created_at;
	      let statusKind: "error" | "idle" | "archive" | "working" | "unread" = archivePending
	        ? "archive"
	        : hasError
	          ? "error"
	          : working
	            ? "working"
	            : unread
	              ? "unread"
	              : "idle";
      if (localStatus === "failed") {
        statusKind = "error";
      } else if (localStatus === "starting") {
        statusKind = "working";
      }
      const summaryProviders =
        summary.providerIds && summary.providerIds.length
          ? summary.providerIds
          : summary.sessions.map((s) => String(s.session.provider_id ?? "").trim()).filter(Boolean);
      const providerIds =
        (providerIdsByTaskFromSessions[tid] ?? []).length > 0 ? providerIdsByTaskFromSessions[tid] : summaryProviders;
      const providerCount = new Set(providerIds).size;
      const harnesses = providerIds
        .map((pid) => HARNESS_CATALOG.find((h) => h.id === pid))
        .filter(Boolean)
        .slice(0, 3) as Array<(typeof HARNESS_CATALOG)[number]>;
      const allowActions = !optimistic;
      const dismissHandler = localStatus === "failed" ? () => dismissOptimisticTask(tid) : undefined;
      const dismissText = localStatus === "failed" ? "Dismiss failed start" : undefined;

      return (
        <TaskRow
          key={tid}
          taskId={tid}
          title={title}
          archived={archived}
          archivePending={archivePending}
          archivePendingAction={pendingAction ?? null}
          statusKind={statusKind}
          selected={selected}
          hovered={hovered}
          isRenaming={renamingTaskId === tid}
          ageIso={ageIso}
          providerCount={providerCount}
          harnesses={harnesses}
          getRenameDraft={getRenameDraft}
          setRenameDraft={setRenameDraft}
          onFocusTask={focusTask}
          onOpenMenu={openTaskMenu}
          menuEnabled={allowActions}
          archiveEnabled={allowActions}
          onDismiss={dismissHandler}
          dismissLabel={dismissText}
          onToggleArchive={onToggleArchive}
          onHoverEnter={(id) => setHoveredTaskId(id)}
          onHoverLeave={(id) => setHoveredTaskId((current) => (current === id ? null : current))}
          onCancelRename={cancelRenameTask}
          onCommitRename={commitRenameTask}
        />
      );
    },
    [
      activeTaskId,
      archivePendingById,
      cancelRenameTask,
      commitRenameTask,
      dismissOptimisticTask,
      focusTask,
      getRenameDraft,
      hoveredTaskId,
      onToggleArchive,
      openTaskMenu,
      providerIdsByTaskFromSessions,
      renamingTaskId,
      setRenameDraft,
      setHoveredTaskId,
      taskLiveInfo.errorByTask,
      taskLiveInfo.lastAssistantMsByTask,
      taskLiveInfo.workingByTask,
    ],
  );

  const renderArchivedRow = useCallback(
    (summary: WorkspaceActiveSnapshotItem) => renderTaskRow(summary, { archived: true }),
    [renderTaskRow],
  );

  const taskListItems = useMemo<TaskListItem[]>(() => {
    const items: TaskListItem[] = [];
    activeTaskSummaries.forEach((summary) => items.push({ kind: "active-task", summary }));
    items.push({ kind: "archived-header" });
    if (!archivedCollapsed) {
      if (workspaceSnapshot.fetchState.archived === "error") {
        items.push({ kind: "archived-error" });
      }
      archivedTaskSummaries.forEach((summary) => items.push({ kind: "archived-task", summary }));
      if (
        archivedTaskSummaries.length === 0 &&
        workspaceSnapshot.archivedLoaded &&
        workspaceSnapshot.fetchState.archived !== "loading"
      ) {
        items.push({ kind: "archived-empty" });
      }
      if (workspaceSnapshot.fetchState.archived === "loading") {
        items.push({ kind: "archived-loading" });
      }
    }
    return items;
  }, [
    activeTaskSummaries,
    archivedCollapsed,
    archivedTaskSummaries,
    workspaceSnapshot.archivedLoaded,
    workspaceSnapshot.fetchState.active,
    workspaceSnapshot.fetchState.archived,
    workspaceSnapshot.initialized,
  ]);

  const activeSectionLastIndex = useMemo(() => {
    if (activeTaskSummaries.length > 0) {
      return activeTaskSummaries.length - 1;
    }
    if (workspaceSnapshot.initialized && workspaceSnapshot.fetchState.active !== "loading") {
      return 0;
    }
    return -1;
  }, [activeTaskSummaries.length, workspaceSnapshot.fetchState.active, workspaceSnapshot.initialized]);

  const renderTaskListItem = useCallback(
    (item: TaskListItem) => {
      switch (item.kind) {
        case "active-task":
          return renderTaskRow(item.summary);
        case "archived-header":
          return (
            <div className="wb-section-header wb-section-header-archived">
              <button
                type="button"
                className="wb-section-toggle"
                onClick={() => {
                  const next = !archivedCollapsed;
                  setArchivedCollapsed(next);
                  if (!next) {
                    workspaceSnapshotStore.ensureArchivedLoaded();
                  }
                }}
                aria-expanded={!archivedCollapsed}
              >
                <span className="wb-section-title">Archived Tasks</span>
                <span className={`wb-section-chev ${archivedCollapsed ? "wb-section-chev-collapsed" : ""}`}>
                  <ChevronDown size={14} />
                </span>
              </button>
            </div>
          );
        case "archived-loading":
          return (
            <div className="wb-archived-loading" aria-live="polite" aria-label="Loading archived tasks">
              <span className="wb-archived-spinner" aria-hidden="true" />
            </div>
          );
        case "archived-error":
          return <div className="wb-muted">Failed to load archived tasks. Retry.</div>;
        case "archived-empty":
          return <div className="wb-muted">No archived tasks.</div>;
        case "archived-task":
          return renderArchivedRow(item.summary);
        default:
          return null;
      }
    },
    [archivedCollapsed, renderArchivedRow, renderTaskRow, workspaceSnapshotStore],
  );

  const computeTaskListItemKey = useCallback((_: number, item: TaskListItem) => {
    switch (item.kind) {
      case "active-task":
        return `active-${item.summary.id}`;
      case "archived-task":
        return `archived-${item.summary.id}`;
      case "archived-header":
        return "archived-header";
      case "archived-loading":
        return "archived-loading";
      case "archived-error":
        return "archived-error";
      case "archived-empty":
        return "archived-empty";
      default:
        return "unknown";
    }
  }, []);

  const onTaskListRangeChanged = useCallback(
    (range: { startIndex: number; endIndex: number }) => {
      if (!workspaceSnapshot.hasMoreActive) return;
      if (workspaceSnapshot.fetchState.active === "loading") return;
      if (activeSectionLastIndex < 0) return;
      if (range.endIndex < activeSectionLastIndex) return;
      workspaceSnapshotStore.loadMoreActive();
    },
    [activeSectionLastIndex, workspaceSnapshot.fetchState.active, workspaceSnapshot.hasMoreActive, workspaceSnapshotStore],
  );

  const { onTaskListScroll, onTaskListScrollerChange } = useWorkbenchTaskScrollbar({
    itemCount: taskListItems.length,
  });

  const loadMoreArchived = useCallback(() => {
    workspaceSnapshotStore.loadMoreArchived();
  }, [workspaceSnapshotStore]);

  const taskListContext = useMemo<TaskListContext>(
    () => ({
      archivedCollapsed,
      archivedFetchState: workspaceSnapshot.fetchState.archived,
      hasMoreArchived: workspaceSnapshot.hasMoreArchived,
      onLoadMoreArchived: loadMoreArchived,
      onScroll: onTaskListScroll,
      onScrollerChange: onTaskListScrollerChange,
    }),
    [
      archivedCollapsed,
      onTaskListScroll,
      onTaskListScrollerChange,
      loadMoreArchived,
      workspaceSnapshot.fetchState.archived,
      workspaceSnapshot.hasMoreArchived,
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
      } catch (e: unknown) {
        window.alert(errorMessage(e) || "Failed to delete task.");
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
    if (primarySessionId) return primarySessionId;
    if (activeSessionIdFromTabResolved) return activeSessionIdFromTabResolved;
    return pickPreferredSessionId(sessions, null);
  }, [activeSessionIdFromTabResolved, primarySessionId, sessions]);
  const activeTaskArchived = Boolean(activeTaskSummary?.task?.archived_at);
  const preserveScrollOnFocus = true;
  // `optimisticSessionIdSet` is derived from React state, but on the first tick of "New Task" we can
  // temporarily focus an optimistic session before the optimistic task/session summary is committed.
  // Avoid opening that transient id until the optimistic summary is committed.
  const optimisticStartingSessionId = String(optimisticStartingTaskRef.current?.primarySessionId ?? "");
  const isOptimisticSessionId =
    !!activeSessionId &&
    (optimisticSessionIdSet.has(activeSessionId) ||
      activeSessionId.startsWith("optimistic-") ||
      (optimisticStartingSessionId && optimisticStartingSessionId === activeSessionId));
  const openSessionId = activeSessionId && !isOptimisticSessionId ? activeSessionId : "";
  useOpenSession(openSessionId, { watchDiff: diffOpen, mode: activeTaskArchived ? "archived" : "active" });

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
    return `task:${short(activeTaskId)} session:${short(activeSessionId)}`;
  }, [activeTaskId, activeSessionId]);

  const sessionCache = useSessionCacheSnapshot();
  const activeEntry = useSessionEntry(activeSessionId ?? "");
  const activeSessionDiff = activeEntry?.diff ?? "";
  const activeDiffContentError = activeSessionId ? diffContentErrorBySessionId[activeSessionId] ?? null : null;
  const activeWorktreeId = activeEntry?.session ? idToString(activeEntry.session.worktree_id) : "";
  const activeWorktreeVcsSnapshot = activeWorktreeId
    ? workspaceSnapshot.worktreeVcsById?.[activeWorktreeId] ?? null
    : null;
  const activeWorktreeVcsSummary: Record<string, unknown> | null = useMemo(() => {
    if (!activeWorktreeVcsSnapshot) return null;
    return { ...activeWorktreeVcsSnapshot.summary };
  }, [activeWorktreeVcsSnapshot]);
  const activeWorktreeVcsComputeState = activeWorktreeVcsSnapshot?.compute_state ?? null;
  const activeWorktreeDiffAvailable = activeWorktreeVcsSnapshot?.available !== false;
  const activeWorktreeDiffUnavailableReason = activeWorktreeVcsSnapshot?.unavailable_reason ?? null;
  const snapshotSummaryStats = useMemo(
    () => getDiffSummaryStats(activeWorktreeVcsSummary),
    [activeWorktreeVcsSummary],
  );
  const snapshotHasCounts =
    snapshotSummaryStats.fileCount !== null || snapshotSummaryStats.lineCount !== null;
  const diffSummary = activeWorktreeDiffAvailable && snapshotHasCounts ? activeWorktreeVcsSummary : null;
  const diffSummaryError =
    activeWorktreeDiffAvailable && activeWorktreeVcsComputeState === "error"
      ? "Failed to compute diff summary."
      : null;
  const diffSummaryLoading =
    !diffSummaryError && activeWorktreeDiffAvailable && (!activeWorktreeVcsSnapshot || !snapshotHasCounts);
  const diffLoading = diffSummaryLoading || diffContentLoading;
  const [webSessions, setWebSessions] = useState<WebSessionInfo[]>([]);
  const [webSessionsLoading, setWebSessionsLoading] = useState(false);
  const [activeWebSessionId, setActiveWebSessionId] = useState<string | null>(null);
  const [activeSessionKind, setActiveSessionKind] = useState("web");
  // TODO: Re-enable web sessions once the feature is ready to ship again.
  const webSessionsEnabled = false;
  const lastSessionSwitchRef = useRef<string | null>(null);
  const loadTestTelemetry = getLoadTestTelemetry();

  useEffect(() => {
    if (!loadTestTelemetry?.enabled) return;
    const nextId = activeSessionId ?? null;
    if (lastSessionSwitchRef.current === nextId) return;
    loadTestTelemetry.startSessionSwitch(lastSessionSwitchRef.current, nextId);
    lastSessionSwitchRef.current = nextId;
  }, [activeSessionId, loadTestTelemetry]);

  useEffect(() => {
    if (!loadTestTelemetry?.enabled) return;
    if (!activeSessionId || !activeEntry || activeEntry.loading) return;
    loadTestTelemetry.finishSessionSwitch(activeSessionId);
  }, [activeEntry?.loading, activeEntry?.updatedAtMs, activeSessionId, loadTestTelemetry]);

  useEffect(() => {
    if (!activeWorktreeId) {
      setActiveWorktree(null);
      return;
    }
    const cached = worktreeCacheRef.current.get(activeWorktreeId);
    if (cached && (!activeTaskArchived || cached.base_commit_sha)) {
      setActiveWorktree(cached);
      return;
    }
    if (!activeTaskArchived) {
      const cachedRoot = workspaceSnapshotStore.getWorktreeRoot(activeWorktreeId);
      const derivedRoot = cachedRoot || deriveManagedWorktreeRoot(daemonDataRoot, workspaceId, activeWorktreeId);
      if (derivedRoot) {
        const derived: Worktree = {
          id: activeWorktreeId,
          workspace_id: workspaceId,
          root_path: derivedRoot,
          base_commit_sha: "",
          created_at: "",
        };
        worktreeCacheRef.current.set(activeWorktreeId, derived);
        setActiveWorktree(derived);
      } else {
        setActiveWorktree(null);
      }
      return;
    }

    let cancelled = false;
    const existing = worktreeFetchRef.current.get(activeWorktreeId);
    const fetchPromise =
      existing ??
      getWorktree(activeWorktreeId)
        .then((wt) => {
          worktreeCacheRef.current.set(activeWorktreeId, wt);
          return wt;
        })
        .catch(() => null)
        .finally(() => {
          worktreeFetchRef.current.delete(activeWorktreeId);
        });
    worktreeFetchRef.current.set(activeWorktreeId, fetchPromise);
    fetchPromise
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
  }, [
    activeTaskArchived,
    activeWorktreeId,
    daemonDataRoot,
    workspaceId,
    workspaceSnapshot,
    workspaceSnapshotStore,
  ]);

  const refreshWebSessions = useCallback(async () => {
    if (!activeSessionId) {
      setWebSessions([]);
      setWebSessionsLoading(false);
      return;
    }
    setWebSessionsLoading(true);
    try {
      const sessions = await listWebSessions();
      const filtered = sessions.filter(
        (session) =>
          session.session_id === activeSessionId && String(session.status).toLowerCase() === "running",
      );
      setWebSessions(filtered);
    } catch {
      setWebSessions([]);
    } finally {
      setWebSessionsLoading(false);
    }
  }, [activeSessionId]);

  useEffect(() => {
    // TODO: Re-enable web sessions polling/refresh when the feature returns.
    /*
    let cancelled = false;
    const run = async () => {
      if (cancelled) return;
      await refreshWebSessions();
    };
    void run();
    if (!activeSessionId) return () => {};
    const timer = window.setInterval(() => void run(), 10000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
    */
  }, [activeSessionId, refreshWebSessions]);

  useEffect(() => {
    if (webSessions.length === 0) {
      setActiveWebSessionId(null);
      return;
    }
    if (!activeWebSessionId || !webSessions.some((session) => session.id === activeWebSessionId)) {
      setActiveWebSessionId(webSessions[0].id);
    }
  }, [activeWebSessionId, webSessions]);

  const daemonBaseUrl = useDaemonBaseUrl() ?? "";
  const sessionSections = useMemo(() => {
    if (!webSessionsEnabled) return [];
    return [
      {
        key: "web",
        label: "Web Sessions",
        sessions: webSessions,
      },
    ];
  }, [webSessions, webSessionsEnabled]);

  useEffect(() => {
    if (!sessionSections.length) return;
    if (!sessionSections.some((section) => section.key === activeSessionKind)) {
      setActiveSessionKind(sessionSections[0].key);
    }
  }, [activeSessionKind, sessionSections]);

  const diffSummaryStats = useMemo(() => getDiffSummaryStats(diffSummary), [diffSummary]);
  const diffSummaryCount = diffSummaryStats.fileCount;
  const diffTooLarge = useMemo(() => isDiffSummaryTooLarge(diffSummary), [diffSummary]);
  const diffTooLargeLabel = useMemo(() => {
    if (!diffTooLarge) return null;
    const details: string[] = [];
    if (diffSummaryStats.fileCount !== null) details.push(`${diffSummaryStats.fileCount} files`);
    if (diffSummaryStats.lineCount !== null) details.push(`${diffSummaryStats.lineCount} lines`);
    const suffix = details.length > 0 ? ` (${details.join(", ")})` : "";
    return `Diff too large to display${suffix}.`;
  }, [diffSummaryStats, diffTooLarge]);

  const diffSummaryReady = snapshotHasCounts || diffSummary !== null || diffSummaryError !== null;
  const diffUnavailableLabel =
    activeWorktreeDiffAvailable
      ? null
      : activeWorktreeDiffUnavailableReason === "no_repo"
        ? "No git repo detected for this workspace yet."
        : activeWorktreeDiffUnavailableReason === "no_target_branch"
          ? "Primary branch is not configured for this workspace."
        : "Diff unavailable for this workspace.";
  const diffHasChanges =
    diffSummaryCount !== null
      ? diffSummaryCount > 0
      : diffSummaryStats.lineCount !== null
        ? diffSummaryStats.lineCount > 0
        : false;
  const hasDiff = activeWorktreeDiffAvailable
    ? diffSummaryError !== null
      ? true
      : diffSummaryReady
        ? diffHasChanges
        : false
    : false;
  const diffEmptyLabel =
    diffUnavailableLabel ??
    (diffLoading || !diffSummaryReady ? "Loading changes..." : "No changes on this worktree.");
  const diffBadgeCount = useMemo(() => {
    if (!activeWorktreeDiffAvailable) return 0;
    if (!snapshotHasCounts) return 0;
    if (snapshotSummaryStats.fileCount !== null) return Math.max(0, snapshotSummaryStats.fileCount);
    if (snapshotSummaryStats.lineCount !== null) return Math.max(0, snapshotSummaryStats.lineCount);
    return 0;
  }, [activeWorktreeDiffAvailable, snapshotHasCounts, snapshotSummaryStats]);
  const gitStatusSignature = useMemo(() => {
    if (activeWorktreeVcsSnapshot) {
      return [
        `base:${String(activeWorktreeVcsSnapshot.base_commit_sha ?? "")}`,
        `head:${String(activeWorktreeVcsSnapshot.head_commit_sha ?? "")}`,
        `available:${activeWorktreeVcsSnapshot.available === false ? "0" : "1"}`,
        `reason:${String(activeWorktreeVcsSnapshot.unavailable_reason ?? "")}`,
        `files:${String(snapshotSummaryStats.fileCount ?? "")}`,
        `adds:${String(snapshotSummaryStats.additions ?? "")}`,
        `dels:${String(snapshotSummaryStats.deletions ?? "")}`,
      ].join("|");
    }
    return "";
  }, [activeWorktreeVcsSnapshot, snapshotSummaryStats]);
  const showReviewPane = diffOpen;
  const showArtifactsPane = artifactsOpen;
  const showSessionsPane = webSessionsEnabled && sessionsOpen;
  const rightPaneOpen = showReviewPane || showArtifactsPane || showSessionsPane;
  const artifacts = useMemo(() => {
    if (!activeSessionId) return [];
    return sessionCache.sessions[activeSessionId]?.artifacts ?? [];
  }, [activeSessionId, sessionCache.sessions]);
  const artifactsLoading = activeSessionId
    ? sessionCache.sessions[activeSessionId]?.artifactsLoading ?? false
    : false;
  const artifactsCount = artifacts.length;

  useEffect(() => {
    if (!artifactsOpen || !activeSessionId) return;
    supervisor.loadArtifacts(activeSessionId);
  }, [activeSessionId, artifactsOpen, supervisor]);

  useEffect(() => {
    artifactPrefetcher.prefetch(activeSessionId ?? null, artifacts, !activeTaskArchived);
  }, [activeSessionId, activeTaskArchived, artifacts]);
  const sessionsCount = webSessionsEnabled ? webSessions.length : 0;
  const diffContentInFlightRef = useRef<Map<string, Promise<void>>>(new Map());
  const diffRefreshTimerRef = useRef<number | null>(null);
  const diffRefreshSignatureRef = useRef<Map<string, string>>(new Map());

  const refreshDiff = useCallback(
    async (sessionId: string) => {
      if (!sessionId) return;
      const optimisticStartingSessionId = String(optimisticStartingTaskRef.current?.primarySessionId ?? "");
      if (optimisticSessionIdSet.has(sessionId) || (optimisticStartingSessionId && optimisticStartingSessionId === sessionId)) {
        return;
      }
      if (sessionId !== activeSessionId) return;
      if (!activeWorktreeDiffAvailable) {
        setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
        supervisor.setDiff(sessionId, "");
        return;
      }
      // If we can't get a summary, do not fetch the full diff (it can be huge and crash the renderer).
      if (!snapshotHasCounts || !activeWorktreeVcsSummary) {
        setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
        supervisor.setDiff(sessionId, "");
        return;
      }
      if (isDiffSummaryTooLarge(activeWorktreeVcsSummary)) {
        setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
        supervisor.setDiff(sessionId, "");
        return;
      }
      const existing = diffContentInFlightRef.current.get(sessionId);
      if (existing) return existing;
      setDiffContentLoading(true);
      setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
      const request = (async () => {
        try {
          const resp = await getSessionDiff(sessionId);
          if (resp.available === false) {
            supervisor.setDiff(sessionId, "");
            setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
            return;
          }
          supervisor.setDiff(sessionId, resp.diff ?? "");
          setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
        } catch (err: unknown) {
          supervisor.setDiff(sessionId, "");
          const detail = errorMessage(err);
          const msg = detail ? `Failed to load diff content: ${detail}` : "Failed to load diff content.";
          setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: msg }));
        }
      })().finally(() => {
        diffContentInFlightRef.current.delete(sessionId);
        setDiffContentLoading(false);
      });
      diffContentInFlightRef.current.set(sessionId, request);
      return request;
    },
    [
      activeSessionId,
      activeWorktreeDiffAvailable,
      activeWorktreeVcsSummary,
      optimisticSessionIdSet,
      snapshotHasCounts,
      supervisor,
    ],
  );

  const toggleDiffPane = useCallback((source: "header_button" | "menu_command" | "unknown" = "unknown") => {
    const nextMode = rightPaneModeRef.current === "diff" ? null : "diff";
    rightPaneModeRef.current = nextMode;
    setRightPaneMode(nextMode);
    trackWorkbenchPanelToggled({
      panelKey: "diff",
      open: nextMode === "diff",
      source,
    });
  }, []);

  const toggleArtifactsPane = useCallback((source: "header_button" | "menu_command" | "unknown" = "unknown") => {
    setArtifactsOpenSeeded(true);
    setArtifactsAutoOpenPending(false);
    const nextMode = rightPaneModeRef.current === "artifacts" ? null : "artifacts";
    rightPaneModeRef.current = nextMode;
    setRightPaneMode(nextMode);
    trackWorkbenchPanelToggled({
      panelKey: "artifacts",
      open: nextMode === "artifacts",
      source,
    });
  }, []);

  const toggleSessionsPane = useCallback((source: "header_button" | "menu_command" | "unknown" = "unknown") => {
    setArtifactsOpenSeeded(true);
    setArtifactsAutoOpenPending(false);
    const nextMode = rightPaneModeRef.current === "sessions" ? null : "sessions";
    rightPaneModeRef.current = nextMode;
    setRightPaneMode(nextMode);
    trackWorkbenchPanelToggled({
      panelKey: "sessions",
      open: nextMode === "sessions",
      source,
    });
  }, []);

  const diffPaneScope = useMemo(() => {
    if (activeSessionId) return `session:${activeSessionId}`;
    return null;
  }, [activeSessionId]);

  const artifactsPaneScope = useMemo(() => {
    return activeSessionId ?? null;
  }, [activeSessionId]);

  const sessionsPaneScope = useMemo(() => {
    return activeSessionId ?? null;
  }, [activeSessionId]);

  useEffect(() => {
    setDiffOpenHydrated(false);
    if (!workspaceId || !diffPaneScope) {
      setRightPaneMode((mode) => (mode === "diff" ? null : mode));
      setDiffOpenHydrated(true);
      return;
    }
    setRightPaneMode((mode) => (mode === "diff" ? null : mode));
    setDiffOpenHydrated(true);
  }, [workspaceId, diffPaneScope]);

  useEffect(() => {
    setArtifactsOpenHydrated(false);
    setArtifactsOpenSeeded(false);
    setArtifactsAutoOpenPending(false);
    if (!workspaceId || !artifactsPaneScope) {
      setRightPaneMode((mode) => (mode === "artifacts" ? null : mode));
      setArtifactsOpenHydrated(true);
      artifactsPaneScopeRef.current = null;
      return;
    }
    setRightPaneMode((mode) => (mode === "artifacts" ? null : mode));
    setArtifactsOpenSeeded(true);
    setArtifactsAutoOpenPending(false);
    setArtifactsOpenHydrated(true);
    artifactsPaneScopeRef.current = artifactsPaneScope;
  }, [workspaceId, artifactsPaneScope]);

  useEffect(() => {
    setSessionsOpenHydrated(false);
    if (!workspaceId || !sessionsPaneScope) {
      setRightPaneMode((mode) => (mode === "sessions" ? null : mode));
      setSessionsOpenHydrated(true);
      sessionsPaneScopeRef.current = null;
      return;
    }
    setRightPaneMode((mode) => (mode === "sessions" ? null : mode));
    setSessionsOpenHydrated(true);
    sessionsPaneScopeRef.current = sessionsPaneScope;
  }, [workspaceId, sessionsPaneScope]);

  useEffect(() => {
    if (!diffOpenHydrated) return;
    if (!workspaceId || !diffPaneScope) return;
    saveWorkbenchDiffPaneOpenV1(workspaceId, diffPaneScope, diffOpen).catch(() => {});
  }, [diffOpen, diffOpenHydrated, workspaceId, diffPaneScope]);

  useEffect(() => {
    if (!artifactsOpenHydrated || !artifactsOpenSeeded) return;
    if (!workspaceId || !artifactsPaneScope) return;
    if (artifactsPaneScopeRef.current !== artifactsPaneScope) return;
    saveWorkbenchArtifactsPaneOpenV1(workspaceId, artifactsPaneScope, artifactsOpen).catch(() => {});
  }, [artifactsOpen, artifactsOpenHydrated, artifactsOpenSeeded, workspaceId, artifactsPaneScope]);

  useEffect(() => {
    if (!sessionsOpenHydrated) return;
    if (!workspaceId || !sessionsPaneScope) return;
    if (sessionsPaneScopeRef.current !== sessionsPaneScope) return;
    saveWorkbenchSessionsPaneOpenV1(workspaceId, sessionsPaneScope, sessionsOpen).catch(() => {});
  }, [sessionsOpen, sessionsOpenHydrated, workspaceId, sessionsPaneScope]);

  useEffect(() => {
    setTerminalOpenHydrated(false);
    if (!workspaceId) {
      setTerminalOpen(false);
      setTerminalOpenHydrated(true);
      return;
    }
    let cancelled = false;
    loadWorkbenchTerminalPanelOpenV1(workspaceId)
      .then((state) => {
        if (cancelled) return;
        if (state) {
          setTerminalOpen(state.open);
          setTerminalHeight(clampTerminalHeight(state.height));
        } else {
          setTerminalOpen(false);
        }
        setTerminalOpenHydrated(true);
      })
      .catch(() => {
        if (cancelled) return;
        setTerminalOpen(false);
        setTerminalOpenHydrated(true);
      });
    return () => {
      cancelled = true;
    };
  }, [clampTerminalHeight, workspaceId]);

  useEffect(() => {
    if (!terminalOpenHydrated) return;
    if (!workspaceId) return;
    saveWorkbenchTerminalPanelOpenV1(workspaceId, {
      v: 1,
      open: terminalOpen,
      height: terminalHeight,
    }).catch(() => {});
  }, [terminalHeight, terminalOpen, terminalOpenHydrated, workspaceId]);

  useEffect(() => {
    if (!terminalOpen) return;
    const id = window.requestAnimationFrame(() => terminalPanelRef.current?.focusActive());
    return () => window.cancelAnimationFrame(id);
  }, [terminalOpen]);

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
    if (!draftHarness) return "Select a harness to start.";
    const missing =
      !(providersById[draftHarness.providerId]?.installed === true && providersById[draftHarness.providerId]?.health === "ok");
    if (missing) {
      const diag = providersById[draftHarness.providerId]?.diagnostics?.[0];
      return diag
        ? `Harness “${draftHarness.providerId}” unavailable: ${diag}`
        : `Harness “${draftHarness.providerId}” unavailable.`;
    }
    return null;
  }, [draftHarness, draftPrompt, startBusy, providersById]);

  const startNewTask = async () => {
    if (!workspaceId) return;
    const prompt = (dictationRecording ? await stopDictation({ awaitFinal: true }) : draftPrompt).trim();
    if (!prompt) return;
    if (startBusy) return;
    if (startBlockedReason && !startBlockedReason.startsWith("Starting")) {
      setStartError(startBlockedReason);
      return;
    }
    setStartBusy(true);
    setStartError(null);

    const nowIso = new Date().toISOString();
    const title = deriveTaskTitle(prompt);
    const attachmentsToSend = draftAttachments.slice();
    const primaryTrack = draftHarness;
    if (!primaryTrack) {
      setStartBusy(false);
      setStartError("Select a harness to start.");
      return;
    }
    const optimisticTaskId = randomUuid();
    const optimisticSessionId = randomUuid();
    const optimisticMessageId = randomUuid();
    const optimisticTurnId = randomUuid();
    const optimisticModelId =
      primaryTrack.modelId ||
      modelIdsFromOptions(providerOptions[primaryTrack.providerId])[0] ||
      (primaryTrack.providerId === "fake" ? "fake-model" : "default");

    const optimisticTask: Task = {
      id: optimisticTaskId,
      workspace_id: workspaceId,
      title,
      status: "running",
      primary_session_id: optimisticSessionId,
      created_at: nowIso,
      updated_at: nowIso,
      last_activity_at: nowIso,
      has_active_session: true,
    };

    const optimisticSession: Session = {
      id: optimisticSessionId,
      task_id: optimisticTaskId,
      workspace_id: workspaceId,
      worktree_id: "",
      provider_id: primaryTrack.providerId,
      model_id: optimisticModelId,
      title: "Session 1",
      agent_role: "assistant",
      status: "starting",
      env_target: "worktree",
      created_at: nowIso,
      updated_at: nowIso,
    };

    const optimisticSummary: SessionSnapshotSummary = {
      session: optimisticSession,
      last_message_at: nowIso,
      last_message_preview: prompt.slice(0, 160),
      activity: { is_working: true, last_turn_status: "running" },
      unread: false,
    };

    const optimisticItem: OptimisticTaskSummary = {
      id: optimisticTaskId,
      task: optimisticTask,
      sessions: [optimisticSummary],
      primarySessionHead: null,
      primarySessionId: optimisticSessionId,
      sort_at: nowIso,
      sortAtMs: Date.parse(nowIso) || Date.now(),
      providerIds: [primaryTrack.providerId],
      localStatus: "starting",
      localPrompt: prompt,
      localMessageId: optimisticMessageId,
    };

    const optimisticMessage: Message = buildOptimisticUserMessage({
      messageId: optimisticMessageId,
      sessionId: optimisticSessionId,
      taskId: optimisticTaskId,
      turnId: optimisticTurnId,
      content: prompt,
      attachments: attachmentsToSend,
      delivery: "immediate",
      createdAt: nowIso,
    });
    const optimisticTurn: SessionTurn = {
      turn_id: optimisticTurnId,
      session_id: optimisticSessionId,
      run_id: null,
      user_message_id: optimisticMessageId,
      status: "running",
      start_seq: null,
      end_seq: null,
      started_at: nowIso,
      updated_at: nowIso,
      assistant_partial: null,
      thought_partial: null,
      metrics_json: null,
      tool_total: 0,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 0,
      tool_failed: 0,
    };

    // Keep the "switch to task tab" + optimistic seed in the same sync commit to avoid a transient
    // render where the task pane is focused but has no session/thread data yet.
    flushSync(() => {
      optimisticStartingTaskRef.current = optimisticItem;
      setOptimisticTasks((prev) => [optimisticItem, ...prev]);
      focusTask(optimisticTaskId, optimisticSessionId);
      setOptimisticFocus({
        taskId: optimisticTaskId,
        sessionId: optimisticSessionId,
        navToken: workbenchStore.getNavToken(),
      });
      supervisor.setSession(optimisticSession);
      supervisor.setTurns(optimisticSessionId, [optimisticTurn], { replace: true });
      supervisor.setMessages(optimisticSessionId, [optimisticMessage], { replace: true });
    });

    setNewTaskDraft({ text: "", modeId: "default" });
    await workbenchStore.flushDraft(NEW_TASK_DRAFT_KEY);
    setDraftAttachments([]);

    let currentTaskId = optimisticTaskId;
    let currentSessionId = optimisticSessionId;
    let primaryMessagePosted = false;

    try {
      const task = await createTask(workspaceId, title, undefined, {
        create_default_session: false,
        id: optimisticTaskId,
      });
      const taskId = idToString(task.id);
      if (!taskId) throw new Error("Task creation failed.");

      if (taskId !== optimisticTaskId) {
        throw new Error("Task creation returned an unexpected id.");
      }

      currentTaskId = taskId;
      setOptimisticTasks((prev) =>
        prev.map((item) => {
          if (item.id !== currentTaskId) return item;
          const nextTask: Task = { ...task, primary_session_id: item.primarySessionId ?? null };
          const nextSessions = item.sessions.map((summary) => ({
            ...summary,
            session: {
              ...summary.session,
              task_id: currentTaskId,
              workspace_id: task.workspace_id ?? summary.session.workspace_id,
            },
          }));
          return {
            ...item,
            task: nextTask,
            sessions: nextSessions,
            sort_at: task.created_at ?? item.sort_at,
            sortAtMs: Date.parse(task.created_at ?? item.sort_at ?? "") || item.sortAtMs,
          };
        }),
      );

      const dt = primaryTrack;
      const installed = providersById[dt.providerId]?.installed === true && providersById[dt.providerId]?.health === "ok";
      if (!installed) {
        const diag = providersById[dt.providerId]?.diagnostics?.[0];
        throw new Error(
          diag ? `Harness “${dt.providerId}” unavailable: ${diag}` : `Harness “${dt.providerId}” unavailable.`,
        );
      }
      const env_target = "worktree";
      const opts = await ensureProviderAuthSummary(dt.providerId).catch(() => undefined);
      const modelIds = modelIdsFromOptions(opts ?? providerOptions[dt.providerId]);
      const modelId = dt.modelId || modelIds[0] || (dt.providerId === "fake" ? "fake-model" : "default");
      const clientSessionId = optimisticSessionId;
      const messageId = optimisticMessageId;
      const turnId = optimisticTurnId;
      const shouldSendInitialPrompt = attachmentsToSend.length === 0;
      const session = await createSession(currentTaskId, dt.providerId, modelId, {
        env_target,
        id: clientSessionId,
        initial_message_id: messageId,
        initial_turn_id: turnId,
        ...(shouldSendInitialPrompt ? { initial_prompt: prompt } : {}),
      });
      const sessionId = idToString(session.id);
      if (!sessionId) throw new Error("Session creation failed.");
      if (sessionId !== clientSessionId) {
        throw new Error("Session creation returned an unexpected id.");
      }
      currentSessionId = sessionId;
      supervisor.setSession(session);
      setOptimisticTasks((prev) =>
        prev.map((item) => {
          if (item.id !== currentTaskId) return item;
          const nextSessions = item.sessions.map((summary) => {
            if (idToString(summary.session.id) !== sessionId) return summary;
            const nextSummary: SessionSnapshotSummary = {
              ...summary,
              session,
              last_message_at: nowIso,
              last_message_preview: summary.last_message_preview ?? prompt.slice(0, 160),
              activity: { is_working: true, last_turn_status: "running" },
            };
            return nextSummary;
          });
          return {
            ...item,
            sessions: nextSessions,
            primarySessionId: sessionId,
            task: { ...item.task, primary_session_id: sessionId },
          };
        }),
      );

      if (shouldSendInitialPrompt) {
        primaryMessagePosted = true;
      } else {
        const posted = await postMessage(sessionId, prompt, "immediate", attachmentsToSend, {
          id: messageId,
          turn_id: turnId,
        });
        supervisor.setMessages(sessionId, [posted]);
        primaryMessagePosted = true;
      }
      setOptimisticTasks((prev) =>
        prev.map((item) =>
          item.id === currentTaskId && item.localStatus === "starting"
            ? { ...item, localStatus: "synced" }
            : item,
        ),
      );

      if (!primaryMessagePosted) {
        throw new Error("Failed to start the first session.");
      }
    } catch (e: unknown) {
      const message = errorMessage(e);
      setOptimisticTasks((prev) =>
        prev.map((item) =>
          item.id === currentTaskId ? { ...item, localStatus: "failed", localError: message } : item,
        ),
      );
      setStartError(message);
    } finally {
      setStartBusy(false);
    }
  };

  useEffect(() => {
    if (!diffOpen || !activeSessionId) return;
    void refreshDiff(activeSessionId);
  }, [activeSessionId, diffOpen, refreshDiff]);

  useEffect(() => {
    if (!activeSessionId) return;
    if (!gitStatusSignature) return;
    const prevSignature = diffRefreshSignatureRef.current.get(activeSessionId) ?? "";
    if (gitStatusSignature === prevSignature) return;
    diffRefreshSignatureRef.current.set(activeSessionId, gitStatusSignature);
    if (diffRefreshTimerRef.current) {
      window.clearTimeout(diffRefreshTimerRef.current);
    }
    diffRefreshTimerRef.current = window.setTimeout(() => {
      if (diffOpen) {
        void refreshDiff(activeSessionId);
      }
    }, 400);
    return () => {
      if (diffRefreshTimerRef.current) {
        window.clearTimeout(diffRefreshTimerRef.current);
        diffRefreshTimerRef.current = null;
      }
    };
  }, [activeSessionId, diffOpen, gitStatusSignature, refreshDiff]);

  const onSplitterMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = diffWidth;
    setDiffResizing(true);
    const onMove = (ev: MouseEvent) => {
      const dx = startX - ev.clientX;
      const next = Math.min(900, startW + dx);
      setDiffWidth(clampDiffWidth(next));
    };
    const onUp = () => {
      setDiffResizing(false);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const onTerminalResizerMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const startY = e.clientY;
    const startH = terminalHeight;
    setTerminalResizing(true);
    const onMove = (ev: MouseEvent) => {
      const dy = startY - ev.clientY;
      const next = clampTerminalHeight(startH + dy);
      setTerminalHeight(next);
    };
    const onUp = () => {
      setTerminalResizing(false);
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

  const activeTask = activeTaskSummary?.task ?? null;
  const activeTaskIsOptimistic = activeTaskSummary ? isOptimisticTask(activeTaskSummary) : false;
  const activeTaskHasAssistantMessage = Boolean(activeTask?.last_assistant_message_at);
  const worktreeChip = useMemo(() => {
    const sess = activeEntry?.session ?? null;
    const worktreeRoot = String(activeWorktree?.root_path ?? "");
    const workspaceRoot = String(workspace?.root_path ?? "");
    const envTarget = String(sess?.env_target ?? "").trim().toLowerCase();
    const inferredWorktree =
      Boolean(worktreeRoot) &&
      (Boolean(activeWorktree?.git_branch) || (Boolean(workspaceRoot) && worktreeRoot !== workspaceRoot));
    const isCloud = envTarget === "cloud";
    const isWorktree = envTarget === "worktree" || (!isCloud && envTarget !== "local" && inferredWorktree);
    const worktreePath = (isWorktree || isCloud) ? worktreeRoot : "";
    const worktreeLabel = isCloud ? "Cloud worker" : worktreePath ? formatWorktreeLabel(worktreePath) : "";

    return {
      worktreeLabel,
      worktreePath,
      canCopyWorktree: Boolean(worktreePath) && !isCloud,
      canOpenTerminal: Boolean(worktreePath),
      copyPath: isCloud ? "" : worktreePath,
    };
  }, [activeEntry, activeWorktree?.git_branch, activeWorktree?.root_path, workspace?.root_path]);
  const singleSessionHeader = useMemo(() => {
    const sess = activeEntry?.session ?? null;
    if (!sess) return null;
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

    return {
      title: activeTask?.title ?? "Conversation",
      lastIso,
      harness,
      modelBase: parsedModel.base || String(sess?.model_id ?? ""),
      effort: parsedModel.effort,
    };
  }, [activeEntry, activeTask?.title]);

  const singleSessionHeaderForRender = useMemo(() => {
    if (singleSessionHeader) return singleSessionHeader;
    if (!activeTaskId) return null;
    return {
      title: activeTask?.title ?? "Conversation",
      lastIso: null,
      harness: "",
      modelBase: "",
      effort: "",
    };
  }, [activeTask?.title, activeTaskId, singleSessionHeader]);

  const showSingleSessionHeader = Boolean(activeTaskId && singleSessionHeaderForRender);

  const [worktreeCopied, setWorktreeCopied] = useState(false);
  const worktreeCopiedTimerRef = useRef<number | null>(null);

  useEffect(() => {
    if (!worktreeCopied) return;
    if (worktreeCopiedTimerRef.current) {
      window.clearTimeout(worktreeCopiedTimerRef.current);
    }
    worktreeCopiedTimerRef.current = window.setTimeout(() => {
      setWorktreeCopied(false);
      worktreeCopiedTimerRef.current = null;
    }, 1100);
    return () => {
      if (worktreeCopiedTimerRef.current) {
        window.clearTimeout(worktreeCopiedTimerRef.current);
        worktreeCopiedTimerRef.current = null;
      }
    };
  }, [worktreeCopied]);

  const copyWorktreeLocation = useCallback(async () => {
    const path = String(worktreeChip.copyPath ?? "").trim();
    if (!path || !worktreeChip.canCopyWorktree) return;
    const ok = await copyTextToClipboard(path);
    if (!ok) {
      window.alert("Clipboard access is blocked; use HTTPS/desktop app or copy manually.");
      return;
    }
    setWorktreeCopied(true);
  }, [worktreeChip.worktreePath]);

  const openWorktreeTerminal = useCallback(async () => {
    const path = String(worktreeChip.worktreePath ?? "").trim();
    if (!path || !worktreeChip.canOpenTerminal) return;
    if (!terminalPanelRef.current) return;
    terminalPanelRef.current.setScope("task");
    setTerminalOpen(true);
    const createdId = await terminalPanelRef.current.createTerminal({
      cwd: path,
      taskId: activeTaskId ?? null,
      sessionId: activeSessionId ?? null,
      worktreeId: activeWorktreeId || null,
      scope: "task",
    });
    if (createdId) terminalPanelRef.current.focusTerminal(createdId);
  }, [activeSessionId, activeTaskId, activeWorktreeId, worktreeChip.worktreePath]);

  const buildSessionLogExport = useCallback(() => {
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

    const title = singleSessionHeader?.title ?? "Conversation";
    const lines: string[] = [];
    lines.push(`# ${title}`);
    lines.push("");
    lines.push(`- Exported: ${exportedAt}`);
    lines.push(`- Harness: ${harness}`);
    lines.push(`- Model: ${parsedModel.base || String(sess.model_id ?? "")}`);
    if (parsedModel.effort) lines.push(`- Effort: ${parsedModel.effort}`);
    if (worktreeChip.worktreePath) lines.push(`- Worktree: ${formatWorktreePath(worktreeChip.worktreePath)}`);
    lines.push(`- Session ID: ${idToString(sess.id)}`);
    lines.push("");
    lines.push("---");
    lines.push("");

    const attachmentLine = (atts: MessageAttachment[]): string => {
      const names = (atts ?? [])
        .map((a) => String(a.name ?? ("blob_id" in a ? a.blob_id : a.kind) ?? "").trim())
        .filter(Boolean);
      if (names.length === 0) return "";
      return `Attachments: ${names.join(", ")}`;
    };

    for (let i = 0; i < thread.groups.length; i++) {
      const g = thread.groups[i];
      if (!g) continue;
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

      const items = Array.isArray(g.items) ? g.items : [];
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

    return { title, markdown: lines.join("\n") };
  }, [activeEntry, singleSessionHeader?.title, worktreeChip.worktreePath]);

  const buildTranscriptExportFromEntry = useCallback(
    (entry: SessionCacheEntry | null) => {
      if (!entry?.session) return;
      const thread = buildWorkbenchThreadViewModel(
        entry.turns ?? [],
        entry.messages ?? [],
        entry.turnToolsByTurnId ?? {},
        entry.events ?? [],
      );

      const title = singleSessionHeader?.title ?? "Conversation";
      const lines: string[] = [];
      lines.push(`# ${title}`);
      lines.push("");

      for (const g of thread.groups ?? []) {
        if (g?.header) {
          lines.push("User:");
          lines.push("");
          lines.push(String(g.header.content ?? ""));
          lines.push("");
        }

        const items = Array.isArray(g.items) ? g.items : [];
        for (const item of items) {
          if (!item || item.kind !== "assistant") continue;
          lines.push("Assistant:");
          lines.push("");
          lines.push(String(item.content ?? ""));
          lines.push("");
        }
      }

      return { title, markdown: lines.join("\n") };
    },
    [singleSessionHeader?.title],
  );

  const buildTranscriptExport = useCallback(() => {
    return buildTranscriptExportFromEntry(activeEntry ?? null);
  }, [activeEntry, buildTranscriptExportFromEntry]);

  const hydrateTranscriptHistory = useCallback(
    async (sessionId: string): Promise<{ ok: boolean; partial: boolean }> => {
      let lastCursor: number | null = null;
      let stalledCount = 0;
      while (true) {
        const entry = supervisor.getSnapshot().sessions[String(sessionId)];
        if (!entry) return { ok: false, partial: true };
        if (!entry.hasMoreTurns) return { ok: true, partial: false };
        if (entry.fetching?.history) {
          await new Promise((resolve) => setTimeout(resolve, 40));
          continue;
        }
        const beforeCursor = entry.oldestTurnSeq ?? null;
        await supervisor.loadMoreTurns(sessionId);
        const nextEntry = supervisor.getSnapshot().sessions[String(sessionId)];
        if (!nextEntry) return { ok: false, partial: true };
        if (!nextEntry.hasMoreTurns) return { ok: true, partial: false };
        const afterCursor = nextEntry.oldestTurnSeq ?? null;
        if (afterCursor === beforeCursor && afterCursor === lastCursor) {
          stalledCount += 1;
          if (stalledCount >= 2) return { ok: false, partial: true };
        } else {
          stalledCount = 0;
        }
        lastCursor = afterCursor;
      }
    },
    [supervisor],
  );

  const exportSessionLog = useCallback(async () => {
    const payload = buildSessionLogExport();
    if (!payload) return;
    try {
      const fileBase = `${sanitizeFileName(payload.title)}-session-log`;
      await saveMarkdownExport(fileBase, payload.markdown);
    } catch (e: unknown) {
      window.alert(errorMessage(e) || "Failed to export session log.");
    }
  }, [buildSessionLogExport]);

  const copySessionLog = useCallback(async () => {
    const payload = buildSessionLogExport();
    if (!payload) return;
    const ok = await copyTextToClipboard(payload.markdown);
    if (!ok) {
      window.alert("Clipboard access is blocked; use HTTPS/desktop app or copy manually.");
    }
  }, [buildSessionLogExport]);

  const exportTranscript = useCallback(async () => {
    const payload = buildTranscriptExport();
    if (!payload) return;
    try {
      const fileBase = `${sanitizeFileName(payload.title)}-transcript`;
      await saveMarkdownExport(fileBase, payload.markdown);
    } catch (e: unknown) {
      window.alert(errorMessage(e) || "Failed to export transcript.");
    }
  }, [buildTranscriptExport]);

  const copyTranscript = useCallback(async () => {
    const sessionId = activeSessionId;
    if (!sessionId || copyTranscriptBusyRef.current) return;
    copyTranscriptBusyRef.current = true;
    setCopyTranscriptBusy(true);
    setTranscriptNotice(null);
    try {
      let hydrationResult = { ok: true, partial: false };
      try {
        hydrationResult = await hydrateTranscriptHistory(sessionId);
      } catch {
        hydrationResult = { ok: false, partial: true };
      }
      const entry = supervisor.getSnapshot().sessions[String(sessionId)] ?? null;
      const payload = buildTranscriptExportFromEntry(entry);
      if (!payload) return;
      const ok = await copyTextToClipboard(payload.markdown);
      if (!ok) {
        setTranscriptNotice("Clipboard access is blocked; use HTTPS/desktop app or copy manually.");
        return;
      }
      if (!hydrationResult.ok || hydrationResult.partial) {
        setTranscriptNotice("Couldn't load full history. Copied what's already loaded.");
      }
    } finally {
      copyTranscriptBusyRef.current = false;
      setCopyTranscriptBusy(false);
      setConvoMenu(null);
    }
  }, [activeSessionId, buildTranscriptExportFromEntry, hydrateTranscriptHistory, supervisor]);

  const desktopUi = isDesktopApp();
  const emitMenuTrace = useCallback(
    (detail: Omit<WebMenuTraceDetail, "layer">) => {
      window.dispatchEvent(
        new CustomEvent<WebMenuTraceDetail>(WEB_MENU_TRACE_EVENT, {
          detail: { ...detail, layer: "workbench" },
        }),
      );
    },
    [],
  );

  useEffect(() => {
    if (!desktopUi) return;

    const onMenuCommand = (event: Event) => {
      const custom = event as CustomEvent<WebMenuCommandDetail>;
      const detail = custom.detail;
      if (!detail) return;

      switch (detail.commandId) {
        case "file.export-transcript":
          if (!activeSessionId) {
            emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "session-missing" });
            return;
          }
          void exportTranscript();
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "export-transcript" });
          return;
        case "file.export-session-log":
          if (!activeSessionId) {
            emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "session-missing" });
            return;
          }
          void exportSessionLog();
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "export-session-log" });
          return;
        case "view.find-tasks":
          if (!taskSearchRef.current) {
            emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "task-search-missing" });
            return;
          }
          taskSearchRef.current.focus();
          taskSearchRef.current.select();
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "focus-task-search" });
          return;
        case "view.toggle-sidebar":
          setSidebarCollapsed((prev) => !prev);
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "toggle-sidebar" });
          return;
        case "view.toggle-diff":
          if (!activeTaskId) {
            emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "task-missing" });
            return;
          }
          toggleDiffPane("menu_command");
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "toggle-diff-pane" });
          return;
        case "view.toggle-artifacts":
          if (!activeTaskId || !activeSessionId) {
            emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "task-or-session-missing" });
            return;
          }
          toggleArtifactsPane("menu_command");
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "toggle-artifacts-pane" });
          return;
        case "view.toggle-sessions":
          if (webSessionsEnabled && activeTaskId && activeSessionId) {
            toggleSessionsPane("menu_command");
            emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "toggle-sessions-pane" });
            return;
          }
          emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "sessions-unavailable" });
          return;
        case "view.toggle-terminal":
          toggleTerminalPanel("menu_command");
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "toggle-terminal" });
          return;
        case "task.new":
          focusNewTask();
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "create-task" });
          return;
        case "task.rename":
          if (activeTaskId && !activeTaskIsOptimistic) {
            beginRenameTask(activeTaskId);
            emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "rename-task" });
            return;
          }
          emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "task-missing-or-optimistic" });
          return;
        case "task.archive-toggle":
          if (activeTaskId) {
            void onToggleArchive(activeTaskId, !Boolean(activeTask?.archived_at), null).catch(() => {});
            emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "toggle-task-archive" });
            return;
          }
          emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "task-missing" });
          return;
        case "task.mark-read-toggle":
          if (activeTaskId && activeTaskHasAssistantMessage) {
            if (isTaskUnread(activeTaskId)) {
              void markTaskRead(activeTaskId);
            } else {
              void markTaskUnread(activeTaskId);
            }
            emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "toggle-task-read" });
            return;
          }
          emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "task-or-message-missing" });
          return;
        case "task.delete":
          if (activeTaskId) {
            void onDeleteTask(activeTaskId);
            emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "delete-task" });
            return;
          }
          emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "task-missing" });
          return;
        case "session.copy-transcript":
          void copyTranscript();
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "copy-transcript" });
          return;
        case "session.copy-session-log":
          void copySessionLog();
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "copy-session-log" });
          return;
        case "session.copy-worktree-location":
          if (!worktreeChip.canCopyWorktree) {
            emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "worktree-unavailable" });
            return;
          }
          void copyWorktreeLocation();
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "copy-worktree-location" });
          return;
        case "session.open-worktree-terminal":
          if (!worktreeChip.canOpenTerminal) {
            emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "worktree-unavailable" });
            return;
          }
          void openWorktreeTerminal();
          emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "open-worktree-terminal" });
          return;
        case "session.interrupt":
          if (activeSessionId) {
            void interruptSession(activeSessionId).catch(() => {});
            emitMenuTrace({ commandId: detail.commandId, status: "handled", note: "interrupt-session" });
            return;
          }
          emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "session-missing" });
          return;
        default:
          emitMenuTrace({ commandId: detail.commandId, status: "ignored", note: "unsupported-command" });
          return;
      }
    };

    window.addEventListener(WEB_MENU_COMMAND_EVENT, onMenuCommand as EventListener);
    return () => {
      window.removeEventListener(WEB_MENU_COMMAND_EVENT, onMenuCommand as EventListener);
    };
  }, [
    activeSessionId,
    activeTask?.archived_at,
    activeTaskHasAssistantMessage,
    activeTaskId,
    activeTaskIsOptimistic,
    beginRenameTask,
    copySessionLog,
    copyTranscript,
    copyWorktreeLocation,
    desktopUi,
    exportSessionLog,
    exportTranscript,
    focusNewTask,
    isTaskUnread,
    markTaskRead,
    markTaskUnread,
    onDeleteTask,
    onToggleArchive,
    openWorktreeTerminal,
    toggleArtifactsPane,
    toggleDiffPane,
    toggleSessionsPane,
    toggleTerminalPanel,
    webSessionsEnabled,
    emitMenuTrace,
    worktreeChip.canCopyWorktree,
    worktreeChip.canOpenTerminal,
  ]);

  useEffect(() => {
    if (!desktopUi) return;
    const activeSessionStatus = String(activeEntry?.session?.status ?? "").toLowerCase();
    const canInterruptSession =
      Boolean(activeSessionId) && (activeSessionStatus === "active" || activeSessionStatus === "running");
    const canToggleArchive = Boolean(activeTaskId) && !Boolean(activeTaskId && archivePendingById[activeTaskId]);

    const items: DesktopMenuItemState[] = [
      { id: "file.export-transcript", enabled: Boolean(activeSessionId) },
      { id: "file.export-session-log", enabled: Boolean(activeSessionId) },
      { id: "view.find-tasks", enabled: true },
      { id: "view.toggle-sidebar", enabled: true, checked: !sidebarCollapsed },
      { id: "view.toggle-diff", enabled: Boolean(activeTaskId), checked: diffOpen },
      { id: "view.toggle-artifacts", enabled: Boolean(activeTaskId && activeSessionId), checked: artifactsOpen },
      { id: "view.toggle-sessions", enabled: Boolean(webSessionsEnabled && activeTaskId && activeSessionId), checked: sessionsOpen },
      { id: "view.toggle-terminal", enabled: true, checked: terminalOpen },
      { id: "task.new", enabled: true },
      { id: "task.rename", enabled: Boolean(activeTaskId) && !activeTaskIsOptimistic },
      { id: "task.archive-toggle", enabled: canToggleArchive },
      { id: "task.mark-read-toggle", enabled: Boolean(activeTaskId) && activeTaskHasAssistantMessage },
      { id: "task.delete", enabled: Boolean(activeTaskId) },
      { id: "session.copy-transcript", enabled: Boolean(activeSessionId) && !copyTranscriptBusy },
      { id: "session.copy-session-log", enabled: Boolean(activeSessionId) },
      { id: "session.copy-worktree-location", enabled: worktreeChip.canCopyWorktree },
      { id: "session.open-worktree-terminal", enabled: worktreeChip.canOpenTerminal },
      { id: "session.interrupt", enabled: canInterruptSession },
    ];

    window.dispatchEvent(
      new CustomEvent<WebMenuStateDetail>(WEB_MENU_STATE_EVENT, {
        detail: { replace: true, items },
      }),
    );
  }, [
    activeEntry?.session?.status,
    activeSessionId,
    activeTaskHasAssistantMessage,
    activeTaskId,
    activeTaskIsOptimistic,
    archivePendingById,
    artifactsOpen,
    copyTranscriptBusy,
    desktopUi,
    diffOpen,
    sessionsOpen,
    sidebarCollapsed,
    terminalOpen,
    webSessionsEnabled,
    worktreeChip.canCopyWorktree,
    worktreeChip.canOpenTerminal,
  ]);

  const [desktopPlatform, setDesktopPlatform] = useState<DesktopPlatform>(() => {
    if (!desktopUi) return "unknown";
    const platform = typeof navigator === "undefined" ? "" : navigator.platform;
    if (/mac/i.test(platform)) return "macos";
    if (/win/i.test(platform)) return "windows";
    if (/linux/i.test(platform)) return "linux";
    return "unknown";
  });

  useEffect(() => {
    if (!desktopUi) return;
    let cancelled = false;
    getDesktopPlatform()
      .then((platform) => {
        if (!cancelled) setDesktopPlatform(platform);
      })
      .catch(() => {
        if (!cancelled) setDesktopPlatform("unknown");
      });
    return () => {
      cancelled = true;
    };
  }, [desktopUi]);

  useEffect(() => {
    if (!desktopUi) return;
    let cancelled = false;
    desktopStorageConsumeNotice()
      .then((notice) => {
        if (!cancelled) {
          setDesktopStorageNotice(notice);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [desktopUi]);

  const useHtmlTopbar = !desktopUi || desktopPlatform !== "macos";
  const workspaceTitle = workspace?.name ?? "";

  type RootStyle = React.CSSProperties & {
    "--wb-sidebar-width": string;
    "--wb-terminal-offset": string;
    "--wb-topbar-height": string;
  };

  const rootStyle = useMemo<RootStyle>(() => {
    const max = Math.max(170, window.innerWidth - 240);
    const clamped = Math.min(max, Math.max(170, Math.round(sidebarWidth)));
    const terminalOffset = terminalOpen ? terminalHeight : 0;
    return {
      "--wb-sidebar-width": `${clamped}px`,
      "--wb-terminal-offset": `${terminalOffset}px`,
      "--wb-topbar-height": useHtmlTopbar ? "46px" : "0px",
    };
  }, [sidebarWidth, terminalHeight, terminalOpen, useHtmlTopbar]);

  useEffect(() => {
    if (!desktopUi) return;
    const title = workspace?.name ?? "";
    document.title = title;
    desktopSetWindowTitle(title).catch(() => {});
  }, [desktopUi, workspace?.name]);

  useEffect(() => {
    if (!desktopUi) return;
    const workspaceIdValue = String(workspaceId || "").trim();
    if (!workspaceIdValue) return;
    const workspaceLabel = String(workspace?.name || "").trim() || workspaceIdValue;
    void desktopRecordWorkspaceVisit(workspaceIdValue, workspaceLabel).catch(() => {});
  }, [desktopUi, workspace?.name, workspaceId]);

  const topbar = useHtmlTopbar ? (
    <div className="wb-topbar" data-tauri-drag-region={desktopUi ? true : undefined}>
      <div className="wb-topbar-left" />
      <div className="wb-topbar-center">
        {workspaceTitle ? <div className="wb-topbar-title">{workspaceTitle}</div> : null}
      </div>
      <div className="wb-topbar-right" data-tauri-drag-region={false}>
        {showDebugIds && (
          <button
            type="button"
            className="wb-topbar-ids"
            title="Click to copy workspace/task/session IDs"
            onClick={() =>
              void copyTextToClipboard(
                JSON.stringify(
                  {
                    workspaceId,
                    taskId: activeTaskId,
                    sessionId: activeSessionId,
                  },
                  null,
                  2,
                ),
              )
            }
            data-tauri-drag-region={false}
          >
            {debugIdLabel}
          </button>
        )}
        <Link
          className="wb-topbar-icon"
          to={`/settings?ws=${encodeURIComponent(String(workspaceId))}`}
          title="Settings"
          aria-label="Settings"
          data-tauri-drag-region={false}
        >
          <Settings size={14} />
        </Link>
      </div>
    </div>
  ) : null;
  if (!workbenchSnap.hydrated) {
    return (
      <div
        className={`wb-root ${sidebarCollapsed ? "wb-root-collapsed" : ""} ${sidebarResizing ? "wb-root-resizing" : ""} ${diffResizing ? "wb-root-diff-resizing" : ""} ${terminalResizing ? "wb-root-terminal-resizing" : ""} ${!useHtmlTopbar ? "wb-root-native-titlebar" : ""}`}
        style={rootStyle}
      >
        <WorktreeBootstrapSnackbar />
        {archiveCleanupSnackbar}
        {transcriptNoticeSnackbar}
        {desktopStorageNoticeSnackbar}
        {topbar}
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
      className={`wb-root ${sidebarCollapsed ? "wb-root-collapsed" : ""} ${sidebarResizing ? "wb-root-resizing" : ""} ${diffResizing ? "wb-root-diff-resizing" : ""} ${terminalResizing ? "wb-root-terminal-resizing" : ""} ${!useHtmlTopbar ? "wb-root-native-titlebar" : ""}`}
      style={rootStyle}
    >
      <WorktreeBootstrapSnackbar />
      <TitleGenerationInstallBanner />
      {archiveCleanupSnackbar}
      {transcriptNoticeSnackbar}
      {desktopStorageNoticeSnackbar}
      {topbar}

      {!activeTaskId ? (
        <HarnessAuthenticationSection
          workspaceId={workspaceId}
          active={true}
          modalOnly
          openProviderId={harnessAuthModalProviderId}
          onModalClosed={onComposerHarnessAuthModalClosed}
        />
      ) : null}

      {workbenchSnap.warnings.length > 0 && (
        <div className="banner" style={{ margin: "8px 12px 0" }}>
          {workbenchSnap.warnings[0]}
        </div>
      )}

      {sidebarCollapsed ? (
        <button
          type="button"
          className="wb-sidebar-tab wb-sidebar-tab-collapsed"
          aria-label="Show sidebar"
          title="Show sidebar"
          onClick={() => setSidebarCollapsed(false)}
        >
          <ChevronsRight size={16} />
        </button>
      ) : (
        <button
          type="button"
          className="wb-sidebar-tab wb-sidebar-tab-open"
          aria-label="Collapse sidebar"
          title="Collapse"
          onClick={() => setSidebarCollapsed(true)}
        >
          <ChevronsLeft size={16} />
        </button>
      )}

      <div className="wb-sidebar" aria-hidden={sidebarCollapsed}>
        <div className="wb-sidebar-top">
          <div className="wb-sidebar-header">
            <input
              ref={taskSearchRef}
              className="wb-search"
              data-testid="workbench-task-search"
              placeholder="Search Tasks"
              value={taskQuery}
              onChange={(e) => setTaskQuery(e.target.value)}
            />
            <button
              type="button"
              className="wb-sidebar-action"
              aria-label="New task"
              title="New Task"
              onClick={focusNewTask}
            >
              <SquarePen size={16} />
            </button>
          </div>
        </div>

        <div className="wb-sidebar-section wb-sidebar-grow" style={{ minHeight: 0, display: "flex" }}>
          <Virtuoso
            style={{ height: "100%" }}
            data={taskListItems}
            overscan={8}
            computeItemKey={computeTaskListItemKey}
            itemContent={(_, item) => renderTaskListItem(item)}
            context={taskListContext}
            rangeChanged={onTaskListRangeChanged}
            components={TASK_LIST_COMPONENTS}
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
                onCancelInstallProvider={cancelProviderInstallFromMenu}
                onInstallAllProviders={installAllProvidersFromMenu}
                installAllBusy={installAllBusy}
                providerOptions={providerOptions}
                ensureProviderAuthSummary={ensureProviderAuthSummary}
                onRequestHarnessAuth={requestHarnessAuthFromComposer}
                draftHarness={draftHarness}
                setDraftHarness={setDraftHarness}
                defaultProviderId={defaultProviderId}
              />

              {dictationDebugText && <div className="wb-banner">{dictationDebugText}</div>}
              {dictationError && <div className="wb-banner">{dictationError}</div>}
              {startError && <div className="wb-banner">{startError}</div>}
              <DictationOnboardingModal
                state={dictationOnboarding}
                onClose={dismissDictationOnboarding}
                onBack={backDictationOnboarding}
                onChooseLocal={chooseDictationOnboardingLocal}
                onChooseCloud={chooseDictationOnboardingCloud}
                onCloudChange={updateDictationOnboardingCloud}
                onSubmitCloud={() => {
                  void submitDictationOnboardingCloud();
                }}
                onSubmitLocal={() => {
                  void submitDictationOnboardingLocal();
                }}
              />
            </div>
          </div>
        ) : null}

        {activeTaskId && (
          <div className="wb-body">
            <div className="wb-convo">
              {showSingleSessionHeader ? (
                <WorkbenchSessionHeader
                  busy={sessions.length === 0}
                  title={singleSessionHeaderForRender?.title ?? "Conversation"}
                  worktreeChip={worktreeChip}
                  worktreeCopied={worktreeCopied}
                  showArtifactsPane={showArtifactsPane}
                  showReviewPane={showReviewPane}
                  terminalOpen={terminalOpen}
                  artifactsCount={artifactsCount}
                  diffBadgeCount={diffBadgeCount}
                  onCopyWorktreeLocation={() => void copyWorktreeLocation()}
                  onOpenWorktreeTerminal={() => void openWorktreeTerminal()}
                  onToggleArtifactsPane={() => toggleArtifactsPane("header_button")}
                  onToggleDiffPane={() => toggleDiffPane("header_button")}
                  onToggleTerminalPanel={() => toggleTerminalPanel("header_button")}
                  onOpenConvoMenu={openConvoMenu}
                />
              ) : null}

              <div className="wb-session">
                {activeSessionId ? (
                  <WorkbenchSessionSlot
                    key={activeSessionId}
                    sessionId={activeSessionId}
                    active={true}
                    scrollState={workbenchSnap.window.scrollByKey[scrollKey(activeSessionId)] ?? null}
                    preserveScrollOnFocus={preserveScrollOnFocus}
                    optimisticFailure={optimisticFailureBySessionId[activeSessionId] ?? null}
                  />
                ) : null}
                {!activeSessionId && (
                  <div className="wb-muted" style={{ padding: 16 }}>
                    Select a session to view this task.
                  </div>
                )}
              </div>
            </div>

            {rightPaneOpen && (
              <>
                <div className="wb-splitter" onMouseDown={onSplitterMouseDown} />
                <div className="wb-right" style={{ width: diffWidth, maxWidth: "100%" }}>
                  {showSessionsPane ? (
                    <div className="wb-right-pane">
                      <SessionsPane
                        sections={sessionSections}
                        activeSection={activeSessionKind}
                        onSectionChange={setActiveSessionKind}
                        selectedSessionId={activeWebSessionId}
                        onSelectSession={setActiveWebSessionId}
                        daemonBaseUrl={daemonBaseUrl}
                        loading={webSessionsLoading}
                      />
                    </div>
                  ) : showReviewPane ? (
                    <div className="wb-right-pane wb-diff">
                      {reviewTab === "git" && hasDiff ? (
                        diffSummaryError ? (
                          <div className="wb-diff-empty">
                            <div className="wb-muted">{diffSummaryError}</div>
                          </div>
                        ) : diffTooLarge ? (
                          <div className="wb-diff-empty">
                            <div className="wb-muted">{diffTooLargeLabel ?? "Diff too large to display."}</div>
                          </div>
                        ) : (
                          <DiffReviewPane
                            diff={activeSessionDiff}
                            labels={activeDiffContentError ? { empty: activeDiffContentError } : undefined}
                          />
                        )
                      ) : (
                        <div className="wb-diff-empty">
                          <div className="wb-muted">
                            {diffEmptyLabel}
                          </div>
                        </div>
                      )}
                    </div>
                  ) : showArtifactsPane ? (
                    <div className="wb-right-pane">
                      <ArtifactsPane artifacts={artifacts} loading={artifactsLoading} />
                    </div>
                  ) : null}
                </div>
              </>
            )}
          </div>
        )}
      </div>

      <div className="wb-terminal-shell" aria-hidden={!terminalOpen}>
        {terminalOpen && (
          <div className="wb-terminal-resizer" onMouseDown={onTerminalResizerMouseDown} />
        )}
        <div
          className="wb-terminal-panel"
          style={{
            height: terminalOpen ? terminalHeight : 0,
            pointerEvents: terminalOpen ? "auto" : "none",
          }}
          aria-hidden={!terminalOpen}
        >
          <TerminalPanel
            ref={terminalPanelRef}
            workspaceId={workspaceId}
            activeTaskId={activeTaskId}
            activeSessionId={activeSessionId}
            open={terminalOpen}
            height={terminalHeight}
            onRequestClose={() => setTerminalOpen(false)}
          />
        </div>
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
            className="wb-menu-item wb-archive-confirm-trigger"
            disabled={Boolean(archivePendingById[taskMenu.taskId])}
            onClick={(e) => {
              const tid = taskMenu.taskId;
              const summary = tasksById[tid];
              const nextArchived = !summary?.task.archived_at;
              const anchor = e.currentTarget.getBoundingClientRect();
              onToggleArchive(tid, nextArchived, anchor).catch(() => { });
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
              const unread = isTaskUnread(tid);
              setTaskMenu(null);
              if (unread) markTaskRead(tid);
              else markTaskUnread(tid);
            }}
            role="menuitem"
          >
            {(() => {
              const tid = taskMenu.taskId;
              const unread = isTaskUnread(tid);
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

      {archiveConfirm && archiveConfirmStyle && (
        <div
          className="wb-archive-confirm wb-menu-tooltip"
          data-open="true"
          role="dialog"
          aria-label="Archive confirmation"
          ref={archiveConfirmRef}
          style={archiveConfirmStyle}
        >
          <div className="wb-archive-confirm-title">Archive conversation?</div>
          <div className="wb-archive-confirm-body">
            Archiving deletes the ctx-managed worktrees and branches associated with this task, including its subagents. Later, you can unarchive to recreate them, but unmerged changes will be lost.
            <br />
            <br />
            If you want to keep changes made here, consider instructing the primary agent to use the Merge Queue to bring the changes into your main branch. Otherwise, tell it to stash the changes into another local or remote branch for later use.
            <br />
            <br />
            In general, we recommend aggressively archiving tasks as you complete work for performance and organization. You can always unarchive any task later, which will restore all conversation history, including subagents.
          </div>
          <label className="wb-archive-confirm-toggle">
            <input
              type="checkbox"
              checked={archiveConfirmDontRemind}
              onChange={(e) => setArchiveConfirmDontRemind(e.target.checked)}
            />
            Don&apos;t ask me again
          </label>
          <div className="wb-archive-confirm-actions">
            <button
              type="button"
              className="wb-snackbar-btn wb-snackbar-btn-secondary"
              onClick={cancelArchiveConfirm}
            >
              Cancel
            </button>
            <button
              type="button"
              className="wb-snackbar-btn wb-archive-confirm-danger"
              onClick={() => {
                void confirmArchive();
              }}
            >
              Archive
            </button>
          </div>
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
              void exportTranscript();
            }}
            role="menuitem"
          >
            Export Transcript
          </button>
          <button
            type="button"
            className="wb-menu-item"
            disabled={!activeSessionId || copyTranscriptBusy}
            onClick={() => {
              void copyTranscript();
            }}
            role="menuitem"
          >
            <span className="wb-menu-item-row">
              <span>Copy Transcript</span>
              {copyTranscriptBusy ? (
                <span
                  className="wb-task-spinner wb-menu-item-spinner"
                  style={{ animationDelay: `${transcriptSpinnerDelayRef.current}ms` }}
                  aria-hidden="true"
                />
              ) : null}
            </span>
          </button>
          <button
            type="button"
            className="wb-menu-item"
            disabled={!activeSessionId}
            onClick={() => {
              setConvoMenu(null);
              void exportSessionLog();
            }}
            role="menuitem"
          >
            Export Session Log
          </button>
          <button
            type="button"
            className="wb-menu-item"
            disabled={!activeSessionId}
            onClick={() => {
              setConvoMenu(null);
              void copySessionLog();
            }}
            role="menuitem"
          >
            Copy Session Log
          </button>
          <button
            type="button"
            className="wb-menu-item"
            disabled={!worktreeChip.canCopyWorktree}
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
            className="wb-menu-item wb-archive-confirm-trigger"
            disabled={
              !activeTaskId || !!activeTask?.archived_at || (activeTaskId ? Boolean(archivePendingById[activeTaskId]) : false)
            }
            onClick={(e) => {
              if (!activeTaskId) return;
              setConvoMenu(null);
              const anchor = e.currentTarget.getBoundingClientRect();
              onToggleArchive(activeTaskId, true, anchor).catch(() => {});
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
