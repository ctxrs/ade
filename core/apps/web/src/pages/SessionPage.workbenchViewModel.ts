import {
  idToString,
  type Message,
  type MessageAttachment,
  type SessionEvent,
  type SessionTurn,
  type SessionTurnTool,
} from "../api/client";
import type {
  AskUserQuestionAnswerState,
  ThreadItem,
  WorkbenchThreadView,
  WorkbenchTurnHeader,
} from "./SessionPage.types";
import { humanToolKind } from "./SessionPage.helpers";
import type { ContextWindowInfo } from "../components/WorkbenchComposer";
import type { SessionViewVerbosity } from "../state/uiStateStore";

type PendingMessageEntry = {
  clientId: string;
  message: Message;
};

function readEventOrderSeq(ev: SessionEvent): number | null {
  const parse = (value: unknown): number | null => {
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (typeof value === "string" && value.trim()) {
      const parsed = Number.parseFloat(value);
      return Number.isFinite(parsed) ? parsed : null;
    }
    return null;
  };

  const payload = ev.payload_json ?? {};
  return parse((payload as any)?.order_seq ?? (payload as any)?.orderSeq);
}

type AssistantOrderSeqLookup = {
  byProviderId: Map<string, number>;
};

function collectAssistantOrderSeq(events: SessionEvent[]): AssistantOrderSeqLookup {
  const byProviderId = new Map<string, number>();
  for (const ev of events) {
    const payload = ev.payload_json ?? {};
    if (ev.event_type === "assistant_message_inserted") {
      const orderSeq = readEventOrderSeq(ev);
      if (!Number.isFinite(orderSeq)) continue;
      const providerId = String(payload?.provider_message_id ?? payload?.providerMessageId ?? "").trim();
      if (providerId) byProviderId.set(providerId, orderSeq as number);
      continue;
    }
    if (ev.event_type === "assistant_chunk" || ev.event_type === "assistant_complete") {
      const orderSeq = readEventOrderSeq(ev);
      if (!Number.isFinite(orderSeq)) continue;
      const providerId = String(
        payload?.message_id ??
          payload?.messageId ??
          payload?.provider_message_id ??
          payload?.providerMessageId ??
          "",
      ).trim();
      if (providerId) byProviderId.set(providerId, orderSeq as number);
    }
  }
  return { byProviderId };
}

function appendStreamingFragment(prev: string, fragment: string): string {
  const p = prev ?? "";
  const f = fragment ?? "";
  if (!p) return f;
  if (!f) return p;
  if (f.startsWith(p)) return f;
  if (p.endsWith(f)) return p;
  return `${p}${f}`;
}

type TurnStreamingMeta = {
  pendingProviderId: string | null;
  lastProviderId: string | null;
};

function readTurnStreamingMeta(turn: SessionTurn): TurnStreamingMeta {
  const t = turn as SessionTurn & {
    assistant_partial_provider_message_id?: string | null;
    assistant_last_provider_message_id?: string | null;
  };
  return {
    pendingProviderId: t.assistant_partial_provider_message_id ?? null,
    lastProviderId: t.assistant_last_provider_message_id ?? null,
  };
}

function readThoughtFullContent(payload: any): string | null {
  const full =
    payload?.full_content ??
    payload?.fullContent ??
    payload?.full ??
    payload?.content ??
    payload?.content_fragment ??
    payload?.contentFragment;
  return typeof full === "string" && full.trim() ? full : null;
}

function isFinalThoughtPayload(payload: any): boolean {
  if (!payload || typeof payload !== "object") return false;
  return (
    payload?.is_final === true ||
    payload?.isFinal === true ||
    typeof payload?.full_content === "string" ||
    typeof payload?.fullContent === "string"
  );
}

function isCrpThoughtEvent(ev: SessionEvent): boolean {
  if (ev.event_type !== "thought_chunk") return false;
  const payload = ev.payload_json ?? {};
  return (
    payload?.crp_seq != null ||
    payload?.crpSeq != null ||
    payload?.crp_channel != null ||
    payload?.crpChannel != null
  );
}

function readThoughtBlockKey(ev: SessionEvent): string | null {
  const turnId = idToString(ev.turn_id);
  if (!turnId) return null;
  const payload = ev.payload_json ?? {};
  const rawItemId = String(payload?.item_id ?? payload?.itemId ?? "").trim();
  const rawSummary = payload?.summary_index ?? payload?.summaryIndex;
  const parsedSummary = typeof rawSummary === "number" ? rawSummary : Number(rawSummary);
  const summaryIndex = Number.isFinite(parsedSummary) ? parsedSummary : 0;
  if (rawItemId) return `${turnId}|${rawItemId}|${summaryIndex}`;
  const fallbackSeq = readEventOrderSeq(ev);
  if (Number.isFinite(fallbackSeq)) {
    return `${turnId}|unknown-${fallbackSeq}|${summaryIndex}`;
  }
  const fallbackId = idToString(ev.id);
  if (!fallbackId) return null;
  return `${turnId}|unknown-${fallbackId}|${summaryIndex}`;
}

function readThoughtItemId(ev: SessionEvent): string | null {
  const payload = ev.payload_json ?? {};
  const value = payload?.item_id ?? payload?.itemId;
  if (typeof value === "string" && value.trim()) return value;
  return null;
}

function collectThoughtStream(events: SessionEvent[]): {
  text: string;
  orderSeq?: number;
  createdAt?: string;
  isCrp: boolean;
} | null {
  const thoughtEvents = events
    .filter((ev) => ev.event_type === "thought_chunk" && shouldRenderThoughtChunk(ev))
    .map((ev) => ({ ev, orderSeq: readEventOrderSeq(ev) }))
    .filter((entry) => Number.isFinite(entry.orderSeq));
  if (thoughtEvents.length === 0) return null;

  const sorted = thoughtEvents.slice().sort((a, b) => {
    const sa = a.orderSeq as number;
    const sb = b.orderSeq as number;
    if (sa !== sb) return sa - sb;
    return String(a.ev.created_at).localeCompare(String(b.ev.created_at));
  });

  let text = "";
  let createdAt: string | undefined;
  let orderSeq: number | undefined;
  let isCrp = false;

  let hasFinal = false;
  for (const { ev, orderSeq: seq } of sorted) {
    const payload = ev.payload_json ?? {};
    const finalText = readThoughtFullContent(payload);
    if (isFinalThoughtPayload(payload) && finalText) {
      text = finalText;
      hasFinal = true;
    } else if (!hasFinal) {
      const fragment = String(payload?.content_fragment ?? payload?.contentFragment ?? "");
      if (fragment) {
        text = appendStreamingFragment(text, fragment);
      }
    }
    createdAt = createdAt ?? ev.created_at;
    if (orderSeq === undefined) orderSeq = seq as number;
    if (isCrpThoughtEvent(ev)) isCrp = true;
  }

  if (!text) return null;


  return { text, orderSeq, createdAt, isCrp };
}

type ThoughtBlock = {
  idKey: string;
  text: string;
  orderSeq?: number;
  createdAt?: string;
  isCrp: boolean;
};

function collectThoughtBlocks(events: SessionEvent[]): ThoughtBlock[] {
  const thoughtEvents = events.filter(
    (ev) => ev.event_type === "thought_chunk" && shouldRenderThoughtChunk(ev),
  );
  if (thoughtEvents.length === 0) return [];

  const crpThoughtEvents = thoughtEvents.filter(isCrpThoughtEvent);
  const nonCrpThoughtEvents = thoughtEvents.filter((ev) => !isCrpThoughtEvent(ev));

  const blocks: ThoughtBlock[] = [];

  if (crpThoughtEvents.length > 0) {
    const groups = new Map<string, SessionEvent[]>();
    for (const ev of crpThoughtEvents) {
      const orderSeq = readEventOrderSeq(ev);
      if (!Number.isFinite(orderSeq)) continue;
      const key = readThoughtBlockKey(ev);
      if (!key) continue;
      const list = groups.get(key) ?? [];
      list.push(ev);
      groups.set(key, list);
    }
    for (const [key, list] of groups.entries()) {
      const sorted = list.slice().sort((a, b) => {
        const sa = readEventOrderSeq(a) as number;
        const sb = readEventOrderSeq(b) as number;
        if (sa !== sb) return sa - sb;
        return String(a.created_at).localeCompare(String(b.created_at));
      });
      let text = "";
      let createdAt: string | undefined;
      let orderSeq: number | undefined;
      let hasFinal = false;
      for (const ev of sorted) {
        const payload = ev.payload_json ?? {};
        const finalText = readThoughtFullContent(payload);
        if (isFinalThoughtPayload(payload) && finalText) {
          text = finalText;
          hasFinal = true;
        } else if (!hasFinal) {
          const fragment = String(payload?.content_fragment ?? payload?.contentFragment ?? "");
          if (fragment) {
            text = appendStreamingFragment(text, fragment);
          }
        }
        createdAt = createdAt ?? ev.created_at;
        if (orderSeq === undefined) orderSeq = readEventOrderSeq(ev) as number;
      }
      if (text.trim() && Number.isFinite(orderSeq)) {
        blocks.push({ idKey: `crp:${key}`, text, orderSeq, createdAt, isCrp: true });
      }
    }
  }

  if (nonCrpThoughtEvents.length > 0) {
    const stream = collectThoughtStream(nonCrpThoughtEvents);
    if (stream) {
      const suffix = Number.isFinite(stream.orderSeq)
        ? `seq:${stream.orderSeq}`
        : stream.createdAt
          ? `ts:${stream.createdAt}`
          : "unknown";
      blocks.push({
        idKey: `stream:${suffix}`,
        text: stream.text,
        orderSeq: stream.orderSeq,
        createdAt: stream.createdAt,
        isCrp: stream.isCrp,
      });
    }
  }

  return blocks
    .slice()
    .sort((a, b) => {
      const sa = a.orderSeq;
      const sb = b.orderSeq;
      if (Number.isFinite(sa) && Number.isFinite(sb)) {
        if (sa !== sb) return (sa as number) - (sb as number);
        return String(a.createdAt ?? "").localeCompare(String(b.createdAt ?? ""));
      }
      if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
      if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
      return 0;
    });
}

