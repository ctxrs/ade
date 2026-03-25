import type {
  Session,
  SessionHeadDelta,
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  SessionSummary,
  Task,
  WorktreeVcsSnapshot,
  WorkspaceActiveSnapshot,
  WorkspaceActiveSnapshotSessionSummaryDeltaEvent,
  WorkspaceActiveSnapshotTaskDeltaEvent,
  WorkspaceActiveTaskSummary,
  WorkspaceArchivedPage,
  WorkspaceIndexCursor,
  WorkspaceTaskSummary,
} from "@ctx/types";
import { idToString } from "../../api/client";
import type { WorkspaceActiveSnapshotPatch } from "../workspaceActiveSnapshotProtocol";
import {
  emptySessionHeadWindow,
  mergeSessionEvents,
  mergeSessionMessages,
  mergeSessionToolSummaries,
  mergeSessionTurns,
  sanitizeSessionHeadSnapshot,
} from "../sessionHeadState";
import type {
  PersistedWorkspaceActiveSnapshotV1,
  PersistedWorkspaceActiveTaskSummaryV1,
} from "../uiStateStore";
import type {
  WorkspaceActiveSnapshotItem,
  WorkspaceActiveSnapshotState,
} from "./storeTypes";
import { findWorkspaceActiveSnapshotInsertIndex } from "./storeOrdering";
import {
  collectWorkspaceActivePrimarySessionIds,
  hasOwnProperty,
  mapWorktreeVcsSnapshots,
  projectPrimarySessionHeadOntoTasks,
  resolvePrimarySessionId,
  sortSessionSummaries,
} from "./projection";
import {
  isSessionHeadCompatibleWithSummary,
  normalizeSessionSummary,
  pickArchivedSessionId,
  pickArchivedSessionIdFromSummaries,
  readPrimarySessionHead,
  readPrimarySessionId,
  sessionToSummary,
  shouldReplaceSessionHead,
  taskSortAt,
} from "./summaryHelpers";

export class WorkspaceActiveSnapshotStoreState {
  private snapshot: WorkspaceActiveSnapshotState;
  private tasks = new Map<string, WorkspaceActiveSnapshotItem>();
  private sessionHeadsById = new Map<string, SessionHeadSnapshot>();
  private worktreeRootsById = new Map<string, string>();
  private activeSessionIds: string[] = [];
  private activeOrder: string[] = [];
  private archivedOrder: string[] = [];
  private totalActive = 0;
  private totalArchived = 0;
  private hasMoreActive = true;
  private hasMoreArchived = false;
  private archivedLoaded = false;
  private archivedCursor: WorkspaceIndexCursor | null = null;
  private snapshotRev = 0;
  private archivedRev = 0;
  private liveSnapshotApplied = false;

  constructor(private readonly workspaceId: string) {
    this.snapshot = {
      workspaceId,
      initialized: false,
      liveSnapshotApplied: false,
      connection: "idle",
      tasksById: {},
      activeIds: [],
      archivedIds: [],
      totalActive: 0,
      totalArchived: 0,
      archivedRev: 0,
      worktreeVcsById: {},
      fetchState: { active: "idle", archived: "idle" },
      hasMoreActive: true,
      hasMoreArchived: false,
      archivedLoaded: false,
    };
  }

  getSnapshot = (): WorkspaceActiveSnapshotState => this.snapshot;

  getSessionHeadSnapshot = (sessionId: string): SessionHeadSnapshot | null => {
    const id = idToString(sessionId);
    if (!id) return null;
    return this.sessionHeadsById.get(id) ?? null;
  };

  getSessionHeadsSnapshot = (): Record<string, SessionHeadSnapshot> => {
    return Object.fromEntries(this.sessionHeadsById.entries()) as Record<string, SessionHeadSnapshot>;
  };

  getWorktreeRoot = (worktreeId: string): string | null => {
    const id = idToString(worktreeId);
    if (!id) return null;
    return this.worktreeRootsById.get(id) ?? null;
  };

  getWorktreeRootsSnapshot = (): Record<string, string> => {
    return Object.fromEntries(this.worktreeRootsById.entries()) as Record<string, string>;
  };

  getWorktreeVcsSnapshot = (worktreeId: string): WorktreeVcsSnapshot | null => {
    const id = idToString(worktreeId);
    if (!id) return null;
    return this.snapshot.worktreeVcsById[id] ?? null;
  };

  getWorktreeVcsSnapshots = (): WorktreeVcsSnapshot[] => {
    return Object.values(this.snapshot.worktreeVcsById ?? {});
  };

  getSnapshotRev = (): number => this.snapshotRev;

  getArchivedRev = (): number => this.archivedRev;

  getActiveSessionIds = (): string[] => this.activeSessionIds.slice();

