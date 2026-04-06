import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { X } from "lucide-react";
import {
  MessageAttachment,
  Workspace,
  daemonFetchRaw,
  getHealth,
  interruptSession,
  markTaskRead as markTaskReadApi,
  markTaskUnread as markTaskUnreadApi,
} from "../api/client";
import { useSessionCacheSnapshot, useSessionSupervisor } from "../state/sessionSupervisor";
import { TerminalPanel } from "../components/TerminalPanel";
import { TitleGenerationInstallBanner } from "../components/TitleGenerationInstallBanner";
import { WorktreeBootstrapSnackbar } from "../components/WorktreeBootstrapSnackbar";
import { type DraftHarness, type WorkbenchModeId } from "../components/WorkbenchComposer";
import type { SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import { isDesktopApp } from "../utils/desktop";
import { copyTextToClipboard } from "../utils/clipboard";
import { hasConfiguredHarnessAuth } from "../utils/providerAuthStatus";
import { useDictationController } from "../utils/useDictationController";
import { NEW_TASK_DRAFT_KEY, useActiveWorkbenchIds, useNewTaskDraft, useWorkbenchShellSnapshot, useWorkbenchStore } from "../workbench/store";
import { useWorkspaceActiveSnapshotSnapshot, useWorkspaceActiveSnapshotStore } from "../state/workspaceActiveSnapshotStore";
import { useHarnessAuthenticationController } from "./settings/hooks/useHarnessAuthenticationController";
import { HarnessAuthenticationSectionView } from "./settings/sections/HarnessAuthenticationSection";
import { useWorkbenchDragDropAttachments } from "./workbenchShell/useWorkbenchDragDropAttachments";
import {
  collectSelectableHarnessProviderIds,
  getHarnessMruStorageKey,
  resolveInitialHarnessSelection,
  shouldFinalizeInitialHarnessSelection,
} from "./workbenchShell/harnessSelection";
import { useWorkbenchOptimisticTasks } from "./workbenchShell/useWorkbenchOptimisticTasks";
import { useWorkbenchProviders } from "./workbenchShell/useWorkbenchProviders";
import { WorkbenchActiveTaskView } from "./workbenchShell/WorkbenchActiveTaskView";
import { WorkbenchEmptyState } from "./workbenchShell/WorkbenchEmptyState";
import { WorkbenchSidebar, WorkbenchTopbar } from "./workbenchShell/WorkbenchShellChrome";
import { WorkbenchPageMenus } from "./workbenchShell/WorkbenchPageMenus";
import { useWorkbenchChromeIntegration } from "./workbenchShell/useWorkbenchChromeIntegration";
import { useWorkbenchSessionBridge } from "./workbenchShell/useWorkbenchSessionBridge";
import { useWorkbenchTaskCreation } from "./workbenchShell/useWorkbenchTaskCreation";
import { useWorkbenchTaskListController } from "./workbenchShell/useWorkbenchTaskListController";
import { useWorkbenchActiveTaskController } from "./workbenchShell/useWorkbenchActiveTaskController";
import { useWorkbenchE2EBridge } from "./workbenchShell/useWorkbenchE2EBridge";
import { useWorkbenchComposerHarnessAuth } from "./workbenchShell/useWorkbenchComposerHarnessAuth";
import type { OptimisticFocus } from "./WorkbenchPage.types";
import { appendSegment } from "./WorkbenchPage.utils";
import { resolveWorkspaceBootstrapGateState } from "./workspaceBootstrapGate";

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
  const [daemonDataRoot, setDaemonDataRoot] = useState<string | null>(null);
  const manualDemoHarnessSelection = useMemo(() => {
    const params = new URLSearchParams(window.location.search);
    return params.get("ctxDemoManualHarness") === "1";
  }, []);

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

  const [newComposerElement, setNewComposerElement] = useState<HTMLDivElement | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(260);
  const [sidebarResizing, setSidebarResizing] = useState(false);
  const [draftHarness, setDraftHarness] = useState<DraftHarness | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const prefetchedProviderOptionsRef = useRef<Set<string>>(new Set());
  const initialHarnessSelectionResolvedRef = useRef(false);
  const composerHarnessAuth = useHarnessAuthenticationController({
    workspaceId,
    enabled: true,
  });

  const focusNewTask = useCallback(() => {
    workbenchStore.focusNewTask();
  }, [workbenchStore]);

  const clearDraftHarness = useCallback(() => {
    setDraftHarness(null);
  }, []);

  const focusTask = useCallback(
    (taskId: string, sessionId?: string | null) => {
      workbenchStore.focusTask(taskId, sessionId);
      return true;
    },
    [workbenchStore],
  );

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
    bootstrapState: providerBootstrapState,
    bootstrapError: providerBootstrapError,
    installAllBusy,
    installProviderFromMenu,
    cancelProviderInstallFromMenu,
    installAllProvidersFromMenu,
    ensureProviderAuthSummary,
    refreshBootstrap,
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

  const { requestHarnessAuthFromComposer } = useWorkbenchComposerHarnessAuth({
    activeTaskId,
    controller: composerHarnessAuth,
    ensureProviderAuthSummary,
    providerOptions,
    setSingleDraftHarness,
  });

  const { dropActive } = useWorkbenchDragDropAttachments({
    scopeElement: newComposerElement,
    activeTaskId,
    setDraftAttachments,
  });

  const { startBlockedReason, startNewTask } = useWorkbenchTaskCreation({
    workspaceId,
    draftPrompt,
    setNewTaskDraft,
    draftAttachments,
    setDraftAttachments,
    draftHarness,
    providersById,
    providerOptions,
    ensureProviderAuthSummary,
    dictationRecording,
    stopDictation,
    focusTask,
    workbenchStore,
    optimisticStartingTaskRef,
    setOptimisticTasks,
    setOptimisticFocus,
    supervisor,
    newTaskDraftKey: NEW_TASK_DRAFT_KEY,
    onStartError: setStartError,
  });

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
      setSidebarWidth((width) => Math.min(max, Math.max(170, Math.round(width))));
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useLayoutEffect(() => {
    if (!workspaceId) return;
    const key = `wb.sidebarWidth.${workspaceId}`;
    try {
      const raw = localStorage.getItem(key);
      const parsed = raw ? Number(raw) : Number.NaN;
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
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.altKey || event.shiftKey) return;
      const hasModifier = event.metaKey || event.ctrlKey;
      if (!hasModifier) return;
      const key = event.key.toLowerCase();
      if (key === "b") {
        event.preventDefault();
        setSidebarCollapsed((prev) => !prev);
      } else if (key === "n") {
        event.preventDefault();
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
      disableAutoselect: manualDemoHarnessSelection,
    });
    if (!shouldFinalizeInitialHarnessSelection(selectedProviderId)) return;
    initialHarnessSelectionResolvedRef.current = true;
    setSingleDraftHarness(selectedProviderId);
  }, [
    activeTaskId,
    draftHarness,
    manualDemoHarnessSelection,
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
    if (!workspaceId) return;
    let cancelled = false;
    const loadWorkspace = async () => {
      const response = await daemonFetchRaw(`/api/workspaces/${workspaceId}`);
      if (cancelled) return;
      if (response.status === 404 || response.status === 400) {
        navigate("/", { replace: true });
        return;
      }
      if (response.status >= 200 && response.status < 300 && response.body) {
        try {
          setWorkspace(JSON.parse(response.body) as Workspace);
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

  const markTaskReadInFlightRef = useRef<Record<string, Promise<void> | undefined>>({});
  const markTaskRead = useCallback(
    async (taskId: string) => {
      if (markTaskReadInFlightRef.current[taskId]) return;
      const promise = (async () => {
        try {
          const updated = await markTaskReadApi(taskId);
          workspaceSnapshotStore.applyTaskUpdate(updated);
        } catch {
          // ignore
        }
      })().finally(() => {
        delete markTaskReadInFlightRef.current[taskId];
      });
      markTaskReadInFlightRef.current[taskId] = promise;
      await promise;
    },
    [workspaceSnapshotStore],
  );

  const markTaskUnread = useCallback(
    async (taskId: string) => {
      try {
        const updated = await markTaskUnreadApi(taskId);
        workspaceSnapshotStore.applyTaskUpdate(updated);
      } catch {
        // ignore
      }
    },
    [workspaceSnapshotStore],
  );

  const {
    sessions,
    activeSessionId,
    taskLiveInfo,
    providerIdsByTaskFromSessions,
    isTaskUnread,
  } = useWorkbenchSessionBridge({
    activeTaskId,
    activeSessionIdFromTab: activeSessionIdFromTabResolved,
    activeTaskSummary,
    tasksById,
    workspaceSnapshot,
    sessionSnap,
    optimisticTasks,
    optimisticTasksById,
    supervisor,
    workbenchStore,
    workspaceSnapshotStore,
    markTaskRead,
  });

  const taskListController = useWorkbenchTaskListController({
    workspaceId,
    activeTaskId,
    tasksById,
    workspaceSnapshot,
    workspaceSnapshotStore,
    optimisticTasks,
    setOptimisticTasks,
    optimisticTasksById,
    taskLiveInfo,
    providerIdsByTaskFromSessions,
    isTaskUnread,
    focusTask,
    focusNewTask,
    markTaskRead,
    markTaskUnread,
    supervisor,
    workbenchStore,
  });

  const activeTaskController = useWorkbenchActiveTaskController({
    workspaceId,
    daemonDataRoot,
    sidebarCollapsed,
    sidebarWidth,
    activeTaskId,
    activeTaskSummary,
    activeSessionId,
    optimisticSessionIdSet,
    optimisticStartingTaskRef,
    workspaceSnapshot,
    workspaceSnapshotStore,
    supervisor,
  });

  useWorkbenchE2EBridge({
    focusNewTask,
    clearDraftHarness,
    focusTask,
    toggleDiffPane: () => activeTaskController.toggleDiffPane("unknown"),
    toggleArtifactsPane: () => activeTaskController.toggleArtifactsPane("unknown"),
  });

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
    const short = (value: string | null) => {
      const text = String(value ?? "");
      return text ? text.slice(0, 8) : "-";
    };
    return `task:${short(activeTaskId)} session:${short(activeSessionId)}`;
  }, [activeTaskId, activeSessionId]);

  const optimisticFailure = activeSessionId
    ? optimisticFailureBySessionId[activeSessionId] ?? null
    : null;
  const canToggleArchive =
    Boolean(activeTaskId) && !taskListController.isArchivePending(activeTaskId);

  const focusTaskSearch = useCallback(() => {
    if (!taskListController.taskSearchRef.current) return false;
    taskListController.taskSearchRef.current.focus();
    taskListController.taskSearchRef.current.select();
    return true;
  }, [taskListController.taskSearchRef]);

  const toggleSidebar = useCallback(() => {
    setSidebarCollapsed((prev) => !prev);
  }, []);

  useEffect(() => {
    if (window.sessionStorage.getItem("ctxE2E") !== "1") return;
    const win = window as Window & {
      __ctxE2E?: {
        focusTask?: (taskId: string, sessionId?: string | null) => boolean;
      };
    };
    win.__ctxE2E ??= {};
    win.__ctxE2E.focusTask = (taskId: string, sessionId?: string | null) => {
      const navToken = workbenchStore.getNavToken();
      return workbenchStore.focusTask(taskId, sessionId, { navToken, source: "system" });
    };
    return () => {
      if (!win.__ctxE2E) return;
      delete win.__ctxE2E.focusTask;
    };
  }, [workbenchStore]);

  const {
    desktopUi,
    desktopStorageNotice,
    setDesktopStorageNotice,
    useHtmlTopbar,
  } = useWorkbenchChromeIntegration({
    enabled: isDesktopApp(),
    workspaceId,
    workspaceName: workspace?.name ?? null,
    state: {
      activeSessionId,
      activeTaskId,
      activeTaskArchived: activeTaskController.activeTaskArchived,
      activeTaskHasAssistantMessage: activeTaskController.activeTaskHasAssistantMessage,
      activeTaskIsOptimistic: activeTaskController.activeTaskIsOptimistic,
      canToggleArchive,
      canInterruptSession: activeTaskController.canInterruptSession,
      copyTranscriptBusy: activeTaskController.copyTranscriptBusy,
      sidebarCollapsed,
      diffOpen: activeTaskController.diffOpen,
      artifactsOpen: activeTaskController.artifactsOpen,
      sessionsOpen: activeTaskController.sessionsOpen,
      terminalOpen: activeTaskController.terminalOpen,
      webSessionsEnabled: activeTaskController.webSessionsEnabled,
      worktreeCanCopy: activeTaskController.worktreeChip.canCopyWorktree,
      worktreeCanOpenTerminal: activeTaskController.worktreeChip.canOpenTerminal,
      isTaskUnread,
    },
    handlers: {
      exportTranscript: activeTaskController.exportTranscript,
      exportSessionLog: activeTaskController.exportSessionLog,
      focusTaskSearch,
      toggleSidebar,
      toggleDiffPane: () => activeTaskController.toggleDiffPane("menu_command"),
      toggleArtifactsPane: () => activeTaskController.toggleArtifactsPane("menu_command"),
      toggleSessionsPane: () => activeTaskController.toggleSessionsPane("menu_command"),
      toggleTerminalPanel: () => activeTaskController.toggleTerminalPanel("menu_command"),
      focusNewTask,
      beginRenameTask: taskListController.beginRenameTask,
      toggleArchiveTask: (taskId, nextArchived) => {
        void taskListController.onToggleArchive(taskId, nextArchived, null).catch(() => {});
      },
      toggleTaskRead: (taskId, unread) => {
        if (unread) {
          void taskListController.markTaskRead(taskId);
          return;
        }
        void taskListController.markTaskUnread(taskId);
      },
      deleteTask: taskListController.onDeleteTask,
      copyTranscript: activeTaskController.copyTranscript,
      copySessionLog: activeTaskController.copySessionLog,
      copyWorktreeLocation: activeTaskController.copyWorktreeLocation,
      copyTaskId: activeTaskController.copyTaskId,
      openWorktreeTerminal: activeTaskController.openWorktreeTerminal,
      interruptSession: (sessionId) => {
        void interruptSession(sessionId).catch(() => {});
      },
    },
  });

  const dismissDesktopStorageNotice = useCallback(() => {
    setDesktopStorageNotice(null);
  }, [setDesktopStorageNotice]);

  const desktopStorageNoticeSubtitle =
    desktopStorageNotice?.reason === "schema_mismatch"
      ? "Desktop detected an outdated local UI state format and reset local UI state."
      : "Desktop detected invalid local UI state data and reset local UI state.";

  const archiveCleanupSnackbar = taskListController.archiveCleanupNotice ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">Archived task, but some cleanup failed.</div>
        <div className="wb-snackbar-subtitle">
          Some worktree files were likely root-owned and could not be removed. Fix permissions and delete them manually if
          needed.
        </div>
      </div>
      <button
        type="button"
        className="wb-snackbar-close"
        onClick={taskListController.dismissArchiveCleanupNotice}
        aria-label="Dismiss"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

  const transcriptNoticeSnackbar = activeTaskController.transcriptNotice ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">{activeTaskController.transcriptNotice}</div>
      </div>
      <button
        type="button"
        className="wb-snackbar-close"
        onClick={activeTaskController.dismissTranscriptNotice}
        aria-label="Dismiss"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

  const desktopStorageNoticeSnackbar = desktopStorageNotice ? (
    <div className="wb-snackbar" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">Local UI state was reset.</div>
        <div className="wb-snackbar-subtitle">{desktopStorageNoticeSubtitle}</div>
      </div>
      <button
        type="button"
        className="wb-snackbar-close"
        onClick={dismissDesktopStorageNotice}
        aria-label="Dismiss"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  ) : null;

  type RootStyle = React.CSSProperties & {
    "--wb-sidebar-width": string;
    "--wb-terminal-offset": string;
    "--wb-topbar-height": string;
  };

  const rootStyle = useMemo<RootStyle>(() => {
    const max = Math.max(170, window.innerWidth - 240);
    const clamped = Math.min(max, Math.max(170, Math.round(sidebarWidth)));
    const terminalOffset = activeTaskController.terminalOpen ? activeTaskController.terminalHeight : 0;
    return {
      "--wb-sidebar-width": `${clamped}px`,
      "--wb-terminal-offset": `${terminalOffset}px`,
      "--wb-topbar-height": useHtmlTopbar ? "46px" : "0px",
    };
  }, [
    activeTaskController.terminalHeight,
    activeTaskController.terminalOpen,
    sidebarWidth,
    useHtmlTopbar,
  ]);

  const copyDebugIds = useCallback(() => {
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
    );
  }, [activeSessionId, activeTaskId, workspaceId]);

  const workspaceTitle = workspace?.name ?? "";
  const slashCommands = useMemo<SlashCommandDescriptor[]>(() => [], []);
  const topbar = useHtmlTopbar ? (
    <div data-tauri-drag-region={desktopUi ? true : undefined}>
      <WorkbenchTopbar
        workspaceId={workspaceId}
        workspaceTitle={workspaceTitle}
        showDebugIds={showDebugIds}
        debugIdLabel={debugIdLabel}
        onCopyDebugIds={copyDebugIds}
      />
    </div>
  ) : null;

  const composerHarnessAuthModal = (
    <HarnessAuthenticationSectionView
      controller={composerHarnessAuth}
      modalOnly
    />
  );

  const onSidebarResizerMouseDown = useCallback(
    (event: React.MouseEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (sidebarCollapsed) return;
      setSidebarResizing(true);
      const startX = event.clientX;
      const startWidth = sidebarWidth;
      const onMove = (moveEvent: MouseEvent) => {
        const dx = moveEvent.clientX - startX;
        const max = Math.max(170, window.innerWidth - 240);
        const next = Math.min(max, Math.max(170, Math.round(startWidth + dx)));
        setSidebarWidth(next);
      };
      const onUp = () => {
        setSidebarResizing(false);
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [sidebarCollapsed, sidebarWidth],
  );

  const workspaceBootstrapGateState = resolveWorkspaceBootstrapGateState({
    workbenchHydrated: workbenchSnap.hydrated,
    providerBootstrapState,
  });

  if (workspaceBootstrapGateState === "loading") {
    return (
      <div
        className={`wb-root ${sidebarCollapsed ? "wb-root-collapsed" : ""} ${sidebarResizing ? "wb-root-resizing" : ""} ${activeTaskController.diffResizing ? "wb-root-diff-resizing" : ""} ${activeTaskController.terminalResizing ? "wb-root-terminal-resizing" : ""} ${!useHtmlTopbar ? "wb-root-native-titlebar" : ""}`}
        style={rootStyle}
      >
        <WorktreeBootstrapSnackbar />
        {composerHarnessAuthModal}
        {archiveCleanupSnackbar}
        {transcriptNoticeSnackbar}
        {desktopStorageNoticeSnackbar}
        {topbar}
        <div className="wb-main">
          <div className="wb-center">
            <div className="wb-muted" style={{ padding: 16 }}>
              Loading workspace...
            </div>
          </div>
        </div>
      </div>
    );
  }

  if (workspaceBootstrapGateState === "error") {
    return (
      <div
        className={`wb-root ${sidebarCollapsed ? "wb-root-collapsed" : ""} ${sidebarResizing ? "wb-root-resizing" : ""} ${activeTaskController.diffResizing ? "wb-root-diff-resizing" : ""} ${activeTaskController.terminalResizing ? "wb-root-terminal-resizing" : ""} ${!useHtmlTopbar ? "wb-root-native-titlebar" : ""}`}
        style={rootStyle}
      >
        <WorktreeBootstrapSnackbar />
        {composerHarnessAuthModal}
        {archiveCleanupSnackbar}
        {transcriptNoticeSnackbar}
        {desktopStorageNoticeSnackbar}
        {topbar}
        <div className="wb-main">
          <div className="wb-center">
            <div style={{ maxWidth: 480, padding: 16 }}>
              <div>Failed to load workspace.</div>
              {providerBootstrapError ? (
                <div className="wb-muted" style={{ paddingTop: 8 }}>
                  {providerBootstrapError}
                </div>
              ) : null}
              <button
                style={{ marginTop: 12 }}
                onClick={() => {
                  void refreshBootstrap();
                }}
                type="button"
              >
                Retry workspace load
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className={`wb-root ${sidebarCollapsed ? "wb-root-collapsed" : ""} ${sidebarResizing ? "wb-root-resizing" : ""} ${activeTaskController.diffResizing ? "wb-root-diff-resizing" : ""} ${activeTaskController.terminalResizing ? "wb-root-terminal-resizing" : ""} ${!useHtmlTopbar ? "wb-root-native-titlebar" : ""}`}
      style={rootStyle}
    >
      <WorktreeBootstrapSnackbar />
      <TitleGenerationInstallBanner />
      {archiveCleanupSnackbar}
      {transcriptNoticeSnackbar}
      {desktopStorageNoticeSnackbar}
      {topbar}

      {composerHarnessAuthModal}

      {workbenchSnap.warnings.length > 0 && (
        <div className="banner" style={{ margin: "8px 12px 0" }}>
          {workbenchSnap.warnings[0]}
        </div>
      )}

      <WorkbenchSidebar
        collapsed={sidebarCollapsed}
        taskSearchRef={taskListController.taskSearchRef}
        taskQuery={taskListController.taskQuery}
        onTaskQueryChange={taskListController.setTaskQuery}
        onNewTask={focusNewTask}
        taskListVirtuosoKey={taskListController.taskListVirtuosoKey}
        taskListItems={taskListController.taskListItems}
        initialTaskListItemCount={taskListController.initialTaskListItemCount}
        computeTaskListItemKey={taskListController.computeTaskListItemKey}
        renderTaskListItem={taskListController.renderTaskListItem}
        taskListContext={taskListController.taskListContext}
        onTaskListRangeChanged={taskListController.onTaskListRangeChanged}
        onExpandSidebar={() => setSidebarCollapsed(false)}
        onCollapseSidebar={() => setSidebarCollapsed(true)}
        onSidebarResizerMouseDown={onSidebarResizerMouseDown}
        onResetSidebarWidth={() => setSidebarWidth(260)}
      />

      <div className="wb-main">
        {!activeTaskId ? (
          <WorkbenchEmptyState
            newComposerRef={setNewComposerElement}
            dropActive={dropActive}
            draftPrompt={draftPrompt}
            setDraftPrompt={setDraftPrompt}
            dictationRecording={dictationRecording}
            onToggleRecording={() => {
              if (dictationRecording) stopDictation().catch(() => {});
              else startDictation().catch(() => {});
            }}
            workspaceId={workspaceId}
            slashCommands={slashCommands}
            draftAttachments={draftAttachments}
            setDraftAttachments={setDraftAttachments}
            onSend={startNewTask}
            sendDisabled={Boolean(startBlockedReason)}
            sendDisabledReason={startBlockedReason}
            draftMode={draftMode}
            setDraftMode={setDraftMode}
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
            dictationDebugText={dictationDebugText}
            dictationError={dictationError}
            startError={startError}
            dictationOnboarding={dictationOnboarding}
            onCloseDictationOnboarding={dismissDictationOnboarding}
            onBackDictationOnboarding={backDictationOnboarding}
            onChooseDictationOnboardingLocal={chooseDictationOnboardingLocal}
            onChooseDictationOnboardingCloud={chooseDictationOnboardingCloud}
            onCloudChangeDictationOnboarding={updateDictationOnboardingCloud}
            onSubmitCloudDictationOnboarding={() => {
              void submitDictationOnboardingCloud();
            }}
            onSubmitLocalDictationOnboarding={() => {
              void submitDictationOnboardingLocal();
            }}
          />
        ) : null}

        {activeTaskId && (
          <WorkbenchActiveTaskView
            sessionsCount={sessions.length}
            showSingleSessionHeader={activeTaskController.showSingleSessionHeader}
            singleSessionTitle={activeTaskController.singleSessionHeaderForRender?.title ?? "Conversation"}
            worktreeChip={activeTaskController.worktreeChip}
            worktreeCopied={activeTaskController.worktreeCopied}
            showArtifactsPane={activeTaskController.showArtifactsPane}
            showReviewPane={activeTaskController.showReviewPane}
            terminalOpen={activeTaskController.terminalOpen}
            artifactsCount={activeTaskController.artifacts.length}
            diffBadgeCount={activeTaskController.diffBadgeCount}
            onCopyWorktreeLocation={() => void activeTaskController.copyWorktreeLocation()}
            onOpenWorktreeTerminal={() => void activeTaskController.openWorktreeTerminal()}
            onToggleArtifactsPane={() => activeTaskController.toggleArtifactsPane("header_button")}
            onToggleDiffPane={() => activeTaskController.toggleDiffPane("header_button")}
            onToggleTerminalPanel={() => activeTaskController.toggleTerminalPanel("header_button")}
            onOpenConvoMenu={activeTaskController.openConvoMenu}
            sessionLoadIssues={activeTaskController.sessionLoadIssues}
            onRetrySessionLoads={activeTaskController.retryActiveSessionLoads}
            activeSessionId={activeSessionId}
            activeSessionRenderable={activeTaskController.activeSessionRenderable}
            optimisticFailure={optimisticFailure}
            rightPaneOpen={activeTaskController.rightPaneOpen}
            onSplitterMouseDown={activeTaskController.onSplitterMouseDown}
            diffWidth={activeTaskController.diffWidth}
            showSessionsPane={activeTaskController.showSessionsPane}
            sessionSections={activeTaskController.sessionSections}
            activeSessionKind={activeTaskController.activeSessionKind}
            onSectionChange={activeTaskController.setActiveSessionKind}
            activeWebSessionId={activeTaskController.activeWebSessionId}
            onSelectWebSession={activeTaskController.setActiveWebSessionId}
            daemonBaseUrl={activeTaskController.daemonBaseUrl}
            webSessionsLoading={activeTaskController.webSessionsLoading}
            hasDiff={activeTaskController.hasDiff}
            gitPaneModel={activeTaskController.gitPaneModel}
            diffLoading={activeTaskController.diffLoading}
            diffSummaryError={activeTaskController.diffSummaryError}
            diffTooLarge={activeTaskController.diffTooLarge}
            diffTooLargeLabel={activeTaskController.diffTooLargeLabel}
            activeSessionDiff={activeTaskController.activeSessionDiff}
            activeDiffContentError={activeTaskController.activeDiffContentError}
            diffEmptyLabel={activeTaskController.diffEmptyLabel}
            artifacts={activeTaskController.artifacts}
            artifactsLoading={activeTaskController.artifactsLoading}
            artifactsError={activeTaskController.artifactsError}
            onRetryArtifactsLoad={activeTaskController.retryArtifactsLoad}
          />
        )}
      </div>

      <div className="wb-terminal-shell" aria-hidden={!activeTaskController.terminalOpen}>
        {activeTaskController.terminalOpen && (
          <div className="wb-terminal-resizer" onMouseDown={activeTaskController.onTerminalResizerMouseDown} />
        )}
        <div
          className="wb-terminal-panel"
          style={{
            height: activeTaskController.terminalOpen ? activeTaskController.terminalHeight : 0,
            pointerEvents: activeTaskController.terminalOpen ? "auto" : "none",
          }}
          aria-hidden={!activeTaskController.terminalOpen}
        >
          <TerminalPanel
            ref={activeTaskController.terminalPanelRef}
            workspaceId={workspaceId}
            activeTaskId={activeTaskId}
            activeSessionId={activeSessionId}
            open={activeTaskController.terminalOpen}
            height={activeTaskController.terminalHeight}
            onRequestClose={activeTaskController.closeTerminalPanel}
          />
        </div>
      </div>

      <WorkbenchPageMenus
        activeTaskController={activeTaskController}
        taskListController={taskListController}
        activeTaskId={activeTaskId}
        activeSessionId={activeSessionId}
      />
    </div>
  );
}