function buildCustomStatusByTurnId(events: SessionEvent[]): Map<string, string> {
  const normalize = (value: unknown): string | null => {
    const t = String(value ?? "").trim();
    return t ? t : null;
  };

  const extractNoticeStatusText = (ev: SessionEvent): string | null => {
    if (ev.event_type !== "notice") return null;
    const payload = ev.payload_json ?? {};
    const meta = payload?._meta ?? payload?.meta ?? {};
    return (
      normalize(meta?.statusText) ??
      normalize(meta?.status_text) ??
      normalize(payload?.statusText) ??
      normalize(payload?.status_text)
    );
  };

  const extractReasoningSummaryText = (ev: SessionEvent): string | null => {
    if (ev.event_type !== "notice") return null;
    const payload = ev.payload_json ?? {};
    if (payload?.kind !== "reasoning_summary") return null;
    return (
      normalize(payload?.text) ??
      normalize(payload?.summary) ??
      normalize(payload?.content)
    );
  };

  const toolStatusVerb = (kind: string): string | null => {
    const k = String(kind ?? "").trim().toLowerCase();
    if (k === "search") return "Searching";
    if (k === "read" || k === "read_file") return "Reading";
    if (k === "execute" || k === "exec") return "Running";
    if (k === "write" || k === "edit") return "Writing";
    return null;
  };

  const deriveToolStatusText = (update: any): string | null => {
    const kind = String(update?.kind ?? update?.tool_kind ?? update?.toolKind ?? "").trim();
    const verb = toolStatusVerb(kind);
    if (!verb) return null;
    if (verb === "Searching") {
      const query = normalize(update?.input?.query ?? update?.input?.q);
      if (query) return `${verb} ${query}`;
    }
    const title = normalize(update?.title);
    if (title) return `${verb} ${title}`;
    return verb;
  };

  const isActiveToolStatus = (status: unknown): boolean => {
    const s = String(status ?? "").trim().toLowerCase();
    return s === "pending" || s === "queued" || s === "running" || s === "in_progress" || s === "inprogress";
  };

  const sorted = events
    .slice()
    .sort((a, b) => {
      const sa = readEventOrderSeq(a);
      const sb = readEventOrderSeq(b);
      if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
      if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
      if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
      return String(a.created_at).localeCompare(String(b.created_at));
    });

  // CRP: "reasoning_summary" is expected to be title-only (pre-split by the runtime).
  // Do not apply UI-side heuristics to parse or truncate it.
  const summaryByTurn = new Map<string, { order: number; text: string }>();
  const noticeByTurn = new Map<string, { order: number; text: string }>();
  const toolsByTurn = new Map<string, Map<string, { order: number; status: string; text: string | null }>>();

  let order = 0;
  for (const ev of sorted) {
    order += 1;
    const turnId = idToString((ev as any).turn_id);
    if (!turnId) continue;

    const summaryText = extractReasoningSummaryText(ev);
    if (summaryText) {
      summaryByTurn.set(turnId, { order, text: summaryText });
      continue;
    }

    const noticeText = extractNoticeStatusText(ev);
    if (noticeText) {
      noticeByTurn.set(turnId, { order, text: noticeText });
      continue;
    }


    if (ev.event_type !== "tool_call" && ev.event_type !== "tool_call_update" && ev.event_type !== "tool_result") {
      continue;
    }

    const update = (ev.payload_json as any)?.update ?? ev.payload_json ?? {};
    const toolCallId =
      normalize(
        ev.payload_json?.tool_call_id ??
          update?.toolCallId ??
          update?.tool_call_id ??
          update?.rawInput?.call_id ??
          update?.raw_input?.call_id ??
          update?.toolCall?.rawInput?.call_id ??
          "",
      ) ?? "";
    if (!toolCallId) continue;

    const status = String(update?.status ?? update?.tool_status ?? update?.toolStatus ?? "").trim();
    const text = deriveToolStatusText(update);
    const perTurn = toolsByTurn.get(turnId) ?? new Map<string, { order: number; status: string; text: string | null }>();
    perTurn.set(toolCallId, { order, status, text });
    toolsByTurn.set(turnId, perTurn);
  }


  const out = new Map<string, string>();
  const allTurnIds = new Set<string>([
    ...summaryByTurn.keys(),
    ...noticeByTurn.keys(),
    ...toolsByTurn.keys(),
  ]);
  for (const turnId of allTurnIds) {
    const summary = summaryByTurn.get(turnId);
    if (summary?.text) {
      out.set(turnId, summary.text);
      continue;
    }

    const perTurnTools = toolsByTurn.get(turnId);
    let bestTool: { order: number; text: string } | null = null;
    if (perTurnTools) {
      for (const tool of perTurnTools.values()) {
        if (!tool.text) continue;
        if (!isActiveToolStatus(tool.status)) continue;
        if (!bestTool || tool.order > bestTool.order) bestTool = { order: tool.order, text: tool.text };
      }
    }
    if (bestTool) {
      out.set(turnId, bestTool.text);
      continue;
    }
    const notice = noticeByTurn.get(turnId);
    if (notice?.text) out.set(turnId, notice.text);
  }

  return out;
}

function normalizeAskUserQuestionAnswers(raw: unknown): Record<string, string> {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(raw)) {
    if (typeof value === "string" && key.trim()) {
      out[key] = value;
    }
  }
  return out;
}

function extractAskUserQuestionAnswer(ev: SessionEvent): {
  toolCallId: string;
  outcome: "submitted" | "cancelled";
  answers: Record<string, string>;
} | null {
  if (ev.event_type !== "notice") return null;
  const payload = ev.payload_json ?? {};
  if (payload.kind !== "ask_user_question_answered") return null;
  const toolCallId = String(payload.tool_call_id ?? "").trim();
  if (!toolCallId) return null;
  const outcomeRaw = String(payload.outcome ?? "").trim();
  const outcome =
    outcomeRaw === "cancelled" ? "cancelled" : outcomeRaw === "submitted" ? "submitted" : "submitted";
  const answers = normalizeAskUserQuestionAnswers(payload.answers ?? payload.answer ?? {});
  return { toolCallId, outcome, answers };
}

export function collectAskUserQuestionAnswers(
  events: SessionEvent[],
  optimistic: Record<string, AskUserQuestionAnswerState>,
): Map<string, AskUserQuestionAnswerState> {
  const map = new Map<string, AskUserQuestionAnswerState>();
  for (const ev of events) {
    const parsed = extractAskUserQuestionAnswer(ev);
    if (!parsed) continue;
    map.set(parsed.toolCallId, { outcome: parsed.outcome, answers: parsed.answers });
  }
  for (const [toolCallId, state] of Object.entries(optimistic)) {
    if (!toolCallId) continue;
    const existing = map.get(toolCallId);
    if (!existing) {
      map.set(toolCallId, state);
      continue;
    }
    if (Object.keys(existing.answers ?? {}).length === 0 && Object.keys(state.answers ?? {}).length > 0) {
      map.set(toolCallId, { outcome: existing.outcome, answers: state.answers });
    }
  }
  return map;
}


function coerceNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number.parseFloat(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

export function normalizeContextWindowMetrics(metrics: any): ContextWindowInfo | null {
  if (!metrics || typeof metrics !== "object") return null;
  const windowTokens =
    coerceNumber(metrics.context_window_tokens) ??
    coerceNumber(metrics.context_window_size) ??
    coerceNumber(metrics.context_window) ??
    coerceNumber(metrics.context_size) ??
    coerceNumber(metrics.window_tokens) ??
    coerceNumber(metrics.max_context_tokens) ??
    coerceNumber(metrics.max_tokens);
  if (!windowTokens || windowTokens <= 0) return null;

  const inputTokens =
    coerceNumber(metrics.total_input_tokens) ??
    coerceNumber(metrics.input_tokens) ??
    coerceNumber(metrics.prompt_tokens) ??
    coerceNumber(metrics.input);
  const outputTokens =
    coerceNumber(metrics.total_output_tokens) ??
    coerceNumber(metrics.output_tokens) ??
    coerceNumber(metrics.completion_tokens) ??
    coerceNumber(metrics.output);
  const contextTokensEstimate =
    coerceNumber(metrics.context_tokens_estimate) ??
    coerceNumber(metrics.context_tokens);

  let usedTokens: number | null = null;
  if (inputTokens != null || outputTokens != null) {
    usedTokens = (inputTokens ?? 0) + (outputTokens ?? 0);
  } else if (contextTokensEstimate != null) {
    usedTokens = contextTokensEstimate;
  }

  let remainingTokens =
    coerceNumber(metrics.remaining_tokens_estimate) ??
    coerceNumber(metrics.remaining_tokens);
  let remainingFraction =
    coerceNumber(metrics.remaining_fraction) ??
    coerceNumber(metrics.remaining_pct) ??
    coerceNumber(metrics.remaining_percent);

  if (remainingFraction != null && remainingFraction > 1) {
    remainingFraction = remainingFraction <= 100 ? remainingFraction / 100 : null;
  }
  if (remainingFraction != null) {
    remainingFraction = Math.max(0, Math.min(1, remainingFraction));
  }

  if (remainingTokens == null && usedTokens != null) {
    remainingTokens = Math.max(0, windowTokens - usedTokens);
  }
  if (usedTokens == null && remainingTokens != null) {
    usedTokens = Math.max(0, windowTokens - remainingTokens);
  }
  if (remainingFraction == null && usedTokens != null) {
    remainingFraction = Math.max(0, Math.min(1, 1 - usedTokens / windowTokens));
  }
  if (usedTokens == null && remainingFraction != null) {
    usedTokens = Math.max(0, Math.round(windowTokens * (1 - remainingFraction)));
  }

  return {
    windowTokens,
    usedTokens: usedTokens ?? undefined,
    remainingTokens: remainingTokens ?? undefined,
    remainingFraction: remainingFraction ?? undefined,
  };
}

function formatMemoryMb(value?: number | null): string {
  if (!Number.isFinite(value)) return "—";
  const mb = value as number;
  const gb = mb / 1024;
  if (gb >= 1) {
    const precision = gb >= 10 ? 0 : 1;
    return `${gb.toFixed(precision)} GB`;
  }
  return `${Math.round(mb)} MB`;
}

export function deriveMessagesKey(messages: Message[]): string {
  if (messages.length === 0) return "0";
  const last = messages[messages.length - 1];
  const lastId = idToString(last?.id);
  const lastUpdated = last?.created_at ?? "";
  const contentHash = hashString(String(last?.content ?? ""));
  return `${messages.length}:${lastId}:${lastUpdated}:${contentHash}`;
}

function hashString(value: string): string {
  let hash = 5381;
  for (let i = 0; i < value.length; i += 1) {
    hash = ((hash << 5) + hash) ^ value.charCodeAt(i);
  }
  return (hash >>> 0).toString(36);
}

export function deriveTurnsKey(turns: SessionTurn[]): string {
  if (turns.length === 0) return "0";
  const first = turns[0];
  const last = turns[turns.length - 1];
  return `${turns.length}:${first.start_seq ?? ""}:${last.start_seq ?? ""}:${last.updated_at ?? ""}`;
}

const compareMessageOrder = (a: Message, b: Message): number => {
  const c = String(a.created_at).localeCompare(String(b.created_at));
  if (c !== 0) return c;
  const sa = Number(a.turn_sequence ?? Number.NaN);
  const sb = Number(b.turn_sequence ?? Number.NaN);
  if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
  if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
  if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
  return String(idToString(a.id)).localeCompare(String(idToString(b.id)));
};

export function mergeMessagesForView(
  messages: Message[],
  pending: PendingMessageEntry[],
  includeQueuedMessageIds: Set<string> = new Set(),
): Message[] {
  const shouldInclude = (message: Message) => {
    if (message.delivery !== "queued") return true;
    const mid = idToString(message.id);
    return !!mid && includeQueuedMessageIds.has(mid);
  };
  const filteredMessages = messages.filter(shouldInclude);
  const filteredPending = pending.filter((entry) => shouldInclude(entry.message));
  if (filteredPending.length === 0) return filteredMessages;
  const byId = new Map<string, Message>();
  for (const m of filteredMessages) {
    const id = idToString(m.id);
    if (id) byId.set(id, m);
  }
  for (const entry of filteredPending) {
    const id = idToString(entry.message.id);
    if (!id || byId.has(id)) continue;
    byId.set(id, entry.message);
  }
  return Array.from(byId.values()).sort(compareMessageOrder);
}

export function mergeQueuedMessagesForPanel(
  queue: Message[],
  pending: PendingMessageEntry[],
): Message[] {
  if (queue.length === 0 && pending.length === 0) return [];
  if (pending.length === 0) return queue.slice();
  const byId = new Map<string, Message>();
  const pendingNoId: Message[] = [];
  for (const msg of queue) {
    const id = idToString(msg.id);
    if (id) byId.set(id, msg);
  }
  for (const entry of pending) {
    const msg = entry.message;
    const id = idToString(msg.id);
    if (!id) {
      pendingNoId.push(msg);
      continue;
    }
    if (!byId.has(id)) byId.set(id, msg);
  }
  const merged = [...byId.values(), ...pendingNoId];
  merged.sort(compareMessageOrder);
  return merged;
}

export function filterQueuedMessagesForPanel(
  queue: Message[],
  turns: SessionTurn[],
): Message[] {
  if (queue.length === 0 || turns.length === 0) return queue;
  const statusByUserMessageId = new Map<string, string>();
  for (const turn of turns) {
    const mid = turn.user_message_id ? idToString(turn.user_message_id) : "";
    if (!mid) continue;
    statusByUserMessageId.set(mid, turn.status);
  }
  return queue.filter((message) => {
    const mid = idToString(message.id);
    if (!mid) return true;
    const status = statusByUserMessageId.get(mid);
    if (!status) return true;
    return status === "queued";
  });
}

export function filterTurnsForQueuedMessages(
  turns: SessionTurn[],
  queuedMessageIds: Set<string>,
): SessionTurn[] {
  if (queuedMessageIds.size === 0) return turns;
  return turns.filter((turn) => {
    const mid = turn.user_message_id ? idToString(turn.user_message_id) : "";
    if (!mid) return true;
    return !queuedMessageIds.has(mid);
  });
}

export function buildPendingTurns(turns: SessionTurn[], messages: Message[]): SessionTurn[] {
  if (messages.length === 0) return [];
  const turnIds = new Set<string>();
  const userMessageIds = new Set<string>();
  for (const turn of turns) {
    const tid = idToString(turn.turn_id);
    if (tid) turnIds.add(tid);
    const uid = turn.user_message_id ? idToString(turn.user_message_id) : "";
    if (uid) userMessageIds.add(uid);
  }
  const pending: SessionTurn[] = [];
  for (const message of messages) {
    if (message.role !== "user") continue;
    const mid = idToString(message.id);
    if (!mid || userMessageIds.has(mid)) continue;
    let turnId = idToString(message.turn_id);
    if (!turnId) turnId = `pending-turn-${mid}`;
    if (turnIds.has(turnId)) continue;
    pending.push({
      turn_id: turnId,
      session_id: message.session_id,
      run_id: null,
      user_message_id: message.id,
      status: message.delivery === "immediate" ? "running" : "queued",
      start_seq: null,
      end_seq: null,
      started_at: message.created_at,
      updated_at: message.created_at,
      assistant_partial: "",
      thought_partial: "",
      metrics_json: null,
      tool_total: 0,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 0,
      tool_failed: 0,
    });
    turnIds.add(turnId);
  }
  return pending;
}

export function filterThreadItemsForVerbosity(items: ThreadItem[], verbosity: SessionViewVerbosity): ThreadItem[] {
  if (verbosity === "terse") {
    return items.filter((item) => item.kind !== "tool" && item.kind !== "tool_group" && item.kind !== "thought");
  }
  return items;
}

type SortableThreadGroup = {
  sort_seq: number;
  group: WorkbenchThreadView["groups"][number];
};

function buildSystemMessageGroups(messages: Message[]): SortableThreadGroup[] {
  const systemMessages = messages
    .filter((m) => m.role === "system")
    .map((m, idx) => ({
      message: m,
      orderSeq: Number(m.order_seq ?? Number.NaN),
      idx,
    }))
    .filter((entry) => Number.isFinite(entry.orderSeq))
    .sort((a, b) => {
      if (a.orderSeq !== b.orderSeq) return (a.orderSeq as number) - (b.orderSeq as number);
      return a.idx - b.idx;
    });
  return systemMessages.flatMap((entry) => {
    const m = entry.message;
    const id = idToString(m.id);
    if (!id) {
      if (import.meta.env.DEV) {
        // eslint-disable-next-line no-console
        console.error("[WorkbenchThreadViewModel] system message missing id", {
          created_at: m.created_at ?? null,
          order_seq: m.order_seq ?? null,
        });
      }
      return [];
    }
    const attachments = Array.isArray((m as any).attachments)
      ? ((m as any).attachments as MessageAttachment[])
      : [];
    return [{
      sort_seq: entry.orderSeq as number,
      group: {
        key: `system-${id}`,
        header: null,
        items: [
          {
            kind: "message",
            id,
            role: "system",
            content: m.content ?? "",
            attachments,
            created_at: m.created_at,
          },
        ],
      },
    }];
  });
}

function mergeGroupsWithSystemMessages(
  groups: SortableThreadGroup[],
  messages: Message[],
): WorkbenchThreadView["groups"] {
  const systemGroups = buildSystemMessageGroups(messages);
  if (systemGroups.length === 0) {
    return groups.map((g) => g.group);
  }
  const combined = [...groups, ...systemGroups];
  combined.sort((a, b) => {
    if (a.sort_seq !== b.sort_seq) return a.sort_seq - b.sort_seq;
    return String(a.group.key).localeCompare(String(b.group.key));
  });
  return combined.map((g) => g.group);
}

export function buildWorkbenchThreadViewModel(
  turns: SessionTurn[],
  messages: Message[],
  toolsByTurnId: Record<string, SessionTurnTool[]>,
  events: SessionEvent[],
  askUserQuestionAnswers?: Map<string, AskUserQuestionAnswerState>,
): WorkbenchThreadView {
  const answers =
    askUserQuestionAnswers ?? collectAskUserQuestionAnswers(events, {});
  if (turns.length > 0) {
    return buildWorkbenchThreadViewModelFromTurns(turns, messages, toolsByTurnId, events, answers);
  }
  return buildWorkbenchThreadViewModelFromEvents(events, messages, answers);
}

function shouldRenderThoughtChunk(ev: SessionEvent): boolean {
  const payload = ev.payload_json ?? {};
  const meta = payload?._meta ?? payload?.meta ?? {};
  if (meta?.heartbeat === true) return false;
  if (isStatusUpdateMeta(meta)) return false;
  const reasoningKind = meta?.codex?.reasoning_kind ?? meta?.codex?.reasoningKind;
  if (reasoningKind === "summary") return false;
  return true;
}

function shouldRenderAssistantChunk(ev: SessionEvent): boolean {
  const payload = ev.payload_json ?? {};
  const meta = payload?._meta ?? payload?.meta ?? {};
  if (meta?.heartbeat === true) return false;
  if (isStatusUpdateMeta(meta)) return false;
  return true;
}

type ActivityEntry = {
  item: ThreadItem;
  created_at: string;
  kind: "tool" | "thought" | "ask_user_question" | "message";
  order_seq?: number;
};

function ensureToolItem(
  toolById: Map<string, Extract<ThreadItem, { kind: "tool" }>>,
  turnId: string,
  toolCallId: string,
  createdAt: string,
) {
  const existing = toolById.get(toolCallId);
  if (existing) return existing;
  const tool: Extract<ThreadItem, { kind: "tool" }> = {
    kind: "tool",
    id: `tool-${turnId}-${toolCallId}`,
    tool_call_id: toolCallId,
    created_at: createdAt,
    updated_at: createdAt,
    tool_kind: "tool",
    title: "Tool",
    status: "pending",
    locations: [],
    input: null,
    output_text: "",
    raw: null,
    updates_seen: 0,
    has_details: true,
  };
  toolById.set(toolCallId, tool);
  return tool;
}

function applyToolUpdateFromEvent(
  tool: Extract<ThreadItem, { kind: "tool" }>,
  ev: SessionEvent,
  update: any,
) {
  tool.updated_at = ev.created_at;
  tool.updates_seen += 1;
  tool.raw = ev.payload_json ?? tool.raw;

  const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
  if (nextKind) tool.tool_kind = nextKind;

  const nextTitle = String(
    update?.title ??
      update?.tool_label ??
      update?.toolLabel ??
      update?.toolCall?.title ??
      update?.toolCall?.tool_label ??
      update?.toolCall?.toolLabel ??
      update?.toolCall?.name ??
      "",
  ).trim();
  if (nextTitle) tool.title = nextTitle;
  else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

  const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
  if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
  else if (ev.event_type === "tool_result") tool.status = "completed";

  const locs = Array.isArray(update?.locations) ? update.locations : [];
  tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

  const input =
    update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? update?.input_preview ?? null;
  if (input != null) tool.input = input;

  const nextOutput = extractToolOutputText(update);
  if (nextOutput) tool.output_text = mergeStreamingText(tool.output_text, nextOutput);
  if (tool.input != null || tool.output_text.trim().length > 0) {
    tool.has_details = true;
  }
}

function buildAskUserQuestionItem(
  ev: SessionEvent,
  turnId: string,
  answersByToolCallId: Map<string, AskUserQuestionAnswerState>,
): Extract<ThreadItem, { kind: "ask_user_question" }> | null {
  if (ev.event_type !== "notice") return null;
  const payload = ev.payload_json ?? {};
  if (payload.kind !== "ask_user_question") return null;
  const toolCallId = String(payload.tool_call_id ?? "").trim();
  if (!toolCallId) return null;
  const answerState = answersByToolCallId.get(toolCallId);
  return {
    kind: "ask_user_question",
    id: `askq-${turnId}-${toolCallId}`,
    turn_id: turnId,
    created_at: ev.created_at,
    tool_call_id: toolCallId,
    input: payload.input ?? payload.input_json ?? payload,
    answers: answerState?.answers,
    outcome: answerState?.outcome,
    answered: Boolean(answerState),
  };
}

function buildNoticeMessageItem(
  ev: SessionEvent,
  turnId: string,
): Extract<ThreadItem, { kind: "message" }> | null {
  if (ev.event_type !== "notice") return null;
  const payload = ev.payload_json ?? {};
  const code = String(payload?.kind ?? payload?.code ?? "").trim().toLowerCase();
  if (code !== "context.compacted" && code !== "context_compacted") return null;
  const message =
    pickFirstString(payload?.message, payload?.text, payload?.summary, payload?.content) ??
    "Context compacted. Earlier turns were summarized.";
  const eventId = idToString(ev.id);
  if (!eventId) {
    if (import.meta.env.DEV) {
      // eslint-disable-next-line no-console
      console.error("[WorkbenchThreadViewModel] notice event missing id", {
        turnId,
        created_at: ev.created_at,
        kind: payload?.kind ?? payload?.code ?? null,
      });
    }
    return null;
  }
  return {
    kind: "message",
    id: `notice-${turnId}-${eventId}`,
    role: "system",
    content: message,
    attachments: [],
    created_at: ev.created_at,
  };
}

function buildTurnActivityTimeline(opts: {
  turnId: string;
  turn: SessionTurn;
  tools: Array<Extract<ThreadItem, { kind: "tool" }>>;
  events: SessionEvent[];
  askUserQuestionAnswers: Map<string, AskUserQuestionAnswerState>;
}): { activity: ActivityEntry[]; tools: Array<Extract<ThreadItem, { kind: "tool" }>> } {
  const toolById = new Map<string, Extract<ThreadItem, { kind: "tool" }>>();
  for (const tool of opts.tools) {
    toolById.set(tool.tool_call_id, tool);
  }

  const thoughtBlocks = collectThoughtBlocks(opts.events);

  const activity: ActivityEntry[] = [];
  const toolInserted = new Set<string>();
  const askInserted = new Set<string>();

  for (const ev of opts.events) {
    if (ev.event_type === "notice") {
      const orderSeq = readEventOrderSeq(ev);
      if (!Number.isFinite(orderSeq)) continue;
      const askItem = buildAskUserQuestionItem(ev, opts.turnId, opts.askUserQuestionAnswers);
      if (askItem && !askInserted.has(askItem.tool_call_id)) {
        activity.push({
          item: askItem,
          created_at: ev.created_at,
          kind: "ask_user_question",
          order_seq: orderSeq as number,
        });
        askInserted.add(askItem.tool_call_id);
      }
    }

    if (ev.event_type === "tool_call" || ev.event_type === "tool_call_update" || ev.event_type === "tool_result") {
      const orderSeq = readEventOrderSeq(ev);
      if (!Number.isFinite(orderSeq)) continue;
      const update = (ev.payload_json as any)?.update ?? ev.payload_json ?? {};
      const toolCallId =
        String(
          ev.payload_json?.tool_call_id ??
            update?.toolCallId ??
            update?.tool_call_id ??
            update?.rawInput?.call_id ??
            update?.raw_input?.call_id ??
            update?.toolCall?.rawInput?.call_id ??
            "",
        ).trim();
      if (!toolCallId) {
        continue;
      }
      const tool = ensureToolItem(toolById, opts.turnId, toolCallId, ev.created_at);
      applyToolUpdateFromEvent(tool, ev, update);
      if (!toolInserted.has(toolCallId)) {
        activity.push({
          item: tool,
          created_at: tool.created_at,
          kind: "tool",
          order_seq: orderSeq as number,
        });
        toolInserted.add(toolCallId);
      }
      continue;
    }
  }

  if (thoughtBlocks.length > 0) {
    thoughtBlocks.forEach((block, index) => {
      if (!Number.isFinite(block.orderSeq)) return;
      const thoughtText = block.text ?? "";
      if (!thoughtText.trim()) return;
      const stableSuffix =
        block.idKey || (Number.isFinite(block.orderSeq) ? `seq:${block.orderSeq}` : `idx:${index}`);
      const thoughtItem: Extract<ThreadItem, { kind: "thought" }> = {
        kind: "thought",
        // Avoid index/text-based IDs; those break MessageList prepend invariants and streaming updates.
        id: `thought-${opts.turnId}-${stableSuffix}`,
        turn_id: opts.turnId,
        created_at: block.createdAt ?? opts.turn.updated_at ?? opts.turn.started_at,
        content: thoughtText,
      };
      activity.push({
        item: thoughtItem,
        created_at: thoughtItem.created_at,
        kind: "thought",
        order_seq: block.orderSeq,
      });
    });
  }

  return { activity, tools: Array.from(toolById.values()) };
}

export function buildWorkbenchThreadViewModelFromTurns(
  turns: SessionTurn[],
  messages: Message[],
  toolsByTurnId: Record<string, SessionTurnTool[]>,
  events: SessionEvent[],
  askUserQuestionAnswers: Map<string, AskUserQuestionAnswerState> = new Map(),
): WorkbenchThreadView {
  const debugEvents: SessionEvent[] = [];
  const groups: SortableThreadGroup[] = [];
  const customStatusByTurnId = buildCustomStatusByTurnId(events);
  const sortedTurns = turns.slice().sort((a, b) => {
    const aSeq = Number(a.start_seq ?? Number.NaN);
    const bSeq = Number(b.start_seq ?? Number.NaN);
    if (Number.isFinite(aSeq) && Number.isFinite(bSeq) && aSeq !== bSeq) return aSeq - bSeq;
    if (Number.isFinite(aSeq) && !Number.isFinite(bSeq)) return -1;
    if (!Number.isFinite(aSeq) && Number.isFinite(bSeq)) return 1;
    const aEnd = Number(a.end_seq ?? Number.NaN);
    const bEnd = Number(b.end_seq ?? Number.NaN);
    if (Number.isFinite(aEnd) && Number.isFinite(bEnd) && aEnd !== bEnd) return aEnd - bEnd;
    if (Number.isFinite(aEnd) && !Number.isFinite(bEnd)) return -1;
    if (!Number.isFinite(aEnd) && Number.isFinite(bEnd)) return 1;
    const aStart = String(a.started_at ?? a.created_at ?? "");
    const bStart = String(b.started_at ?? b.created_at ?? "");
    if (aStart !== bStart) return aStart.localeCompare(bStart);
    const aId = idToString(a.turn_id) ?? "";
    const bId = idToString(b.turn_id) ?? "";
    return aId.localeCompare(bId);
  });

  const messageById = new Map<string, Message>();
  const messagesByTurnId = new Map<string, Message[]>();
  for (const m of messages) {
    const mid = idToString(m.id);
    if (mid) messageById.set(mid, m);
    const turnId = idToString(m.turn_id);
    if (turnId) {
      const list = messagesByTurnId.get(turnId) ?? [];
      list.push(m);
      messagesByTurnId.set(turnId, list);
    }
  }

  const eventsByTurnId = new Map<string, SessionEvent[]>();
  for (const ev of events) {
    const turnId = idToString(ev.turn_id);
    if (!turnId) continue;
    const list = eventsByTurnId.get(turnId) ?? [];
    list.push(ev);
    eventsByTurnId.set(turnId, list);
  }

  for (const turn of sortedTurns) {
    const turnId = idToString(turn.turn_id);
    if (!turnId) {
      if (import.meta.env.DEV) {
        // eslint-disable-next-line no-console
        console.error("[WorkbenchThreadViewModel] turn missing turn_id", {
          started_at: turn.started_at ?? null,
          created_at: turn.created_at ?? null,
        });
      }
      continue;
    }
    const userMessageId = turn.user_message_id ? idToString(turn.user_message_id) : "";

    const turnMessages = messagesByTurnId.get(turnId) ?? [];
    const fallbackUserMessage =
      !userMessageId ? turnMessages.find((m) => m.role === "user") : undefined;
    const userMessage = userMessageId ? messageById.get(userMessageId) : fallbackUserMessage;
    const headerId = turnId;

    const header: WorkbenchTurnHeader | null = userMessage
      ? {
        id: headerId,
        content: userMessage.content ?? "",
        attachments: Array.isArray((userMessage as any).attachments)
          ? ((userMessage as any).attachments as MessageAttachment[])
          : [],
        created_at: userMessage.created_at,
      }
      : null;
    const headerOrderSeq = Number(userMessage?.order_seq ?? Number.NaN);

    let tools = (toolsByTurnId[turnId] ?? []).map((tool) => {
      const toolKind = String(tool.tool_kind ?? "tool");
      const title = String(tool.title ?? humanToolKind(toolKind));
      const summaryOnly = (tool as any).summary_only === true;
      const hasDetails =
        !summaryOnly && (tool.input_json != null || String(tool.output_text ?? "").trim().length > 0);
      return {
        kind: "tool",
        id: `tool-${turnId}-${tool.tool_call_id}`,
        tool_call_id: tool.tool_call_id,
        created_at: tool.created_at,
        updated_at: tool.updated_at ?? tool.created_at,
        tool_kind: toolKind,
        title,
        status: String(tool.status ?? "pending"),
        locations: [],
        input: tool.input_json ?? null,
        output_text: String(tool.output_text ?? ""),
        raw: tool,
        updates_seen: 1,
        has_details: hasDetails,
      } satisfies Extract<ThreadItem, { kind: "tool" }>;
    });

    const eventsForTurn = eventsByTurnId.get(turnId) ?? [];
    const { activity } = buildTurnActivityTimeline({
      turnId,
      turn,
      tools,
      events: eventsForTurn,
      askUserQuestionAnswers,
    });
    const assistantOrderSeq = collectAssistantOrderSeq(eventsForTurn);

    const assistantMessages = (messagesByTurnId.get(turnId) ?? [])
      .filter((m) => m.role === "assistant")
      .slice()
      .sort((a, b) => {
        const sa = Number(a.turn_sequence ?? Number.NaN);
        const sb = Number(b.turn_sequence ?? Number.NaN);
        if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
        if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
        if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
        return String(a.created_at).localeCompare(String(b.created_at));
      });

    type TimelineEntry = {
      item: ThreadItem;
      created_at: string;
      kind: "assistant" | "tool" | "thought" | "ask_user_question";
      order_seq?: number;
      turn_sequence?: number;
    };

    const timeline: TimelineEntry[] = [];
    for (const m of assistantMessages) {
      const orderSeq = Number(m.order_seq ?? Number.NaN);
      if (!Number.isFinite(orderSeq)) continue;
      const messageId = idToString(m.id);
      if (!messageId) {
        // Missing message ids break MessageList identity invariants; treat this as a bug.
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.error("[WorkbenchThreadViewModel] assistant message missing id", {
            turnId,
            created_at: m.created_at,
            order_seq: m.order_seq ?? null,
            turn_sequence: m.turn_sequence ?? null,
          });
        }
        continue;
      }
      timeline.push({
        item: {
          kind: "assistant",
          // Use the message id so item identity is stable even if sequencing metadata is backfilled later.
          id: `assistant-msg-${messageId}`,
          turn_id: turnId,
          created_at: m.created_at,
          content: m.content ?? "",
          thought: "",
          is_complete: true,
        },
        created_at: m.created_at,
        kind: "assistant",
        turn_sequence: Number(m.turn_sequence ?? Number.NaN),
        order_seq: orderSeq as number,
      });
    }

    const statusText =
      turn.status === "running" || turn.status === "queued"
        ? customStatusByTurnId.get(turnId) ?? null
        : null;
    const pendingContent = String(turn.assistant_partial ?? "");
    const pendingTrimmed = pendingContent.trim();
    const statusTrimmed = statusText?.trim() ?? "";
    const { pendingProviderId, lastProviderId } = readTurnStreamingMeta(turn);
    // provider_message_id lets us drop the streaming partial once the final message is inserted.
    const isDuplicatePending =
      !!pendingProviderId && !!lastProviderId && pendingProviderId === lastProviderId;
    if (pendingTrimmed.length > 0 && pendingTrimmed !== statusTrimmed && !isDuplicatePending) {
      const pendingOrderSeq = pendingProviderId
        ? assistantOrderSeq.byProviderId.get(pendingProviderId)
        : undefined;
      if (!Number.isFinite(pendingOrderSeq)) {
        // Skip rendering partials without order_seq to avoid fallback ordering.
      } else {
      const pendingCreatedAt = turn.started_at ?? turn.updated_at;
      timeline.push({
        item: {
          kind: "assistant",
          id: `assistant-${turnId}-pending`,
          turn_id: turnId,
          created_at: pendingCreatedAt,
          content: pendingContent,
          thought: "",
          is_complete: false,
        },
        created_at: pendingCreatedAt,
        kind: "assistant",
        turn_sequence: Number.MAX_SAFE_INTEGER,
        order_seq: pendingOrderSeq as number,
      });
      }
    }

    for (const entry of activity) {
      timeline.push({
        item: entry.item,
        created_at: entry.created_at,
        kind: entry.kind,
        order_seq: entry.order_seq,
      });
    }

    timeline.sort((a, b) => {
      const aSeq = a.order_seq;
      const bSeq = b.order_seq;
      if (Number.isFinite(aSeq) && Number.isFinite(bSeq)) {
        if (aSeq !== bSeq) return (aSeq as number) - (bSeq as number);
        const tcmp = String(a.created_at).localeCompare(String(b.created_at));
        if (tcmp !== 0) return tcmp;
      } else if (Number.isFinite(aSeq) && !Number.isFinite(bSeq)) {
        return -1;
      } else if (!Number.isFinite(aSeq) && Number.isFinite(bSeq)) {
        return 1;
      }
      if (a.kind === "assistant" && b.kind === "assistant") {
        const sa = Number(a.turn_sequence ?? Number.NaN);
        const sb = Number(b.turn_sequence ?? Number.NaN);
        if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
      }
      const aRank = a.kind === "assistant" ? 2 : 1;
      const bRank = b.kind === "assistant" ? 2 : 1;
      if (aRank !== bRank) return aRank - bRank;
      return String(a.item.id).localeCompare(String(b.item.id));
    });

    const items: ThreadItem[] = timeline.map((entry) => entry.item);

    if (items.length === 0) {
      items.push({ kind: "spacer", id: `spacer-${turnId}`, created_at: turn.started_at });
    }

    const assistantMessagesContent = assistantMessages
      .map((m) => m.content ?? "")
      .filter((c) => c.trim().length > 0)
      .join("\n\n");
    items.push({
      kind: "turn_status",
      id: `turn-status-${turnId}`,
      turn_id: turnId,
      created_at: turn.updated_at ?? turn.started_at,
      status: turn.status,
      started_at: turn.started_at,
      updated_at: turn.updated_at ?? turn.started_at,
      custom_status: statusText,
      assistant_messages_content: assistantMessagesContent,
    });

    const timelineOrderSeq = timeline
      .map((entry) => entry.order_seq)
      .filter((seq): seq is number => Number.isFinite(seq));
    const groupOrderSeq = Number.isFinite(headerOrderSeq)
      ? (headerOrderSeq as number)
      : timelineOrderSeq.length > 0
        ? Math.min(...timelineOrderSeq)
        : Number.NaN;
    if (!Number.isFinite(groupOrderSeq)) {
      continue;
    }
    groups.push({
      sort_seq: groupOrderSeq as number,
      group: { key: `turn-${turnId}`, header, items },
    });
  }

  return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
}

