import { idToString, type Message } from "../api/client";

export type AssistantStreamingState = {
  content: string;
  providerMessageId: string | null;
};

export type AssistantStreamingStore = {
  assistantStreamingByTurnId: Record<string, AssistantStreamingState>;
  assistantStreamingRev: number;
};

function appendStreamingFragment(previous: string, fragment: string): string {
  if (!previous) return fragment;
  if (!fragment) return previous;
  if (fragment.startsWith(previous)) return fragment;
  if (previous.endsWith(fragment)) return previous;
  return `${previous}${fragment}`;
}

function normalizeTurnId(turnId: string | null | undefined): string {
  return idToString(turnId ?? "") || "";
}

function updateState(
  store: AssistantStreamingStore,
  turnId: string,
  next: AssistantStreamingState | null,
): boolean {
  const current = store.assistantStreamingByTurnId[turnId] ?? null;
  if (next === null) {
    if (!current) return false;
    const { [turnId]: _removed, ...rest } = store.assistantStreamingByTurnId;
    store.assistantStreamingByTurnId = rest;
    store.assistantStreamingRev += 1;
    return true;
  }
  if (
    current &&
    current.content === next.content &&
    current.providerMessageId === next.providerMessageId
  ) {
    return false;
  }
  store.assistantStreamingByTurnId = {
    ...store.assistantStreamingByTurnId,
    [turnId]: next,
  };
  store.assistantStreamingRev += 1;
  return true;
}

export function clearAllAssistantStreaming(store: AssistantStreamingStore): boolean {
  if (Object.keys(store.assistantStreamingByTurnId).length === 0) return false;
  store.assistantStreamingByTurnId = {};
  store.assistantStreamingRev += 1;
  return true;
}

export function clearAssistantStreaming(
  store: AssistantStreamingStore,
  turnId: string | null | undefined,
): boolean {
  const normalizedTurnId = normalizeTurnId(turnId);
  if (!normalizedTurnId) return false;
  return updateState(store, normalizedTurnId, null);
}

export function applyAssistantChunkToStreaming(
  store: AssistantStreamingStore,
  turnId: string | null | undefined,
  fragment: string,
  providerMessageId?: string | null,
): boolean {
  const normalizedTurnId = normalizeTurnId(turnId);
  if (!normalizedTurnId || !fragment) return false;
  const current = store.assistantStreamingByTurnId[normalizedTurnId] ?? null;
  const providerId = providerMessageId?.trim() || null;
  const nextContent =
    providerId && current?.providerMessageId && providerId !== current.providerMessageId
      ? fragment
      : appendStreamingFragment(current?.content ?? "", fragment);
  return updateState(store, normalizedTurnId, {
    content: nextContent,
    providerMessageId: providerId ?? current?.providerMessageId ?? null,
  });
}

export function applyAssistantCompleteToStreaming(
  store: AssistantStreamingStore,
  turnId: string | null | undefined,
  fullContent: string,
  providerMessageId?: string | null,
): boolean {
  const normalizedTurnId = normalizeTurnId(turnId);
  if (!normalizedTurnId) return false;
  const current = store.assistantStreamingByTurnId[normalizedTurnId] ?? null;
  const nextContent = String(fullContent || current?.content || "");
  if (!nextContent) return false;
  return updateState(store, normalizedTurnId, {
    content: nextContent,
    providerMessageId: providerMessageId?.trim() || current?.providerMessageId || null,
  });
}

export function reconcileAssistantStreamingWithMessages(
  store: AssistantStreamingStore,
  incoming: Message[],
): boolean {
  let changed = false;
  for (const message of incoming) {
    if (message.role !== "assistant") continue;
    const normalizedTurnId = normalizeTurnId(message.turn_id);
    if (!normalizedTurnId) continue;
    const current = store.assistantStreamingByTurnId[normalizedTurnId];
    if (!current) continue;
    const pendingTrimmed = current.content.trim();
    const messageTrimmed = String(message.content ?? "").trim();
    if (!pendingTrimmed || pendingTrimmed !== messageTrimmed) continue;
    if (clearAssistantStreaming(store, normalizedTurnId)) {
      changed = true;
    }
  }
  return changed;
}