  getFetchState = (target: "active" | "archived"): "idle" | "loading" | "error" => {
    return this.snapshot.fetchState[target];
  };

  getHasMoreArchived = (): boolean => this.hasMoreArchived;

  getArchivedCursor = (): WorkspaceIndexCursor | null => this.archivedCursor;

  hasLiveSnapshotApplied = (): boolean => this.liveSnapshotApplied;

  setConnection(connection: WorkspaceActiveSnapshotState["connection"]): boolean {
    if (this.snapshot.connection === connection) return false;
    this.snapshot = { ...this.snapshot, connection };
    return true;
  }

  setFetchState(target: "active" | "archived", state: "idle" | "loading" | "error"): boolean {
    if (this.snapshot.fetchState[target] === state) return false;
    this.snapshot = {
      ...this.snapshot,
      fetchState: { ...this.snapshot.fetchState, [target]: state },
    };
    return true;
  }

  updateSnapshotRev(nextRev: number, opts?: { allowReset?: boolean }): boolean {
    if (!Number.isFinite(nextRev)) return false;
    if (opts?.allowReset && nextRev < this.snapshotRev) {
      if (nextRev === this.snapshotRev) return false;
      this.snapshotRev = nextRev;
      return true;
    }
    if (nextRev <= this.snapshotRev) return false;
    this.snapshotRev = nextRev;
    return true;
  }

  updateArchivedRev(nextRev: number): boolean {
    if (!Number.isFinite(nextRev) || nextRev === this.archivedRev) return false;
    this.archivedRev = nextRev;
    this.archivedLoaded = false;
    this.archivedCursor = null;
    this.syncSnapshot();
    return true;
  }

  applyWorktreeRoot(worktreeId: string, root: string): boolean {
    const id = idToString(worktreeId);
    const nextRoot = String(root ?? "").trim();
    if (!id || !nextRoot || this.worktreeRootsById.get(id) === nextRoot) return false;
    this.worktreeRootsById.set(id, nextRoot);
    return true;
  }

  applyWorktreeVcsSnapshot(snapshot: WorktreeVcsSnapshot): boolean {
    const worktreeId = idToString(snapshot.worktree_id);
    if (!worktreeId) return false;
    const prev = this.snapshot.worktreeVcsById[worktreeId];
    if (prev?.rev === snapshot.rev) return false;
    this.snapshot = {
      ...this.snapshot,
      worktreeVcsById: {
        ...this.snapshot.worktreeVcsById,
        [worktreeId]: snapshot,
      },
    };
    return true;
  }

  applyWorkerPatch(patch: WorkspaceActiveSnapshotPatch) {
    this.snapshot = patch.snapshot;
    this.tasks = new Map(Object.entries(patch.snapshot.tasksById));
    this.activeOrder = patch.snapshot.activeIds.slice();
    this.archivedOrder = patch.snapshot.archivedIds.slice();
    this.totalActive = patch.snapshot.totalActive;
    this.totalArchived = patch.snapshot.totalArchived;
    this.archivedRev = patch.snapshot.archivedRev;
    this.hasMoreActive = patch.snapshot.hasMoreActive;
    this.hasMoreArchived = patch.snapshot.hasMoreArchived;
    this.archivedLoaded = patch.snapshot.archivedLoaded;
    this.activeSessionIds = patch.activeSessionIds.slice();
    this.sessionHeadsById = new Map(Object.entries(patch.sessionHeads));
    this.worktreeRootsById = new Map(Object.entries(patch.worktreeRoots));
    this.snapshotRev = patch.snapshotRev;
    this.archivedRev = patch.archivedRev;
    this.liveSnapshotApplied = Boolean(patch.snapshot.liveSnapshotApplied);
  }

  buildPersistedSnapshot(): Omit<
    PersistedWorkspaceActiveSnapshotV1,
    "v" | "workspaceId" | "updatedAtMs"
  > {
    const tasks: PersistedWorkspaceActiveTaskSummaryV1[] = [];
    for (const id of this.activeOrder) {
      const item = this.tasks.get(id);
      if (!item || item.task.archived_at) continue;
      const summary = this.buildPersistedSummary(item);
      if (summary) tasks.push(summary);
    }
    const totalCount = Math.max(this.totalActive, tasks.length);
    return {
      snapshotRev: this.snapshotRev,
      archivedRev: this.archivedRev,
      worktreeVcsSnapshots: this.getWorktreeVcsSnapshots(),
      active: {
        tasks,
        totalCount,
      },
    };
  }

