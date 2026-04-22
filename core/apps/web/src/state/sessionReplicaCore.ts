import type {
  Message,
  Session,
  SessionActivityState,
  SessionEvent,
  SessionHead,
  SessionHeadDelta,
  SessionHeadSnapshot,
  SessionHeadWindow,
  SessionSnapshot,
  SessionState,
  SessionSummaryCheckpoint,
  SessionTurn,
  SessionTurnToolSummary,
  WorkspaceActiveSnapshotEvent,
} from "@ctx/types";
import { idToString } from "../api/client";
import { clearAllAssistantStreaming, type AssistantStreamingState } from "./assistantStreaming";
import {
  clearSessionHeadV1,
  clearSessionHistoryPagesV1,
  loadSessionHeadV1,
  saveSessionHeadV1,
} from "./uiStateStore";
import {
  applyReplicaTranscriptEvent,
  ensureReplicaEventSeq,
  isStreamOnlyAssistantChunk,
  mergeReplicaEventsIntoEntry,
  mergeReplicaMessagesIntoEntry,
  mergeReplicaTurnsIntoEntry,
  rebuildReplicaTranscriptAuxState,
} from "./sessionReplicaTranscript";
import {
  isBoundedSessionHead,
  shouldPreserveExistingTranscriptWindow,
  shouldRepairSessionHeadReplace,
} from "./sessionHeadRepair";
import {
  mergeTurnStatus,
  reconcileActivityInterruptedFromTurns,
  reconcileLatestTurnInterruptedFromActivity,
} from "./sessionSupervisor/cachePolicy";
import {
  noteFinalDeltaReceived,
  noteGapRepairMismatch,
  noteGapRecoveryFinished,
  noteGapRecoveryStarted,
  noteProjectionOrSeqRegression,
} from "./foregroundFreshnessTelemetry";
import type {
  SessionReplicaAppendMode,
  SessionReplicaCommand,
  SessionReplicaConfig,
  SessionReplicaData,
  SessionReplicaFreshnessState,
  SessionReplicaHeadSeedMode,
  SessionReplicaReplaceMode,
  SessionReplicaPatch,
} from "./sessionReplicaProtocol";
import { isAuthoritativeSessionReplicaReplace } from "./sessionReplicaProtocol";
import {
  buildCanonicalReplicaPatch,
  buildStreamingOverlayReplicaPatch,
} from "./sessionReplicaPatches";

export type SessionReplicaApi = {
  getSessionHead: (sessionId: string, limit?: number, includeEvents?: boolean) => Promise<SessionHeadSnapshot | null>;
  getSessionState?: (sessionId: string) => Promise<SessionState | null>;
  getSessionSnapshot?: (sessionId: string, limit?: number, includeEvents?: boolean) => Promise<SessionSnapshot | null>;
  setAuth?: (baseUrl?: string | null, authToken?: string | null, runId?: string | null) => void;
};

type SessionReplicaEntry = {
  sessionId: string;
  session?: Session;
  activity?: SessionActivityState | null;
  activityLastEventSeq?: number;
  activityProjectionRev?: number;
  freshness: SessionReplicaFreshnessState;
  summaryCheckpoint?: SessionSummaryCheckpoint | null;
  headWindow?: SessionHeadWindow | null;
  projectionRev?: number;
  stateRev?: number;
  turns: SessionTurn[];
  turnsRev: number;
  assistantStreamingByTurnId: Record<string, AssistantStreamingState>;
  assistantStreamingRev: number;
  messages: Message[];
  messagesRev: number;
  events: SessionEvent[];
  eventsRev: number;
  toolSummaries: SessionTurnToolSummary[];
  lastEventSeq?: number;
  hasMoreTurns: boolean;
  loading: boolean;
  requestToken: number;
  hydrated: boolean;
  nextTransientSeq: number;
  startedTurnIds: Set<string>;
  toolStatusByKey: Map<string, string>;
  toolIdsByTurn: Map<string, Set<string>>;
};

const normalizeId = (value: unknown): string => (typeof value === "string" ? value.trim() : "");

const SHOULD_EMIT_DEV_DIAGNOSTICS = import.meta.env.DEV && import.meta.env.MODE !== "test";

const isFinalThoughtEvent = (event: SessionEvent | null | undefined): boolean => {
  if (!event || String(event.event_type ?? "") !== "thought_chunk") return false;
  const payload = event.payload_json ?? {};
  return payload?.is_final === true
    || payload?.isFinal === true
    || typeof payload?.full_content === "string"
    || typeof payload?.fullContent === "string";
};

const PARTIAL_EVENT_TYPES = new Set(["assistant_chunk", "assistant_complete", "context_window_update"]);
const FINAL_DELTA_EVENT_TYPES = new Set(["assistant_complete", "assistant_message_inserted"]);

