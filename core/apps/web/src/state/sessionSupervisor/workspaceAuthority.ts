import { idToString, type SessionHeadSnapshot } from "../../api/client";
import type { WorkspaceActiveSnapshotState } from "../workspaceActiveSnapshotStore";
import { collectWorkspaceActivePrimarySessionIds } from "../workspaceActiveSnapshot/projection";
import type { SessionReplicaCommand } from "../sessionReplicaProtocol";
import { classifyActiveSnapshotSeedMode } from "./activeSnapshotSeed";
import type { ConnectionStatus, InternalEntry } from "./entryState";
import { sameIdList } from "./cachePolicy";
import type {
  SessionSupervisorWorkspaceEvent,
  SessionSupervisorWorkspaceSessionHeads,
  SessionSupervisorWorkspaceSnapshotState,
} from "./workspaceInputs";

type SessionSupervisorWorkspaceAuthorityHost = {
  getWorkspaceSnapshotState(): SessionSupervisorWorkspaceSnapshotState;
  setWorkspaceSnapshotState(state: SessionSupervisorWorkspaceSnapshotState): void;
  getWorkspaceSessionHeadsById(): Map<string, SessionHeadSnapshot>;
  setWorkspaceSessionHeadsById(heads: Map<string, SessionHeadSnapshot>): void;
  getWorkspaceActivePrimarySessionIds(): string[];
  setWorkspaceActivePrimarySessionIds(sessionIds: string[]): void;
  mapConnection(connection: WorkspaceActiveSnapshotState["connection"]): ConnectionStatus;
  setConnection(next: ConnectionStatus): void;
  syncActiveSnapshot(state: WorkspaceActiveSnapshotState): void;
  markOpenSessionsRecovering(): void;
  refreshSubscriptions(opts?: { emitIfUnchanged?: boolean }): void;
  emitSubscribedSessions(): void;
  clearTaskThoughts(taskId: string): Promise<void>;
  publish(): void;
  syncSupportLoadsForOpenSession(entry: InternalEntry): void;
  replicaDispatch(cmd: SessionReplicaCommand): void;
  entries: Map<string, InternalEntry>;
  ensureEntry(sessionId: string): InternalEntry;
  setSessionLoadState(entry: InternalEntry, next: InternalEntry["loadState"]): void;
};

export const setWorkspaceSnapshotState = (
  host: SessionSupervisorWorkspaceAuthorityHost,
  state: SessionSupervisorWorkspaceSnapshotState,
) => {
  host.setWorkspaceSnapshotState(state);
  if (!state) {
    host.setWorkspaceActivePrimarySessionIds([]);
    host.setConnection("disconnected");
    return;
  }
  const nextWorkspaceActivePrimarySessionIds = collectWorkspaceActivePrimarySessionIds(state);
  const activePrimaryMembershipChanged = !sameIdList(
    nextWorkspaceActivePrimarySessionIds,
    host.getWorkspaceActivePrimarySessionIds(),
  );
  host.setWorkspaceActivePrimarySessionIds(nextWorkspaceActivePrimarySessionIds);
  const next = host.mapConnection(state.connection);
  host.setConnection(next);
  host.syncActiveSnapshot(state);
  if (next !== "connected") {
    host.markOpenSessionsRecovering();
  }
  host.refreshSubscriptions({ emitIfUnchanged: activePrimaryMembershipChanged });
};

export const setWorkspaceSessionHeads = (
  host: SessionSupervisorWorkspaceAuthorityHost,
  heads: SessionSupervisorWorkspaceSessionHeads,
) => {
  host.setWorkspaceSessionHeadsById(new Map(Object.entries(heads)));
  for (const [sessionId, head] of host.getWorkspaceSessionHeadsById().entries()) {
    const entry = host.entries.get(sessionId);
    if (!entry) continue;
    if (classifyActiveSnapshotSeedMode(entry, head) !== "repair_replace") continue;
    host.replicaDispatch({ type: "seed_head", sessionId, head, mode: "repair_replace" });
  }
  for (const entry of host.entries.values()) {
    host.syncSupportLoadsForOpenSession(entry);
  }
  host.emitSubscribedSessions();
};

export const ingestWorkspaceEvent = (
  host: SessionSupervisorWorkspaceAuthorityHost,
  evt: SessionSupervisorWorkspaceEvent,
) => {
  let changed = false;
  let subscriptionCursorsChanged = false;
  if (evt.type === "archived_task_upsert") {
    const taskId = idToString(evt.task?.task?.id);
    if (taskId) {
      void host.clearTaskThoughts(taskId);
    }
  } else if (evt.type === "archived_task_delete") {
    const taskId = idToString(evt.task_id);
    if (taskId) {
      void host.clearTaskThoughts(taskId);
    }
  } else if (evt.type === "session_gap") {
    const sessionId = idToString(evt.session_id);
    if (sessionId) {
      const entry = host.entries.get(sessionId);
      if (entry) {
        host.setSessionLoadState(entry, "recovering");
        entry.error = undefined;
        entry.turnsHydrated = false;
        entry.updatedAtMs = Date.now();
        changed = true;
        if (entry.subscribed) {
          subscriptionCursorsChanged = true;
        }
      }
    }
  }
  if (changed) {
    host.publish();
  }
  if (subscriptionCursorsChanged) {
    host.emitSubscribedSessions();
  }
  host.replicaDispatch({ type: "workspace_event", event: evt });
};

export const syncActiveSnapshot = (
  host: Pick<
    SessionSupervisorWorkspaceAuthorityHost,
    "ensureEntry" | "replicaDispatch"
  >,
  state: WorkspaceActiveSnapshotState,
) => {
  for (const taskId of state.activeIds) {
    const item = state.tasksById[taskId];
    const head = item?.primarySessionHead;
    if (!head) continue;
    const sessionId = idToString(head.session?.id);
    if (!sessionId) continue;
    const entry = host.ensureEntry(sessionId);
    const mode = classifyActiveSnapshotSeedMode(entry, head, { allowRecoveringRefresh: true });
    if (!mode) continue;
    host.replicaDispatch({ type: "seed_head", sessionId, head, mode });
  }
};