  applyCachedSnapshot(cached: PersistedWorkspaceActiveSnapshotV1) {
    this.snapshotRev = Math.max(this.snapshotRev, cached.snapshotRev ?? 0);
    this.archivedRev = Math.max(this.archivedRev, cached.archivedRev ?? 0);
    const activeTasks = Array.isArray(cached.active?.tasks) ? cached.active.tasks : [];
    const archivedHeads = this.collectArchivedHeads();
    for (const [id, item] of this.tasks.entries()) {
      if (!item.task.archived_at) {
        this.tasks.delete(id);
      }
    }
    this.activeOrder = [];
    this.sessionHeadsById.clear();
    for (const [sessionId, head] of archivedHeads) {
      this.sessionHeadsById.set(sessionId, head);
    }

    const nextActiveIds = new Set<string>();
    for (const summary of activeTasks) {
      const task = summary?.task;
      if (!task || typeof task !== "object" || task.archived_at) continue;
      const existing = this.tasks.get(idToString(task.id));
      const normalized = this.normalizeActiveSummary(summary, existing);
      nextActiveIds.add(normalized.id);
      this.tasks.set(normalized.id, normalized);
      this.placeInOrders(normalized);
    }

    const totalCountRaw = cached.active?.totalCount;
    const totalCount =
      typeof totalCountRaw === "number" && Number.isFinite(totalCountRaw)
        ? totalCountRaw
        : nextActiveIds.size;
    this.totalActive = Math.max(totalCount, nextActiveIds.size);
    this.snapshotRev = Math.max(this.snapshotRev, cached.snapshotRev ?? 0);
    this.snapshot = {
      ...this.snapshot,
      worktreeVcsById: mapWorktreeVcsSnapshots(cached.worktreeVcsSnapshots ?? []),
      initialized: true,
    };
    this.syncSnapshot();
  }

  applyWorkspaceSnapshot(
    snapshot: WorkspaceActiveSnapshot,
    heads?: SessionHeadSnapshot[] | null,
    opts?: { resetSnapshotRev?: boolean },
  ) {
    const incomingRev = typeof snapshot.snapshot_rev === "number" ? snapshot.snapshot_rev : 0;
    this.snapshotRev = opts?.resetSnapshotRev
      ? incomingRev
      : Math.max(this.snapshotRev, incomingRev);
    if (typeof snapshot.archived_rev === "number" && snapshot.archived_rev > this.archivedRev) {
      this.archivedRev = snapshot.archived_rev;
      this.archivedLoaded = false;
      this.archivedCursor = null;
    }

    const archivedHeads = this.collectArchivedHeads();
    for (const [id, item] of this.tasks.entries()) {
      if (!item.task.archived_at) {
        this.tasks.delete(id);
      }
    }
    this.activeOrder = [];
    this.sessionHeadsById.clear();
    for (const [sessionId, head] of archivedHeads) {
      this.sessionHeadsById.set(sessionId, head);
    }

    const activeTasks = snapshot.active?.tasks ?? [];
    const nextWorktreeVcsById = mapWorktreeVcsSnapshots(snapshot.worktree_vcs_snapshots ?? []);
    const nextActiveIds = new Set<string>();
    for (const summary of activeTasks) {
      const existing = this.tasks.get(idToString(summary.task.id));
      const normalized = this.normalizeActiveSummary(summary, existing);
      nextActiveIds.add(normalized.id);
      this.tasks.set(normalized.id, normalized);
      this.placeInOrders(normalized);
    }

    const nextTotalActive = Number.isFinite(snapshot.active?.total_count)
      ? snapshot.active.total_count
      : nextActiveIds.size;
    this.totalActive = Math.max(nextTotalActive, nextActiveIds.size);
    if (Array.isArray(heads) && heads.length > 0) {
      this.applyActiveHeads(heads);
    }
    this.snapshot = {
      ...this.snapshot,
      worktreeVcsById: nextWorktreeVcsById,
      initialized: true,
    };
    this.liveSnapshotApplied = true;
    this.syncSnapshot();
  }

  applyArchivedPage(page: WorkspaceArchivedPage, items: Array<WorkspaceActiveSnapshotItem | null>) {
    if (typeof page.archived_rev === "number" && page.archived_rev > this.archivedRev) {
      this.archivedRev = page.archived_rev;
    }
    this.totalArchived = page.total_archived ?? this.totalArchived;
    for (const item of items) {
      if (item) {
        this.upsertArchivedItem(item, { adjustCounts: false, sync: false });
      }
    }
    this.archivedCursor = page.next_cursor ?? null;
    this.hasMoreArchived = Boolean(page.next_cursor);
    this.archivedLoaded = true;
    this.syncSnapshot();
  }

  resetArchivedCursor() {
    this.archivedCursor = null;
  }

  markArchivedExhausted(): boolean {
    if (!this.hasMoreArchived) return false;
    this.hasMoreArchived = false;
    this.syncSnapshot();
    return true;
  }