function buildWorkbenchThreadViewModelFromEvents(
  events: SessionEvent[],
  messages: Message[],
  askUserQuestionAnswers: Map<string, AskUserQuestionAnswerState> = new Map(),
): WorkbenchThreadView {
  type ToolItem = Extract<ThreadItem, { kind: "tool" }>;
  type TurnGroup = {
    key: string;
    header: WorkbenchTurnHeader | null;
    first_at: string;
    toolItems: ToolItem[];
    toolById: Map<string, ToolItem>;
    assistant: Extract<ThreadItem, { kind: "assistant" }> | null;
    thought_first_at: string | null;
    thought_last_at: string | null;
    thought_is_crp: boolean;
    assistant_first_at: string | null;
    assistant_complete_at: string | null;
  };

  const debugEvents: SessionEvent[] = [];

  const userMessages = messages
    .filter((m) => m.role === "user")
    .map((m, idx) => ({ message: m, orderSeq: Number(m.order_seq ?? Number.NaN), idx }))
    .filter((entry) => Number.isFinite(entry.orderSeq))
    .sort((a, b) => {
      if (a.orderSeq !== b.orderSeq) return (a.orderSeq as number) - (b.orderSeq as number);
      const aId = idToString(a.message.id) ?? "";
      const bId = idToString(b.message.id) ?? "";
      if (aId !== bId) return aId.localeCompare(bId);
      return a.idx - b.idx;
    })
    .map((entry) => entry.message);

  const assistantMessages = messages
    .filter((m) => m.role === "assistant")
    .map((m, idx) => ({ message: m, orderSeq: Number(m.order_seq ?? Number.NaN), idx }))
    .filter((entry) => Number.isFinite(entry.orderSeq))
    .sort((a, b) => {
      if (a.orderSeq !== b.orderSeq) return (a.orderSeq as number) - (b.orderSeq as number);
      const aId = idToString(a.message.id) ?? "";
      const bId = idToString(b.message.id) ?? "";
      if (aId !== bId) return aId.localeCompare(bId);
      return a.idx - b.idx;
    })
    .map((entry) => entry.message);

  const ensureTool = (g: TurnGroup, toolCallId: string, createdAt: string) => {
    const existing = g.toolById.get(toolCallId);
    if (existing) return existing;
    const t: ToolItem = {
      kind: "tool",
      id: `tool-${toolCallId}`,
      tool_call_id: toolCallId,
      created_at: createdAt,
      updated_at: createdAt,
      tool_kind: "tool",
      title: "Tool",
      status: "pending",
      locations: [],
      input: null,
      output_text: "",
      raw: null,
      updates_seen: 0,
      has_details: true,
    };
    g.toolById.set(toolCallId, t);
    g.toolItems.push(t);
    return t;
  };

  // If messages haven't been refreshed yet (common in the Workbench "Start" flow), fall back to
  // grouping by `user_message` events so streamed assistant/tool updates still render.
  if (userMessages.length === 0) {
    const userEvents = events
      .filter((e) => e.event_type === "user_message")
      .map((ev) => ({ ev, orderSeq: readEventOrderSeq(ev) }))
      .filter((entry) => Number.isFinite(entry.orderSeq))
      .slice()
      .sort((a, b) => {
        const sa = a.orderSeq as number;
        const sb = b.orderSeq as number;
        if (sa !== sb) return sa - sb;
        return String(a.ev.created_at).localeCompare(String(b.ev.created_at));
      })
      .map((entry) => entry.ev);

    const eventsInRangeExclusive = (startIso: string, endIso: string | null) => {
      const start = Date.parse(startIso);
      const end = endIso ? Date.parse(endIso) : Number.POSITIVE_INFINITY;
      return events.filter((e) => {
        const t = Date.parse(String(e.created_at));
        if (!Number.isFinite(t) || !Number.isFinite(start)) return false;
        return t >= start && t < end;
      });
    };

    const groups: SortableThreadGroup[] = [];

    if (userEvents.length === 0) {
      // As a last resort, show any tool activity even without a user turn anchor.
      const g: TurnGroup = {
        key: "no-user-messages",
        header: null,
        first_at: events[0]?.created_at ?? new Date().toISOString(),
        toolItems: [],
        toolById: new Map(),
        assistant: null,
        thought_first_at: null,
        thought_last_at: null,
        thought_is_crp: false,
        assistant_first_at: null,
        assistant_complete_at: null,
      };
      let thought = "";
      let thoughtAt: string | null = null;
      let thoughtIsCrp = false;
      let thoughtOrderSeq: number | undefined;
      let assistantOrderSeq: number | undefined;
      const activity: ActivityEntry[] = [];
      const askItems: Array<Extract<ThreadItem, { kind: "ask_user_question" }>> = [];
      const askInserted = new Set<string>();
      const noticeInserted = new Set<string>();
      const toolInserted = new Set<string>();
      for (const ev of events) {
        if (ev.event_type === "notice") {
          const orderSeq = readEventOrderSeq(ev);
          if (!Number.isFinite(orderSeq)) continue;
          const askItem = buildAskUserQuestionItem(ev, g.key, askUserQuestionAnswers);
          if (askItem && !askInserted.has(askItem.tool_call_id)) {
            askItems.push(askItem);
            activity.push({
              item: askItem,
              created_at: ev.created_at,
              kind: "ask_user_question",
              order_seq: orderSeq as number,
            });
            askInserted.add(askItem.tool_call_id);
          }
          const noticeItem = buildNoticeMessageItem(ev, g.key);
          if (noticeItem && !noticeInserted.has(noticeItem.id)) {
            activity.push({
              item: noticeItem,
              created_at: noticeItem.created_at,
              kind: "message",
              order_seq: orderSeq as number,
            });
            noticeInserted.add(noticeItem.id);
          }
        }
        if (ev.event_type === "thought_chunk") {
          if (!shouldRenderThoughtChunk(ev)) continue;
          const orderSeq = readEventOrderSeq(ev);
          if (!Number.isFinite(orderSeq)) continue;
          const fragment = String(ev.payload_json?.content_fragment ?? "");
          if (fragment) {
            thought = appendStreamingFragment(thought, fragment);
            thoughtAt = thoughtAt ?? ev.created_at;
            thoughtOrderSeq = thoughtOrderSeq ?? (orderSeq as number);
            if (isCrpThoughtEvent(ev)) thoughtIsCrp = true;
          }
        }
        if (ev.event_type === "assistant_chunk" || ev.event_type === "assistant_complete") {
          const orderSeq = readEventOrderSeq(ev);
          if (!Number.isFinite(orderSeq)) continue;
          assistantOrderSeq = assistantOrderSeq ?? (orderSeq as number);
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              turn_id: g.key,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          if (ev.event_type === "assistant_chunk") {
            if (!shouldRenderAssistantChunk(ev)) continue;
            const fragment = String(ev.payload_json?.content_fragment ?? "");
            if (fragment) {
              g.assistant_first_at = g.assistant_first_at ?? ev.created_at;
              g.assistant.content += fragment;
            }
          } else {
            const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
            g.assistant_complete_at = ev.created_at;
            if (full) g.assistant.content = full;
            g.assistant.is_complete = true;
          }
        }
        const update = (ev.payload_json as any)?.update ?? ev.payload_json ?? {};
        const toolCallId =
          String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
        if (!toolCallId) continue;
        const orderSeq = readEventOrderSeq(ev);
        if (!Number.isFinite(orderSeq)) continue;
        const tool = ensureTool(g, toolCallId, ev.created_at);
        if (!toolInserted.has(toolCallId)) {
          activity.push({
            item: tool,
            created_at: tool.created_at,
            kind: "tool",
            order_seq: orderSeq as number,
          });
          toolInserted.add(toolCallId);
        }
      }
      type TimelineEntry = {
        item: ThreadItem;
        created_at: string;
        kind: "assistant" | "tool" | "thought" | "ask_user_question" | "message";
        order_seq?: number;
      };
      const timeline: TimelineEntry[] = [];
      for (const entry of activity) {
        timeline.push({
          item: entry.item,
          created_at: entry.created_at,
          kind: entry.kind,
          order_seq: entry.order_seq,
        });
      }
      const thoughtContent = thought;
      if (thoughtContent.trim() && Number.isFinite(thoughtOrderSeq)) {
        timeline.push({
          item: {
            kind: "thought",
            id: `thought-${g.key}`,
            turn_id: g.key,
            created_at: thoughtAt ?? g.first_at,
            content: thoughtContent,
          },
          created_at: thoughtAt ?? g.first_at,
          kind: "thought",
          order_seq: thoughtOrderSeq,
        });
      }
      if (g.assistant && Number.isFinite(assistantOrderSeq)) {
        timeline.push({
          item: {
            ...g.assistant,
            thought_seconds: (() => {
              if (!g.thought_first_at) return undefined;
              const start = Date.parse(g.thought_first_at);
              const endRaw = g.assistant_first_at ?? g.assistant_complete_at ?? g.thought_last_at;
              if (!endRaw) return undefined;
              const end = Date.parse(endRaw);
              if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 1;
              return Math.max(1, Math.round((end - start) / 1000));
            })(),
          },
          created_at: g.assistant.created_at,
          kind: "assistant",
          order_seq: assistantOrderSeq,
        });
      }
      timeline.sort((a, b) => {
        const aSeq = a.order_seq;
        const bSeq = b.order_seq;
        if (Number.isFinite(aSeq) && Number.isFinite(bSeq)) {
          if (aSeq !== bSeq) return (aSeq as number) - (bSeq as number);
          return String(a.created_at).localeCompare(String(b.created_at));
        }
        if (Number.isFinite(aSeq) && !Number.isFinite(bSeq)) return -1;
        if (!Number.isFinite(aSeq) && Number.isFinite(bSeq)) return 1;
        return 0;
      });
      const items: ThreadItem[] = timeline.map((entry) => entry.item);
      if (items.length === 0) items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
      const groupOrderSeq = timeline
        .map((entry) => entry.order_seq)
        .filter((seq): seq is number => Number.isFinite(seq))
        .reduce((min, seq) => Math.min(min, seq), Number.POSITIVE_INFINITY);
      if (!Number.isFinite(groupOrderSeq)) {
        return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
      }
      groups.push({ sort_seq: groupOrderSeq, group: { key: g.key, header: g.header, items } });
      return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
    }

    for (let i = 0; i < userEvents.length; i++) {
      const u = userEvents[i];
      const nextUser = userEvents[i + 1] ?? null;
      const userOrderSeq = readEventOrderSeq(u);
      const mid = String(u.payload_json?.message_id ?? "").trim();
      if (!mid) {
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.error("[WorkbenchThreadViewModel] user_message event missing payload.message_id", {
            created_at: u.created_at,
            event_id: idToString(u.id) ?? null,
          });
        }
        continue;
      }
      if (!mid) {
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.error("[WorkbenchThreadViewModel] user event missing message_id and id", {
            created_at: u.created_at,
          });
        }
        continue;
      }

      const header: WorkbenchTurnHeader = {
        id: mid,
        content: String(u.payload_json?.content ?? ""),
        attachments: Array.isArray(u.payload_json?.attachments)
          ? (u.payload_json.attachments as MessageAttachment[])
          : [],
        created_at: u.created_at,
      };

      const g: TurnGroup = {
        key: `m-${mid}`,
        header,
        first_at: u.created_at,
        toolItems: [],
        toolById: new Map(),
        assistant: null,
        thought_first_at: null,
        thought_last_at: null,
        thought_is_crp: false,
        assistant_first_at: null,
        assistant_complete_at: null,
      };

      const evs = eventsInRangeExclusive(u.created_at, nextUser?.created_at ?? null);
      const askItems: Array<Extract<ThreadItem, { kind: "ask_user_question" }>> = [];
      const askInserted = new Set<string>();
      const noticeItems: Array<Extract<ThreadItem, { kind: "message" }>> = [];
      const noticeInserted = new Set<string>();

      for (const ev of evs) {
        const eventId = idToString(ev.id);
        if (!eventId) {
          if (import.meta.env.DEV) {
            // eslint-disable-next-line no-console
            console.error("[WorkbenchThreadViewModel] event missing id (events-only view)", {
              created_at: ev.created_at,
              event_type: ev.event_type,
            });
          }
          continue;
        }
        if (ev.created_at < g.first_at) g.first_at = ev.created_at;

        switch (ev.event_type) {
          case "notice": {
            const askItem = buildAskUserQuestionItem(ev, g.key, askUserQuestionAnswers);
            if (askItem && !askInserted.has(askItem.tool_call_id)) {
              askItems.push(askItem);
              askInserted.add(askItem.tool_call_id);
            }
            const noticeItem = buildNoticeMessageItem(ev, g.key);
            if (noticeItem && !noticeInserted.has(noticeItem.id)) {
              noticeItems.push(noticeItem);
              noticeInserted.add(noticeItem.id);
            }
            break;
          }
          case "error": {
            const message = extractErrorMessage(ev.payload_json) ?? "Error";
            const provider = String(ev.payload_json?.provider ?? "").trim();
            const output = provider ? `${message}\nprovider: ${provider}` : message;
            const tool = ensureTool(g, `error-${eventId}`, ev.created_at);
            tool.tool_kind = "error";
            tool.title = "Error";
            tool.status = "failed";
            tool.updated_at = ev.created_at;
            tool.updates_seen += 1;
            tool.input = ev.payload_json ?? null;
            tool.output_text = output;
            tool.raw = ev;
            break;
          }
          case "assistant_chunk": {
            if (!shouldRenderAssistantChunk(ev)) break;
            const fragment = String(ev.payload_json?.content_fragment ?? "");
            if (!fragment) break;
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                turn_id: g.key,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.assistant_first_at = g.assistant_first_at ?? ev.created_at;
            g.assistant.content += fragment;
            break;
          }
          case "assistant_complete": {
            const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                turn_id: g.key,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.assistant_complete_at = ev.created_at;
            if (full) g.assistant.content = full;
            g.assistant.is_complete = true;
            break;
          }
          case "thought_chunk": {
            if (!shouldRenderThoughtChunk(ev)) break;
            const fragment = String(ev.payload_json?.content_fragment ?? "");
            if (!fragment) break;
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                turn_id: g.key,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.thought_first_at = g.thought_first_at ?? ev.created_at;
            g.thought_last_at = ev.created_at;
            g.assistant.thought += fragment;
            break;
          }
          case "tool_call":
          case "tool_call_update":
          case "tool_result": {
            const update = (ev.payload_json as any)?.update ?? ev.payload_json ?? {};
            const toolCallId =
              String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
            if (!toolCallId) {
              debugEvents.push(ev);
              break;
            }
            const tool = ensureTool(g, toolCallId, ev.created_at);
            tool.updated_at = ev.created_at;
            tool.updates_seen += 1;
            tool.raw = ev.payload_json;

            const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
            if (nextKind) tool.tool_kind = nextKind;

            const nextTitle = String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
            if (nextTitle) tool.title = nextTitle;
            else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

            const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
            if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
            else if (ev.event_type === "tool_result") tool.status = "completed";

            const locs = Array.isArray(update?.locations) ? update.locations : [];
            tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

            const rawInput =
              update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? null;
            if (rawInput != null) tool.input = rawInput;

            const output =
              update?.outputText ??
              update?.output_text ??
              update?.toolCall?.outputText ??
              update?.toolCall?.output_text ??
              update?.result ??
              null;
            if (typeof output === "string") tool.output_text = output;

            break;
          }
          default: {
            break;
          }
        }
      }

      const items: ThreadItem[] = [];
      const activityItems = [...g.toolItems, ...askItems, ...noticeItems];
      activityItems.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
      items.push(...activityItems);
      if (g.assistant?.thought.trim()) {
        const thoughtContent = g.assistant.thought;
        if (thoughtContent.trim()) {
          items.push({
            kind: "thought",
            id: `thought-${g.key}`,
            turn_id: g.key,
            created_at: g.thought_first_at ?? g.assistant_first_at ?? g.first_at,
            content: thoughtContent,
          });
        }
      }
      if (g.assistant) {
        items.push({
          ...g.assistant,
          thought_seconds: (() => {
            if (!g.thought_first_at) return undefined;
            const start = Date.parse(g.thought_first_at);
            const endRaw = g.assistant_first_at ?? g.assistant_complete_at ?? g.thought_last_at;
            if (!endRaw) return undefined;
            const end = Date.parse(endRaw);
            if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 1;
            return Math.max(1, Math.round((end - start) / 1000));
          })(),
        });
      }
      if (items.length === 0) {
        items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
      }

      if (!Number.isFinite(userOrderSeq)) {
        continue;
      }
      groups.push({ sort_seq: userOrderSeq as number, group: { key: g.key, header: g.header, items } });
    }

    return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
  }

  const eventsInRange = (startIso: string, endIso: string | null) => {
    const start = Date.parse(startIso);
    const end = endIso ? Date.parse(endIso) : Number.POSITIVE_INFINITY;
    return events.filter((e) => {
      const t = Date.parse(String(e.created_at));
      return Number.isFinite(t) && t >= start && t <= end;
    });
  };

  const groups: SortableThreadGroup[] = [];

  for (let i = 0; i < userMessages.length; i++) {
    const u = userMessages[i];
    const nextUser = userMessages[i + 1] ?? null;
    const userOrderSeq = Number(u.order_seq ?? Number.NaN);

    const mid = idToString(u.id);
    if (!mid) {
      if (import.meta.env.DEV) {
        // eslint-disable-next-line no-console
        console.error("[WorkbenchThreadViewModel] user message missing id (events-only view)", {
          created_at: u.created_at ?? null,
          order_seq: u.order_seq ?? null,
        });
      }
      continue;
    }
    const g: TurnGroup = {
      key: `m-${mid}`,
      header: {
        id: mid,
        content: u.content ?? "",
        attachments: Array.isArray((u as any).attachments) ? ((u as any).attachments as MessageAttachment[]) : [],
        created_at: u.created_at,
      },
      first_at: u.created_at,
      toolItems: [],
      toolById: new Map(),
      assistant: null,
      thought_first_at: null,
      thought_last_at: null,
      thought_is_crp: false,
      assistant_first_at: null,
      assistant_complete_at: null,
    };

    const nextUserOrderSeq = nextUser ? Number(nextUser.order_seq ?? Number.NaN) : Number.NaN;
    const assistant = assistantMessages.find((a) => {
      const aSeq = Number(a.order_seq ?? Number.NaN);
      if (!Number.isFinite(userOrderSeq) || !Number.isFinite(aSeq)) return false;
      if (aSeq <= (userOrderSeq as number)) return false;
      if (!nextUser) return true;
      if (Number.isFinite(nextUserOrderSeq)) return aSeq < (nextUserOrderSeq as number);
      return true;
    });

    const endAt = assistant?.created_at ?? nextUser?.created_at ?? null;
    const evs = eventsInRange(u.created_at, endAt);
    const askItems: Array<Extract<ThreadItem, { kind: "ask_user_question" }>> = [];
    const askInserted = new Set<string>();
    const noticeItems: Array<Extract<ThreadItem, { kind: "message" }>> = [];
    const noticeInserted = new Set<string>();

    for (const ev of evs) {
      const eventId = idToString(ev.id);
      if (!eventId) {
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.error("[WorkbenchThreadViewModel] event missing id (events-only view)", {
            created_at: ev.created_at,
            event_type: ev.event_type,
          });
        }
        continue;
      }
      if (ev.created_at < g.first_at) g.first_at = ev.created_at;

      switch (ev.event_type) {
        case "notice": {
          const askItem = buildAskUserQuestionItem(ev, g.key, askUserQuestionAnswers);
          if (askItem && !askInserted.has(askItem.tool_call_id)) {
            askItems.push(askItem);
            askInserted.add(askItem.tool_call_id);
          }
          const noticeItem = buildNoticeMessageItem(ev, g.key);
          if (noticeItem && !noticeInserted.has(noticeItem.id)) {
            noticeItems.push(noticeItem);
            noticeInserted.add(noticeItem.id);
          }
          break;
        }
        case "error": {
          const message = extractErrorMessage(ev.payload_json) ?? "Error";
          const provider = String(ev.payload_json?.provider ?? "").trim();
          const output = provider ? `${message}\nprovider: ${provider}` : message;
          const tool = ensureTool(g, `error-${eventId}`, ev.created_at);
          tool.tool_kind = "error";
          tool.title = "Error";
          tool.status = "failed";
          tool.updated_at = ev.created_at;
          tool.updates_seen += 1;
          tool.input = ev.payload_json ?? null;
          tool.output_text = output;
          tool.raw = ev;
          break;
        }
        case "assistant_chunk": {
          if (!shouldRenderAssistantChunk(ev)) break;
          const fragment = String(ev.payload_json?.content_fragment ?? "");
          if (!fragment) break;
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              turn_id: g.key,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.assistant_first_at = g.assistant_first_at ?? ev.created_at;
          g.assistant.content += fragment;
          break;
        }
        case "assistant_complete": {
          const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              turn_id: g.key,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.assistant_complete_at = ev.created_at;
          if (full) g.assistant.content = full;
          g.assistant.is_complete = true;
          break;
        }
        case "thought_chunk": {
          if (!shouldRenderThoughtChunk(ev)) break;
          const fragment = String(ev.payload_json?.content_fragment ?? "");
          if (!fragment) break;
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              turn_id: g.key,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.thought_first_at = g.thought_first_at ?? ev.created_at;
          g.thought_last_at = ev.created_at;
          g.assistant.thought = appendStreamingFragment(g.assistant.thought, fragment);
          if (isCrpThoughtEvent(ev)) g.thought_is_crp = true;
          break;
        }
        case "tool_call":
        case "tool_call_update":
        case "tool_result": {
          const update = (ev.payload_json as any)?.update ?? ev.payload_json ?? {};
          const toolCallId =
            String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
          if (!toolCallId) {
            debugEvents.push(ev);
            break;
          }
          const tool = ensureTool(g, toolCallId, ev.created_at);
          tool.updated_at = ev.created_at;
          tool.updates_seen += 1;
          tool.raw = ev.payload_json;

          const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
          if (nextKind) tool.tool_kind = nextKind;

          const nextTitle = String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
          if (nextTitle) tool.title = nextTitle;
          else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

          const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
          if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
          else if (ev.event_type === "tool_result") tool.status = "completed";

          const locs = Array.isArray(update?.locations) ? update.locations : [];
          tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

          const rawInput =
            update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? null;
          if (rawInput != null) tool.input = rawInput;

          const output =
            update?.outputText ??
            update?.output_text ??
            update?.toolCall?.outputText ??
            update?.toolCall?.output_text ??
            update?.result ??
            null;
          if (typeof output === "string") tool.output_text = output;

          break;
        }
        default: {
          break;
        }
      }
    }

    if (!g.assistant && assistant) {
      g.assistant = {
        kind: "assistant",
        id: `assistant-${g.key}`,
        turn_id: g.key,
        created_at: assistant.created_at,
        content: assistant.content ?? "",
        thought: "",
        is_complete: true,
      };
      g.assistant_complete_at = assistant.created_at;
    } else if (g.assistant && assistant && !g.assistant.content) {
      g.assistant.content = assistant.content ?? "";
      g.assistant.is_complete = true;
      g.assistant_complete_at = assistant.created_at;
    }

    const items: ThreadItem[] = [];
    const activityItems = [...g.toolItems, ...askItems, ...noticeItems];
    activityItems.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
    items.push(...activityItems);
    if (g.assistant?.thought.trim()) {
      const thoughtContent = g.assistant.thought;
      if (thoughtContent.trim()) {
        items.push({
          kind: "thought",
          id: `thought-${g.key}`,
          turn_id: g.key,
          created_at: g.thought_first_at ?? g.assistant_first_at ?? g.first_at,
          content: thoughtContent,
        });
      }
    }
    if (g.assistant) {
      items.push({
        ...g.assistant,
        thought_seconds: (() => {
          if (!g.thought_first_at) return undefined;
          const start = Date.parse(g.thought_first_at);
          const endRaw = g.assistant_first_at ?? g.assistant_complete_at ?? g.thought_last_at;
          if (!endRaw) return undefined;
          const end = Date.parse(endRaw);
          if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 1;
          return Math.max(1, Math.round((end - start) / 1000));
        })(),
      });
    }
    if (items.length === 0) {
      items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
    }

    if (!Number.isFinite(userOrderSeq)) {
      continue;
    }
    groups.push({ sort_seq: userOrderSeq as number, group: { key: g.key, header: g.header, items } });
  }

  // If there are no user messages (should be rare), fall back to an event-only group.
  if (groups.length === 0) {
    const g: TurnGroup = {
      key: "no-user-messages",
      header: null,
      first_at: events[0]?.created_at ?? new Date().toISOString(),
      toolItems: [],
      toolById: new Map(),
      assistant: null,
      thought_first_at: null,
      thought_last_at: null,
      thought_is_crp: false,
      assistant_first_at: null,
      assistant_complete_at: null,
    };
    const askItems: Array<Extract<ThreadItem, { kind: "ask_user_question" }>> = [];
    const askInserted = new Set<string>();
    const noticeItems: Array<Extract<ThreadItem, { kind: "message" }>> = [];
    const noticeInserted = new Set<string>();
    for (const ev of events) {
      if (ev.event_type === "notice") {
        const askItem = buildAskUserQuestionItem(ev, g.key, askUserQuestionAnswers);
        if (askItem && !askInserted.has(askItem.tool_call_id)) {
          askItems.push(askItem);
          askInserted.add(askItem.tool_call_id);
        }
        const noticeItem = buildNoticeMessageItem(ev, g.key);
        if (noticeItem && !noticeInserted.has(noticeItem.id)) {
          noticeItems.push(noticeItem);
          noticeInserted.add(noticeItem.id);
        }
      }
      const update = (ev.payload_json as any)?.update ?? ev.payload_json ?? {};
      const toolCallId =
        String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
      if (!toolCallId) continue;
      ensureTool(g, toolCallId, ev.created_at);
    }
    const activityItems = [...g.toolItems, ...askItems, ...noticeItems];
    activityItems.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
    const items: ThreadItem[] = [...activityItems];
    if (items.length === 0) items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
    const groupOrderSeq = events
      .map((ev) => readEventOrderSeq(ev))
      .filter((seq): seq is number => Number.isFinite(seq))
      .reduce((min, seq) => Math.min(min, seq), Number.POSITIVE_INFINITY);
    if (Number.isFinite(groupOrderSeq)) {
      groups.push({ sort_seq: groupOrderSeq, group: { key: g.key, header: g.header, items } });
    }
  }

  return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
}

