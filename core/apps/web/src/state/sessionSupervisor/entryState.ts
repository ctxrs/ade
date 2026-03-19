import type {
  Artifact,
  GitStatusSummary,
  Message,
  Session,
  SessionEvent,
  SessionHeadWindow,
  SessionSummaryCheckpoint,
  SessionTurn,
  SessionTurnTool,
  SessionTurnToolSummary,
  SubagentInvocation,
} from "../../api/client";
import type { SessionActivityState } from "@ctx/types";

export type ConnectionStatus = "connecting" | "connected" | "disconnected" | "idle";

export type SessionMode = "active" | "archived";

export type SessionLoadState = "pending_hydration" | "live" | "recovering" | "fatal";

export type SessionFreshnessState = "bootstrap" | "authoritative" | "recovering";

export type SessionSupportLoadErrorKey = "state" | "artifacts" | "subagentInvocations";

export type SessionSupportLoadErrors = Partial<Record<SessionSupportLoadErrorKey, string>>;

export type SessionSupervisorSnapshot = {
  connection: ConnectionStatus;
  sessions: Record<string, SessionCacheEntry>;
};

export type SessionCacheEntry = {
  sessionId: string;
  mode?: SessionMode;
  loadState: SessionLoadState;
  freshness: SessionFreshnessState;
  session?: Session;
  activity?: SessionActivityState | null;
  acpModels?: unknown;
  acpModes?: unknown;
  acpCurrentModelId?: string;
  acpCommands?: unknown;
  acpSlashCommands?: unknown;
  turns: SessionTurn[];
  turnToolsByTurnId: Record<string, SessionTurnTool[]>;
  turnToolsLoading: string[];
  toolSummaries: SessionTurnToolSummary[];
  toolSummariesReady: boolean;
  hasMoreTurns: boolean;
  events: SessionEvent[];
  messages: Message[];
  messagesRev?: number;
  turnsRev?: number;
  eventsRev?: number;
  artifacts: Artifact[];
  artifactsLoading: boolean;
  subagentInvocations: SubagentInvocation[];
  subagentInvocationsLoaded?: boolean;
  subagentInvocationsLoading: boolean;
  stateLoaded: boolean;
  stateLoading: boolean;
  stateRev?: number;
  loadErrors?: SessionSupportLoadErrors;
  queue: Message[];
  diff?: string;
  gitStatusSummary?: GitStatusSummary | null;
  summaryCheckpoint?: SessionSummaryCheckpoint | null;
  headWindow?: SessionHeadWindow | null;
  projectionRev?: number;
  diagnosticsByPath?: Record<string, unknown[]>;
  lastEventSeq?: number;
  loading: boolean;
  error?: string;
  subscribed: boolean;
  oldestTurnSeq?: number;
  fetching?: {
    head: boolean;
    history: boolean;
  };
  updatedAtMs: number;
};

type ThoughtCacheEntry = {
  key: string;
  event: SessionEvent;
  updatedAtMs?: number;
};

export type OpenOptions = {
  watchDiff?: boolean;
  force?: boolean;
  silent?: boolean;
  mode?: SessionMode;
};

export type InternalEntry = SessionCacheEntry & {
  refCount: number;
  warmUntilMs: number;
  historyExtended: boolean;
  acpMetaUpdatedAtMs?: number;
  seqSet: Set<number>;
  nextTransientSeq: number;
  startedTurnIds: Set<string>;
  turnsHydrated: boolean;
  oldestTurnSeq?: number;
  toolStatusByKey: Map<string, string>;
  toolIdsByTurn: Map<string, Set<string>>;
  turnToolsLoadingSet: Set<string>;
  turnToolsHydratedByTurnId: Record<string, boolean>;
  turnsRev: number;
  messagesRev: number;
  eventsRev: number;
  artifactsLoaded: boolean;
  artifactsFetchedAtMs?: number;
  subagentInvocationsLoaded: boolean;
  subagentInvocationsFetchedAtMs?: number;
  subagentInvocationsAppliedRev?: number;
  stateLoaded: boolean;
  stateLoading: boolean;
  stateRev?: number;
  stateAppliedRev?: number;
  stateFetchToken: number;
  loadErrors: SessionSupportLoadErrors;
  diagnosticsByPath: Record<string, unknown[]>;
  headFromCache: boolean;
  thoughtCacheByKey: Record<string, ThoughtCacheEntry>;
  thoughtCacheLoaded: boolean;
  thoughtCacheLoading: boolean;
  thoughtCacheDirty: boolean;
  thoughtCacheOwnerTaskKey?: string;
  thoughtCacheLoadToken: number;
  supportFreshnessEpoch: number;
  stateAutoLoadKey?: string;
  subagentAutoLoadKey?: string;
  fetching: {
    head: boolean;
    history: boolean;
  };
};

export function createInternalEntry(
  sessionId: string,
  opts: { transientSeqStart: number; warmTtlMs: number },
): InternalEntry {
  return {
    sessionId,
    mode: undefined,
    loadState: "pending_hydration",
    freshness: "bootstrap",
    session: undefined,
    activity: null,
    acpModels: undefined,
    acpModes: undefined,
    acpCurrentModelId: undefined,
    acpCommands: undefined,
    acpSlashCommands: undefined,
    turns: [],
    turnToolsByTurnId: {},
    turnToolsLoading: [],
    toolSummaries: [],
    toolSummariesReady: false,
    hasMoreTurns: true,
    events: [],
    eventsRev: 0,
    messages: [],
    messagesRev: 0,
    turnsRev: 0,
    artifacts: [],
    artifactsLoading: false,
    subagentInvocations: [],
    subagentInvocationsLoading: false,
    stateLoaded: false,
    stateLoading: false,
    stateRev: undefined,
    stateAppliedRev: undefined,
    stateFetchToken: 0,
    loadErrors: {},
    queue: [],
    diff: undefined,
    gitStatusSummary: null,
  summaryCheckpoint: null,
  headWindow: null,
  projectionRev: undefined,
  diagnosticsByPath: {},
    lastEventSeq: undefined,
    loading: false,
    error: undefined,
    subscribed: false,
    updatedAtMs: Date.now(),
    refCount: 0,
    warmUntilMs: Date.now() + opts.warmTtlMs,
    acpMetaUpdatedAtMs: undefined,
    seqSet: new Set<number>(),
    nextTransientSeq: opts.transientSeqStart,
    startedTurnIds: new Set<string>(),
    turnsHydrated: false,
    oldestTurnSeq: undefined,
    toolStatusByKey: new Map(),
    toolIdsByTurn: new Map(),
    turnToolsLoadingSet: new Set(),
    turnToolsHydratedByTurnId: {},
    artifactsLoaded: false,
    artifactsFetchedAtMs: undefined,
    subagentInvocationsLoaded: false,
    subagentInvocationsFetchedAtMs: undefined,
    subagentInvocationsAppliedRev: undefined,
    headFromCache: false,
    historyExtended: false,
    thoughtCacheByKey: {},
    thoughtCacheLoaded: false,
    thoughtCacheLoading: false,
    thoughtCacheDirty: false,
    thoughtCacheOwnerTaskKey: undefined,
    thoughtCacheLoadToken: 0,
    supportFreshnessEpoch: 0,
    stateAutoLoadKey: undefined,
    subagentAutoLoadKey: undefined,
    fetching: {
      head: false,
      history: false,
    },
  };
}