  applyTaskUpdate(task: Task): boolean {
    const id = idToString(task.id);
    if (!id) return false;
    const existing = this.tasks.get(id);
    if (!existing) return false;
    const prevArchived = Boolean(existing.task.archived_at);
    const nextArchived = Boolean(task.archived_at);
    const stableSortAt = taskSortAt(task, existing.sort_at);
    const stableSortAtMs = Date.parse(stableSortAt ?? "") || existing.sortAtMs || Date.now();
    const updated: WorkspaceActiveSnapshotItem = {
      ...existing,
      task: { ...task },
      sortAtMs: stableSortAtMs,
      sort_at: stableSortAt ?? existing.sort_at,
    };
    this.tasks.set(id, updated);
    if (prevArchived !== nextArchived) {
      this.archivedLoaded = false;
      this.archivedCursor = null;
      this.hasMoreArchived = true;
    }
    this.updateCountsForMove(existing, updated);
    this.placeInOrders(updated);
    this.syncSnapshot();
    return true;
  }

  removeTask(taskId: string | undefined, opts?: { adjustCounts?: boolean; sync?: boolean }): boolean {
    const deleteId = idToString(taskId ?? "");
    if (!deleteId) return false;
    const existing = this.tasks.get(deleteId);
    if (!existing) return false;
    if (existing.primarySessionId) {
      this.sessionHeadsById.delete(existing.primarySessionId);
    }
    this.tasks.delete(deleteId);
    this.activeOrder = this.activeOrder.filter((id) => id !== deleteId);
    this.archivedOrder = this.archivedOrder.filter((id) => id !== deleteId);
    if (opts?.adjustCounts) {
      if (existing.task.archived_at) {
        this.totalArchived = Math.max(0, this.totalArchived - 1);
      } else {
        this.totalActive = Math.max(0, this.totalActive - 1);
      }
    }
    if (opts?.sync ?? true) {
      this.syncSnapshot();
    }
    return true;
  }

  upsertActiveSummary(summary: WorkspaceActiveTaskSummary): boolean {
    const existing = this.tasks.get(idToString(summary.task.id));
    const normalized = this.normalizeActiveSummary(summary, existing);
    if (existing?.primarySessionId && existing.primarySessionId !== normalized.primarySessionId) {
      this.sessionHeadsById.delete(existing.primarySessionId);
    }
    this.tasks.set(normalized.id, normalized);
    if (existing) {
      this.updateCountsForMove(existing, normalized);
    } else {
      this.totalActive += 1;
    }
    this.placeInOrders(normalized);
    this.syncSnapshot();
    return true;
  }

  upsertArchivedItem(
    item: WorkspaceActiveSnapshotItem,
    opts?: { adjustCounts?: boolean; sync?: boolean },
  ): boolean {
    const existing = this.tasks.get(item.id);
    this.tasks.set(item.id, item);
    const adjustCounts = opts?.adjustCounts ?? true;
    if (adjustCounts) {
      if (existing) {
        this.updateCountsForMove(existing, item);
      } else {
        this.totalArchived += 1;
      }
    }
    this.placeInOrders(item);
    if (opts?.sync ?? true) {
      this.syncSnapshot();
    }
    return true;
  }

  applyTaskDelta(evt: WorkspaceActiveSnapshotTaskDeltaEvent): boolean {
    const delta = evt.delta;
    const taskId = idToString(delta?.task?.id ?? "");
    if (!taskId) return false;

    const existing = this.tasks.get(taskId);
    if (!existing) return false;

    if (delta.kind === "archived") {
      return this.removeTask(taskId, { adjustCounts: true });
    }

    const nextTask = {
      ...existing.task,
      ...delta.task,
      id: existing.task.id,
      workspace_id: existing.task.workspace_id,
    };
    const sortAt = taskSortAt(nextTask, existing.sort_at ?? null);
    const sortAtMs = Date.parse(sortAt) || existing.sortAtMs || Date.now();
    const nextPrimarySessionId = idToString(nextTask.primary_session_id ?? "") || null;
    if (existing.primarySessionId && existing.primarySessionId !== nextPrimarySessionId) {
      this.sessionHeadsById.delete(existing.primarySessionId);
    }
    const primarySessionHead = nextPrimarySessionId
      ? (this.sessionHeadsById.get(nextPrimarySessionId) ?? null)
      : null;
    const nextItem: WorkspaceActiveSnapshotItem = {
      ...existing,
      task: nextTask,
      primarySessionId: nextPrimarySessionId,
      primarySessionHead,
      sortAtMs,
      sort_at: sortAt || null,
    };
    this.tasks.set(taskId, nextItem);
    this.placeInOrders(nextItem);
    this.syncSnapshot();
    return true;
  }

