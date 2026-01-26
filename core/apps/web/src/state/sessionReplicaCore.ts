import type {
  Artifact,
  Message,
  Session,
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
import { loadSessionHeadV1, saveSessionHeadV1 } from "./uiStateStore";
import type { SessionReplicaCommand, SessionReplicaConfig, SessionReplicaData, SessionReplicaPatch } from "./sessionReplicaProtocol";

export type SessionReplicaApi = {
  getSessionHead: (sessionId: string, limit?: number, includeEvents?: boolean) => Promise<SessionHeadSnapshot | null>;
  getSessionState?: (sessionId: string) => Promise<SessionState | null>;
  getSessionSnapshot?: (sessionId: string, limit?: number, includeEvents?: boolean) => Promise<SessionSnapshot | null>;
  listSessionArtifacts?: (sessionId: string) => Promise<Artifact[]>;
  setAuth?: (baseUrl?: string | null, authToken?: string | null) => void;
};

type SessionReplicaEntry = {
  sessionId: string;
  session?: Session;
  summaryCheckpoint?: SessionSummaryCheckpoint | null;
  headWindow?: SessionHeadWindow | null;
  stateRev?: number;
  turns: SessionTurn[];
  messages: Message[];
  events: SessionEvent[];
  toolSummaries: SessionTurnToolSummary[];
  lastEventSeq?: number;
  hasMoreTurns: boolean;
  loading: boolean;
  requestToken: number;
  hydrated: boolean;
};

const normalizeId = (value: unknown): string => {
  if (typeof value === "string") return value.trim();
  return String(idToString(value as { 0?: string } | string) || "").trim();
};

const PARTIAL_EVENT_TYPES: Set<SessionEvent["event_type"]> = new Set([
  "assistant_chunk",
  "thought_chunk",
]);

const isPartialEvent = (event: SessionEvent | null | undefined): boolean => {
  if (!event) return false;
  return PARTIAL_EVENT_TYPES.has(String(event.event_type ?? ""));
};

const stripTurnPartials = (turns: SessionTurn[]): SessionTurn[] =>
  turns.map((turn) => ({
    ...turn,
    assistant_partial: null,
    thought_partial: null,
  }));

const stripPartialEvents = (events: SessionEvent[]): SessionEvent[] =>
  events.filter((event) => !isPartialEvent(event));

const sanitizeHeadForCache = (head: SessionHead): SessionHead => ({
  ...head,
  turns: stripTurnPartials(head.turns ?? []),
  events: stripPartialEvents(head.events ?? []),
});

const mergePartial = (p: string, n: string): string => {
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
};

const mergeTurn = (prev: SessionTurn, next: SessionTurn): SessionTurn => {
  const assistant_partial = mergePartial(prev.assistant_partial ?? "", next.assistant_partial ?? "");
  const thought_partial = mergePartial(prev.thought_partial ?? "", next.thought_partial ?? "");
  return {
    ...prev,
    ...next,
    assistant_partial,
    thought_partial,
    tool_total: Math.max(prev.tool_total ?? 0, next.tool_total ?? 0),
    tool_pending: Math.max(prev.tool_pending ?? 0, next.tool_pending ?? 0),
    tool_running: Math.max(prev.tool_running ?? 0, next.tool_running ?? 0),
    tool_completed: Math.max(prev.tool_completed ?? 0, next.tool_completed ?? 0),
    tool_failed: Math.max(prev.tool_failed ?? 0, next.tool_failed ?? 0),
  };
};

const compareTurnOrder = (a: SessionTurn, b: SessionTurn): number => {
  const sa = Number(a.start_seq ?? Number.NaN);
  const sb = Number(b.start_seq ?? Number.NaN);
  if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) {
    return sa - sb;
  }
  return String(a.started_at).localeCompare(String(b.started_at));
};

const mergeTurns = (base: SessionTurn[], incoming: SessionTurn[]): SessionTurn[] => {
  if (incoming.length === 0) return base;
  const byId = new Map<string, SessionTurn>();
  for (const t of base) {
    const id = normalizeId(t.turn_id);
    if (id) byId.set(id, t);
  }
  for (const t of incoming) {
    const id = normalizeId(t.turn_id);
    if (!id) continue;
    const prev = byId.get(id);
    byId.set(id, prev ? mergeTurn(prev, t) : t);
  }
  return Array.from(byId.values()).sort(compareTurnOrder);
};