const isPartialEvent = (event: SessionEvent | null | undefined): boolean => {
  if (!event) return false;
  const type = String(event.event_type ?? "");
  return PARTIAL_EVENT_TYPES.has(type) || (type === "thought_chunk" && !isFinalThoughtEvent(event));
};

const resolveFinalDeltaTurnId = (delta: SessionHeadDelta): string => {
  const messageTurnId =
    delta.message?.role === "assistant" ? normalizeId(delta.message.turn_id ?? "") : "";
  const eventType = String(delta.event?.event_type ?? "");
  if (FINAL_DELTA_EVENT_TYPES.has(eventType)) return normalizeId(delta.event?.turn_id ?? messageTurnId);
  return messageTurnId;
};

const stripTurnPartials = (turns: SessionTurn[]): SessionTurn[] =>
  turns.map((turn) => ({ ...turn, assistant_partial: null, thought_partial: null }));

const stripPartialEvents = (events: SessionEvent[]): SessionEvent[] =>
  events.filter((event) => !isPartialEvent(event));

const sanitizeHeadForCache = (head: SessionHead): SessionHead => ({
  ...head,
  turns: stripTurnPartials(head.turns ?? []),
  events: stripPartialEvents(head.events ?? []),
});

const mergeToolSummaries = (
  previous: SessionTurnToolSummary[],
  next: SessionTurnToolSummary[],
): SessionTurnToolSummary[] => {
  const byId = new Map<string, SessionTurnToolSummary>();
  for (const summary of previous) byId.set(String(summary.tool_call_id), summary);
  for (const summary of next) byId.set(String(summary.tool_call_id), summary);
  return Array.from(byId.values());
};

const isOlderVersion = (
  incomingLastEventSeq: number | null,
  incomingProjectionRev: number | null,
  existingLastEventSeq: number | null,
  existingProjectionRev: number | null,
): boolean => {
  if (incomingLastEventSeq !== null && existingLastEventSeq !== null) return incomingLastEventSeq < existingLastEventSeq;
  if (existingLastEventSeq !== null && incomingLastEventSeq === null) return true;
  if (incomingLastEventSeq !== null && existingLastEventSeq === null) return false;
  return incomingProjectionRev !== null
    && existingProjectionRev !== null
    && incomingProjectionRev < existingProjectionRev;
};

const mergePartial = (previous: string, next: string): string => {
  if (!previous) return next;
  if (!next) return previous;
  if (next.startsWith(previous)) return next;
  if (previous.startsWith(next)) return previous;
  return next.length >= previous.length ? next : previous;
};

const mergeTurn = (previous: SessionTurn, next: SessionTurn): SessionTurn => ({
  ...previous,
  ...next,
  status: mergeTurnStatus(previous.status, next.status),
  assistant_partial: null,
  thought_partial: mergePartial(previous.thought_partial ?? "", next.thought_partial ?? ""),
  end_seq: next.end_seq ?? previous.end_seq,
  updated_at:
    String(next.updated_at ?? "").localeCompare(String(previous.updated_at ?? "")) >= 0
      ? next.updated_at
      : previous.updated_at,
  tool_total: Math.max(previous.tool_total ?? 0, next.tool_total ?? 0),
  tool_pending: Math.max(previous.tool_pending ?? 0, next.tool_pending ?? 0),
  tool_running: Math.max(previous.tool_running ?? 0, next.tool_running ?? 0),
  tool_completed: Math.max(previous.tool_completed ?? 0, next.tool_completed ?? 0),
  tool_failed: Math.max(previous.tool_failed ?? 0, next.tool_failed ?? 0),
  metrics_json: next.metrics_json ?? previous.metrics_json,
});

const compareTurnOrder = (left: SessionTurn, right: SessionTurn): number => {
  const leftSeq = Number(left.start_seq ?? Number.NaN);
  const rightSeq = Number(right.start_seq ?? Number.NaN);
  if (Number.isFinite(leftSeq) && Number.isFinite(rightSeq) && leftSeq !== rightSeq) return leftSeq - rightSeq;
  return String(left.started_at).localeCompare(String(right.started_at));
};

const mergeTurns = (base: SessionTurn[], incoming: SessionTurn[]): SessionTurn[] => {
  if (incoming.length === 0) return base;
  const byId = new Map<string, SessionTurn>();
  for (const turn of base) {
    const id = normalizeId(turn.turn_id);
    if (id) byId.set(id, turn);
  }
  for (const turn of incoming) {
    const id = normalizeId(turn.turn_id);
    if (!id) continue;
    byId.set(id, byId.has(id) ? mergeTurn(byId.get(id)!, turn) : turn);
  }
  return Array.from(byId.values()).sort(compareTurnOrder);
};

