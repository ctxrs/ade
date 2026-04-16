import type { SessionEvent, SessionTurn } from "../../api/client";
import type { SessionActivityState } from "@ctx/types";
import { isFinalThoughtEvent } from "./thoughtProjection";

const mergePartial = (p: string, n: string): string => {
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
};

export const mergeTurn = (prev: SessionTurn, next: SessionTurn): SessionTurn => {
  const thought_partial = mergePartial(prev.thought_partial ?? "", next.thought_partial ?? "");
  const status = mergeTurnStatus(prev.status, next.status);
  return {
    ...prev,
    ...next,
    status,
    assistant_partial: null,
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
  if (prev === "failed" && next === "interrupted") {
    return "interrupted";
  }
  if (prev === "interrupted" && (next === "completed" || next === "failed")) {
    return "interrupted";
  }
  const prevPriority = TURN_STATUS_PRIORITY[prev] ?? 0;
  const nextPriority = TURN_STATUS_PRIORITY[next] ?? 0;
  return nextPriority >= prevPriority ? next : prev;
};

export const reconcileLatestTurnInterruptedFromActivity = (
  turns: SessionTurn[],
  activity: SessionActivityState | null | undefined,
): boolean => {
  if ((activity?.last_turn_status ?? null) !== "interrupted") return false;
  const latestTurn = turns.at(-1);
  if (!latestTurn) return false;
  const nextStatus = mergeTurnStatus(latestTurn.status, "interrupted");
  if (nextStatus === latestTurn.status) return false;
  turns[turns.length - 1] = { ...latestTurn, status: nextStatus };
  return true;
};

export const reconcileActivityInterruptedFromTurns = (
  activity: SessionActivityState | null | undefined,
  turns: SessionTurn[],
): SessionActivityState | null => {
  const nextActivity = activity ?? null;
  if (!nextActivity) return null;
  const latestTurn = turns.at(-1);
  if (!latestTurn || latestTurn.status !== "interrupted") return nextActivity;
  if (nextActivity.last_turn_status === "interrupted") return nextActivity;
  if (nextActivity.last_turn_status !== "completed" && nextActivity.last_turn_status !== "failed") {
    return nextActivity;
  }
  return {
    ...nextActivity,
    is_working: false,
    last_turn_status: "interrupted",
  };
};

const PARTIAL_EVENT_TYPES = new Set(["assistant_chunk", "assistant_complete", "context_window_update"]);

export const isPartialEvent = (event: SessionEvent | null | undefined): boolean => {
  if (!event) return false;
  const type = String(event.event_type ?? "");
  if (PARTIAL_EVENT_TYPES.has(type)) return true;
  if (type === "thought_chunk") return !isFinalThoughtEvent(event);
  return false;
};

export const stripTurnPartials = (turns: SessionTurn[]): SessionTurn[] => {
  return turns.map((turn) => {
    return {
      ...turn,
      assistant_partial: null,
      thought_partial: null,
    };
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