const mergeMessages = (base: Message[], incoming: Message[]): Message[] => {
  if (incoming.length === 0) return base;
  const byId = new Map<string, Message>();
  for (const m of base) {
    const id = normalizeId(m.id);
    if (id) byId.set(id, m);
  }
  for (const m of incoming) {
    const id = normalizeId(m.id);
    if (!id) continue;
    byId.set(id, m);
  }
  return Array.from(byId.values()).sort((a, b) => {
    const c = String(a.created_at).localeCompare(String(b.created_at));
    if (c !== 0) return c;
    const sa = Number(a.turn_sequence ?? Number.NaN);
    const sb = Number(b.turn_sequence ?? Number.NaN);
    if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
    if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
    if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
    return String(normalizeId(a.id)).localeCompare(String(normalizeId(b.id)));
  });
};

const mergeEvents = (base: SessionEvent[], incoming: SessionEvent[]): SessionEvent[] => {
  if (incoming.length === 0) return base;
  const bySeq = new Map<number, SessionEvent>();
  for (const ev of base) {
    if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
  }
  for (const ev of incoming) {
    if (typeof ev.seq === "number") bySeq.set(ev.seq, ev);
  }
  return Array.from(bySeq.values()).sort((a, b) => a.seq - b.seq);
};

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
  if ("summary_checkpoint" in head) data.summaryCheckpoint = head.summary_checkpoint ?? null;
  if ("head_window" in head) data.headWindow = head.head_window ?? null;
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
  has_more_turns: head.has_more_turns,
  activity: head.activity,
  summary_checkpoint: head.summary_checkpoint ?? null,
  head_window: head.head_window,
});

export class SessionReplicaCore {
  private entries = new Map<string, SessionReplicaEntry>();
  private config: SessionReplicaConfig = { eventBufferLimit: 800, headLimit: 60 };
  private gapAlertedSessionIds = new Set<string>();
  constructor(private deps: { api: SessionReplicaApi; emit: (patches: SessionReplicaPatch[]) => void }) {}

  handleCommand = (cmd: SessionReplicaCommand) => {
    switch (cmd.type) {
      case "init":
        if (cmd.config) this.config = cmd.config;
        this.deps.api.setAuth?.(cmd.baseUrl ?? null, cmd.authToken ?? null);
        return;
      case "open_session":
        this.openSession(cmd.sessionId, { force: cmd.force, silent: cmd.silent }).catch(() => {});
        return;
      case "close_session":
        this.closeSession(cmd.sessionId);
        return;
      case "refresh_session":
        this.openSession(cmd.sessionId, { force: true, silent: true, emitOp: "append" }).catch(() => {});
        return;
      case "seed_head":
        this.seedHead(cmd.sessionId, cmd.head);
        return;
      case "workspace_event":
        this.handleWorkspaceEvent(cmd.event);
        return;
      case "set_session":
        this.setSession(cmd.session);
        return;
      case "replace_session_id":
        this.replaceSessionId(cmd.oldSessionId, cmd.newSessionId);
        return;
      case "replace_session_task_id":
        this.replaceSessionTaskId(cmd.sessionId, cmd.taskId);
        return;
      default:
        return;
    }
  };

  private emitPatches(patches: SessionReplicaPatch[]) { if (patches.length) this.deps.emit(patches); }
  private emitPatch(op: SessionReplicaPatch["op"], sessionId: string, data: SessionReplicaData) {
    const id = normalizeId(sessionId);
    if (id) this.emitPatches([{ op, sessionId: id, data }]);
  }

  private ensureEntry(sessionId: string): SessionReplicaEntry {
    const id = normalizeId(sessionId);
    const existing = this.entries.get(id);
    if (existing) return existing;
    const entry: SessionReplicaEntry = {
      sessionId: id,
      session: undefined,
      summaryCheckpoint: undefined,
      headWindow: undefined,
      stateRev: undefined,
      turns: [],
      messages: [],
      events: [],
      toolSummaries: [],
      lastEventSeq: undefined,
      hasMoreTurns: true,
      loading: false,
      requestToken: 0,
      hydrated: false,
    };
    this.entries.set(id, entry);
    return entry;
  }

  private setSession(session: Session) {
    const sessionId = normalizeId(session.id);
    if (!sessionId) return;
    const entry = this.ensureEntry(sessionId);
    entry.session = session;
    this.emitPatch("append", sessionId, { session });
  }

  private replaceSessionId(oldSessionId: string, newSessionId: string) {
    const from = normalizeId(oldSessionId);
    const to = normalizeId(newSessionId);
    if (!from || !to || from === to) return;
    const entry = this.entries.get(from);
    if (!entry) return;
    this.entries.delete(from);
    entry.sessionId = to;
    if (entry.session) {
      entry.session = { ...entry.session, id: to };
    }
    entry.turns = entry.turns.map((turn) =>
      idToString(turn.session_id) === from ? { ...turn, session_id: to } : turn,
    );
    entry.messages = entry.messages.map((msg) =>
      idToString(msg.session_id) === from ? { ...msg, session_id: to } : msg,
    );
    entry.events = entry.events.map((event) =>
      idToString(event.session_id) === from ? { ...event, session_id: to } : event,
    );
    entry.toolSummaries = entry.toolSummaries.map((summary) =>
      idToString(summary.session_id) === from ? { ...summary, session_id: to } : summary,
    );
    if (entry.summaryCheckpoint && idToString(entry.summaryCheckpoint.session_id) === from) {
      entry.summaryCheckpoint = { ...entry.summaryCheckpoint, session_id: to };
    }
    this.entries.set(to, entry);
  }