const mergeMessages = (base: Message[], incoming: Message[]): Message[] => {
  if (incoming.length === 0) return base;
  const byId = new Map<string, Message>();
  for (const message of base) {
    const id = normalizeId(message.id);
    if (id) byId.set(id, message);
  }
  for (const message of incoming) {
    const id = normalizeId(message.id);
    if (id) byId.set(id, message);
  }
  return Array.from(byId.values()).sort((left, right) => {
    const createdAt = String(left.created_at).localeCompare(String(right.created_at));
    if (createdAt !== 0) return createdAt;
    const leftSeq = Number(left.turn_sequence ?? Number.NaN);
    const rightSeq = Number(right.turn_sequence ?? Number.NaN);
    if (Number.isFinite(leftSeq) && Number.isFinite(rightSeq) && leftSeq !== rightSeq) return leftSeq - rightSeq;
    if (Number.isFinite(leftSeq) && !Number.isFinite(rightSeq)) return -1;
    if (!Number.isFinite(leftSeq) && Number.isFinite(rightSeq)) return 1;
    return String(normalizeId(left.id)).localeCompare(String(normalizeId(right.id)));
  });
};

const mergeEvents = (base: SessionEvent[], incoming: SessionEvent[]): SessionEvent[] => {
  if (incoming.length === 0) return base;
  const bySeq = new Map<number, SessionEvent>();
  for (const event of base) {
    if (typeof event.seq === "number") bySeq.set(event.seq, event);
  }
  for (const event of incoming) {
    if (typeof event.seq === "number") bySeq.set(event.seq, event);
  }
  return Array.from(bySeq.values()).sort((left, right) => Number(left.seq ?? 0) - Number(right.seq ?? 0));
};

const TRANSIENT_SEQ_START = -4503599627370496; // -(2 ** 52)

const headToData = (head: SessionHead | SessionHeadSnapshot): SessionReplicaData => {
  const data: SessionReplicaData = {
    turns: head.turns ?? [],
    messages: head.messages ?? [],
    events: head.events ?? [],
    toolSummaries: head.tool_summaries ?? [],
    lastEventSeq: head.last_event_seq,
    hasMoreTurns: head.has_more_turns,
  };
  if (head.session) data.session = head.session;
  if ("activity" in head) data.activity = head.activity ?? null;
  if ("summary_checkpoint" in head) data.summaryCheckpoint = head.summary_checkpoint ?? null;
  if ("head_window" in head) data.headWindow = head.head_window ?? null;
  if ("projection_rev" in head && typeof head.projection_rev === "number") data.projectionRev = head.projection_rev;
  if ("state_rev" in head && typeof head.state_rev === "number") data.stateRev = head.state_rev;
  return data;
};

const snapshotToHead = (head: SessionHeadSnapshot): SessionHead => ({
  session: head.session,
  turns: head.turns,
  tool_summaries: head.tool_summaries,
  events: head.events,
  messages: head.messages,
  last_event_seq: head.last_event_seq,
  projection_rev: head.projection_rev,
  has_more_turns: head.has_more_turns,
  activity: head.activity,
  summary_checkpoint: head.summary_checkpoint ?? null,
  head_window: head.head_window,
});

export class SessionReplicaCore {
  private entries = new Map<string, SessionReplicaEntry>();
  private config: SessionReplicaConfig = { eventBufferLimit: 800, headLimit: 60 };
  private gapAlertedSessionIds = new Set<string>();
  private gapRepairBaselineBySessionId = new Map<string, { lastEventSeq: number | null }>();
  constructor(private deps: { api: SessionReplicaApi; emit: (patches: SessionReplicaPatch[]) => void }) {}

  handleCommand = (cmd: SessionReplicaCommand) => {
    switch (cmd.type) {
      case "init":
        if (cmd.config) this.config = cmd.config;
        this.deps.api.setAuth?.(cmd.baseUrl ?? null, cmd.authToken ?? null, cmd.runId ?? null);
        return;
      case "update_auth":
        this.deps.api.setAuth?.(cmd.baseUrl ?? null, cmd.authToken ?? null, cmd.runId ?? null);
        return;
      case "open_session":
        this.openSession(cmd.sessionId, {
          force: cmd.force,
          silent: cmd.silent,
          skipCache: cmd.skipCache,
          skipBoundedBootstrapCache: cmd.skipBoundedBootstrapCache,
          hydrateIfNeeded: cmd.hydrateIfNeeded,
          forceHydrate: cmd.forceHydrate,
        }).catch(() => {});
        return;
      case "close_session":
        this.closeSession(cmd.sessionId);
        return;
      case "drop_session":
        this.dropSession(cmd.sessionId);
        return;
      case "refresh_session":
        this.hydrateSessionHead(cmd.sessionId, {
          force: true,
          silent: true,
          emitOp: "append",
        }).catch(() => {});
        return;
      case "hydrate_session_head":
        this.hydrateSessionHead(cmd.sessionId, {
          force: cmd.force,
          silent: cmd.silent,
        }).catch(() => {});
        return;
      case "seed_head":
        this.seedHead(cmd.sessionId, cmd.head, cmd.mode);
        return;
      case "workspace_event":
        this.handleWorkspaceEvent(cmd.event);
        return;
      case "set_session":
        this.setSession(cmd.session);
        return;
      default:
        return;
    }
  };

