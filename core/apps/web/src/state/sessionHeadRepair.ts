import type { Message, SessionHead, SessionHeadSnapshot, SessionTurn } from "../api/client";

type HeadActivityLike = {
  is_working?: boolean | null;
  last_turn_status?: string | null;
} | null | undefined;

type TranscriptCoverageEntry = {
  turnsHydrated?: boolean;
  turns: readonly Pick<SessionTurn, "turn_id">[];
  messages: readonly Pick<Message, "id">[];
  freshness?: string | null;
  loadState?: string | null;
  lastEventSeq?: number | null;
  projectionRev?: number | null;
  activity?: HeadActivityLike;
};

const readVersion = (value: number | null | undefined): number =>
  typeof value === "number" && Number.isFinite(value) ? value : -1;

export const isBoundedSessionHead = (
  head: Pick<SessionHead | SessionHeadSnapshot, "head_window">,
): boolean =>
  Boolean(head.head_window?.truncated) ||
  [
    head.head_window?.turn_limit,
    head.head_window?.message_limit,
    head.head_window?.event_limit,
    head.head_window?.byte_limit,
  ].some((limit) => typeof limit === "number" && limit > 0);

const coversTurns = (
  entry: TranscriptCoverageEntry,
  head: Pick<SessionHead | SessionHeadSnapshot, "turns">,
): boolean => {
  const headTurns = Array.isArray(head.turns) ? head.turns : [];
  if (headTurns.length === 0) return true;
  const entryTurnIds = new Set(entry.turns.map((turn) => String(turn.turn_id ?? "").trim()).filter(Boolean));
  return headTurns.every((turn) => entryTurnIds.has(String(turn.turn_id ?? "").trim()));
};

const coversMessages = (
  entry: TranscriptCoverageEntry,
  head: Pick<SessionHead | SessionHeadSnapshot, "messages">,
): boolean => {
  const headMessages = Array.isArray(head.messages) ? head.messages : [];
  if (headMessages.length === 0) return true;
  const entryMessageIds = new Set(entry.messages.map((message) => String(message.id ?? "").trim()).filter(Boolean));
  return headMessages.every((message) => entryMessageIds.has(String(message.id ?? "").trim()));
};

export function shouldRepairSessionHeadReplace(
  entry: TranscriptCoverageEntry,
  head: SessionHead | SessionHeadSnapshot,
): boolean {
  const missingTranscript =
    !entry.turnsHydrated &&
    entry.turns.length === 0 &&
    entry.messages.length === 0;
  if (missingTranscript) {
    return false;
  }

  if (isBoundedSessionHead(head)) {
    return true;
  }

  const headLastEventSeq = readVersion(head.last_event_seq);
  const entryLastEventSeq = readVersion(entry.lastEventSeq);
  const headProjectionRev = readVersion(head.projection_rev);
  const entryProjectionRev = readVersion(entry.projectionRev);
  const versionsNotOlder =
    (headLastEventSeq < 0 || entryLastEventSeq < 0 || headLastEventSeq >= entryLastEventSeq) &&
    (headProjectionRev < 0 || entryProjectionRev < 0 || headProjectionRev >= entryProjectionRev);
  const recovering = entry.freshness === "recovering" || entry.loadState === "recovering";

  if (recovering && versionsNotOlder && coversTurns(entry, head) && coversMessages(entry, head)) {
    return true;
  }

  if (!coversTurns(entry, head)) {
    return true;
  }

  if (!coversMessages(entry, head)) {
    return true;
  }

  if (headLastEventSeq > entryLastEventSeq) {
    return true;
  }

  if (headProjectionRev > entryProjectionRev) {
    return true;
  }

  const headWorking = Boolean(head.activity?.is_working);
  const entryWorking = Boolean(entry.activity?.is_working);
  const headStatus = head.activity?.last_turn_status ?? null;
  const entryStatus = entry.activity?.last_turn_status ?? null;
  return headWorking !== entryWorking || headStatus !== entryStatus;
}