  private replaceSessionTaskId(sessionId: string, taskId: string) {
    const id = normalizeId(sessionId);
    const nextTaskId = String(taskId || "").trim();
    if (!id || !nextTaskId) return;
    const entry = this.entries.get(id);
    if (!entry) return;
    if (entry.session) {
      entry.session = { ...entry.session, task_id: nextTaskId };
    }
    entry.messages = entry.messages.map((msg) => ({ ...msg, task_id: nextTaskId }));
  }

  private closeSession(sessionId: string) {
    const id = normalizeId(sessionId);
    if (id) this.entries.delete(id);
  }

  private applyHead(
    entry: SessionReplicaEntry,
    head: SessionHead | SessionHeadSnapshot,
    emitOp: "append" | "replace" = "replace",
  ) {
    const data = headToData(head);
    let turns = data.turns ?? [];
    let messages = data.messages ?? [];
    let events = data.events ?? [];
    const incomingSeq = typeof data.lastEventSeq === "number" ? data.lastEventSeq : -1;
    const existingSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
    if (existingSeq > incomingSeq) {
      turns = mergeTurns(turns, entry.turns);
      messages = mergeMessages(messages, entry.messages);
      events = mergeEvents(events, entry.events);
    }

    entry.session = data.session ?? entry.session;
    if (data.summaryCheckpoint !== undefined) {
      entry.summaryCheckpoint = data.summaryCheckpoint ?? null;
    }
    if (data.headWindow !== undefined) {
      entry.headWindow = data.headWindow ?? null;
    }
    if (data.stateRev !== undefined) {
      entry.stateRev = data.stateRev;
    }
    entry.turns = turns;
    entry.messages = messages;
    entry.events = events;
    entry.toolSummaries = data.toolSummaries ?? entry.toolSummaries;
    entry.lastEventSeq = Math.max(existingSeq, incomingSeq);
    entry.hasMoreTurns = data.hasMoreTurns ?? entry.hasMoreTurns;
    entry.hydrated = true;

    const patch: SessionReplicaData = {
      session: entry.session,
      turns: entry.turns,
      messages: entry.messages,
      events: entry.events,
      toolSummaries: entry.toolSummaries,
      lastEventSeq: entry.lastEventSeq,
      hasMoreTurns: entry.hasMoreTurns,
      summaryCheckpoint: entry.summaryCheckpoint ?? null,
      headWindow: entry.headWindow ?? null,
    };
    if (entry.stateRev !== undefined) {
      patch.stateRev = entry.stateRev;
    }
    this.emitPatch(emitOp, entry.sessionId, patch);
  }

  private async openSession(
    sessionId: string,
    opts?: { force?: boolean; silent?: boolean; minEventSeq?: number; skipCache?: boolean; emitOp?: "append" | "replace" },
  ) {
    const id = normalizeId(sessionId);
    if (!id) return;
    const entry = this.ensureEntry(id);
    if (entry.loading && !opts?.force) return;
    const minSeq = typeof opts?.minEventSeq === "number" ? opts.minEventSeq : undefined;
    if (!opts?.force && entry.hydrated) {
      const entrySeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
      if (minSeq === undefined || entrySeq >= minSeq) {
        if (!opts?.silent) this.emitPatch("append", id, { loading: false, error: null });
        return;
      }
    }
    const token = ++entry.requestToken;
    if (!opts?.skipCache) {
      const cached = await loadSessionHeadV1(id).catch(() => null);
      if (token !== entry.requestToken) return;
      if (cached?.head && (minSeq === undefined || cached.head.last_event_seq >= minSeq)) {
        this.applyHead(entry, cached.head, opts?.emitOp);
      }
    }
    if (!opts?.force && entry.hydrated) {
      const entrySeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
      if (minSeq === undefined || entrySeq >= minSeq) {
        if (!opts?.silent) this.emitPatch("append", id, { loading: false, error: null });
        return;
      }
    }

    entry.loading = true;
    if (!opts?.silent) this.emitPatch("append", id, { loading: true, error: null });

    try {
      const head = await this.deps.api.getSessionHead(id, this.config.headLimit, true);
      if (token !== entry.requestToken) return;
      if (head) {
        const persisted = snapshotToHead(head);
        this.applyHead(entry, persisted, opts?.emitOp);
        await saveSessionHeadV1(id, sanitizeHeadForCache(persisted)).catch(() => {});
      }
      entry.loading = false;
      if (!opts?.silent) this.emitPatch("append", id, { loading: false });
    } catch (err) {
      entry.loading = false;
      const message = err instanceof Error && err.message ? err.message : typeof err === "string" ? err : "request failed";
      if (!opts?.silent) this.emitPatch("append", id, { loading: false, error: message });
    }
  }

