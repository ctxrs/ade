export type SessionSubscriptionCursor = {
  sessionId: string;
  replay: SessionSubscriptionReplay;
};

export type SessionSubscriptionReplay =
  | {
      kind: "auto";
    }
  | {
      kind: "reset";
    }
  | {
      kind: "resume";
      afterSeq: number;
    };

const normalizeSessionId = (value: unknown): string => {
  if (typeof value !== "string") return "";
  return value.trim();
};

const AUTO_REPLAY: SessionSubscriptionReplay = { kind: "auto" };
const RESET_REPLAY: SessionSubscriptionReplay = { kind: "reset" };

const normalizeReplay = (
  value: SessionSubscriptionReplay | null | undefined,
): SessionSubscriptionReplay => {
  switch (value?.kind) {
    case "reset":
      return RESET_REPLAY;
    case "resume":
      return typeof value.afterSeq === "number" && Number.isFinite(value.afterSeq)
        ? { kind: "resume", afterSeq: value.afterSeq }
        : AUTO_REPLAY;
    default:
      return AUTO_REPLAY;
  }
};

const mergeReplay = (
  left: SessionSubscriptionReplay,
  right: SessionSubscriptionReplay,
): SessionSubscriptionReplay => {
  if (left.kind === "reset" || right.kind === "reset") {
    return RESET_REPLAY;
  }
  if (left.kind === "resume" && right.kind === "resume") {
    return { kind: "resume", afterSeq: Math.max(left.afterSeq, right.afterSeq) };
  }
  if (left.kind === "resume") {
    return left;
  }
  if (right.kind === "resume") {
    return right;
  }
  return AUTO_REPLAY;
};

const sameReplay = (
  left: SessionSubscriptionReplay | null | undefined,
  right: SessionSubscriptionReplay | null | undefined,
): boolean => {
  if (left?.kind !== right?.kind) return false;
  if (left?.kind === "resume" && right?.kind === "resume") {
    return left.afterSeq === right.afterSeq;
  }
  return true;
};

export function normalizeSessionSubscriptionCursors(
  sessions: SessionSubscriptionCursor[],
): SessionSubscriptionCursor[] {
  const ordered = new Map<string, SessionSubscriptionCursor>();
  for (const session of sessions) {
    const sessionId = normalizeSessionId(session.sessionId);
    if (!sessionId) continue;
    const replay = normalizeReplay(session.replay);
    const previous = ordered.get(sessionId);
    if (!previous) {
      ordered.set(sessionId, { sessionId, replay });
      continue;
    }
    ordered.set(sessionId, {
      sessionId,
      replay: mergeReplay(previous.replay, replay),
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
    if (!sameReplay(left[i]?.replay, right[i]?.replay)) return false;
  }
  return true;
}
