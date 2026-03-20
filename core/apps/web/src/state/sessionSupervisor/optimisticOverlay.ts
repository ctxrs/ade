import { idToString, type Message } from "../../api/client";

type SessionSupervisorOptimisticOverlayEntry = {
  messages: Message[];
  queue: Message[];
  optimisticThreadMessages?: Message[];
  optimisticQueuedMessages?: Message[];
  optimisticQueueRemovalIds?: string[];
  overlayRev?: number;
  updatedAtMs: number;
};

function normalizeMessageId(messageId: string): string {
  return String(messageId || "").trim();
}

function messageIdOf(message: Message): string {
  return normalizeMessageId(idToString(message.id));
}

function upsertById(messages: readonly Message[], message: Message): { next: Message[]; changed: boolean } {
  const messageId = messageIdOf(message);
  if (!messageId) return { next: [...messages, message], changed: true };
  const index = messages.findIndex((entry) => messageIdOf(entry) === messageId);
  if (index < 0) return { next: [...messages, message], changed: true };
  if (messages[index] === message) return { next: [...messages], changed: false };
  const next = messages.slice();
  next[index] = message;
  return { next, changed: true };
}

function removeById(messages: readonly Message[], messageId: string): { next: Message[]; changed: boolean } {
  const normalizedId = normalizeMessageId(messageId);
  if (!normalizedId) return { next: [...messages], changed: false };
  const next = messages.filter((message) => messageIdOf(message) !== normalizedId);
  return { next, changed: next.length !== messages.length };
}

function addHiddenId(ids: readonly string[], messageId: string): { next: string[]; changed: boolean } {
  const normalizedId = normalizeMessageId(messageId);
  if (!normalizedId || ids.includes(normalizedId)) return { next: [...ids], changed: false };
  return { next: [...ids, normalizedId], changed: true };
}

function removeHiddenId(ids: readonly string[], messageId: string): { next: string[]; changed: boolean } {
  const normalizedId = normalizeMessageId(messageId);
  if (!normalizedId) return { next: [...ids], changed: false };
  const next = ids.filter((id) => id !== normalizedId);
  return { next, changed: next.length !== ids.length };
}

function bumpOverlayRev(entry: SessionSupervisorOptimisticOverlayEntry) {
  entry.overlayRev = (entry.overlayRev ?? 0) + 1;
  entry.updatedAtMs = Date.now();
}

export function upsertOptimisticThreadMessage(
  entry: SessionSupervisorOptimisticOverlayEntry,
  message: Message,
): boolean {
  const { next, changed } = upsertById(entry.optimisticThreadMessages ?? [], message);
  if (!changed) return false;
  entry.optimisticThreadMessages = next;
  bumpOverlayRev(entry);
  return true;
}

export function removeOptimisticThreadMessage(
  entry: SessionSupervisorOptimisticOverlayEntry,
  messageId: string,
): boolean {
  const { next, changed } = removeById(entry.optimisticThreadMessages ?? [], messageId);
  if (!changed) return false;
  entry.optimisticThreadMessages = next;
  bumpOverlayRev(entry);
  return true;
}

export function upsertOptimisticQueuedMessage(
  entry: SessionSupervisorOptimisticOverlayEntry,
  message: Message,
): boolean {
  const { next, changed } = upsertById(entry.optimisticQueuedMessages ?? [], message);
  if (!changed) return false;
  entry.optimisticQueuedMessages = next;
  bumpOverlayRev(entry);
  return true;
}

export function removeOptimisticQueuedMessage(
  entry: SessionSupervisorOptimisticOverlayEntry,
  messageId: string,
): boolean {
  const { next, changed } = removeById(entry.optimisticQueuedMessages ?? [], messageId);
  if (!changed) return false;
  entry.optimisticQueuedMessages = next;
  bumpOverlayRev(entry);
  return true;
}

export function addOptimisticQueueRemovalId(
  entry: SessionSupervisorOptimisticOverlayEntry,
  messageId: string,
): boolean {
  const { next, changed } = addHiddenId(entry.optimisticQueueRemovalIds ?? [], messageId);
  if (!changed) return false;
  entry.optimisticQueueRemovalIds = next;
  bumpOverlayRev(entry);
  return true;
}

export function removeOptimisticQueueRemovalId(
  entry: SessionSupervisorOptimisticOverlayEntry,
  messageId: string,
): boolean {
  const { next, changed } = removeHiddenId(entry.optimisticQueueRemovalIds ?? [], messageId);
  if (!changed) return false;
  entry.optimisticQueueRemovalIds = next;
  bumpOverlayRev(entry);
  return true;
}

export function reconcileOptimisticOverlay(
  entry: SessionSupervisorOptimisticOverlayEntry,
): boolean {
  const liveMessageIds = new Set(entry.messages.map((message) => messageIdOf(message)).filter(Boolean));
  const liveQueueIds = new Set(entry.queue.map((message) => messageIdOf(message)).filter(Boolean));

  const nextOptimisticThreadMessages = (entry.optimisticThreadMessages ?? []).filter((message) => {
    const messageId = messageIdOf(message);
    return !messageId || !liveMessageIds.has(messageId);
  });
  const nextOptimisticQueuedMessages = (entry.optimisticQueuedMessages ?? []).filter((message) => {
    const messageId = messageIdOf(message);
    return !messageId || !liveQueueIds.has(messageId);
  });
  const nextOptimisticQueueRemovalIds = (entry.optimisticQueueRemovalIds ?? []).filter(
    (messageId) => liveQueueIds.has(normalizeMessageId(messageId)),
  );

  if (
    nextOptimisticThreadMessages.length === (entry.optimisticThreadMessages ?? []).length &&
    nextOptimisticQueuedMessages.length === (entry.optimisticQueuedMessages ?? []).length &&
    nextOptimisticQueueRemovalIds.length === (entry.optimisticQueueRemovalIds ?? []).length
  ) {
    return false;
  }

  entry.optimisticThreadMessages = nextOptimisticThreadMessages;
  entry.optimisticQueuedMessages = nextOptimisticQueuedMessages;
  entry.optimisticQueueRemovalIds = nextOptimisticQueueRemovalIds;
  bumpOverlayRev(entry);
  return true;
}