  private emitPatches(patches: SessionReplicaPatch[]) { if (patches.length) this.deps.emit(patches); }
  private emitPatch(
    op: "append",
    sessionId: string,
    data: SessionReplicaData & { appendMode: SessionReplicaAppendMode },
  ): void;
  private emitPatch(op: "replace", sessionId: string, data: SessionReplicaData): void;
  private emitPatch(op: "evict", sessionId: string, data: { eventsBeforeSeq?: number }): void;
  private emitPatch(
    op: SessionReplicaPatch["op"],
    sessionId: string,
    data: SessionReplicaData | { eventsBeforeSeq?: number },
  ) {
    const id = normalizeId(sessionId);
    if (!id) return;
    this.emitPatches([{ op, sessionId: id, data } as SessionReplicaPatch]);
  }

  private ensureEntry(sessionId: string): SessionReplicaEntry {
    const id = normalizeId(sessionId);
    const existing = this.entries.get(id);
    if (existing) return existing;
    const entry: SessionReplicaEntry = {
      sessionId: id,
      session: undefined,
      activity: null,
      activityLastEventSeq: undefined,
      activityProjectionRev: undefined,
      freshness: "bootstrap",
      summaryCheckpoint: undefined,
      headWindow: undefined,
      projectionRev: undefined,
      stateRev: undefined,
      turns: [],
      turnsRev: 0,
      assistantStreamingByTurnId: {},
      assistantStreamingRev: 0,
      messages: [],
      messagesRev: 0,
      events: [],
      eventsRev: 0,
      toolSummaries: [],
      lastEventSeq: undefined,
      hasMoreTurns: true,
      loading: false,
      requestToken: 0,
      hydrated: false,
      nextTransientSeq: TRANSIENT_SEQ_START,
      startedTurnIds: new Set<string>(),
      toolStatusByKey: new Map<string, string>(),
      toolIdsByTurn: new Map<string, Set<string>>(),
    };
    this.entries.set(id, entry);
    return entry;
  }

  private ensureEventSeq(entry: SessionReplicaEntry, event: SessionEvent): SessionEvent {
    return ensureReplicaEventSeq(entry, event);
  }

  private normalizeEvents(entry: SessionReplicaEntry, events: SessionEvent[]): SessionEvent[] {
    if (events.length === 0) return events;
    return events.map((event) => this.ensureEventSeq(entry, event));
  }

  private setSession(session: Session) {
    const sessionId = normalizeId(session.id);
    if (!sessionId) return;
    const entry = this.ensureEntry(sessionId);
    entry.session = session;
    this.emitPatch("append", sessionId, { session, appendMode: "metadata_update" });
  }

  private closeSession(sessionId: string) {
    const id = normalizeId(sessionId);
    if (!id) return;
    const entry = this.entries.get(id);
    if (!entry) return;
    entry.requestToken += 1;
    entry.loading = false;
  }

  private dropSession(sessionId: string) {
    const id = normalizeId(sessionId);
    if (!id) return;
    this.entries.delete(id);
  }

