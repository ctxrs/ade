import type { SessionHeadDelta, SessionHeadSnapshot } from "@ctx/types";
import { idToString } from "../../api/client";
import {
  mergeSessionEvents,
  mergeSessionMessages,
  mergeSessionToolSummaries,
  mergeSessionTurns,
  sanitizeSessionHeadSnapshot,
} from "../sessionHeadState";
import { createSeedHeadSnapshot } from "./sessionHeadSeed";
import type { WorkspaceActiveSnapshotItem } from "./storeTypes";
import { shouldReplaceSessionHead } from "./summaryHelpers";

export function applySessionHeadDeltaToSnapshot(params: {
  delta: SessionHeadDelta;
  tasks: Map<string, WorkspaceActiveSnapshotItem>;
  sessionHeadsById: Map<string, SessionHeadSnapshot>;
}): boolean {
  const { delta, tasks, sessionHeadsById } = params;
  const sessionId = idToString(delta?.session_id ?? "");
  if (!sessionId) return false;
  let existing = sessionHeadsById.get(sessionId);
  if (!existing) {
    const seeded = createSeedHeadSnapshot(tasks, sessionId);
    if (!seeded) return false;
    existing = seeded;
    sessionHeadsById.set(sessionId, seeded);
  }
  let changed = false;
  let turns = existing.turns ?? [];
  let toolSummaries = Array.isArray(existing.tool_summaries) ? existing.tool_summaries : [];
  let messages = existing.messages ?? [];
  let events = existing.events ?? [];
  if (delta.turn) {
    turns = mergeSessionTurns(turns, [delta.turn]);
    changed = true;
  }
  if (delta.message) {
    messages = mergeSessionMessages(messages, [delta.message]);
    changed = true;
  }
  if (delta.event) {
    events = mergeSessionEvents(events, [delta.event]);
    changed = true;
  }
  const incomingToolSummaries = Array.isArray(delta.tool_summaries) ? delta.tool_summaries : [];
  if (incomingToolSummaries.length > 0) {
    toolSummaries = mergeSessionToolSummaries(toolSummaries, incomingToolSummaries, turns);
    changed = true;
  }
  const next: SessionHeadSnapshot = sanitizeSessionHeadSnapshot({
    ...existing,
    turns,
    tool_summaries: toolSummaries,
    messages,
    events,
    ...(delta.session ? { session: delta.session } : {}),
    ...("activity" in delta ? { activity: delta.activity ?? undefined } : {}),
    ...(typeof delta.last_event_seq === "number" ? { last_event_seq: delta.last_event_seq } : {}),
    ...(typeof delta.projection_rev === "number" ? { projection_rev: delta.projection_rev } : {}),
    ...(typeof delta.state_rev === "number" ? { state_rev: delta.state_rev } : {}),
  });

  if (
    !changed &&
    next.last_event_seq === existing.last_event_seq &&
    (next.projection_rev ?? 0) === (existing.projection_rev ?? 0)
  ) {
    return false;
  }
  if (!shouldReplaceSessionHead(existing, next)) return false;
  sessionHeadsById.set(sessionId, next);
  return true;
}