  applySessionSummaryDelta(evt: WorkspaceActiveSnapshotSessionSummaryDeltaEvent): boolean {
    const delta = evt.delta;
    const sessionId = idToString(delta.session_id ?? "");
    if (!sessionId) return false;
    const taskIdHint = idToString(delta.task_id ?? "");

    const tryUpdate = (taskId: string): boolean => {
      const task = this.tasks.get(taskId);
      if (!task) return false;
      const nextSessions = task.sessions.slice();
      const sessionIdx = nextSessions.findIndex((s) => idToString(s.session.id) === sessionId);
      if (sessionIdx < 0) return false;
      const current = nextSessions[sessionIdx];
      const nextSummary: SessionSnapshotSummary = { ...current };
      let changed = false;
      const currentProjectionRev =
        typeof current.projection_rev === "number" ? current.projection_rev : null;
      const incomingLastEventSeq =
        typeof delta.last_event_seq === "number" ? delta.last_event_seq : null;

      if (hasOwnProperty(delta, "last_message_at")) {
        const incoming = delta.last_message_at;
        if (typeof incoming === "string" && incoming) {
          const currentValue = nextSummary.last_message_at;
          const incMs = Date.parse(incoming);
          const curMs = currentValue ? Date.parse(currentValue) : Number.NaN;
          const shouldUpdate =
            !currentValue ||
            (Number.isFinite(incMs) && Number.isFinite(curMs) ? incMs > curMs : incoming > currentValue);
          if (shouldUpdate && nextSummary.last_message_at !== incoming) {
            nextSummary.last_message_at = incoming;
            changed = true;
          }
        }
      }
      if (hasOwnProperty(delta, "last_message_preview")) {
        const incoming = delta.last_message_preview;
        if (typeof incoming === "string") {
          const nextValue = incoming.length ? incoming : null;
          if (nextSummary.last_message_preview !== nextValue) {
            nextSummary.last_message_preview = nextValue;
            changed = true;
          }
        } else if (incoming === null && nextSummary.last_message_preview !== null) {
          nextSummary.last_message_preview = null;
          changed = true;
        }
      }
      if (hasOwnProperty(delta, "last_event_seq")) {
        const incoming = delta.last_event_seq;
        if (typeof incoming === "number") {
          const nextValue = Math.max(nextSummary.last_event_seq ?? incoming, incoming);
          if (nextSummary.last_event_seq !== nextValue) {
            nextSummary.last_event_seq = nextValue;
            changed = true;
          }
        }
      }
      if (typeof delta.projection_rev === "number") {
        const nextCurrentProjectionRev = nextSummary.projection_rev ?? 0;
        if (delta.projection_rev > nextCurrentProjectionRev) {
          nextSummary.projection_rev = delta.projection_rev;
          changed = true;
        }
      }
      if (typeof delta.state_rev === "number") {
        const nextCurrentStateRev = nextSummary.state_rev ?? 0;
        if (delta.state_rev > nextCurrentStateRev) {
          nextSummary.state_rev = delta.state_rev;
          changed = true;
        }
      }

      if (!changed) return false;
      nextSessions[sessionIdx] = nextSummary;
      this.tasks.set(taskId, { ...task, sessions: sortSessionSummaries(nextSessions) });
      return true;
    };

    let changed = false;
    if (taskIdHint && tryUpdate(taskIdHint)) {
      changed = true;
    } else {
      for (const taskId of this.tasks.keys()) {
        if (taskIdHint && taskId === taskIdHint) continue;
        if (tryUpdate(taskId)) {
          changed = true;
          break;
        }
      }
    }
    if (changed) {
      this.syncSnapshot();
    }
    return changed;
  }

  applySessionSummary(summary: SessionSnapshotSummary): boolean {
    const taskId = idToString(summary.session.task_id);
    if (!taskId) return false;
    const task = this.tasks.get(taskId);
    if (!task) return false;
    const nextSessions = task.sessions.slice();
    const sessionId = idToString(summary.session.id);
    const sessionIdx = nextSessions.findIndex((s) => idToString(s.session.id) === sessionId);
    const normalized = normalizeSessionSummary(summary);
    if (sessionIdx >= 0) {
      nextSessions[sessionIdx] = normalized;
    } else {
      nextSessions.push(normalized);
    }
    this.tasks.set(taskId, { ...task, sessions: sortSessionSummaries(nextSessions) });
    this.syncSnapshot();
    return true;
  }

