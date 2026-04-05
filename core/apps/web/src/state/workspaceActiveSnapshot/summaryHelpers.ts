import type {
  Session,
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  SessionSummary,
  Task,
} from "@ctx/types";
import { idToString } from "../../api/client";
import { sanitizeSessionHeadSnapshot } from "../sessionHeadState";
import { asRecord, readString } from "./projection";

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
    projection_rev: summary.projection_rev ?? undefined,
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
    projection_rev: undefined,
    state_rev: undefined,
    activity: { is_working: false, last_turn_status: null },
    unread: undefined,
  });
}

export function shouldReplaceSessionHead(
  prev: SessionHeadSnapshot | null | undefined,
  next: SessionHeadSnapshot,
): boolean {
  if (!prev) return true;
  const prevSeq = typeof prev.last_event_seq === "number" ? prev.last_event_seq : -1;
  const nextSeq = typeof next.last_event_seq === "number" ? next.last_event_seq : -1;
  if (prevSeq < 0 && nextSeq >= 0) return true;
  if (prevSeq >= 0 && nextSeq < 0) return false;
  if (prevSeq >= 0 && nextSeq >= 0) {
    return nextSeq >= prevSeq;
  }
  const prevProjectionRev = typeof prev.projection_rev === "number" ? prev.projection_rev : -1;
  const nextProjectionRev = typeof next.projection_rev === "number" ? next.projection_rev : -1;
  if (prevProjectionRev >= 0 && nextProjectionRev >= 0 && nextProjectionRev < prevProjectionRev) {
    return false;
  }
  if (prevProjectionRev >= 0 && nextProjectionRev >= 0 && nextProjectionRev > prevProjectionRev) {
    return true;
  }
  return true;
}

export function isSessionHeadCompatibleWithSummary(
  summary: SessionSnapshotSummary | null | undefined,
  head: SessionHeadSnapshot | null | undefined,
): boolean {
  if (!summary || !head) return true;

  const summarySessionId = idToString(summary.session?.id ?? "");
  const headSessionId = idToString(head.session?.id ?? "");
  if (summarySessionId && headSessionId && summarySessionId !== headSessionId) {
    return false;
  }

  const summaryLastEventSeq =
    typeof summary.last_event_seq === "number" && summary.last_event_seq >= 0
      ? summary.last_event_seq
      : null;
  const headLastEventSeq =
    typeof head.last_event_seq === "number" && head.last_event_seq >= 0
      ? head.last_event_seq
      : null;

  const summaryProjectionRev =
    typeof summary.projection_rev === "number" ? summary.projection_rev : null;
  const headProjectionRev =
    typeof head.projection_rev === "number" ? head.projection_rev : null;
  if (
    summaryProjectionRev !== null &&
    headProjectionRev !== null &&
    headProjectionRev < summaryProjectionRev
  ) {
    // Keep a usable head when only the projection cursor advanced but the durable event cursor matches.
    if (summaryLastEventSeq === null || headLastEventSeq === null || headLastEventSeq < summaryLastEventSeq) {
      return false;
    }
  }
  if (
    summaryLastEventSeq !== null &&
    (headLastEventSeq === null || headLastEventSeq < summaryLastEventSeq)
  ) {
    return false;
  }

  return true;
}

export function readPrimarySessionHead(summary: unknown): SessionHeadSnapshot | null {
  if (!summary || typeof summary !== "object") return null;
  const rec = summary as Record<string, unknown>;
  const head = rec.primary_session_head ?? rec.primarySessionHead ?? null;
  if (!head || typeof head !== "object") return null;
  return sanitizeSessionHeadSnapshot(head as SessionHeadSnapshot);
}

export function readPrimarySessionId(summary: unknown): string | null {
  const rec = asRecord(summary);
  if (Object.keys(rec).length === 0) return null;
  const fromPrimary = idToString(readString(asRecord(asRecord(rec.primary_session).session).id) ?? "");
  if (fromPrimary) return fromPrimary;
  const fromHead = idToString(
    readString(asRecord(asRecord(rec.primary_session_head).session).id) ??
      readString(asRecord(asRecord(rec.primarySessionHead).session).id) ??
      "",
  );
  if (fromHead) return fromHead;
  return null;
}
