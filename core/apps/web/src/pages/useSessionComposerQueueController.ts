import { useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import {
  deleteMessage,
  type Message,
  type MessageAttachment,
  postMessage,
  type Session,
  type SessionTurn,
  idToString,
  interruptSession,
} from "../api/client";
import { randomUuid } from "../utils/randomUuid";
import { errorMessage } from "../utils/errorMessage";
import {
  filterQueuedMessagesForPanel,
  mergeMessagesForView,
  mergeQueuedMessagesForPanel,
} from "./SessionPage.workbenchViewModel";
import { buildOptimisticUserMessage } from "./SessionPage.optimisticMessage";
import { getQueuedAttachments } from "./sessionView/SessionQueuePanel";

export type PendingMessageEntry = {
  clientId: string;
  message: Message;
};

export const shouldDropPendingMessage = (pending: Message, realIds: Set<string>): boolean => {
  const pendingId = idToString(pending.id);
  return Boolean(pendingId && realIds.has(pendingId));
};

type Params = {
  sessionId: string;
  session: Session | null;
  input: string;
  setInput: (next: string) => void;
  draftAttachments: MessageAttachment[];
  setDraftAttachments: (next: SetStateAction<MessageAttachment[]>) => void;
  messages: Message[];
  messagesKey: string;
  queue: Message[];
  turns: SessionTurn[];
  turnsKey: string;
  hasActiveTurn: boolean;
  queuedMessagesEnabled: boolean;
  sessionIsAuthoritative: boolean;
  resolveSendText: () => Promise<string>;
  setAtBottom: Dispatch<SetStateAction<boolean>>;
  onDraftPersistNow?: (() => void | Promise<void>) | null;
};

type Result = {
  sendBusy: boolean;
  sendError: string | null;
  queueActionBusy: boolean;
  queueForPanel: Message[];
  displayMessages: Message[];
  pendingQueueMessageIdSet: Set<string>;
  queuedMessageIdsForThread: Set<string>;
  sendNow: () => Promise<void>;
  onRemoveQueued: (messageId: string) => Promise<void>;
  onEditQueued: (message: Message) => Promise<void>;
  onSendQueuedNow: (message: Message) => Promise<void>;
};

type PendingSessionHandoff = {
  fromSessionId: string;
  messageIds: string[];
};

const shouldKeepQueueRemovalOnError = (error: unknown) => {
  const msg = errorMessage(error);
  return msg.startsWith("400") || msg.startsWith("404");
};

const isTurnAlreadyRunningSendError = (value: unknown): boolean => {
  const message = errorMessage(value).trim().toLowerCase();
  if (!message) return false;
  return (
    message.includes("a turn is already running")
    || message.includes("turn is already running")
    || message.includes("stop it or wait for it to finish")
  );
};

export function shouldCarryPendingMessagesAcrossSessionChange({
  previousSessionId,
  nextSessionId,
  handoff,
  pendingMessages,
  messageCount,
  turnCount,
}: {
  previousSessionId: string;
  nextSessionId: string;
  handoff: PendingSessionHandoff | null;
  pendingMessages: PendingMessageEntry[];
  messageCount: number;
  turnCount: number;
}): boolean {
  if (!previousSessionId || previousSessionId === nextSessionId) return false;
  if (!handoff || handoff.fromSessionId !== previousSessionId) return false;
  if (messageCount > 0 || turnCount > 0) return false;
  const pendingIds = new Set(pendingMessages.map((entry) => entry.clientId));
  return handoff.messageIds.some((messageId) => pendingIds.has(messageId));
}

export function reassignPendingMessagesToSession(
  pendingMessages: PendingMessageEntry[],
  nextSessionId: string,
  handoff: PendingSessionHandoff,
): PendingMessageEntry[] {
  const handoffIds = new Set(handoff.messageIds);
  return pendingMessages
    .filter((entry) => handoffIds.has(entry.clientId))
    .map((entry) => ({
      ...entry,
      message: {
        ...entry.message,
        session_id: nextSessionId,
      },
    }));
}

export function useSessionComposerQueueController(params: Params): Result {
  const {
    sessionId,
    session,
    input,
    setInput,
    draftAttachments,
    setDraftAttachments,
    messages,
    messagesKey,
    queue,
    turns,
    turnsKey,
    hasActiveTurn,
    queuedMessagesEnabled,
    sessionIsAuthoritative,
    resolveSendText,
    setAtBottom,
    onDraftPersistNow,
  } = params;
  const [sendBusy, setSendBusy] = useState(false);
  const sendBusyRef = useRef(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [queueActionBusyId, setQueueActionBusyId] = useState<string | null>(null);
  const [pendingMessages, setPendingMessages] = useState<PendingMessageEntry[]>([]);
  const [pendingQueueMessages, setPendingQueueMessages] = useState<PendingMessageEntry[]>([]);
  const [optimisticQueueRemovalIds, setOptimisticQueueRemovalIds] = useState<string[]>([]);
  const previousSessionIdRef = useRef(sessionId);
  const pendingSessionHandoffRef = useRef<PendingSessionHandoff | null>(null);
  const pendingMessagesRef = useRef(pendingMessages);
  const messageCountRef = useRef(messages.length);
  const turnCountRef = useRef(turns.length);

  useEffect(() => {
    pendingMessagesRef.current = pendingMessages;
  }, [pendingMessages]);

  useEffect(() => {
    messageCountRef.current = messages.length;
  }, [messages.length]);

  useEffect(() => {
    turnCountRef.current = turns.length;
  }, [turns.length]);

  useEffect(() => {
    const previousSessionId = previousSessionIdRef.current;
    previousSessionIdRef.current = sessionId;
    const handoff = pendingSessionHandoffRef.current;
    const shouldCarryPendingMessages = shouldCarryPendingMessagesAcrossSessionChange({
      previousSessionId,
      nextSessionId: sessionId,
      handoff,
      pendingMessages: pendingMessagesRef.current,
      messageCount: messageCountRef.current,
      turnCount: turnCountRef.current,
    });
    if (shouldCarryPendingMessages && handoff) {
      setPendingMessages((prev) => reassignPendingMessagesToSession(prev, sessionId, handoff));
      pendingSessionHandoffRef.current = {
        fromSessionId: sessionId,
        messageIds: handoff.messageIds,
      };
    } else {
      setPendingMessages([]);
      pendingSessionHandoffRef.current = null;
    }
    setPendingQueueMessages([]);
    setOptimisticQueueRemovalIds([]);
    setSendBusy(false);
    sendBusyRef.current = false;
    setSendError(null);
    setQueueActionBusyId(null);
  }, [sessionId]);

  const setSendBusySafe = (next: boolean) => {
    sendBusyRef.current = next;
    setSendBusy(next);
  };

  const optimisticQueueRemovalSet = useMemo(
    () => new Set(optimisticQueueRemovalIds),
    [optimisticQueueRemovalIds],
  );
  const markQueueOptimisticallyRemoved = (messageId: string) => {
    if (!messageId) return;
    setOptimisticQueueRemovalIds((prev) => (prev.includes(messageId) ? prev : [...prev, messageId]));
  };
  const rollbackOptimisticQueueRemoval = (messageId: string) => {
    if (!messageId) return;
    setOptimisticQueueRemovalIds((prev) => prev.filter((id) => id !== messageId));
  };
  const mergedQueueForPanel = useMemo(
    () => mergeQueuedMessagesForPanel(queue, pendingQueueMessages.map((entry) => entry.message)),
    [queue, pendingQueueMessages],
  );
  const queueForPanel = useMemo(() => {
    const filtered = filterQueuedMessagesForPanel(mergedQueueForPanel, turns);
    if (optimisticQueueRemovalIds.length === 0) return filtered;
    return filtered.filter((message) => {
      const mid = idToString(message.id);
      return !mid || !optimisticQueueRemovalSet.has(mid);
    });
  }, [mergedQueueForPanel, optimisticQueueRemovalIds.length, optimisticQueueRemovalSet, turns, turnsKey]);
  const pendingQueueMessageIdSet = useMemo(() => {
    return new Set(
      pendingQueueMessages
        .map((entry) => idToString(entry.message.id))
        .filter((messageId): messageId is string => !!messageId),
    );
  }, [pendingQueueMessages]);
  const queuedMessageIdsForThread = useMemo(() => {
    const ids = new Set<string>();
    for (const message of queueForPanel) {
      const mid = idToString(message.id);
      if (mid) ids.add(mid);
    }
    if (optimisticQueueRemovalIds.length > 0) {
      for (const mid of optimisticQueueRemovalIds) {
        ids.add(mid);
      }
    }
    return ids;
  }, [optimisticQueueRemovalIds, queueForPanel]);
  const turnStatusByUserMessageId = useMemo(() => {
    const map = new Map<string, string>();
    for (const turn of turns) {
      const mid = turn.user_message_id ? idToString(turn.user_message_id) : "";
      if (!mid) continue;
      map.set(mid, String(turn.status));
    }
    return map;
  }, [turns, turnsKey]);
  const queuedMessageIdsToShow = useMemo(() => {
    const ids = new Set<string>();
    for (const message of messages) {
      if (message.delivery !== "queued") continue;
      const mid = idToString(message.id);
      if (!mid) continue;
      const status = turnStatusByUserMessageId.get(mid);
      if (status && status !== "queued") {
        ids.add(mid);
      }
    }
    return ids;
  }, [messages, messagesKey, turnStatusByUserMessageId]);
  const displayMessages = useMemo(
    () => mergeMessagesForView(messages, pendingMessages.map((entry) => entry.message), queuedMessageIdsToShow),
    [messages, messagesKey, pendingMessages, queuedMessageIdsToShow],
  );

  useEffect(() => {
    if (optimisticQueueRemovalIds.length === 0) return;
    const liveIds = new Set(
      mergedQueueForPanel.map((message) => idToString(message.id)).filter((id): id is string => !!id),
    );
    setOptimisticQueueRemovalIds((prev) => {
      const next = prev.filter((id) => liveIds.has(id));
      return next.length === prev.length ? prev : next;
    });
  }, [mergedQueueForPanel, optimisticQueueRemovalIds.length]);

  useEffect(() => {
    if (pendingMessages.length === 0) return;
    const realIds = new Set(messages.map((message) => idToString(message.id)));
    setPendingMessages((prev) => {
      if (prev.length === 0) return prev;
      const next = prev.filter((entry) => !shouldDropPendingMessage(entry.message, realIds));
      return next.length === prev.length ? prev : next;
    });
  }, [messages, messagesKey, pendingMessages.length]);

  useEffect(() => {
    const handoff = pendingSessionHandoffRef.current;
    if (!handoff) return;
    const pendingIds = new Set(pendingMessages.map((entry) => entry.clientId));
    if (handoff.messageIds.some((messageId) => pendingIds.has(messageId))) return;
    pendingSessionHandoffRef.current = null;
  }, [pendingMessages]);

  useEffect(() => {
    if (pendingQueueMessages.length === 0) return;
    const realIds = new Set(queue.map((message) => idToString(message.id)));
    setPendingQueueMessages((prev) => {
      if (prev.length === 0) return prev;
      const next = prev.filter((entry) => !shouldDropPendingMessage(entry.message, realIds));
      return next.length === prev.length ? prev : next;
    });
  }, [pendingQueueMessages.length, queue]);

  const queueActionBusy = queueActionBusyId !== null;

  const sendNow = async () => {
    if (!sessionId) return;
    if (sendBusyRef.current) return;
    if (hasActiveTurn && !queuedMessagesEnabled && sessionIsAuthoritative) {
      setSendError("A turn is already running. Stop it or wait for it to finish.");
      return;
    }
    setSendBusySafe(true);
    let text = "";
    try {
      text = await resolveSendText();
    } catch (error: unknown) {
      setSendError(errorMessage(error));
      setSendBusySafe(false);
      return;
    }
    if (!text) {
      setSendBusySafe(false);
      return;
    }
    const attachmentsToSend = draftAttachments.slice();
    const shouldQueue = hasActiveTurn && queuedMessagesEnabled && sessionIsAuthoritative;
    const requestedDelivery = shouldQueue ? "queued" : undefined;
    const messageId = randomUuid();
    const turnId = randomUuid();
    const optimisticMessage: Message = buildOptimisticUserMessage({
      messageId,
      sessionId,
      taskId: String(session?.task_id ?? ""),
      turnId,
      content: text,
      attachments: attachmentsToSend,
      delivery: shouldQueue ? "queued" : "immediate",
    });
    setSendError(null);
    if (shouldQueue) {
      setPendingQueueMessages((prev) => [...prev, { clientId: messageId, message: optimisticMessage }]);
      pendingSessionHandoffRef.current = null;
    } else {
      setPendingMessages((prev) => [...prev, { clientId: messageId, message: optimisticMessage }]);
      pendingSessionHandoffRef.current =
        messages.length === 0 && turns.length === 0
          ? {
              fromSessionId: sessionId,
              messageIds: [messageId],
            }
          : null;
    }
    setAtBottom(true);
    setInput("");
    setDraftAttachments([]);
    try {
      const posted = await postMessage(sessionId, text, requestedDelivery, attachmentsToSend, {
        id: messageId,
        turn_id: turnId,
      });
      if (shouldQueue) {
        setPendingQueueMessages((prev) =>
          prev.map((entry) => (entry.clientId === messageId ? { ...entry, message: posted } : entry)),
        );
      } else {
        setPendingMessages((prev) =>
          prev.map((entry) => (entry.clientId === messageId ? { ...entry, message: posted } : entry)),
        );
      }
      try {
        await onDraftPersistNow?.();
      } catch {
        // best-effort
      }
    } catch (error: unknown) {
      if (shouldQueue) {
        setPendingQueueMessages((prev) => prev.filter((entry) => entry.clientId !== messageId));
      } else {
        setPendingMessages((prev) => prev.filter((entry) => entry.clientId !== messageId));
        const handoff = pendingSessionHandoffRef.current;
        if (handoff?.messageIds.includes(messageId)) {
          pendingSessionHandoffRef.current = null;
        }
      }
      if (isTurnAlreadyRunningSendError(error)) {
        return;
      }
      setInput(text);
      setDraftAttachments(attachmentsToSend);
      setSendError(errorMessage(error));
    } finally {
      setSendBusySafe(false);
    }
  };

  const onRemoveQueued = async (messageId: string) => {
    if (!sessionId) return;
    if (!messageId) return;
    if (queueActionBusy) return;
    markQueueOptimisticallyRemoved(messageId);
    setQueueActionBusyId(messageId);
    setSendError(null);
    try {
      await deleteMessage(sessionId, messageId);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== messageId),
      );
    } catch (error: unknown) {
      if (!shouldKeepQueueRemovalOnError(error)) {
        rollbackOptimisticQueueRemoval(messageId);
      }
      setSendError(errorMessage(error));
    } finally {
      setQueueActionBusyId(null);
    }
  };

  const onEditQueued = async (message: Message) => {
    if (!sessionId) return;
    if (queueActionBusy) return;
    const messageId = idToString(message.id);
    if (!messageId) return;
    const attachments = getQueuedAttachments(message);
    setInput(message.content ?? "");
    setDraftAttachments(attachments);
    markQueueOptimisticallyRemoved(messageId);
    setQueueActionBusyId(messageId);
    setSendError(null);
    try {
      await deleteMessage(sessionId, messageId);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== messageId),
      );
    } catch (error: unknown) {
      if (!shouldKeepQueueRemovalOnError(error)) {
        rollbackOptimisticQueueRemoval(messageId);
      }
      setSendError(errorMessage(error));
    } finally {
      setQueueActionBusyId(null);
    }
  };

  const onSendQueuedNow = async (message: Message) => {
    if (!sessionId) return;
    if (queueActionBusy || sendBusyRef.current) return;
    const messageId = idToString(message.id);
    if (!messageId) return;
    const attachments = getQueuedAttachments(message);
    const content = message.content ?? "";
    markQueueOptimisticallyRemoved(messageId);
    setQueueActionBusyId(messageId);
    setSendError(null);
    pendingSessionHandoffRef.current = null;
    try {
      await interruptSession(sessionId);
    } catch (error: unknown) {
      rollbackOptimisticQueueRemoval(messageId);
      setSendError(errorMessage(error));
      setQueueActionBusyId(null);
      return;
    }
    try {
      await deleteMessage(sessionId, messageId);
      setPendingQueueMessages((prev) =>
        prev.filter((entry) => idToString(entry.message.id) !== messageId),
      );
    } catch (error: unknown) {
      if (!shouldKeepQueueRemovalOnError(error)) {
        rollbackOptimisticQueueRemoval(messageId);
      }
      setSendError(errorMessage(error));
      setQueueActionBusyId(null);
      return;
    }
    setSendBusySafe(true);

    const optimisticMessageId = randomUuid();
    const turnId = randomUuid();
    const optimisticMessage: Message = buildOptimisticUserMessage({
      messageId: optimisticMessageId,
      sessionId,
      taskId: String(session?.task_id ?? ""),
      turnId,
      content,
      attachments,
      delivery: "immediate",
    });
    setPendingMessages((prev) => [...prev, { clientId: optimisticMessageId, message: optimisticMessage }]);
    setAtBottom(true);

    try {
      const posted = await postMessage(sessionId, content, "immediate", attachments, {
        id: optimisticMessageId,
        turn_id: turnId,
      });
      setPendingMessages((prev) =>
        prev.map((entry) => (entry.clientId === optimisticMessageId ? { ...entry, message: posted } : entry)),
      );
    } catch (error: unknown) {
      setPendingMessages((prev) => prev.filter((entry) => entry.clientId !== optimisticMessageId));
      setSendError(errorMessage(error));
    } finally {
      setSendBusySafe(false);
      setQueueActionBusyId(null);
    }
  };

  return {
    sendBusy,
    sendError,
    queueActionBusy,
    queueForPanel,
    displayMessages,
    pendingQueueMessageIdSet,
    queuedMessageIdsForThread,
    sendNow,
    onRemoveQueued,
    onEditQueued,
    onSendQueuedNow,
  };
}