  private applyHead(
    entry: SessionReplicaEntry,
    head: SessionHead | SessionHeadSnapshot,
    emitOp: "append" | "replace" = "replace",
    opts?: {
      appendMode?: SessionReplicaAppendMode;
      replaceMode?: SessionReplicaReplaceMode;
      freshness?: SessionReplicaFreshnessState;
    },
  ) {
    const authoritative = isAuthoritativeSessionReplicaReplace(opts?.replaceMode);
    const preservingRepairReplace = opts?.replaceMode === "repair_replace";
    const previousFreshness = entry.freshness;
    const data = headToData(head);
    let turns = data.turns ?? [];
    let messages = data.messages ?? [];
    entry.events = this.normalizeEvents(entry, entry.events);
    let events = this.normalizeEvents(entry, data.events ?? []);
    const incomingSeq = typeof data.lastEventSeq === "number" ? data.lastEventSeq : -1;
    const existingSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
    const incomingProjectionRev = typeof data.projectionRev === "number" ? data.projectionRev : null;
    const existingProjectionRev = typeof entry.projectionRev === "number" ? entry.projectionRev : null;
    const incomingIsOlder = isOlderVersion(
      incomingSeq >= 0 ? incomingSeq : null,
      incomingProjectionRev,
      existingSeq >= 0 ? existingSeq : null,
      existingProjectionRev,
    );
    const incomingIsNarrower =
      (!authoritative || preservingRepairReplace) &&
      (turns.length < entry.turns.length ||
        messages.length < entry.messages.length ||
        events.length < entry.events.length ||
        (preservingRepairReplace &&
          shouldPreserveExistingTranscriptWindow(entry, { turns, messages })));
    let toolSummaries = data.toolSummaries ?? entry.toolSummaries;
    if (incomingIsOlder || (!authoritative && existingSeq > incomingSeq) || incomingIsNarrower) {
      turns = mergeTurns(turns, entry.turns);
      messages = mergeMessages(messages, entry.messages);
      events = mergeEvents(events, entry.events);
      toolSummaries = mergeToolSummaries(data.toolSummaries ?? [], entry.toolSummaries);
    }

    entry.session = data.session ?? entry.session;
    if (data.activity !== undefined) {
      const existingActivitySeq =
        typeof entry.activityLastEventSeq === "number" ? entry.activityLastEventSeq : null;
      const existingActivityProjectionRev =
        typeof entry.activityProjectionRev === "number" ? entry.activityProjectionRev : null;
      const incomingActivityIsOlder = isOlderVersion(
        incomingSeq >= 0 ? incomingSeq : null,
        incomingProjectionRev,
        existingActivitySeq,
        existingActivityProjectionRev,
      );
      if (!incomingActivityIsOlder) {
        entry.activity = data.activity ?? null;
        entry.activityLastEventSeq = incomingSeq >= 0 ? incomingSeq : entry.activityLastEventSeq;
        entry.activityProjectionRev = incomingProjectionRev ?? entry.activityProjectionRev;
        if (reconcileLatestTurnInterruptedFromActivity(entry.turns, entry.activity)) {
          entry.turnsRev += 1;
        }
        entry.activity = reconcileActivityInterruptedFromTurns(entry.activity, entry.turns);
      }
    }
    if (opts?.freshness) {
      entry.freshness = opts.freshness;
      if (previousFreshness === "recovering" && opts.freshness !== "recovering") {
        const baseline = this.gapRepairBaselineBySessionId.get(entry.sessionId);
        if (
          baseline &&
          typeof baseline.lastEventSeq === "number" &&
          typeof entry.lastEventSeq === "number" &&
          entry.lastEventSeq < baseline.lastEventSeq
        ) {
          noteGapRepairMismatch(entry.sessionId, baseline.lastEventSeq, entry.lastEventSeq);
        }
        this.gapRepairBaselineBySessionId.delete(entry.sessionId);
        noteGapRecoveryFinished(entry.sessionId);
      }
    }
    if (data.summaryCheckpoint !== undefined) {
      if (!incomingIsOlder) {
        entry.summaryCheckpoint = data.summaryCheckpoint ?? null;
      }
    }
    if (data.headWindow !== undefined) {
      if (!incomingIsOlder) {
        entry.headWindow = data.headWindow ?? null;
      }
    }
    if (data.stateRev !== undefined) {
      entry.stateRev =
        typeof entry.stateRev === "number" ? Math.max(entry.stateRev, data.stateRev) : data.stateRev;
    }
    if (data.projectionRev !== undefined) {
      entry.projectionRev =
        typeof entry.projectionRev === "number"
          ? Math.max(entry.projectionRev, data.projectionRev)
          : data.projectionRev;
    }
    entry.turns = turns;
    entry.turnsRev += 1;
    if (authoritative || (!incomingIsOlder && !incomingIsNarrower)) {
      clearAllAssistantStreaming(entry);
    }
    entry.messages = messages;
    entry.messagesRev += 1;
    entry.events = events;
    entry.eventsRev += 1;
    entry.toolSummaries = toolSummaries;
    entry.lastEventSeq =
      incomingSeq >= 0
        ? Math.max(existingSeq, incomingSeq)
        : entry.lastEventSeq;
    entry.hasMoreTurns = data.hasMoreTurns ?? entry.hasMoreTurns;
    entry.hydrated = true;
    rebuildReplicaTranscriptAuxState(entry);
    if (emitOp === "append") {
      this.emitPatch("append", entry.sessionId, buildCanonicalReplicaPatch(entry, {
        ...opts,
        appendMode: opts?.appendMode ?? "head_refresh",
      }));
      return;
    }
    this.emitPatch("replace", entry.sessionId, buildCanonicalReplicaPatch(entry, {
      replaceMode: opts?.replaceMode,
    }));
  }

