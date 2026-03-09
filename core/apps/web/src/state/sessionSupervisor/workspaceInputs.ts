import type { SessionHeadSnapshot, WorkspaceActiveSnapshotEvent } from "../../api/client";
import type { WorkspaceActiveSnapshotState } from "../workspaceActiveSnapshotStore";

export type SessionSupervisorSubscribedSessionIdsSink = ((sessionIds: string[]) => void) | null;

export type SessionSupervisorWorkspaceSnapshotState = WorkspaceActiveSnapshotState | null;

export type SessionSupervisorWorkspaceSessionHeads = Record<string, SessionHeadSnapshot>;

export type SessionSupervisorWorkspaceEvent = WorkspaceActiveSnapshotEvent;