function extractToolOutputText(update: any): string {
  const direct =
    update?.outputText ??
    update?.output_text ??
    update?.output_preview ??
    update?.result ??
    update?.rawOutput?.aggregated_output ??
    update?.rawOutput?.output ??
    null;
  if (typeof direct === "string" && direct.trim()) return direct.trim();

  const blocks = Array.isArray(update?.content) ? update.content : [];
  const parts: string[] = [];
  for (const b of blocks) {
    const c = b?.content ?? b;
    const t = c?.text;
    if (typeof t === "string") parts.push(t);
  }
  return parts.join("").trim();
}

function mergeStreamingText(prev: string, next: string): string {
  const p = prev ?? "";
  const n = next ?? "";
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
}

function pickFirstString(...values: any[]): string | null {
  for (const v of values) {
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return null;
}

function isNonToolStatus(value: string): boolean {
  const s = value.trim().toLowerCase();
  return ![
    "pending",
    "queued",
    "running",
    "in_progress",
    "completed",
    "failed",
    "error",
    "ok",
    "success",
    "succeeded",
  ].includes(s);
}

function formatElapsedSeconds(totalSeconds: number): string {
  const clamped = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(clamped / 3600);
  const minutes = Math.floor((clamped % 3600) / 60);
  const seconds = clamped % 60;
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function isStatusUpdateMeta(meta: any): boolean {
  if (!meta || typeof meta !== "object") return false;
  const codexMeta = meta?.codex ?? {};
  const reasoningKind = codexMeta?.reasoning_kind ?? codexMeta?.reasoningKind;
  if (reasoningKind === "status") return true;

  const statusText = pickFirstString(
    meta?.status_text,
    meta?.statusText,
    meta?.status_string,
    meta?.statusString,
    codexMeta?.status_text,
    codexMeta?.statusText,
    codexMeta?.status_string,
    codexMeta?.statusString,
  );
  if (statusText) return true;

  const statusValue =
    typeof meta?.status === "string"
      ? meta.status
      : typeof codexMeta?.status === "string"
        ? codexMeta.status
        : null;
  if (statusValue && isNonToolStatus(statusValue)) return true;

  return false;
}

function normalizeToolStatus(status: string, eventType: string): string {
  const s = status.toLowerCase();
  if (s === "inprogress") return "in_progress";
  if (s === "in_progress") return "in_progress";
  if (s === "running") return "in_progress";
  if (s === "pending" || s === "queued") return "pending";
  if (s === "completed" || s === "complete" || s === "ok" || s === "succeeded") return "completed";
  if (s === "failed" || s === "error") return "failed";
  if (eventType === "tool_result") return "completed";
  return s || "pending";
}

function mergeEvents(prev: SessionEvent[], incoming: SessionEvent[]): SessionEvent[] {
  const map = new Map<string, SessionEvent>();
  for (const ev of prev) {
    const id = idToString(ev.id);
    if (!id) {
      if (import.meta.env.DEV) {
        // eslint-disable-next-line no-console
        console.error("[WorkbenchThreadViewModel] event missing id (mergeEvents)", {
          created_at: ev.created_at,
          event_type: ev.event_type,
        });
      }
      continue;
    }
    map.set(id, ev);
  }
  for (const ev of incoming) {
    const id = idToString(ev.id);
    if (!id) {
      if (import.meta.env.DEV) {
        // eslint-disable-next-line no-console
        console.error("[WorkbenchThreadViewModel] event missing id (mergeEvents)", {
          created_at: ev.created_at,
          event_type: ev.event_type,
        });
      }
      continue;
    }
    map.set(id, ev);
  }
  return [...map.values()].sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
}

type AuthMethodOption = { id: string; name: string };
type SessionErrorInfo = { message: string; provider?: string };
type ProviderGuardNotice = {
  kind: "provider_guard_warning" | "provider_guard_kill";
  stage: string;
  provider?: string;
  message?: string;
  pid?: number;
  memoryMb?: number | null;
  limitHighMb?: number | null;
  limitMaxMb?: number | null;
  systemTotalMb?: number | null;
  systemUsedMb?: number | null;
  gracePeriodMs?: number | null;
  killAtMs?: number | null;
  createdAtMs?: number | null;
};

type AuthUi = {
  status: "unknown" | "required" | "failed" | "authenticated";
  provider?: string;
  message?: string;
  methods: AuthMethodOption[];
};

export function deriveAuthUi(events: SessionEvent[]): AuthUi {
  const fromMethodsValue = (value: any): AuthMethodOption[] => {
    const list = Array.isArray(value) ? value : [];
    return list
      .map((m: any) => ({
        id: m?.methodId ?? m?.method_id ?? m?.id,
        name: m?.name ?? m?.label ?? (m?.methodId ?? m?.method_id ?? m?.id),
      }))
      .filter((m: any) => typeof m.id === "string" && m.id.length > 0)
      .map((m: any) => ({ id: String(m.id), name: String(m.name ?? m.id) }));
  };

  let status: AuthUi["status"] = "unknown";
  let provider: string | undefined;
  let message: string | undefined;
  let methods: AuthMethodOption[] = [];

  const lastInit = [...events].reverse().find((e) => e.event_type === "init");
  const initMethods =
    lastInit?.payload_json?.auth_methods ??
    lastInit?.payload_json?.authMethods ??
    lastInit?.payload_json?.auth_methods;
  const initMethodOptions = fromMethodsValue(initMethods);

  for (const ev of events) {
    if (ev.event_type === "auth_required") {
      status = "required";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
      methods = fromMethodsValue(ev.payload_json?.auth_methods ?? ev.payload_json?.authMethods);
      continue;
    }

    if (ev.event_type !== "notice") continue;
    const kind = ev.payload_json?.kind;
    if (kind === "auth_required") {
      status = "required";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
      methods = fromMethodsValue(ev.payload_json?.auth_methods ?? ev.payload_json?.authMethods);
    }
    if (kind === "auth_failed") {
      status = "failed";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
    }
    if (kind === "auth_finished") {
      status = "authenticated";
      provider = ev.payload_json?.provider;
      message = undefined;
      methods = [];
    }
  }

  if ((status === "required" || status === "failed") && methods.length === 0) {
    methods = initMethodOptions;
  }

  return { status, provider, message, methods };
}

export function deriveProviderGuardNotice(events: SessionEvent[]): ProviderGuardNotice | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const ev = events[i];
    if (ev.event_type !== "notice") continue;
    const payload = ev.payload_json ?? {};
    const kind = String(payload.kind ?? "").trim();
    if (kind !== "provider_guard_warning" && kind !== "provider_guard_kill") continue;
    return {
      kind,
      stage: String(payload.stage ?? "").trim(),
      provider: typeof payload.provider === "string" ? payload.provider : undefined,
      message: typeof payload.message === "string" ? payload.message : undefined,
      pid: coerceNumber(payload.pid) ?? undefined,
      memoryMb: coerceNumber(payload.memory_mb),
      limitHighMb: coerceNumber(payload.limit_high_mb),
      limitMaxMb: coerceNumber(payload.limit_max_mb),
      systemTotalMb: coerceNumber(payload.system_total_mb),
      systemUsedMb: coerceNumber(payload.system_used_mb),
      gracePeriodMs: coerceNumber(payload.grace_period_ms),
      killAtMs: coerceNumber(payload.kill_at_ms),
      createdAtMs: parseIsoMs(ev.created_at),
    };
  }
  return null;
}

function readNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const text = value.trim();
  return text ? text : null;
}

function extractErrorDetails(payload: any): string | null {
  if (!payload) return null;
  const direct =
    readNonEmptyString(payload.details) ??
    readNonEmptyString(payload.detail) ??
    readNonEmptyString(payload.additional_details) ??
    readNonEmptyString(payload.additionalDetails);
  if (direct) return direct;

  const codexInfo = payload.codex_error_info ?? payload.codexErrorInfo;
  const codexText = extractErrorMessageFromObject(codexInfo);
  if (codexText) return codexText;

  const kind = readNonEmptyString(payload.kind);
  return kind;
}

function extractErrorMessage(payload: any): string | null {
  if (!payload) return null;
  const details = extractErrorDetails(payload);
  const direct =
    readNonEmptyString(payload.message) ??
    readNonEmptyString(payload.error) ??
    readNonEmptyString(payload.error_message) ??
    readNonEmptyString(payload.errorMessage);
  if (direct) {
    if (details && !direct.includes(details)) {
      return `${direct}\nDetails: ${details}`;
    }
    return direct;
  }

  const update = payload.update ?? payload;
  const updateText = extractErrorMessageFromObject(update);
  if (updateText) {
    if (details && !updateText.includes(details)) {
      return `${updateText}\nDetails: ${details}`;
    }
    return updateText;
  }

  const meta = update?._meta ?? update?.meta ?? payload._meta ?? payload.meta ?? null;
  const metaText =
    readNonEmptyString(meta?.statusText) ??
    readNonEmptyString(meta?.status_text) ??
    readNonEmptyString(meta?.message) ??
    readNonEmptyString(meta?.error);
  if (metaText) {
    if (details && !metaText.includes(details)) {
      return `${metaText}\nDetails: ${details}`;
    }
    return metaText;
  }
  return details;
}