  private async openSession(
    sessionId: string,
    opts?: {
      force?: boolean;
      silent?: boolean;
      minEventSeq?: number;
      skipCache?: boolean;
      skipBoundedBootstrapCache?: boolean;
      hydrateIfNeeded?: boolean;
      forceHydrate?: boolean;
      emitOp?: "append" | "replace";
    },
  ) {
    const id = normalizeId(sessionId);
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (entry.loading && !opts?.force) return;
    const shouldHydrate =
      Boolean(opts?.forceHydrate) ||
      (Boolean(opts?.hydrateIfNeeded) && entry.freshness !== "authoritative");
    const minSeq = typeof opts?.minEventSeq === "number" ? opts.minEventSeq : undefined;
    if (!opts?.force && entry.hydrated) {
      const entrySeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
      if ((minSeq === undefined || entrySeq >= minSeq) && !shouldHydrate) {
        if (!opts?.silent) this.emitPatch("append", id, {
          loading: false,
          error: null,
          appendMode: "metadata_update",
        });
        return;
      }
    }
    const token = ++entry.requestToken;
    if (!opts?.skipCache) {
      const cached = await loadSessionHeadV1(id).catch(() => null);
      if (token !== entry.requestToken) return;
      if (cached?.head && (minSeq === undefined || cached.head.last_event_seq >= minSeq)) {
        const shouldSkipBoundedBootstrapCache =
          opts?.skipBoundedBootstrapCache && isBoundedSessionHead(cached.head);
        if (!shouldSkipBoundedBootstrapCache) {
          this.applyHead(entry, cached.head, opts?.emitOp, {
            appendMode: opts?.emitOp === "append" ? "head_refresh" : undefined,
            replaceMode: opts?.emitOp === "append" ? undefined : "bootstrap_seed",
            freshness: "bootstrap",
          });
        }
      }
    }
    if (!opts?.force && entry.hydrated) {
      const entrySeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
      if ((minSeq === undefined || entrySeq >= minSeq) && !shouldHydrate) {
        if (!opts?.silent) this.emitPatch("append", id, {
          loading: false,
          error: null,
          appendMode: "metadata_update",
        });
        return;
      }
    }

    entry.loading = true;
    if (!opts?.silent) this.emitPatch("append", id, {
      loading: true,
      error: null,
      appendMode: "metadata_update",
    });
    if (!shouldHydrate) {
      entry.loading = false;
      if (!opts?.silent) this.emitPatch("append", id, {
        loading: false,
        error: null,
        appendMode: "metadata_update",
      });
      return;
    }
    try {
      const head = await this.deps.api.getSessionHead(id, this.config.headLimit, true);
      if (token !== entry.requestToken) return;
      if (head) {
        const persisted = snapshotToHead(head);
        this.applyHead(entry, persisted, opts?.emitOp, {
          appendMode: opts?.emitOp === "append" ? "head_refresh" : undefined,
          replaceMode: opts?.emitOp === "append" ? undefined : "authoritative_replace",
          freshness: "authoritative",
        });
        await this.persistHead(entry);
      }
      entry.loading = false;
      if (!opts?.silent) this.emitPatch("append", id, {
        loading: false,
        error: null,
        appendMode: "metadata_update",
      });
    } catch (err) {
      entry.loading = false;
      const message = err instanceof Error && err.message ? err.message : typeof err === "string" ? err : "request failed";
      if (!opts?.silent) this.emitPatch("append", id, {
        loading: false,
        error: message,
        appendMode: "metadata_update",
      });
    }
  }

  private async hydrateSessionHead(
    sessionId: string,
    opts?: {
      force?: boolean;
      silent?: boolean;
      emitOp?: "append" | "replace";
    },
  ) {
    const id = normalizeId(sessionId);
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (entry.loading && !opts?.force) return;
    if (!opts?.force && entry.hydrated) return;
    const token = ++entry.requestToken;
    entry.loading = true;
    if (!opts?.silent) this.emitPatch("append", id, {
      loading: true,
      error: null,
      appendMode: "metadata_update",
    });

    try {
      const head = await this.deps.api.getSessionHead(id, this.config.headLimit, true);
      if (token !== entry.requestToken) return;
      if (head) {
        const persisted = snapshotToHead(head);
        this.applyHead(entry, persisted, opts?.emitOp, {
          appendMode: opts?.emitOp === "append" ? "head_refresh" : undefined,
          replaceMode: opts?.emitOp === "append" ? undefined : "authoritative_replace",
          freshness: "authoritative",
        });
        await this.persistHead(entry);
      }
      entry.loading = false;
      if (!opts?.silent) this.emitPatch("append", id, {
        loading: false,
        appendMode: "metadata_update",
      });
    } catch (err) {
      entry.loading = false;
      const message = err instanceof Error && err.message ? err.message : typeof err === "string" ? err : "request failed";
      if (!opts?.silent) this.emitPatch("append", id, {
        loading: false,
        error: message,
        appendMode: "metadata_update",
      });
    }
  }

  private seedHead(
    sessionId: string,
    head: SessionHeadSnapshot,
    mode: SessionReplicaHeadSeedMode,
  ) {
    const id = normalizeId(sessionId);
    if (!id) return;
    const entry = this.ensureEntry(id);
    this.applyHead(entry, head, "replace", {
      replaceMode: mode,
      freshness: mode === "repair_replace" ? "authoritative" : "bootstrap",
    });
    entry.hydrated = true;
  }

