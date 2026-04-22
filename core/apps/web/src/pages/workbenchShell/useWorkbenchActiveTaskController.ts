import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type MutableRefObject } from "react";

import {
  type WebSessionInfo,
  type Worktree,
  getSessionDiff,
  getWorktree,
  idToString,
  listWebSessions,
  type SessionTurn,
} from "../../api/client";
import { useDaemonBaseUrl } from "../../api/useDaemonConnection";
import { artifactPrefetcher } from "../../state/artifactPrefetch";
import { type SessionSupervisor, useOpenSession, useSessionCacheSnapshot } from "../../state/sessionSupervisor";
import type { WorkspaceActiveSnapshotItem, WorkspaceActiveSnapshotState } from "../../state/workspaceActiveSnapshotStore";
import type { TerminalPanelHandle } from "../../components/TerminalPanel";
import { HARNESS_CATALOG } from "../../utils/harnessCatalog";
import { trackWorkbenchPanelToggled } from "../../utils/analytics";
import { errorMessage } from "../../utils/errorMessage";
import { composeModelId, parseModelId } from "../../utils/modelEffort";
import { hasSessionActiveTurn } from "../../utils/sessionActivity";
import {
  loadWorkbenchTerminalPanelOpenV1,
  saveWorkbenchArtifactsPaneOpenV1,
  saveWorkbenchDiffPaneOpenV1,
  saveWorkbenchSessionsPaneOpenV1,
  saveWorkbenchTerminalPanelOpenV1,
} from "../../workbench/persistence";
import type { OptimisticTaskSummary } from "./WorkbenchPage.types";
import {
  deriveManagedWorktreeRoot,
  formatWorktreeLabel,
  isOptimisticTask,
  parseMs,
} from "./WorkbenchPage.utils";
import { canRenderWorkbenchActiveSession } from "./workbenchTaskActivity";
import { getDiffSummaryStats, isDiffSummaryTooLarge } from "./useWorkbenchDiffPane";
import { useWorkbenchSessionActions } from "./useWorkbenchSessionActions";
import {
  resolveMeasuredSessionSwitchId,
  useWorkbenchSessionSwitchMetrics,
} from "./useWorkbenchSessionSwitchMetrics";

const compareSessionTurnOrder = (left: SessionTurn, right: SessionTurn): number => {
  const leftSeq = Number(left.start_seq ?? Number.NaN);
  const rightSeq = Number(right.start_seq ?? Number.NaN);
  if (Number.isFinite(leftSeq) && Number.isFinite(rightSeq) && leftSeq !== rightSeq) {
    return leftSeq - rightSeq;
  }
  if (Number.isFinite(leftSeq) && !Number.isFinite(rightSeq)) return -1;
  if (!Number.isFinite(leftSeq) && Number.isFinite(rightSeq)) return 1;
  const leftStartedAt = String(left.started_at ?? "");
  const rightStartedAt = String(right.started_at ?? "");
  if (leftStartedAt !== rightStartedAt) {
    return leftStartedAt.localeCompare(rightStartedAt);
  }
  return String(left.turn_id ?? "").localeCompare(String(right.turn_id ?? ""));
};

const getLatestTurnStatus = (turns: SessionTurn[] | null | undefined): SessionTurn["status"] | null => {
  let latestTurn: SessionTurn | null = null;
  for (const turn of turns ?? []) {
    if (!latestTurn || compareSessionTurnOrder(turn, latestTurn) > 0) {
      latestTurn = turn;
    }
  }
  return latestTurn?.status ?? null;
};
import { buildGitPaneModel } from "./worktreeGitPaneModel";

type PaneMode = "diff" | "artifacts" | "sessions" | null;

type WorkspaceSnapshotStore = {
  getWorktreeRoot: (worktreeId: string) => string | null | undefined;
  setVcsOpenSessionIds?: (sessionIds: string[]) => void;
};

