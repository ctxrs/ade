export type SessionSubscriptionCursor = {
  sessionId: string;
  afterSeq: number | null;
};

const normalizeSessionId = (value: unknown): string => {
  if (typeof value !== "string") return "";
  return value.trim();
};

const normalizeAfterSeq = (value: unknown): number | null => {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
};

export function normalizeSessionSubscriptionCursors(
  sessions: SessionSubscriptionCursor[],
): SessionSubscriptionCursor[] {
  const ordered = new Map<string, SessionSubscriptionCursor>();
  for (const session of sessions) {
    const sessionId = normalizeSessionId(session.sessionId);
    if (!sessionId) continue;
    const afterSeq = normalizeAfterSeq(session.afterSeq);
    const previous = ordered.get(sessionId);
    if (!previous) {
      ordered.set(sessionId, { sessionId, afterSeq });
      continue;
    }
    ordered.set(sessionId, {
      sessionId,
      afterSeq:
        afterSeq === null
          ? previous.afterSeq
          : previous.afterSeq === null
            ? afterSeq
            : Math.max(previous.afterSeq, afterSeq),
    });
  }
  return Array.from(ordered.values());
}

export function sameSessionSubscriptionCursorIds(
  left: SessionSubscriptionCursor[],
  right: SessionSubscriptionCursor[],
): boolean {
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i += 1) {
    if (left[i]?.sessionId !== right[i]?.sessionId) return false;
  }
  return true;
}

export function sameSessionSubscriptionCursors(
  left: SessionSubscriptionCursor[],
  right: SessionSubscriptionCursor[],
): boolean {
  if (left.length !== right.length) return false;
  for (let i = 0; i < left.length; i += 1) {
    if (left[i]?.sessionId !== right[i]?.sessionId) return false;
    if ((left[i]?.afterSeq ?? null) !== (right[i]?.afterSeq ?? null)) return false;
  }
  return true;
}