  private seedHead(sessionId: string, head: SessionHeadSnapshot) {
    const id = normalizeId(sessionId);
    if (!id) return;
    const entry = this.ensureEntry(id);
    this.applyHead(entry, head);
    entry.hydrated = true;
  }

  private handleWorkspaceEvent(evt: WorkspaceActiveSnapshotEvent) {
    const evtType = (evt as { type?: string }).type;
    if (evtType === "session_head_delta" || evtType === "session_delta") {
      const delta = (evt as { delta?: SessionHeadDelta }).delta;
      if (delta) this.applyHeadDelta(delta);
      return;
    }
    if (evtType === "session_head_reset") {
      const head = (evt as { head?: SessionHeadSnapshot }).head;
      const sessionId = normalizeId(head?.session?.id ?? "");
      if (!head || !sessionId) return;
      const entry = this.ensureEntry(sessionId);
      this.applyHead(entry, head);
      return;
    }
    if (evtType === "session_gap") {
      const sessionId = normalizeId((evt as { session_id?: unknown }).session_id);
      const afterSeq = typeof (evt as { after_seq?: number }).after_seq === "number" ? (evt as { after_seq?: number }).after_seq : undefined;
      if (!sessionId) return;
      if (typeof window !== "undefined" && import.meta.env.DEV) {
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
      const entry = this.entries.get(sessionId);
      if (!entry) return;
      entry.hydrated = false;
      if (typeof afterSeq === "number") {
        const entrySeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
        if (afterSeq > entrySeq) {
          entry.lastEventSeq = afterSeq;
          this.emitPatch("append", sessionId, { lastEventSeq: afterSeq });
        }
      }
      if (entry.refCount > 0) {
        void this.ensureLoaded(sessionId, { force: true, silent: true });
      }
      return;
    }
  }

  private applyHeadDelta(delta: SessionHeadDelta) {
    const sessionId = normalizeId(delta.session_id);
    if (!sessionId) return;
    const entry = this.ensureEntry(sessionId);
    const turns: SessionTurn[] = [];
    const messages: Message[] = [];
    const events: SessionEvent[] = [];
    if (delta.turn) turns.push(delta.turn);
    if (delta.message) messages.push(delta.message);
    if (delta.event) events.push(delta.event);
    if (turns.length) entry.turns = mergeTurns(entry.turns, turns);
    if (messages.length) entry.messages = mergeMessages(entry.messages, messages);
    if (events.length) entry.events = mergeEvents(entry.events, events);
    if (entry.events.length > this.config.eventBufferLimit) {
      const trimmed = entry.events.slice(-this.config.eventBufferLimit);
      const beforeSeq = trimmed[0]?.seq;
      entry.events = trimmed;
      if (typeof beforeSeq === "number") {
        this.emitPatch("evict", sessionId, { eventsBeforeSeq: beforeSeq });
      }
    }
    const incomingSeq = typeof delta.last_event_seq === "number" ? delta.last_event_seq : -1;
    const existingSeq = typeof entry.lastEventSeq === "number" ? entry.lastEventSeq : -1;
    entry.lastEventSeq = Math.max(existingSeq, incomingSeq);
    const prevStateRev = entry.stateRev;
    if (typeof delta.state_rev === "number") entry.stateRev = delta.state_rev;
    const data: SessionReplicaData = { lastEventSeq: entry.lastEventSeq };
    if (entry.stateRev !== undefined) data.stateRev = entry.stateRev;
    if (turns.length) data.turns = turns;
    if (messages.length) data.messages = messages;
    if (events.length) data.events = events;
    if (entry.session) data.session = entry.session;
    if (entry.summaryCheckpoint !== undefined) data.summaryCheckpoint = entry.summaryCheckpoint ?? null;
    if (entry.headWindow !== undefined) data.headWindow = entry.headWindow ?? null;
    this.emitPatch("append", sessionId, data);
    entry.hydrated = true;
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
      has_more_turns: entry.hasMoreTurns,
      summary_checkpoint: entry.summaryCheckpoint ?? null,
      head_window: entry.headWindow ?? undefined,
    };
    await saveSessionHeadV1(entry.sessionId, sanitizeHeadForCache(head)).catch(() => {});
  }
}