  applySessionHeadDelta(delta: SessionHeadDelta): boolean {
    const sessionId = idToString(delta?.session_id ?? "");
    if (!sessionId) return false;
    let existing = this.sessionHeadsById.get(sessionId);
    if (!existing) {
      const seeded = this.seedHeadSnapshot(sessionId);
      if (!seeded) return false;
      existing = seeded;
      this.sessionHeadsById.set(sessionId, seeded);
    }
    let changed = false;
    let turns = existing.turns ?? [];
    let toolSummaries = Array.isArray(existing.tool_summaries) ? existing.tool_summaries : [];
    let messages = existing.messages ?? [];
    let events = existing.events ?? [];
    if (delta.turn) {
      turns = mergeSessionTurns(turns, [delta.turn]);
      changed = true;
    }
    if (delta.message) {
      messages = mergeSessionMessages(messages, [delta.message]);
      changed = true;
    }
    if (delta.event) {
      events = mergeSessionEvents(events, [delta.event]);
      changed = true;
    }
    const incomingToolSummaries = Array.isArray(delta.tool_summaries) ? delta.tool_summaries : [];
    if (incomingToolSummaries.length > 0) {
      toolSummaries = mergeSessionToolSummaries(toolSummaries, incomingToolSummaries, turns);
      changed = true;
    }
    const next: SessionHeadSnapshot = sanitizeSessionHeadSnapshot({
      ...existing,
      turns,
      tool_summaries: toolSummaries,
      messages,
      events,
      ...(delta.session ? { session: delta.session } : {}),
      ...("activity" in delta ? { activity: delta.activity ?? undefined } : {}),
      ...(typeof delta.last_event_seq === "number" ? { last_event_seq: delta.last_event_seq } : {}),
      ...(typeof delta.projection_rev === "number" ? { projection_rev: delta.projection_rev } : {}),
      ...(typeof delta.state_rev === "number" ? { state_rev: delta.state_rev } : {}),
    });

    if (
      !changed &&
      next.last_event_seq === existing.last_event_seq &&
      (next.projection_rev ?? 0) === (existing.projection_rev ?? 0)
    ) {
      return false;
    }
    if (!shouldReplaceSessionHead(existing, next)) return false;
    this.sessionHeadsById.set(sessionId, next);
    changed = true;
    if (projectPrimarySessionHeadOntoTasks(this.tasks, next)) {
      changed = true;
    }
    if (changed) {
      this.syncSnapshot();
    }
    return changed;
  }

  applySessionHeadSeed(head: SessionHeadSnapshot | null | undefined): boolean {
    if (!head) return false;
    const sessionId = idToString(head?.session?.id ?? "");
    if (!sessionId) return false;
    const sanitized = sanitizeSessionHeadSnapshot(head);
    const prev = this.sessionHeadsById.get(sessionId);
    if (!shouldReplaceSessionHead(prev, sanitized)) return false;
    this.sessionHeadsById.set(sessionId, sanitized);
    projectPrimarySessionHeadOntoTasks(this.tasks, sanitized);
    this.syncSnapshot();
    return true;
  }

  buildArchivedItem(
    summary: WorkspaceTaskSummary,
    primaryHead?: SessionHeadSnapshot | null,
  ): WorkspaceActiveSnapshotItem | null {
    void primaryHead;
    const task = summary.task;
    const id = idToString(task.id);
    if (!id) return null;
    const existing = this.tasks.get(id);
    const providerIds = (summary.provider_ids ?? []).filter(Boolean);
    const summaries = existing?.sessions ?? [];
    const sessionList = summaries.map((item) => item.session).filter(Boolean);
    const summarySessions = Array.isArray(summary.sessions) ? summary.sessions : [];
    if (existing?.primarySessionHead) {
      this.rememberSessionHead(existing.primarySessionHead);
    }
    const summaryPrimaryId = readPrimarySessionId(summary);
    const primarySessionId =
      summaryPrimaryId ||
      pickArchivedSessionIdFromSummaries(task, summarySessions) ||
      pickArchivedSessionId(task, sessionList);
    let primarySessionHead = null;
    if (!primarySessionHead && primarySessionId) {
      primarySessionHead = this.sessionHeadsById.get(primarySessionId) ?? null;
    }
    if (!primarySessionHead && existing?.primarySessionHead) {
      primarySessionHead = existing.primarySessionHead;
    }
    const sortAt = taskSortAt(task);
    return {
      id,
      task: { ...task },
      sessions: sortSessionSummaries(summaries),
      providerIds: providerIds.length ? providerIds : existing?.providerIds,
      primarySessionId: primarySessionId || null,
      primarySessionHead: primarySessionHead ?? null,
      sortAtMs: Date.parse(sortAt) || Date.now(),
      sort_at: sortAt || null,
    };
  }

