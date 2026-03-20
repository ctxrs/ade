import { getSessionHistory, listTurnTools } from "../../api/client";
import type { WorkspaceOwnerScope } from "../scopeIdentity";
import { loadSessionHistoryPageV1, saveSessionHistoryPageV1 } from "../uiStateStore";
import type { InternalEntry } from "./entryState";
import { summarizeToolPayload } from "./toolStateProjection";

type LoadMoreTurnsContext = {
  sessionId: string;
  entry: InternalEntry;
  turnPageLimit: number;
  resolveEntryWorkspaceOwnerScope: (entry: InternalEntry) => WorkspaceOwnerScope | null;
  mergeTurns: (entry: InternalEntry, turns: typeof entry.turns) => void;
  mergeMessages: (entry: InternalEntry, messages: typeof entry.messages) => void;
  publish: () => void;
  persistHead: (entry: InternalEntry) => Promise<void>;
};

export async function loadMoreTurnsForEntry({
  sessionId,
  entry,
  turnPageLimit,
  resolveEntryWorkspaceOwnerScope,
  mergeTurns,
  mergeMessages,
  publish,
  persistHead,
}: LoadMoreTurnsContext): Promise<number | null> {
  if (entry.fetching.history) return null;
  if (!entry.hasMoreTurns) return 0;
  const beforeSeq = entry.oldestTurnSeq;
  if (beforeSeq == null || !Number.isFinite(beforeSeq)) {
    return 0;
  }
  entry.fetching.history = true;
  const beforeLen = entry.turns.length;
  try {
    const ownerScope = resolveEntryWorkspaceOwnerScope(entry);
    const cached = ownerScope
      ? await loadSessionHistoryPageV1(ownerScope, sessionId, beforeSeq, turnPageLimit)
      : null;
    if (cached?.page) {
      const page = cached.page;
      mergeTurns(entry, page.turns);
      mergeMessages(entry, page.messages);
      entry.hasMoreTurns = page.has_more;
      entry.oldestTurnSeq = page.next_cursor ?? entry.oldestTurnSeq;
      entry.historyExtended = true;
      entry.updatedAtMs = Date.now();
      publish();
      await persistHead(entry);
      return entry.turns.length - beforeLen;
    }
    const page = await getSessionHistory(sessionId, beforeSeq, turnPageLimit);
    mergeTurns(entry, page.turns);
    mergeMessages(entry, page.messages);
    entry.hasMoreTurns = page.has_more;
    entry.oldestTurnSeq = page.next_cursor ?? entry.oldestTurnSeq;
    entry.historyExtended = true;
    entry.updatedAtMs = Date.now();
    publish();
    if (ownerScope) {
      await saveSessionHistoryPageV1(ownerScope, sessionId, beforeSeq, turnPageLimit, page);
    }
    await persistHead(entry);
    return entry.turns.length - beforeLen;
  } finally {
    entry.fetching.history = false;
  }
}

type LoadTurnToolsContext = {
  sessionId: string;
  turnId: string;
  entry: InternalEntry;
  publish: () => void;
};

export async function loadTurnToolsForEntry({
  sessionId,
  turnId,
  entry,
  publish,
}: LoadTurnToolsContext): Promise<void> {
  if (entry.turnToolsHydratedByTurnId?.[turnId]) return;
  if (entry.turnToolsLoadingSet.has(turnId)) return;
  entry.turnToolsLoadingSet.add(turnId);
  entry.turnToolsLoading = [...entry.turnToolsLoadingSet];
  publish();
  try {
    const tools = await listTurnTools(sessionId, turnId);
    const summarized = tools.map(summarizeToolPayload);
    entry.turnToolsByTurnId = {
      ...entry.turnToolsByTurnId,
      [turnId]: summarized,
    };
    entry.turnToolsHydratedByTurnId[turnId] = true;
  } finally {
    entry.turnToolsLoadingSet.delete(turnId);
    entry.turnToolsLoading = [...entry.turnToolsLoadingSet];
    entry.updatedAtMs = Date.now();
    publish();
  }
}
