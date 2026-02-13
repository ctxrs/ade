import type { SessionEvent } from "../../api/client";
import { idToString } from "../../api/client";
import { pickFirstString } from "./eventNormalization";

export const readThoughtFullContent = (payload: any): string | null => {
  return pickFirstString(
    payload?.full_content,
    payload?.fullContent,
    payload?.full,
    payload?.content,
    payload?.content_fragment,
    payload?.contentFragment,
  );
};

export const isFinalThoughtPayload = (payload: any): boolean => {
  if (!payload || typeof payload !== "object") return false;
  return (
    payload?.is_final === true ||
    payload?.isFinal === true ||
    typeof payload?.full_content === "string" ||
    typeof payload?.fullContent === "string"
  );
};

export const isFinalThoughtEvent = (event: SessionEvent | null | undefined): boolean => {
  if (!event) return false;
  if (String(event.event_type ?? "") !== "thought_chunk") return false;
  const payload = event.payload_json ?? {};
  return isFinalThoughtPayload(payload);
};

export const normalizeFinalThoughtPayload = (payload: any): any => {
  const full = readThoughtFullContent(payload);
  if (!full) return payload;
  return {
    ...payload,
    full_content: full,
    is_final: true,
  };
};

export const buildThoughtCacheKey = (event: SessionEvent): string | null => {
  const turnId = idToString(event.turn_id);
  if (!turnId) return null;
  const payload = event.payload_json ?? {};
  const itemId = pickFirstString(payload?.item_id, payload?.itemId);
  const rawSummary = payload?.summary_index ?? payload?.summaryIndex;
  const parsedSummary = typeof rawSummary === "number" ? rawSummary : Number(rawSummary);
  const summaryIndex = Number.isFinite(parsedSummary) ? parsedSummary : 0;
  const fallback = `unknown-${payload?.order_seq ?? payload?.orderSeq ?? idToString(event.id) ?? "missing"}`;
  return `${turnId}|${itemId ?? fallback}|${summaryIndex}`;
};