  private syncSnapshot() {
    const tasksById: Record<string, WorkspaceActiveSnapshotItem> = {};
    for (const [id, item] of this.tasks.entries()) {
      tasksById[id] = item;
    }
    this.hasMoreActive = this.totalActive > this.activeOrder.length;
    this.activeSessionIds = collectWorkspaceActivePrimarySessionIds({
      activeIds: this.activeOrder,
      archivedIds: this.archivedOrder,
      tasksById,
    });
    this.snapshot = {
      ...this.snapshot,
      liveSnapshotApplied: this.liveSnapshotApplied,
      tasksById,
      activeIds: [...this.activeOrder],
      archivedIds: [...this.archivedOrder],
      totalActive: this.totalActive,
      totalArchived: this.totalArchived,
      archivedRev: this.archivedRev,
      hasMoreActive: this.hasMoreActive,
      hasMoreArchived: this.hasMoreArchived,
      archivedLoaded: this.archivedLoaded,
    };
  }

  private applyActiveHeads(heads: SessionHeadSnapshot[]): boolean {
    if (!Array.isArray(heads) || heads.length === 0) return false;
    let changed = false;
    for (const head of heads) {
      if (!head || typeof head !== "object") continue;
      const sessionId = idToString(head.session?.id ?? "");
      if (!sessionId) continue;
      const sanitized = sanitizeSessionHeadSnapshot(head);
      const primarySummary = this.resolvePrimarySessionSummary(sessionId);
      if (!isSessionHeadCompatibleWithSummary(primarySummary, sanitized)) {
        continue;
      }
      const prev = this.sessionHeadsById.get(sessionId);
      if (shouldReplaceSessionHead(prev, sanitized)) {
        this.sessionHeadsById.set(sessionId, sanitized);
        changed = true;
      }
      if (projectPrimarySessionHeadOntoTasks(this.tasks, sanitized)) {
        changed = true;
      }
    }
    return changed;
  }

  private buildPersistedSummary(
    item: WorkspaceActiveSnapshotItem,
  ): PersistedWorkspaceActiveTaskSummaryV1 | null {
    if (!item.task) return null;
    const sessions = Array.isArray(item.sessions) ? item.sessions : [];
    const primaryId = resolvePrimarySessionId(item);
    let primary = primaryId
      ? sessions.find((summary) => idToString(summary.session.id) === primaryId) ?? null
      : null;
    if (!primary && sessions.length > 0) {
      primary = sessions[0] ?? null;
    }
    if (!primary && item.primarySessionHead?.session) {
      primary = sessionToSummary(item.primarySessionHead.session);
    }
    const head =
      item.primarySessionHead ||
      (primaryId ? this.sessionHeadsById.get(primaryId) ?? null : null);
    const sortAt = taskSortAt(item.task, item.sort_at);
    return {
      task: item.task,
      primary_session: primary ?? null,
      primary_session_head: head ? sanitizeSessionHeadSnapshot(head) : null,
      sessions,
      sort_at: sortAt,
    };
  }

  private updateCountsForMove(prev: WorkspaceActiveSnapshotItem, next: WorkspaceActiveSnapshotItem) {
    const prevArchived = Boolean(prev.task.archived_at);
    const nextArchived = Boolean(next.task.archived_at);
    if (prevArchived === nextArchived) return;
    if (prevArchived) {
      this.totalArchived = Math.max(0, this.totalArchived - 1);
      this.totalActive += 1;
    } else {
      this.totalActive = Math.max(0, this.totalActive - 1);
      this.totalArchived += 1;
    }
  }

  private seedHeadSnapshot(sessionId: string): SessionHeadSnapshot | null {
    for (const item of this.tasks.values()) {
      for (const summary of item.sessions) {
        const candidateId = idToString(summary.session.id);
        if (candidateId !== sessionId) continue;
        return {
          session: summary.session,
          turns: [],
          tool_summaries: [],
          events: [],
          messages: [],
          last_event_seq: summary.last_event_seq ?? 0,
          projection_rev: summary.projection_rev ?? 0,
          state_rev: summary.state_rev ?? 0,
          activity: undefined,
          has_more_turns: false,
          history_cursor: null,
          has_more_history: false,
          summary_checkpoint: undefined,
          head_window: emptySessionHeadWindow(),
        };
      }
    }
    return null;
  }

  private rememberSessionHead(head: SessionHeadSnapshot | null) {
    if (!head) return;
    const sessionId = idToString(head?.session?.id ?? "");
    if (!sessionId) return;
    const sanitized = sanitizeSessionHeadSnapshot(head);
    const primarySummary = this.resolvePrimarySessionSummary(sessionId);
    if (!isSessionHeadCompatibleWithSummary(primarySummary, sanitized)) return;
    const prev = this.sessionHeadsById.get(sessionId);
    if (!shouldReplaceSessionHead(prev, sanitized)) return;
    this.sessionHeadsById.set(sessionId, sanitized);
  }

  private resolvePrimarySessionSummary(sessionId: string): SessionSnapshotSummary | null {
    const id = idToString(sessionId);
    if (!id) return null;
    for (const item of this.tasks.values()) {
      const primarySessionId = resolvePrimarySessionId(item);
      if (primarySessionId !== id) continue;
      return item.sessions.find((summary) => idToString(summary.session.id) === id) ?? null;
    }
    return null;
  }

