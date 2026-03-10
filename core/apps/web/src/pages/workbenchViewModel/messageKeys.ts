import type { Message, SessionTurn } from "../../api/client";

function serializeForKey(value: unknown): string {
  try {
    return JSON.stringify(value, (_key, inner) => (inner === undefined ? "__undefined__" : inner)) ?? "null";
  } catch {
    return String(value);
  }
}

function deriveCollectionKey(values: unknown[]): string {
  if (values.length === 0) return "0";
  let hash = 5381;
  for (const value of values) {
    const serialized = serializeForKey(value);
    for (let i = 0; i < serialized.length; i += 1) {
      hash = ((hash << 5) + hash) ^ serialized.charCodeAt(i);
    }
  }
  return `${values.length}:${(hash >>> 0).toString(36)}`;
}

export function deriveMessagesKey(messages: Message[]): string {
  return deriveCollectionKey(messages);
}

export function deriveTurnsKey(turns: SessionTurn[]): string {
  return deriveCollectionKey(turns);
}
