import type {
  SessionHeadSnapshot,
  Task,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import type { SessionSubscriptionCursor } from "./sessionSubscription";
import type { PersistedWorkspaceActiveSnapshotV1 } from "./uiStateStore";
import type { WorkspaceActiveSnapshotState } from "./workspaceActiveSnapshot/storeTypes";

export type WorkspaceActiveSnapshotCommand =
  | {
      type: "init";
      workspaceId: string;
      connectionSeq: number;
      authToken?: string | null;
      baseUrl?: string | null;
      wsBaseUrl?: string | null;
      runId?: string | null;
      e2eEnabled?: boolean;
    }
  | {
      type: "update_auth";
      connectionSeq: number;
      authToken?: string | null;
      baseUrl?: string | null;
      wsBaseUrl?: string | null;
      runId?: string | null;
    }
  | {
      type: "seed_cache";
      snapshot: PersistedWorkspaceActiveSnapshotV1;
    }
  | { type: "set_subscribed_sessions"; sessions: SessionSubscriptionCursor[] }
  | { type: "set_foreground_session_id"; sessionId: string | null }
  | { type: "ensure_archived_loaded" }
  | { type: "load_more_archived" }
  | { type: "apply_task_update"; task: Task }
  | { type: "e2e_set_enabled"; enabled: boolean }
  | { type: "e2e_close_stream" }
  | { type: "e2e_set_drop_messages"; drop: boolean };

export type WorkspaceActiveSnapshotPatch = {
  snapshot: WorkspaceActiveSnapshotState;
  sessionHeads: Record<string, SessionHeadSnapshot>;
  worktreeRoots: Record<string, string>;
  events: WorkspaceActiveSnapshotEvent[];
  snapshotRev: number;
  archivedRev: number;
  activeSessionIds: string[];
  persist: boolean;
  oldestEventReceivedAtMs?: number | null;
  oldestForegroundEventReceivedAtMs?: number | null;
};

export type WorkspaceActiveSnapshotWorkerMessage = {
  type: "patch";
  patch: WorkspaceActiveSnapshotPatch;
};