  private collectArchivedHeads(): Map<string, SessionHeadSnapshot> {
    const archived = new Map<string, SessionHeadSnapshot>();
    for (const item of this.tasks.values()) {
      if (!item.task.archived_at) continue;
      const primaryId = resolvePrimarySessionId(item);
      if (!primaryId) continue;
      const head = this.sessionHeadsById.get(primaryId) ?? item.primarySessionHead ?? null;
      if (head) archived.set(primaryId, head);
    }
    return archived;
  }

  private normalizeActiveSummary(
    summary: WorkspaceActiveTaskSummary | PersistedWorkspaceActiveTaskSummaryV1,
    existing?: WorkspaceActiveSnapshotItem,
  ): WorkspaceActiveSnapshotItem {
    const id = idToString(summary.task.id);
    const summaryHasPrimary = hasOwnProperty(summary, "primary_session") || hasOwnProperty(summary, "primarySession");
    const summaryHasSessions = hasOwnProperty(summary, "sessions");
    const summaryHasHead =
      hasOwnProperty(summary, "primary_session_head") || hasOwnProperty(summary, "primarySessionHead");
    const summaryHasSortAt = hasOwnProperty(summary, "sort_at") || hasOwnProperty(summary, "sortAt");

    const fallbackSortAt = summaryHasSortAt
      ? (summary as PersistedWorkspaceActiveTaskSummaryV1).sort_at ?? null
      : existing?.sort_at ?? null;
    const sortAt = taskSortAt(summary.task, fallbackSortAt);
    const sortAtMs = Date.parse(sortAt) || existing?.sortAtMs || Date.now();
    const existingPrimarySessionId = resolvePrimarySessionId(existing);
    const primarySessionId =
      readPrimarySessionId(summary) ||
      existingPrimarySessionId ||
      idToString((summary as PersistedWorkspaceActiveTaskSummaryV1).primary_session?.session?.id ?? "");

    const existingSessions = existing?.sessions ?? [];
    const primaryFromSummary = summaryHasPrimary
      ? (summary as PersistedWorkspaceActiveTaskSummaryV1).primary_session
      : null;
    let primarySummary = primaryFromSummary ? normalizeSessionSummary(primaryFromSummary) : null;
    if (!primarySummary && primarySessionId) {
      primarySummary =
        existingSessions.find((item) => idToString(item.session.id) === primarySessionId) ?? null;
    }

    let primaryHead = summaryHasHead ? readPrimarySessionHead(summary) : existing?.primarySessionHead ?? null;
    if (!primaryHead && primarySessionId) {
      primaryHead = this.sessionHeadsById.get(primarySessionId) ?? null;
    }
    if (primaryHead && !isSessionHeadCompatibleWithSummary(primarySummary, primaryHead)) {
      primaryHead = null;
    }
    this.rememberSessionHead(primaryHead);

    const sessionsRaw = summaryHasSessions
      ? Array.isArray((summary as PersistedWorkspaceActiveTaskSummaryV1).sessions)
        ? (summary as PersistedWorkspaceActiveTaskSummaryV1).sessions
        : []
      : existingSessions;
    const sessions = sessionsRaw.map((item) => normalizeSessionSummary(item));

    const merged: SessionSnapshotSummary[] = [];
    const seen = new Set<string>();
    const addSummary = (item: SessionSnapshotSummary) => {
      const sid = idToString(item.session.id);
      if (!sid || seen.has(sid)) return;
      seen.add(sid);
      merged.push(item);
    };
    if (primarySummary) {
      addSummary(primarySummary);
    }
    sessions.forEach(addSummary);

    return {
      id,
      task: { ...summary.task },
      sessions: sortSessionSummaries(merged),
      primarySessionHead: primaryHead ?? null,
      primarySessionId: primarySessionId || null,
      sortAtMs,
      sort_at: sortAt || null,
    };
  }

  private placeInOrders(item: WorkspaceActiveSnapshotItem) {
    const { id } = item;
    this.activeOrder = this.activeOrder.filter((existing) => existing !== id);
    this.archivedOrder = this.archivedOrder.filter((existing) => existing !== id);
    if (item.task.archived_at) {
      this.archivedOrder.splice(
        findWorkspaceActiveSnapshotInsertIndex(this.tasks, this.archivedOrder, item.sortAtMs, id),
        0,
        id,
      );
    } else {
      this.activeOrder.splice(
        findWorkspaceActiveSnapshotInsertIndex(this.tasks, this.activeOrder, item.sortAtMs, id),
        0,
        id,
      );
    }
  }
}
