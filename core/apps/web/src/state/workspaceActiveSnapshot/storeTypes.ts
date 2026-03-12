import type {
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  Task,
  WorktreeVcsSnapshot,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";

export type WorkspaceActiveSnapshotItem = {
  id: string;
  task: Task;
  sessions: SessionSnapshotSummary[];
  providerIds?: string[];
  primarySessionHead?: SessionHeadSnapshot | null;
  primarySessionId?: string | null;
  sort_at?: string | null;
  sortAtMs: number;
};

export type WorkspaceActiveSnapshotState = {
  workspaceId: string;
  initialized: boolean;
  connection: "idle" | "connecting" | "connected" | "disconnected";
  tasksById: Record<string, WorkspaceActiveSnapshotItem>;
  activeIds: string[];
  archivedIds: string[];
  totalActive: number;
  totalArchived: number;
  archivedRev: number;
  worktreeVcsById: Record<string, WorktreeVcsSnapshot>;
  fetchState: {
    active: "idle" | "loading" | "error";
    archived: "idle" | "loading" | "error";
  };
  hasMoreActive: boolean;
  hasMoreArchived: boolean;
  archivedLoaded: boolean;
};

export type WorkspaceActiveSnapshotEventSource = {
  subscribe: (listener: () => void) => () => void;
  subscribeEvents: (listener: (event: WorkspaceActiveSnapshotEvent) => void) => () => void;
  getSnapshot: () => WorkspaceActiveSnapshotState;
  getSessionHeadSnapshot: (sessionId: string) => SessionHeadSnapshot | null;
  getSessionHeadsSnapshot?: () => Record<string, SessionHeadSnapshot>;
  getWorktreeRoot: (worktreeId: string) => string | null;
  getWorktreeVcsSnapshot: (worktreeId: string) => WorktreeVcsSnapshot | null;
  setSubscribedSessionIds?: (sessionIds: string[]) => void;
  setForegroundTaskId?: (taskId: string | null) => void;
};