  private handleWorkspaceEvent(evt: WorkspaceActiveSnapshotEvent) {
    const evtType = (evt as { type?: string }).type;
    if (evtType === "session_head_delta" || evtType === "session_delta") {
      const delta = (evt as { delta?: SessionHeadDelta }).delta;
      if (delta) {
        const turnId = resolveFinalDeltaTurnId(delta);
        if (turnId) {
          noteFinalDeltaReceived({
            sessionId: normalizeId(delta.session_id),
            turnId,
            emittedAtMs:
              typeof delta.emitted_at_ms === "number" && Number.isFinite(delta.emitted_at_ms)
                ? delta.emitted_at_ms
                : null,
            lastEventSeq: delta.last_event_seq,
          });
        }
        this.applyHeadDelta(delta);
      }
      return;
    }
    if (evtType === "session_head_seed") {
      const head = (evt as { head?: SessionHeadSnapshot }).head;
      const sessionId = normalizeId(head?.session?.id ?? "");
      if (!head || !sessionId) return;
      const entry = this.ensureEntry(sessionId);
      const replaceMode: SessionReplicaReplaceMode =
        isBoundedSessionHead(head) || shouldRepairSessionHeadReplace(entry, head)
          ? "repair_replace"
          : "authoritative_replace";
      this.applyHead(entry, head, "replace", {
        replaceMode,
        freshness: "authoritative",
      });
      return;
    }
    if (evtType === "session_gap") {
      const sessionId = normalizeId((evt as { session_id?: unknown }).session_id);
      const afterSeq = typeof (evt as { after_seq?: number }).after_seq === "number" ? (evt as { after_seq?: number }).after_seq : undefined;
      if (!sessionId) return;
      noteGapRecoveryStarted(
        sessionId,
        typeof (evt as { reason?: unknown }).reason === "string" ? String((evt as { reason?: unknown }).reason) : null,
      );
      const previousEntry = this.entries.get(sessionId);
      this.gapRepairBaselineBySessionId.set(sessionId, {
        lastEventSeq:
          typeof previousEntry?.lastEventSeq === "number" ? previousEntry.lastEventSeq : null,
      });
      if (typeof window !== "undefined" && SHOULD_EMIT_DEV_DIAGNOSTICS) {
        const prevSeq = this.entries.get(sessionId)?.lastEventSeq;
        const message = [
          "ctx session_gap detected.",
          `session_id=${sessionId}`,
          `after_seq=${afterSeq ?? "unknown"}`,
          `last_event_seq=${prevSeq ?? "unknown"}`,
        ].join("\n");
        if (!this.gapAlertedSessionIds.has(sessionId)) {
          this.gapAlertedSessionIds.add(sessionId);
          try {
            window.alert(message);
          } catch {}
        }
        // eslint-disable-next-line no-console
        console.warn(message);
      }
      void clearSessionHeadV1(sessionId).catch(() => {});
      void clearSessionHistoryPagesV1(sessionId).catch(() => {});
      const entry = this.entries.get(sessionId);
      if (!entry) return;
      entry.hydrated = false;
      entry.freshness = "recovering";
      this.emitPatch("append", sessionId, {
        freshness: "recovering",
        error: null,
        appendMode: "metadata_update",
      });
      this.hydrateSessionHead(sessionId, {
        force: true,
        emitOp: "replace",
      }).catch(() => {});
      return;
    }
  }

