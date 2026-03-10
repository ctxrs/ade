import { type Message, idToString } from "../../api/client";

export type PendingMessageEntry = {
  clientId: string;
  message: Message;
};

export const shouldDropPendingMessage = (pending: Message, realIds: Set<string>): boolean => {
  const pendingId = idToString(pending.id);
  return Boolean(pendingId && realIds.has(pendingId));
};
