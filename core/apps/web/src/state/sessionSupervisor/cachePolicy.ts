import type { SessionEvent, SessionTurn } from "../../api/client";
import { isFinalThoughtEvent } from "./thoughtProjection";

const mergePartial = (p: string, n: string): string => {
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
};

export const mergeTurn = (prev: SessionTurn, next: SessionTurn): SessionTurn => {
  const assistant_partial = mergePartial(prev.assistant_partial ?? "", next.assistant_partial ?? "");
  const thought_partial = mergePartial(prev.thought_partial ?? "", next.thought_partial ?? "");
  const status = mergeTurnStatus(prev.status, next.status);
  return {
    ...prev,
    ...next,
    status,
    assistant_partial,
    thought_partial,
    end_seq: next.end_seq ?? prev.end_seq,
    updated_at:
      String(next.updated_at ?? "").localeCompare(String(prev.updated_at ?? "")) >= 0
        ? next.updated_at
        : prev.updated_at,
    tool_total: Math.max(prev.tool_total ?? 0, next.tool_total ?? 0),
    tool_pending: Math.max(prev.tool_pending ?? 0, next.tool_pending ?? 0),
    tool_running: Math.max(prev.tool_running ?? 0, next.tool_running ?? 0),
    tool_completed: Math.max(prev.tool_completed ?? 0, next.tool_completed ?? 0),
    tool_failed: Math.max(prev.tool_failed ?? 0, next.tool_failed ?? 0),
  };
};

const TURN_STATUS_PRIORITY: Record<NonNullable<SessionTurn["status"]>, number> = {
  queued: 0,
  running: 1,
  completed: 2,
  interrupted: 3,
  failed: 4,
};

export const mergeTurnStatus = (
  prev: SessionTurn["status"] | null | undefined,
  next: SessionTurn["status"] | null | undefined,
): SessionTurn["status"] => {
  if (!prev) return next ?? prev ?? "queued";
  if (!next) return prev;
  const prevPriority = TURN_STATUS_PRIORITY[prev] ?? 0;
  const nextPriority = TURN_STATUS_PRIORITY[next] ?? 0;
  return nextPriority >= prevPriority ? next : prev;
};

const PARTIAL_EVENT_TYPES = new Set(["assistant_chunk"]);

export const isPartialEvent = (event: SessionEvent | null | undefined): boolean => {
  if (!event) return false;
  const type = String(event.event_type ?? "");
  if (PARTIAL_EVENT_TYPES.has(type)) return true;
  if (type === "thought_chunk") return !isFinalThoughtEvent(event);
  return false;
};

export const stripTurnPartials = (turns: SessionTurn[]): SessionTurn[] => {
  return turns.map((turn) => {
    const next = {
      ...turn,
      assistant_partial: null,
      thought_partial: null,
    } as SessionTurn & {
      assistant_partial_provider_message_id?: string | null;
      assistant_last_provider_message_id?: string | null;
      thought_partial_provider_item_id?: string | null;
    };
    next.assistant_partial_provider_message_id = null;
    next.assistant_last_provider_message_id = null;
    next.thought_partial_provider_item_id = null;
    return next;
  });
};

export const stripPartialEvents = (events: SessionEvent[]): SessionEvent[] => {
  return events.filter((event) => !isPartialEvent(event));
};

export const appendFragment = (p: string | null | undefined, f: string | null | undefined): string => {
  if (!p) return f || "";
  if (!f) return p;
  if (f.startsWith(p)) return f;
  if (p.endsWith(f)) return p;
  return `${p}${f}`;
};

export const dedupeIds = (ids: string[]): string[] => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of ids) {
    const id = String(raw || "").trim();
    if (!id || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
};

export const mergeOrderedIds = (...groups: string[][]): string[] => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const group of groups) {
    for (const raw of group) {
      const id = String(raw || "").trim();
      if (!id || seen.has(id)) continue;
      seen.add(id);
      out.push(id);
    }
  }
  return out;
};

export const sameIdList = (a: string[], b: string[]): boolean => {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
};