  private applyHeadDelta(delta: SessionHeadDelta) {
    const sessionId = normalizeId(delta.session_id);
    if (!sessionId) return;
    const rawEvent = delta.event ?? null;
    const rawStreamOnlyAssistantChunk = rawEvent ? isStreamOnlyAssistantChunk(rawEvent) : false;
    const toolSummaries = Array.isArray(delta.tool_summaries) ? delta.tool_summaries : [];
    const streamOnlyCandidate =
      rawStreamOnlyAssistantChunk &&
      !delta.turn &&
      !delta.message &&
      toolSummaries.length === 0;
    const existingEntry = this.entries.get(sessionId);
    if (streamOnlyCandidate && !existingEntry) return;
    const entry = existingEntry ?? this.ensureEntry(sessionId);
    const previousAssistantStreamingRev = entry.assistantStreamingRev;
    const turns: SessionTurn[] = [];
    const messages: Message[] = [];
    const events: SessionEvent[] = [];
    if (delta.turn) turns.push(delta.turn);
    if (delta.message) messages.push(delta.message);
    const event = rawEvent && !rawStreamOnlyAssistantChunk ? this.ensureEventSeq(entry, rawEvent) : rawEvent;
    const streamOnlyAssistantChunk = event ? isStreamOnlyAssistantChunk(event) : false;
    if (event && !streamOnlyAssistantChunk) events.push(event);
    if (turns.length) {
      mergeReplicaTurnsIntoEntry(entry, turns);
    }
    if (messages.length) {
      mergeReplicaMessagesIntoEntry(entry, messages);
    }
    const { newEvents, evictedBeforeSeq } =
      events.length > 0
        ? mergeReplicaEventsIntoEntry(entry, events, this.config.eventBufferLimit)
        : { newEvents: [] as SessionEvent[] };
    for (const event of newEvents) {
      applyReplicaTranscriptEvent(entry, event);
    }
    if (event && streamOnlyAssistantChunk) {
      applyReplicaTranscriptEvent(entry, event);
    }
    if (toolSummaries.length) {
      const byId = new Map(entry.toolSummaries.map((summary) => [String(summary.tool_call_id), summary]));
      for (const summary of toolSummaries) {
        byId.set(String(summary.tool_call_id), summary);
      }
      entry.toolSummaries = Array.from(byId.values());
      rebuildReplicaTranscriptAuxState(entry);
    }
    if (typeof evictedBeforeSeq === "number") {
      this.emitPatch("evict", sessionId, { eventsBeforeSeq: evictedBeforeSeq });
    }
    const streamOnlyDelta =
      streamOnlyCandidate &&
      streamOnlyAssistantChunk &&
      turns.length === 0 &&
      messages.length === 0 &&
      events.length === 0 &&
      toolSummaries.length === 0;
    if (streamOnlyDelta) {
      if (entry.assistantStreamingRev === previousAssistantStreamingRev) return;
      this.emitPatch("append", sessionId, buildStreamingOverlayReplicaPatch(entry));
      return;
    }
    entry.freshness = "authoritative";
    const incomingSeq = typeof delta.last_event_seq === "number" ? delta.last_event_seq : -1;
    const existingSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
    if (incomingSeq >= 0 && existingSeq >= 0 && incomingSeq < existingSeq) {
      noteProjectionOrSeqRegression(sessionId, "last_event_seq", incomingSeq, existingSeq);
    }
    if (typeof delta.projection_rev === "number") {
      if (
        typeof entry.projectionRev === "number" &&
        delta.projection_rev < entry.projectionRev
      ) {
        noteProjectionOrSeqRegression(
          sessionId,
          "projection_rev",
          delta.projection_rev,
          entry.projectionRev,
        );
      }
      entry.projectionRev =
        typeof entry.projectionRev === "number"
          ? Math.max(entry.projectionRev, delta.projection_rev)
          : delta.projection_rev;
    }
    entry.lastEventSeq = Math.max(existingSeq, incomingSeq);
    if (typeof delta.state_rev === "number") {
      entry.stateRev =
        typeof entry.stateRev === "number" ? Math.max(entry.stateRev, delta.state_rev) : delta.state_rev;
    }
    if (delta.session) {
      entry.session = delta.session;
    }
    if (delta.activity !== undefined && delta.activity !== null) {
      entry.activity = delta.activity;
      if (typeof delta.last_event_seq === "number") {
        entry.activityLastEventSeq =
          typeof entry.activityLastEventSeq === "number"
            ? Math.max(entry.activityLastEventSeq, delta.last_event_seq)
            : delta.last_event_seq;
      }
      if (typeof delta.projection_rev === "number") {
        entry.activityProjectionRev =
          typeof entry.activityProjectionRev === "number"
            ? Math.max(entry.activityProjectionRev, delta.projection_rev)
            : delta.projection_rev;
      }
    }
    if (reconcileLatestTurnInterruptedFromActivity(entry.turns, entry.activity)) {
      entry.turnsRev += 1;
    }
    entry.activity = reconcileActivityInterruptedFromTurns(entry.activity, entry.turns);
    entry.hydrated = true;
    this.emitPatch("append", sessionId, buildCanonicalReplicaPatch(entry, {
      appendMode: "stream_delta",
    }));
    void this.persistHead(entry);
  }

  private async persistHead(entry: SessionReplicaEntry) {
    if (!entry.session || typeof entry.lastEventSeq !== "number") return;
    const head: SessionHead = {
      session: entry.session,
      turns: entry.turns,
      tool_summaries: entry.toolSummaries,
      events: entry.events,
      messages: entry.messages,
      last_event_seq: entry.lastEventSeq,
      projection_rev: entry.projectionRev,
      activity: entry.activity ?? undefined,
      has_more_turns: entry.hasMoreTurns,
      summary_checkpoint: entry.summaryCheckpoint ?? null,
      head_window: entry.headWindow ?? undefined,
    };
    await saveSessionHeadV1(entry.sessionId, sanitizeHeadForCache(head)).catch(() => {});
  }
}