function extractErrorMessageFromObject(value: any): string | null {
  if (!value) return null;
  if (typeof value === "string") return readNonEmptyString(value);
  if (typeof value !== "object") return null;

  const direct =
    readNonEmptyString(value.message) ??
    readNonEmptyString(value.error_message) ??
    readNonEmptyString(value.errorMessage);
  if (direct) return direct;

  const data = value.data ?? value.details ?? value.detail;
  const dataText =
    readNonEmptyString(data) ??
    readNonEmptyString(data?.message) ??
    readNonEmptyString(data?.error);
  if (dataText) return dataText;

  const nested = value.error ?? value.cause;
  const nestedText =
    typeof nested === "object"
      ? extractErrorMessageFromObject(nested)
      : readNonEmptyString(nested);
  if (nestedText) return nestedText;

  const meta = value._meta ?? value.meta;
  return readNonEmptyString(meta?.statusText) ?? readNonEmptyString(meta?.status_text);
}

export function deriveSessionError(
  turns: SessionTurn[],
  events: SessionEvent[],
): SessionErrorInfo | null {
  if (turns.length === 0) return null;
  const lastTurn = turns[turns.length - 1];
  if (lastTurn.status !== "failed") return null;
  const turnId = idToString(lastTurn.turn_id);
  let errorEvent: SessionEvent | null = null;
  for (let i = events.length - 1; i >= 0; i--) {
    const ev = events[i];
    if (ev.event_type !== "error") continue;
    if (turnId && idToString(ev.turn_id) !== turnId) continue;
    errorEvent = ev;
    break;
  }
  if (!errorEvent) {
    return { message: "Harness error." };
  }
  const message = extractErrorMessage(errorEvent.payload_json) ?? "Harness error.";
  const provider =
    readNonEmptyString(errorEvent.payload_json?.provider) ??
    readNonEmptyString(errorEvent.payload_json?.provider_id) ??
    readNonEmptyString(errorEvent.payload_json?.providerId) ??
    undefined;
  return { message, provider };
}
