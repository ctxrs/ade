import type {
  Artifact,
  Message,
  Session,
  SessionEvent,
  SessionHeadSnapshot,
  SessionHeadWindow,
  SessionSummaryCheckpoint,
  SessionTurn,
  SessionTurnToolSummary,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import type { GitStatusSummary } from "../api/client";

export type SessionReplicaConfig = {
  eventBufferLimit: number;
  headLimit: number;
};

export type SessionReplicaCommand =
  | {
      type: "init";
      config: SessionReplicaConfig;
      baseUrl?: string | null;
      authToken?: string | null;
      runId?: string | null;
    }
  | {
      type: "update_auth";
      baseUrl?: string | null;
      authToken?: string | null;
      runId?: string | null;
    }
  | {
      type: "open_session";
      sessionId: string;
      force?: boolean;
      silent?: boolean;
    }
  | { type: "close_session"; sessionId: string }
  | { type: "refresh_session"; sessionId: string }
  | { type: "hydrate_session_head"; sessionId: string; force?: boolean; silent?: boolean }
  | { type: "seed_head"; sessionId: string; head: SessionHeadSnapshot }
  | { type: "workspace_event"; event: WorkspaceActiveSnapshotEvent }
  | { type: "set_session"; session: Session };

export type SessionReplicaData = {
  session?: Session;
  acpMeta?: {
    models?: unknown;
    modes?: unknown;
    currentModelId?: string;
  };
  turns?: SessionTurn[];
  messages?: Message[];
  events?: SessionEvent[];
  toolSummaries?: SessionTurnToolSummary[];
  headWindow?: SessionHeadWindow | null;
  summaryCheckpoint?: SessionSummaryCheckpoint | null;
  lastEventSeq?: number;
  hasMoreTurns?: boolean;
  stateRev?: number;
  artifacts?: Artifact[];
  gitStatusSummary?: GitStatusSummary | null;
  loading?: boolean;
  error?: string | null;
  turnsHydrated?: boolean;
  stateLoaded?: boolean;
  stateLoading?: boolean;
  artifactsLoaded?: boolean;
  subagentNotice?: boolean;
};

export type SessionReplicaPatch =
  | { op: "append"; sessionId: string; data: SessionReplicaData }
  | { op: "replace"; sessionId: string; data: SessionReplicaData }
  | { op: "evict"; sessionId: string; data: { eventsBeforeSeq?: number } };

export type SessionReplicaWorkerMessage = {
  type: "patches";
  patches: SessionReplicaPatch[];
};
