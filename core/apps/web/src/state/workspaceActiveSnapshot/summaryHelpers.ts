import type {
  Session,
  SessionSnapshotSummary,
  SessionSummary,
  Task,
} from "@ctx/types";
import { idToString } from "../../api/client";

export function taskSortAt(task: Task, fallback?: string | null): string {
  if (task.archived_at) return task.archived_at;
  if (task.created_at) return task.created_at;
  if (fallback) return fallback;
  return task.updated_at ?? "";
}

export function pickArchivedSessionId(task: Task, sessions: Session[]): string | null {
  const primaryId = idToString(task.primary_session_id ?? "");
  if (primaryId && (sessions.length === 0 || sessions.some((session) => idToString(session.id) === primaryId))) {
    return primaryId;
  }
  const nonSubagents = sessions.filter((session) => session.relationship !== "sub_agent");
  const pool = nonSubagents.length ? nonSubagents : sessions;
  const selected = pool[0];
  return selected ? idToString(selected.id) : null;
}

export function pickArchivedSessionIdFromSummaries(
  task: Task,
  sessions: SessionSummary[],
): string | null {
  const primaryId = idToString(task.primary_session_id ?? "");
  if (primaryId && (sessions.length === 0 || sessions.some((session) => idToString(session.id) === primaryId))) {
    return primaryId;
  }
  const nonSubagents = sessions.filter((session) => session.relationship !== "sub_agent");
  const pool = nonSubagents.length ? nonSubagents : sessions;
  const selected = pool[0];
  return selected ? idToString(selected.id) : null;
}

export function normalizeSessionSummary(summary: SessionSnapshotSummary): SessionSnapshotSummary {
  return {
    session: { ...summary.session },
    last_message_at: summary.last_message_at ?? null,
    last_message_preview: summary.last_message_preview ?? null,
    last_event_seq: summary.last_event_seq ?? null,
    state_rev: summary.state_rev ?? undefined,
    activity: summary.activity ?? { is_working: false, last_turn_status: null },
    unread: summary.unread,
  };
}

export function sessionToSummary(session: Session): SessionSnapshotSummary {
  return normalizeSessionSummary({
    session,
    last_message_at: null,
    last_message_preview: null,
    last_event_seq: null,
    state_rev: undefined,
    activity: { is_working: false, last_turn_status: null },
    unread: undefined,
  });
}
