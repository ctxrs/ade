import type {
  ConnectionStatus,
  InternalEntry,
  SessionCacheEntry,
  SessionSupervisorSnapshot,
} from "./entryState";

export type SessionSupervisorSnapshotProjectionHost = {
  maxCachedSessions: number;
  listeners: Set<() => void>;
  snapshot: SessionSupervisorSnapshot;
  entries: Map<string, InternalEntry>;
};

export function mapConnection(
  connection: "idle" | "connecting" | "connected" | "disconnected",
): ConnectionStatus {
  if (connection === "connected") return "connected";
  if (connection === "disconnected") return "disconnected";
  if (connection === "connecting") return "connecting";
  return "idle";
}

export function setConnection(
  this: SessionSupervisorSnapshotProjectionHost,
  next: ConnectionStatus,
) {
  const prev = this.snapshot.connection;
  if (prev === next) return;
  this.snapshot = { ...this.snapshot, connection: next };
  for (const listener of this.listeners) listener();
}

export function evictIfNeeded(this: SessionSupervisorSnapshotProjectionHost) {
  if (this.entries.size <= this.maxCachedSessions) return;
  const now = Date.now();
  const candidates = [...this.entries.values()]
    .filter((entry) => entry.refCount === 0 && now > entry.warmUntilMs)
    .sort((a, b) => a.updatedAtMs - b.updatedAtMs);
  for (const entry of candidates) {
    if (this.entries.size <= this.maxCachedSessions) break;
    this.entries.delete(entry.sessionId);
  }
}

const cloneSessionEntry = (entry: InternalEntry): SessionCacheEntry => ({
  sessionId: entry.sessionId,
  mode: entry.mode,
  loadState: entry.loadState,
  freshness: entry.freshness,
  session: entry.session,
  activity: entry.activity ?? null,
  acpModels: entry.acpModels,
  acpModes: entry.acpModes,
  acpCurrentModelId: entry.acpCurrentModelId,
  acpCommands: entry.acpCommands,
  acpSlashCommands: entry.acpSlashCommands,
  turns: entry.turns,
  turnToolsByTurnId: entry.turnToolsByTurnId,
  turnToolsLoading: [...entry.turnToolsLoadingSet],
  toolSummaries: entry.toolSummaries,
  toolSummariesReady: entry.toolSummariesReady,
  hasMoreTurns: entry.hasMoreTurns,
  events: entry.events,
  eventsRev: entry.eventsRev,
  messages: entry.messages,
  messagesRev: entry.messagesRev,
  turnsRev: entry.turnsRev,
  artifacts: entry.artifacts,
  artifactsLoading: entry.artifactsLoading,
  subagentInvocations: entry.subagentInvocations,
  subagentInvocationsLoaded: entry.subagentInvocationsLoaded,
  subagentInvocationsLoading: entry.subagentInvocationsLoading,
  stateLoaded: entry.stateLoaded,
  stateLoading: entry.stateLoading,
  stateRev: entry.stateRev,
  loadErrors: { ...entry.loadErrors },
  queue: entry.queue,
  diff: entry.diff,
  gitStatusSummary: entry.gitStatusSummary ?? null,
  summaryCheckpoint: entry.summaryCheckpoint ?? null,
  headWindow: entry.headWindow ?? null,
  diagnosticsByPath: entry.diagnosticsByPath,
  lastEventSeq: entry.lastEventSeq,
  loading: entry.loading,
  error: entry.error,
  subscribed: entry.subscribed,
  oldestTurnSeq: entry.oldestTurnSeq,
  fetching: { ...entry.fetching },
  updatedAtMs: entry.updatedAtMs,
});

export function publish(this: SessionSupervisorSnapshotProjectionHost) {
  evictIfNeeded.call(this);
  const sessions: Record<string, SessionCacheEntry> = {};
  for (const [id, entry] of this.entries) {
    sessions[id] = cloneSessionEntry(entry);
  }
  this.snapshot = { connection: this.snapshot.connection, sessions };
  for (const listener of this.listeners) listener();
}
