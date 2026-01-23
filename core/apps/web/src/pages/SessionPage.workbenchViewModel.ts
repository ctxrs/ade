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

function buildCustomStatusByTurnId(events: SessionEvent[]): Map<string, string> {
  const normalize = (value: unknown): string | null => {
    const t = String(value ?? "").trim();
    return t ? t : null;
  };

  const extractNoticeStatusText = (ev: SessionEvent): string | null => {
    if (ev.event_type !== "notice") return null;
    const payload = ev.payload_json ?? {};
    const meta =
      payload?.acp_update?._meta ??
      payload?.acp_update?.meta ??
      payload?._meta ??
      payload?.meta ??
      {};
    return (
      normalize(meta?.statusText) ??
      normalize(meta?.status_text) ??
      normalize(payload?.statusText) ??
      normalize(payload?.status_text)
    );
  };

  const toolStatusVerb = (kind: string): string | null => {
    const k = String(kind ?? "").trim().toLowerCase();
    if (k === "search") return "Searching";
    if (k === "read" || k === "read_file") return "Reading";
    if (k === "execute") return "Running";
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
      const sa = typeof (a as any).seq === "number" ? ((a as any).seq as number) : Number.NaN;
      const sb = typeof (b as any).seq === "number" ? ((b as any).seq as number) : Number.NaN;
      if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
      if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
      if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
      return String(a.created_at).localeCompare(String(b.created_at));
    });

  const noticeByTurn = new Map<string, { order: number; text: string }>();
  const toolsByTurn = new Map<string, Map<string, { order: number; status: string; text: string | null }>>();

  let order = 0;
  for (const ev of sorted) {
    order += 1;
    const turnId = idToString((ev as any).turn_id);
    if (!turnId) continue;

    const noticeText = extractNoticeStatusText(ev);
    if (noticeText) {
      noticeByTurn.set(turnId, { order, text: noticeText });
      continue;
    }

    if (ev.event_type !== "tool_call" && ev.event_type !== "tool_call_update" && ev.event_type !== "tool_result") {
      continue;
    }

    const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
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
  const allTurnIds = new Set<string>([...noticeByTurn.keys(), ...toolsByTurn.keys()]);
  for (const turnId of allTurnIds) {
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

export function mergeMessagesForView(messages: Message[], pending: PendingMessageEntry[]): Message[] {
  if (pending.length === 0) return messages;
  const byId = new Map<string, Message>();
  for (const m of messages) {
    const id = idToString(m.id);
    if (id) byId.set(id, m);
  }
  for (const entry of pending) {
    const id = idToString(entry.message.id);
    if (!id || byId.has(id)) continue;
    byId.set(id, entry.message);
  }
  return Array.from(byId.values()).sort(compareMessageOrder);
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
  sort_at: string;
  group: WorkbenchThreadView["groups"][number];
};

function buildSystemMessageGroups(messages: Message[]): SortableThreadGroup[] {
  const systemMessages = messages
    .filter((m) => m.role === "system")
    .slice()
    .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
  return systemMessages.map((m, idx) => {
    const id = idToString(m.id) || `system-${idx}`;
    const attachments = Array.isArray((m as any).attachments)
      ? ((m as any).attachments as MessageAttachment[])
      : [];
    return {
      sort_at: m.created_at,
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
    };
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
    const cmp = String(a.sort_at).localeCompare(String(b.sort_at));
    if (cmp !== 0) return cmp;
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
  const meta =
    payload?.acp_update?._meta ??
    payload?.acp_update?.meta ??
    payload?._meta ??
    payload?.meta ??
    {};
  if (meta?.heartbeat === true) return false;
  if (isStatusUpdateMeta(meta)) return false;
  const reasoningKind = meta?.codex?.reasoning_kind ?? meta?.codex?.reasoningKind;
  if (reasoningKind === "summary") return false;
  return true;
}

type ActivityEntry = {
  item: ThreadItem;
  created_at: string;
  kind: "tool" | "thought" | "ask_user_question";
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

  const nextTitle = String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
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

  const activity: ActivityEntry[] = [];
  const toolInserted = new Set<string>();
  const askInserted = new Set<string>();

  for (const ev of opts.events) {
    if (ev.event_type === "notice") {
      const askItem = buildAskUserQuestionItem(ev, opts.turnId, opts.askUserQuestionAnswers);
      if (askItem && !askInserted.has(askItem.tool_call_id)) {
        activity.push({
          item: askItem,
          created_at: ev.created_at,
          kind: "ask_user_question",
          order_seq: typeof ev.seq === "number" ? ev.seq : undefined,
        });
        askInserted.add(askItem.tool_call_id);
      }
    }

    if (ev.event_type === "thought_chunk") {
      if (!shouldRenderThoughtChunk(ev)) {
        continue;
      }
      const fragment = String(ev.payload_json?.content_fragment ?? "");
      if (!fragment) {
        continue;
      }
      const last = activity[activity.length - 1];
      if (last && last.item.kind === "thought") {
        (last.item as Extract<ThreadItem, { kind: "thought" }>).content = mergeStreamingText(
          (last.item as Extract<ThreadItem, { kind: "thought" }>).content,
          fragment,
        );
        continue;
      }
      const thoughtItem: Extract<ThreadItem, { kind: "thought" }> = {
        kind: "thought",
        id: `thought-${opts.turnId}-${ev.seq ?? ev.created_at}`,
        turn_id: opts.turnId,
        created_at: ev.created_at,
        content: fragment,
      };
      activity.push({
        item: thoughtItem,
        created_at: ev.created_at,
        kind: "thought",
        order_seq: typeof ev.seq === "number" ? ev.seq : undefined,
      });
      continue;
    }

    if (ev.event_type === "tool_call" || ev.event_type === "tool_call_update" || ev.event_type === "tool_result") {
      const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
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
          order_seq: typeof ev.seq === "number" ? ev.seq : undefined,
        });
        toolInserted.add(toolCallId);
      }
      continue;
    }
  }

  const fallbackThought = String(opts.turn.thought_partial ?? "").trim();
  if (fallbackThought && !activity.some((entry) => entry.item.kind === "thought")) {
    const fallbackItem: Extract<ThreadItem, { kind: "thought" }> = {
      kind: "thought",
      id: `thought-${opts.turnId}-fallback`,
      turn_id: opts.turnId,
      created_at: opts.turn.updated_at ?? opts.turn.started_at,
      content: fallbackThought,
    };
    activity.push({
      item: fallbackItem,
      created_at: fallbackItem.created_at,
      kind: "thought",
    });
  }

  const remainingTools = Array.from(toolById.values()).filter((tool) => !toolInserted.has(tool.tool_call_id));
  remainingTools.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
  for (const tool of remainingTools) {
    activity.push({ item: tool, created_at: tool.created_at, kind: "tool" });
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

  for (const turn of turns) {
    const turnId = idToString(turn.turn_id) || `turn-${turn.started_at}`;
    const userMessageId = turn.user_message_id ? idToString(turn.user_message_id) : "";

    const userMessage = userMessageId ? messageById.get(userMessageId) : undefined;

    const header: WorkbenchTurnHeader | null = userMessage
      ? {
        id: userMessageId || turnId,
        content: userMessage.content ?? "",
        attachments: Array.isArray((userMessage as any).attachments)
          ? ((userMessage as any).attachments as MessageAttachment[])
          : [],
        created_at: userMessage.created_at,
      }
      : null;

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

    const { activity } = buildTurnActivityTimeline({
      turnId,
      turn,
      tools,
      events: eventsByTurnId.get(turnId) ?? [],
      askUserQuestionAnswers,
    });

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
      timeline.push({
        item: {
          kind: "assistant",
          id: `assistant-${turnId}-${m.turn_sequence ?? m.created_at}`,
          turn_id: turnId,
          created_at: m.created_at,
          content: m.content ?? "",
          thought: "",
          is_complete: true,
        },
        created_at: m.created_at,
        kind: "assistant",
        turn_sequence: Number(m.turn_sequence ?? Number.NaN),
      });
    }

    const pendingContent = String(turn.assistant_partial ?? "");
    if (pendingContent.trim().length > 0) {
      timeline.push({
        item: {
          kind: "assistant",
          id: `assistant-${turnId}-pending`,
          turn_id: turnId,
          created_at: turn.updated_at ?? turn.started_at,
          content: pendingContent,
          thought: "",
          is_complete: false,
        },
        created_at: turn.updated_at ?? turn.started_at,
        kind: "assistant",
        turn_sequence: Number.MAX_SAFE_INTEGER,
      });
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
      if (Number.isFinite(aSeq) && Number.isFinite(bSeq) && aSeq !== bSeq) {
        return (aSeq ?? 0) - (bSeq ?? 0);
      }

      const tcmp = String(a.created_at).localeCompare(String(b.created_at));
      if (tcmp !== 0) return tcmp;
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

    const statusText = customStatusByTurnId.get(turnId) ?? null;
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

    groups.push({
      sort_at: header?.created_at ?? turn.started_at,
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
    assistant_first_at: string | null;
    assistant_complete_at: string | null;
  };

  const debugEvents: SessionEvent[] = [];

  const userMessages = messages
    .filter((m) => m.role === "user")
    .slice()
    .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

  const assistantMessages = messages
    .filter((m) => m.role === "assistant")
    .slice()
    .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

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
      .slice()
      .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

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
        assistant_first_at: null,
        assistant_complete_at: null,
      };
      let thought = "";
      let thoughtAt: string | null = null;
      const askItems: Array<Extract<ThreadItem, { kind: "ask_user_question" }>> = [];
      const askInserted = new Set<string>();
      for (const ev of events) {
        if (ev.event_type === "notice") {
          const askItem = buildAskUserQuestionItem(ev, g.key, askUserQuestionAnswers);
          if (askItem && !askInserted.has(askItem.tool_call_id)) {
            askItems.push(askItem);
            askInserted.add(askItem.tool_call_id);
          }
        }
        if (ev.event_type === "thought_chunk") {
          const fragment = String(ev.payload_json?.content_fragment ?? "");
          if (fragment) {
            thought += fragment;
            thoughtAt = thoughtAt ?? ev.created_at;
          }
        }
        const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
        const toolCallId =
          String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
        if (!toolCallId) continue;
        ensureTool(g, toolCallId, ev.created_at);
      }
      const items: ThreadItem[] = [];
      if (thought.trim()) {
        items.push({
          kind: "thought",
          id: `thought-${g.key}`,
          turn_id: g.key,
          created_at: thoughtAt ?? g.first_at,
          content: thought,
        });
      }
      const activityItems = [...g.toolItems, ...askItems];
      activityItems.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
      items.push(...activityItems);
      if (items.length === 0) items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
      groups.push({ sort_at: g.first_at, group: { key: g.key, header: g.header, items } });
      return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
    }

    for (let i = 0; i < userEvents.length; i++) {
      const u = userEvents[i];
      const nextUser = userEvents[i + 1] ?? null;
      const mid =
        String(u.payload_json?.message_id ?? "").trim() ||
        (idToString(u.id) || `msg-${u.created_at}`);

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
        assistant_first_at: null,
        assistant_complete_at: null,
      };

      const evs = eventsInRangeExclusive(u.created_at, nextUser?.created_at ?? null);
      const askItems: Array<Extract<ThreadItem, { kind: "ask_user_question" }>> = [];
      const askInserted = new Set<string>();

      for (const ev of evs) {
        const eventId = idToString(ev.id) || `${ev.created_at}`;
        if (ev.created_at < g.first_at) g.first_at = ev.created_at;

        switch (ev.event_type) {
          case "notice": {
            const askItem = buildAskUserQuestionItem(ev, g.key, askUserQuestionAnswers);
            if (askItem && !askInserted.has(askItem.tool_call_id)) {
              askItems.push(askItem);
              askInserted.add(askItem.tool_call_id);
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
            const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
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
      const activityItems = [...g.toolItems, ...askItems];
      activityItems.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
      items.push(...activityItems);
      if (g.assistant?.thought.trim()) {
        items.push({
          kind: "thought",
          id: `thought-${g.key}`,
          turn_id: g.key,
          created_at: g.thought_first_at ?? g.assistant_first_at ?? g.first_at,
          content: g.assistant.thought,
        });
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

      groups.push({ sort_at: g.first_at, group: { key: g.key, header: g.header, items } });
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

    const mid = idToString(u.id) || `msg-${u.created_at}`;
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
      assistant_first_at: null,
      assistant_complete_at: null,
    };

    const assistant = assistantMessages.find((a) => {
      const ta = Date.parse(String(a.created_at));
      const tu = Date.parse(String(u.created_at));
      if (!Number.isFinite(ta) || !Number.isFinite(tu) || ta <= tu) return false;
      if (!nextUser) return true;
      const tn = Date.parse(String(nextUser.created_at));
      return !Number.isFinite(tn) || ta < tn;
    });

    const endAt = assistant?.created_at ?? nextUser?.created_at ?? null;
    const evs = eventsInRange(u.created_at, endAt);
    const askItems: Array<Extract<ThreadItem, { kind: "ask_user_question" }>> = [];
    const askInserted = new Set<string>();

    for (const ev of evs) {
      const eventId = idToString(ev.id) || `${ev.created_at}`;
      if (ev.created_at < g.first_at) g.first_at = ev.created_at;

      switch (ev.event_type) {
        case "notice": {
          const askItem = buildAskUserQuestionItem(ev, g.key, askUserQuestionAnswers);
          if (askItem && !askInserted.has(askItem.tool_call_id)) {
            askItems.push(askItem);
            askInserted.add(askItem.tool_call_id);
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
          const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
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
    const activityItems = [...g.toolItems, ...askItems];
    activityItems.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
    items.push(...activityItems);
    if (g.assistant?.thought.trim()) {
      items.push({
        kind: "thought",
        id: `thought-${g.key}`,
        turn_id: g.key,
        created_at: g.thought_first_at ?? g.assistant_first_at ?? g.first_at,
        content: g.assistant.thought,
      });
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

    groups.push({ sort_at: g.first_at, group: { key: g.key, header: g.header, items } });
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
      assistant_first_at: null,
      assistant_complete_at: null,
    };
    const askItems: Array<Extract<ThreadItem, { kind: "ask_user_question" }>> = [];
    const askInserted = new Set<string>();
    for (const ev of events) {
      if (ev.event_type === "notice") {
        const askItem = buildAskUserQuestionItem(ev, g.key, askUserQuestionAnswers);
        if (askItem && !askInserted.has(askItem.tool_call_id)) {
          askItems.push(askItem);
          askInserted.add(askItem.tool_call_id);
        }
      }
      const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
      const toolCallId =
        String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
      if (!toolCallId) continue;
      ensureTool(g, toolCallId, ev.created_at);
    }
    const activityItems = [...g.toolItems, ...askItems];
    activityItems.sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
    const items: ThreadItem[] = [...activityItems];
    if (items.length === 0) items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
    groups.push({ sort_at: g.first_at, group: { key: g.key, header: g.header, items } });
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
    const key = idToString(ev.id) || `${ev.created_at}-${ev.event_type}`;
    map.set(key, ev);
  }
  for (const ev of incoming) {
    const key = idToString(ev.id) || `${ev.created_at}-${ev.event_type}`;
    map.set(key, ev);
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

function extractErrorMessage(payload: any): string | null {
  if (!payload) return null;
  const direct =
    readNonEmptyString(payload.message) ??
    readNonEmptyString(payload.error) ??
    readNonEmptyString(payload.error_message) ??
    readNonEmptyString(payload.errorMessage);
  if (direct) return direct;

  const acpError = payload.acp_error ?? payload.acpError;
  if (acpError && typeof acpError === "object") {
    const acpDataText = extractErrorMessageFromObject((acpError as any).data);
    if (acpDataText) return acpDataText;
  }
  const acpErrorText = extractErrorMessageFromObject(acpError);
  if (acpErrorText) return acpErrorText;

  const update = payload.acp_update ?? payload.acpUpdate ?? payload.update;
  const updateText = extractErrorMessageFromObject(update);
  if (updateText) return updateText;

  const meta = update?._meta ?? update?.meta ?? payload._meta ?? payload.meta ?? null;
  return (
    readNonEmptyString(meta?.statusText) ??
    readNonEmptyString(meta?.status_text) ??
    readNonEmptyString(meta?.message) ??
    readNonEmptyString(meta?.error)
  );
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
