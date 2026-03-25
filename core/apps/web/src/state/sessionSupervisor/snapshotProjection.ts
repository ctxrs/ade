import type {
  ConnectionStatus,
  InternalEntry,
  SessionCacheEntry,
  SessionSupervisorSnapshot,
} from "./entryState";
import { buildSessionThreadProjectionFromSnapshot } from "../sessionThreadProjection/applySnapshot";

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

const cloneSessionEntry = (entry: InternalEntry): SessionCacheEntry => {
  const baseThreadProjection = buildSessionThreadProjectionFromSnapshot(entry);
  const overlay = entry.overlay;
  const support = entry.support;
  return {
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
    turnToolsByTurnId: support.turnToolsByTurnId,
    turnToolsLoading: [...support.turnToolsLoadingSet],
    toolSummaries: entry.toolSummaries,
    toolSummariesReady: support.toolSummariesReady,
    hasMoreTurns: entry.hasMoreTurns,
    events: entry.events,
    eventsRev: entry.eventsRev,
    messages: entry.messages,
    messagesRev: entry.messagesRev,
    turnsRev: entry.turnsRev,
    artifacts: support.artifacts,
    artifactsLoading: support.artifactsLoading,
    subagentInvocations: support.subagentInvocations,
    subagentInvocationsLoaded: support.subagentInvocationsLoaded,
    subagentInvocationsLoading: support.subagentInvocationsLoading,
    stateLoaded: support.stateLoaded,
    stateLoading: support.stateLoading,
    stateRev: entry.stateRev,
    loadErrors: { ...support.loadErrors },
    queue: entry.queue,
    optimisticThreadMessages: overlay.optimisticThreadMessages,
    optimisticQueuedMessages: overlay.optimisticQueuedMessages,
    optimisticQueueRemovalIds: overlay.optimisticQueueRemovalIds,
    overlayRev: overlay.overlayRev,
    diff: support.diff,
    gitStatusSummary: support.gitStatusSummary ?? null,
    summaryCheckpoint: entry.summaryCheckpoint ?? null,
    headWindow: entry.headWindow ?? null,
    projectionRev: baseThreadProjection.projectionRev,
    threadProjection: baseThreadProjection,
    diagnosticsByPath: entry.diagnosticsByPath,
    lastEventSeq: entry.lastEventSeq,
    loading: entry.loading,
    error: entry.error,
    subscribed: entry.subscribed,
    oldestTurnSeq: entry.oldestTurnSeq,
    fetching: { ...support.fetching },
    updatedAtMs: entry.updatedAtMs,
  };
};

export function publish(this: SessionSupervisorSnapshotProjectionHost) {
  evictIfNeeded.call(this);
  const sessions: Record<string, SessionCacheEntry> = {};
  for (const [id, entry] of this.entries) {
    sessions[id] = cloneSessionEntry(entry);
  }
  this.snapshot = { connection: this.snapshot.connection, sessions };
  for (const listener of this.listeners) listener();
}