type WorkbenchActiveTaskControllerArgs = {
  workspaceId: string;
  daemonDataRoot: string | null;
  sidebarCollapsed: boolean;
  sidebarWidth: number;
  activeTaskId: string | null;
  activeTaskSummary: WorkspaceActiveSnapshotItem | OptimisticTaskSummary | null;
  activeSessionId: string | null;
  optimisticSessionIdSet: Set<string>;
  optimisticStartingTaskRef: MutableRefObject<{ primarySessionId?: string | null } | null>;
  workspaceSnapshot: WorkspaceActiveSnapshotState;
  workspaceSnapshotStore: WorkspaceSnapshotStore;
  supervisor: Pick<
    SessionSupervisor,
    | "getSnapshot"
    | "loadMoreTurns"
    | "loadSessionState"
    | "loadSubagentInvocations"
    | "setDiff"
  >;
};

export function useWorkbenchActiveTaskController({
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
}: WorkbenchActiveTaskControllerArgs) {
  const sessionCache = useSessionCacheSnapshot();
  const activeEntry = activeSessionId ? sessionCache.sessions[activeSessionId] ?? null : null;
  const activeSessionRenderable = canRenderWorkbenchActiveSession(activeEntry);
  const activeLoadErrors = activeEntry?.loadErrors;
  const activeSessionDiff = activeEntry?.diff ?? "";
  const activeTask = activeTaskSummary?.task ?? null;
  const activeTaskIsOptimistic = activeTaskSummary ? isOptimisticTask(activeTaskSummary) : false;
  const activeTaskHasAssistantMessage = Boolean(activeTask?.last_assistant_message_at);
  const activeTaskArchived = Boolean(activeTask?.archived_at);

  const clampDiffWidth = useCallback((value: number) => {
    const gutterPx = 8;
    const containerWidth =
      (document.querySelector(".wb-body") as HTMLElement | null)?.clientWidth ?? window.innerWidth;
    const min = Math.min(320, containerWidth);
    const max = Math.max(min, containerWidth - gutterPx);
    return Math.min(max, Math.max(min, Math.round(value)));
  }, []);

  const [rightPaneMode, setRightPaneMode] = useState<PaneMode>(null);
  const [diffWidth, setDiffWidth] = useState(() => clampDiffWidth(480));
  const [diffResizing, setDiffResizing] = useState(false);
  const [diffOpenHydrated, setDiffOpenHydrated] = useState(false);
  const [diffContentLoading, setDiffContentLoading] = useState(false);
  const [diffContentErrorBySessionId, setDiffContentErrorBySessionId] = useState<Record<string, string | undefined>>(
    {},
  );
  const [artifactsOpenHydrated, setArtifactsOpenHydrated] = useState(false);
  const [artifactsOpenSeeded, setArtifactsOpenSeeded] = useState(false);
  const [, setArtifactsAutoOpenPending] = useState(false);
  const artifactsPaneScopeRef = useRef<string | null>(null);
  const [sessionsOpenHydrated, setSessionsOpenHydrated] = useState(false);
  const sessionsPaneScopeRef = useRef<string | null>(null);
  const terminalPanelRef = useRef<TerminalPanelHandle | null>(null);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [terminalHeight, setTerminalHeight] = useState(260);
  const [terminalResizing, setTerminalResizing] = useState(false);
  const [terminalOpenHydrated, setTerminalOpenHydrated] = useState(false);
  const rightPaneModeRef = useRef<PaneMode>(rightPaneMode);
  const terminalOpenRef = useRef<boolean>(terminalOpen);
  const diffOpen = rightPaneMode === "diff";
  const artifactsOpen = rightPaneMode === "artifacts";
  const sessionsOpen = rightPaneMode === "sessions";

  const clampTerminalHeight = useCallback((value: number) => {
    const min = 160;
    const max = Math.max(min, window.innerHeight - 160);
    return Math.min(max, Math.max(min, Math.round(value)));
  }, []);

  useLayoutEffect(() => {
    setDiffWidth((width) => {
      const clamped = clampDiffWidth(width);
      return clamped === width ? width : clamped;
    });
  }, [clampDiffWidth, sidebarCollapsed, sidebarWidth]);

  useEffect(() => {
    rightPaneModeRef.current = rightPaneMode;
  }, [rightPaneMode]);

  useEffect(() => {
    terminalOpenRef.current = terminalOpen;
  }, [terminalOpen]);

  useEffect(() => {
    const onResize = () => {
      setTerminalHeight((height) => clampTerminalHeight(height));
      setDiffWidth((width) => clampDiffWidth(width));
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [clampDiffWidth, clampTerminalHeight]);

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
    const onKeyDown = (event: KeyboardEvent) => {
      const isToggleKey = event.code === "Backquote" || event.key === "`";
      if (event.ctrlKey && !event.metaKey && !event.altKey && isToggleKey) {
        event.preventDefault();
        toggleTerminalPanel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggleTerminalPanel]);

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

  const optimisticStartingSessionId = String(optimisticStartingTaskRef.current?.primarySessionId ?? "");
  const activeSessionIdValue = activeSessionId ?? "";
  const isOptimisticSessionId =
    !!activeSessionIdValue &&
    (optimisticSessionIdSet.has(activeSessionIdValue) ||
      activeSessionIdValue.startsWith("optimistic-") ||
      optimisticStartingSessionId === activeSessionIdValue);
  const openSessionId = activeSessionId && !isOptimisticSessionId ? activeSessionId : "";
  useOpenSession(openSessionId, { watchDiff: diffOpen });

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
  const gitPaneModel = useMemo(() => buildGitPaneModel(activeWorktreeVcsSnapshot), [activeWorktreeVcsSnapshot]);
  useEffect(() => {
    workspaceSnapshotStore.setVcsOpenSessionIds?.(diffOpen && openSessionId && gitPaneModel.inventoryDemandAllowed ? [openSessionId] : []);
  }, [diffOpen, gitPaneModel.inventoryDemandAllowed, openSessionId, workspaceSnapshotStore]);
  const snapshotSummaryStats = useMemo(
    () => getDiffSummaryStats(activeWorktreeVcsSummary),
    [activeWorktreeVcsSummary],
  );
  const snapshotHasCounts = snapshotSummaryStats.fileCount !== null || snapshotSummaryStats.lineCount !== null;
  const diffSummary = activeWorktreeDiffAvailable && snapshotHasCounts ? activeWorktreeVcsSummary : null;
  const diffSummaryError =
    activeWorktreeDiffAvailable && activeWorktreeVcsComputeState === "error"
      ? "Failed to compute diff summary."
      : null;
  const diffSummaryLoading =
    !diffSummaryError && activeWorktreeDiffAvailable && (!activeWorktreeVcsSnapshot || !snapshotHasCounts);
  const diffLoading = diffSummaryLoading || diffContentLoading;

  const [activeWorktree, setActiveWorktree] = useState<Worktree | null>(null);
  const worktreeCacheRef = useRef<Map<string, Worktree>>(new Map());
  const worktreeFetchRef = useRef<Map<string, Promise<Worktree | null>>>(new Map());
  const [webSessions, setWebSessions] = useState<WebSessionInfo[]>([]);
  const [webSessionsLoading, setWebSessionsLoading] = useState(false);
  const [activeWebSessionId, setActiveWebSessionId] = useState<string | null>(null);
  const [activeSessionKind, setActiveSessionKind] = useState("web");
  const webSessionsEnabled = false;
  useWorkbenchSessionSwitchMetrics({
    activeEntry,
    activeSessionId: activeSessionId ?? null,
    isOptimisticSessionId,
  });

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
        .then((worktree) => {
          worktreeCacheRef.current.set(activeWorktreeId, worktree);
          return worktree;
        })
        .catch(() => null)
        .finally(() => {
          worktreeFetchRef.current.delete(activeWorktreeId);
        });
    worktreeFetchRef.current.set(activeWorktreeId, fetchPromise);
    fetchPromise
      .then((worktree) => {
        if (cancelled) return;
        setActiveWorktree(worktree);
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
    if (!webSessionsEnabled) return;
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
  }, [activeSessionId, refreshWebSessions, webSessionsEnabled]);

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
  const diffUnavailableLabel = gitPaneModel.unavailableLabel;
  const hasDiff =
    !!diffSummaryError ||
    gitPaneModel.loading ||
    !gitPaneModel.listReady ||
    gitPaneModel.totalCount > 0 ||
    !!diffUnavailableLabel;
  const diffEmptyLabel = diffUnavailableLabel ?? (diffLoading || !diffSummaryReady ? "Loading changes..." : "No changes on this worktree.");
  const diffBadgeCount = useMemo(() => {
    return gitPaneModel.badgeCount;
  }, [gitPaneModel.badgeCount]);
  const gitStatusSignature = useMemo(() => {
    if (activeWorktreeVcsSnapshot) {
      return [
        `rev:${String(activeWorktreeVcsSnapshot.rev ?? "")}`,
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
  const activeSessionCacheEntry = activeSessionId ? sessionCache.sessions[activeSessionId] : undefined;
  const artifacts = useMemo(() => {
    return activeSessionCacheEntry?.artifacts ?? [];
  }, [activeSessionCacheEntry]);
  const artifactsLoading = activeSessionCacheEntry?.artifactsLoading ?? false;
  const artifactsError =
    activeSessionCacheEntry?.stateLoaded || artifacts.length > 0 ? null : (activeLoadErrors?.state ?? null);
  const sessionLoadIssues = useMemo(() => {
    const issues: Array<{ key: "state" | "subagentInvocations"; message: string }> = [];
    if (activeLoadErrors?.state) {
      issues.push({ key: "state", message: activeLoadErrors.state });
    }
    if (activeLoadErrors?.subagentInvocations) {
      issues.push({
        key: "subagentInvocations",
        message: activeLoadErrors.subagentInvocations,
      });
    }
    return issues;
  }, [activeLoadErrors?.state, activeLoadErrors?.subagentInvocations]);

  useEffect(() => {
    if (!artifactsOpen || !activeSessionId) return;
    supervisor.loadSessionState(activeSessionId);
  }, [activeSessionId, artifactsOpen, supervisor]);

  useEffect(() => {
    artifactPrefetcher.prefetch(activeSessionId ?? null, artifacts, !activeTaskArchived);
  }, [activeSessionId, activeTaskArchived, artifacts]);

  const retryActiveSessionLoads = useCallback(() => {
    if (!activeSessionId) return;
    supervisor.loadSessionState(activeSessionId, { force: true });
    supervisor.loadSubagentInvocations(activeSessionId, { force: true });
  }, [activeSessionId, supervisor]);

  const retryArtifactsLoad = useCallback(() => {
    if (!activeSessionId) return;
    supervisor.loadSessionState(activeSessionId, { force: true });
  }, [activeSessionId, supervisor]);

  const diffContentInFlightRef = useRef<Map<string, Promise<void>>>(new Map());
  const diffRefreshTimerRef = useRef<number | null>(null);
  const diffRefreshSignatureRef = useRef<Map<string, string>>(new Map());

  const refreshDiff = useCallback(
    async (sessionId: string) => {
      if (!sessionId) return;
      const currentOptimisticSessionId = String(optimisticStartingTaskRef.current?.primarySessionId ?? "");
      if (
        optimisticSessionIdSet.has(sessionId) ||
        (currentOptimisticSessionId && currentOptimisticSessionId === sessionId)
      ) {
        return;
      }
      if (sessionId !== activeSessionId) return;
      if (!activeWorktreeDiffAvailable) {
        setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
        supervisor.setDiff(sessionId, "");
        return;
      }
      if (gitPaneModel.totalCount <= 0 || !activeWorktreeVcsSummary) {
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
          const response = await getSessionDiff(sessionId);
          if (response.available === false) {
            supervisor.setDiff(sessionId, "");
            setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
            return;
          }
          supervisor.setDiff(sessionId, response.diff ?? "");
          setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: undefined }));
        } catch (error: unknown) {
          supervisor.setDiff(sessionId, "");
          const detail = errorMessage(error);
          const message = detail ? `Failed to load diff content: ${detail}` : "Failed to load diff content.";
          setDiffContentErrorBySessionId((prev) => ({ ...prev, [sessionId]: message }));
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
      gitPaneModel.totalCount,
      optimisticSessionIdSet,
      optimisticStartingTaskRef,
      supervisor,
    ],
  );

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

  const onSplitterMouseDown = useCallback(
    (event: React.MouseEvent) => {
      event.preventDefault();
      const startX = event.clientX;
      const startWidth = diffWidth;
      setDiffResizing(true);
      const onMove = (moveEvent: MouseEvent) => {
        const dx = startX - moveEvent.clientX;
        const next = Math.min(900, startWidth + dx);
        setDiffWidth(clampDiffWidth(next));
      };
      const onUp = () => {
        setDiffResizing(false);
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [clampDiffWidth, diffWidth],
  );

  const onTerminalResizerMouseDown = useCallback(
    (event: React.MouseEvent) => {
      event.preventDefault();
      const startY = event.clientY;
      const startHeight = terminalHeight;
      setTerminalResizing(true);
      const onMove = (moveEvent: MouseEvent) => {
        const dy = startY - moveEvent.clientY;
        const next = clampTerminalHeight(startHeight + dy);
        setTerminalHeight(next);
      };
      const onUp = () => {
        setTerminalResizing(false);
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [clampTerminalHeight, terminalHeight],
  );

  const worktreeChip = useMemo(() => {
    const session = activeEntry?.session ?? null;
    const worktreeRoot = String(activeWorktree?.root_path ?? "");
    const executionEnvironment = String(session?.execution_environment ?? "").trim();
    const worktreePath = worktreeRoot;
    const worktreeLabel = worktreePath
      ? formatWorktreeLabel(worktreePath)
      : executionEnvironment === "sandbox"
        ? "Sandbox worktree"
        : executionEnvironment === "host"
          ? "Session worktree"
          : "";

    return {
      worktreeLabel,
      worktreePath,
      canCopyWorktree: Boolean(worktreePath),
      canOpenTerminal: Boolean(worktreePath),
      copyPath: worktreePath,
    };
  }, [activeEntry, activeWorktree?.root_path]);

  const singleSessionHeader = useMemo(() => {
    const session = activeEntry?.session ?? null;
    if (!session) return null;
    const parsedModel = parseModelId(
      composeModelId(session?.model_id ?? "", session?.reasoning_effort ?? null),
    );
    const harness =
      HARNESS_CATALOG.find((item) => item.id === (session?.provider_id ?? ""))?.label ??
      (session?.provider_id ?? "Provider");

    const lastIso = (() => {
      if (!activeEntry) return null;
      let bestIso: string | null = activeEntry.session?.updated_at ?? null;
      let bestMs = bestIso ? parseMs(bestIso) ?? -1 : -1;
      for (const message of activeEntry.messages ?? []) {
        const ms = parseMs(message.created_at);
        if (ms !== null && ms >= bestMs) {
          bestMs = ms;
          bestIso = message.created_at;
        }
      }
      for (const event of activeEntry.events ?? []) {
        const ms = parseMs(event.created_at);
        if (ms !== null && ms >= bestMs) {
          bestMs = ms;
          bestIso = event.created_at;
        }
      }
      return bestIso;
    })();

    return {
      title: activeTask?.title ?? "Conversation",
      lastIso,
      harness,
      modelBase: parsedModel.base || String(session?.model_id ?? ""),
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
  const {
    copyTranscriptBusy,
    transcriptNotice,
    setTranscriptNotice,
    transcriptSpinnerDelayMs,
    worktreeCopied,
    copyWorktreeLocation,
    copyTaskId,
    openWorktreeTerminal,
    exportSessionLog,
    copySessionLog,
    exportTranscript,
    copyTranscript,
  } = useWorkbenchSessionActions({
    activeEntry,
    activeSessionId,
    activeTaskId,
    activeWorktreeId,
    singleSessionTitle: singleSessionHeader?.title ?? null,
    worktreePath: worktreeChip.worktreePath,
    canCopyWorktree: worktreeChip.canCopyWorktree,
    canCopyTaskId: Boolean(activeTaskId) && !activeTaskIsOptimistic,
    canOpenTerminal: worktreeChip.canOpenTerminal,
    terminalPanelRef,
    setTerminalOpen,
    getSupervisorSnapshot: () => supervisor.getSnapshot(),
    loadMoreTurns: async (sessionId) => {
      await supervisor.loadMoreTurns(sessionId);
    },
  });

  const dismissTranscriptNotice = useCallback(() => {
    setTranscriptNotice(null);
  }, [setTranscriptNotice]);

  const canInterruptSession =
    Boolean(activeSessionId) &&
    hasSessionActiveTurn(activeEntry?.activity, getLatestTurnStatus(activeEntry?.turns ?? null));

  const [convoMenu, setConvoMenu] = useState<{ style: React.CSSProperties } | null>(null);
  const convoMenuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (!convoMenu) return;
      const element = event.target as HTMLElement | null;
      if (element && (element.closest(".wb-convo-menu") || element.closest(".wb-convo-menu-trigger"))) return;
      setConvoMenu(null);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setConvoMenu(null);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [convoMenu]);

  const openConvoMenu = useCallback((triggerEl: HTMLElement) => {
    const rect = triggerEl.getBoundingClientRect();
    const left = Math.min(rect.left, window.innerWidth - 240);
    const top = Math.min(rect.bottom + 6, window.innerHeight - 220);
    setConvoMenu((prev) => (prev ? null : { style: { left, top } }));
  }, []);

  const closeConvoMenu = useCallback(() => {
    setConvoMenu(null);
  }, []);

  const closeTerminalPanel = useCallback(() => {
    setTerminalOpen(false);
  }, []);

  return {
    activeTask,
    activeTaskArchived,
    activeTaskIsOptimistic,
    activeTaskHasAssistantMessage,
    activeSessionRenderable,
    showSingleSessionHeader,
    singleSessionHeaderForRender,
    worktreeChip,
    worktreeCopied,
    copyTranscriptBusy,
    transcriptNotice,
    dismissTranscriptNotice,
    transcriptSpinnerDelayMs,
    copyWorktreeLocation,
    copyTaskId,
    openWorktreeTerminal,
    exportSessionLog,
    copySessionLog,
    exportTranscript,
    copyTranscript,
    canInterruptSession,
    diffOpen,
    artifactsOpen,
    sessionsOpen,
    showReviewPane,
    showArtifactsPane,
    showSessionsPane,
    rightPaneOpen,
    diffWidth,
    diffResizing,
    onSplitterMouseDown,
    terminalOpen,
    terminalHeight,
    terminalResizing,
    terminalPanelRef,
    closeTerminalPanel,
    onTerminalResizerMouseDown,
    toggleDiffPane,
    toggleArtifactsPane,
    toggleSessionsPane,
    toggleTerminalPanel,
    sessionLoadIssues,
    retryActiveSessionLoads,
    activeSessionDiff,
    activeDiffContentError,
    activeWebSessionId,
    setActiveWebSessionId,
    activeSessionKind,
    setActiveSessionKind,
    daemonBaseUrl,
    webSessionsEnabled,
    webSessionsLoading,
    sessionSections,
    hasDiff,
    diffSummaryError,
    diffTooLarge,
    diffTooLargeLabel,
    diffEmptyLabel,
    artifacts,
    artifactsLoading,
    artifactsError,
    retryArtifactsLoad,
    convoMenu,
    convoMenuRef,
    openConvoMenu,
    closeConvoMenu,
    gitPaneModel,
    diffLoading,
    diffBadgeCount,
  };
}
