import { idToString, type Message, type SessionEvent, type SessionHeadSnapshot, type SessionHeadWindow, type SessionTurn, type SessionTurnToolSummary } from "../api/client";
import { isPartialEvent, mergeTurn, stripPartialEvents, stripTurnPartials } from "./sessionSupervisor/cachePolicy";

const HEAD_EVENT_BUFFER_LIMIT = 800;

export const emptySessionHeadWindow = (): SessionHeadWindow => ({
  turn_limit: 0,
  message_limit: 0,
  event_limit: 0,
  byte_limit: 0,
  turn_count: 0,
  message_count: 0,
  event_count: 0,
  bytes: 0,
  truncated: false,
});

export const sanitizeSessionHeadSnapshot = (head: SessionHeadSnapshot): SessionHeadSnapshot => {
  const turns = Array.isArray(head.turns) ? stripTurnPartials(head.turns) : head.turns ?? [];
  const events = Array.isArray(head.events) ? stripPartialEvents(head.events) : head.events ?? [];
  return {
    ...head,
    turns,
    events,
  };
};

export const compareSessionTurnOrder = (a: SessionTurn, b: SessionTurn): number => {
  const sa = Number(a.start_seq ?? Number.NaN);
  const sb = Number(b.start_seq ?? Number.NaN);
  if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) {
    return sa - sb;
  }
  const startedAtOrder = String(a.started_at ?? "").localeCompare(String(b.started_at ?? ""));
  if (startedAtOrder !== 0) return startedAtOrder;
  return String(a.turn_id ?? "").localeCompare(String(b.turn_id ?? ""));
};

export const mergeSessionTurns = (prev: SessionTurn[], incoming: SessionTurn[]): SessionTurn[] => {
  if (incoming.length === 0) return prev;
  const byId = new Map<string, SessionTurn>();
  for (const turn of prev) {
    const id = idToString(turn.turn_id);
    if (id) byId.set(id, turn);
  }
  for (const turn of incoming) {
    const id = idToString(turn.turn_id);
    if (!id) continue;
    const existing = byId.get(id);
    byId.set(id, existing ? mergeTurn(existing, turn) : turn);
  }
  return Array.from(byId.values()).sort(compareSessionTurnOrder);
};

export const mergeSessionMessages = (prev: Message[], incoming: Message[]): Message[] => {
  if (incoming.length === 0) return prev;
  const byId = new Map<string, Message>();
  for (const message of prev) {
    const id = idToString(message.id);
    if (id) byId.set(id, message);
  }
  for (const message of incoming) {
    const id = idToString(message.id);
    if (!id) continue;
    byId.set(id, message);
  }
  return Array.from(byId.values()).sort((a, b) => {
    const aOrderSeq = Number(a.order_seq ?? Number.NaN);
    const bOrderSeq = Number(b.order_seq ?? Number.NaN);
    if (Number.isFinite(aOrderSeq) && Number.isFinite(bOrderSeq) && aOrderSeq !== bOrderSeq) {
      return aOrderSeq - bOrderSeq;
    }
    if (Number.isFinite(aOrderSeq) && !Number.isFinite(bOrderSeq)) return -1;
    if (!Number.isFinite(aOrderSeq) && Number.isFinite(bOrderSeq)) return 1;
    const createdAtOrder = String(a.created_at).localeCompare(String(b.created_at));
    if (createdAtOrder !== 0) return createdAtOrder;
    const aTurnSequence = Number(a.turn_sequence ?? Number.NaN);
    const bTurnSequence = Number(b.turn_sequence ?? Number.NaN);
    if (Number.isFinite(aTurnSequence) && Number.isFinite(bTurnSequence) && aTurnSequence !== bTurnSequence) {
      return aTurnSequence - bTurnSequence;
    }
    if (Number.isFinite(aTurnSequence) && !Number.isFinite(bTurnSequence)) return -1;
    if (!Number.isFinite(aTurnSequence) && Number.isFinite(bTurnSequence)) return 1;
    return String(idToString(a.id)).localeCompare(String(idToString(b.id)));
  });
};

export const mergeSessionEvents = (prev: SessionEvent[], incoming: SessionEvent[]): SessionEvent[] => {
  if (incoming.length === 0) return prev;
  const bySeq = new Map<number, SessionEvent>();
  for (const event of prev) {
    if (typeof event.seq === "number") bySeq.set(event.seq, event);
  }
  for (const event of incoming) {
    if (typeof event.seq === "number" && !isPartialEvent(event)) {
      bySeq.set(event.seq, event);
    }
  }
  const next = Array.from(bySeq.values()).sort((a, b) => Number(a.seq ?? 0) - Number(b.seq ?? 0));
  return next.length > HEAD_EVENT_BUFFER_LIMIT ? next.slice(-HEAD_EVENT_BUFFER_LIMIT) : next;
};

export const mergeSessionToolSummaries = (
  prev: SessionTurnToolSummary[],
  incoming: SessionTurnToolSummary[],
  turns: SessionTurn[],
): SessionTurnToolSummary[] => {
  if (incoming.length === 0 && prev.length === 0) return prev;
  const byId = new Map(prev.map((summary) => [String(summary?.tool_call_id ?? ""), summary]));
  for (const summary of incoming) {
    const id = String(summary?.tool_call_id ?? "").trim();
    if (!id) continue;
    byId.set(id, summary);
  }
  const allowedTurnIds = new Set(turns.map((turn) => idToString(turn?.turn_id ?? "")));
  return Array.from(byId.values()).filter((summary) => {
    const toolId = String(summary?.tool_call_id ?? "").trim();
    if (!toolId) return false;
    return allowedTurnIds.has(idToString(summary?.turn_id ?? ""));
  });
};
