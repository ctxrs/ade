import type {
  SessionHeadSnapshot,
  Task,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import type { PersistedWorkspaceActiveSnapshotV1 } from "./uiStateStore";
import type { WorkspaceActiveSnapshotState } from "./workspaceActiveSnapshotStoreCore";

export type WorkspaceActiveSnapshotCommand =
  | {
      type: "init";
      workspaceId: string;
      authToken?: string | null;
      baseUrl?: string | null;
      wsBaseUrl?: string | null;
    }
  | {
      type: "seed_cache";
      snapshot: PersistedWorkspaceActiveSnapshotV1;
    }
  | { type: "set_subscribed_session_ids"; sessionIds: string[] }
  | { type: "set_foreground_task_id"; taskId: string | null }
  | { type: "ensure_archived_loaded" }
  | { type: "load_more_archived" }
  | { type: "apply_task_update"; task: Task };

export type WorkspaceActiveSnapshotPatch = {
  snapshot: WorkspaceActiveSnapshotState;
  sessionHeads: Record<string, SessionHeadSnapshot>;
  worktreeRoots: Record<string, string>;
  events: WorkspaceActiveSnapshotEvent[];
  snapshotRev: number;
  archivedRev: number;
  activeSessionIds: string[];
  persist: boolean;
};

export type WorkspaceActiveSnapshotWorkerMessage = {
  type: "patch";
  patch: WorkspaceActiveSnapshotPatch;
};
