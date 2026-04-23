import {
  idToString,
  recordClientCounterMetric,
  type Message,
  type SessionEvent,
  type SessionTurn,
  type SessionTurnTool,
} from "../../api/client";
import type {
  AskUserQuestionAnswerState,
  ThreadItem,
  WorkbenchThreadView,
  WorkbenchTurnHeader,
} from "../sessionView/SessionPage.types";
import type { AssistantStreamingState } from "../../state/assistantStreaming";
import { humanToolKind, isPlaceholderToolLabel, normalizeDisplayToolLabel, toolDisplayTitleFromPayload } from "../sessionView/SessionPage.helpers";
import {
  buildPendingTurns,
  filterQueuedMessagesForPanel,
  filterTurnsForQueuedMessages,
  mergeMessagesForView,
  mergeQueuedMessagesForPanel,
} from "./messageMerge";
import {
  deriveAuthUi,
  deriveProviderGuardNotice,
  deriveSessionError,
  extractErrorMessage,
  type AuthUi,
  type ProviderGuardNotice,
  type SessionErrorInfo,
} from "./authDerivations";
import {
  buildCustomStatusByTurnId,
  collectAssistantOrderSeq,
  collectThoughtBlocks,
  readEventOrderSeq,
} from "./timelineProjection";
import type { SessionViewVerbosity } from "../../state/uiStateStore";
import { collectAskUserQuestionAnswers } from "./askUserQuestions";
import { mergeGroupsWithSystemMessages } from "./systemMessageGroups";

export {
  buildPendingTurns,
  filterQueuedMessagesForPanel,
  filterTurnsForQueuedMessages,
  mergeMessagesForView,
  mergeQueuedMessagesForPanel,
};
export { deriveAuthUi, deriveProviderGuardNotice, deriveSessionError };
export { collectAskUserQuestionAnswers } from "./askUserQuestions";
export { normalizeContextWindowMetrics } from "./contextWindow";
export { deriveMessagesKey, deriveTurnsKey } from "./messageKeys";

const devInvariantLogKeys = new Set<string>();
const telemetryInvariantKeys = new Set<string>();

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};

function recordThreadInvariantCounter(reason: string, details: Record<string, string> = {}): void {
  const key = `${reason}:${JSON.stringify(details)}`;
  if (telemetryInvariantKeys.has(key)) return;
  if (telemetryInvariantKeys.size > 500) telemetryInvariantKeys.clear();
  telemetryInvariantKeys.add(key);
  recordClientCounterMetric("workbench.thread.contract_violation_count", {
    reason,
    ...details,
  });
}

function logViewModelInvariant(reason: string, details: Record<string, unknown>): void {
  if (!import.meta.env.DEV) return;
  const key = `${reason}:${JSON.stringify(details)}`;
  if (devInvariantLogKeys.has(key)) return;
  if (devInvariantLogKeys.size > 500) devInvariantLogKeys.clear();
  devInvariantLogKeys.add(key);
  // eslint-disable-next-line no-console
  console.error("[WorkbenchThreadViewModel][contract-violation]", { reason, ...details });
}

function isTerminalTurnStatus(
  status: SessionTurn["status"] | null | undefined,
): status is Extract<SessionTurn["status"], "completed" | "failed" | "interrupted"> {
  return status === "completed" || status === "failed" || status === "interrupted";
}

function readMessageOrderSeq(message: unknown): number {
  const record = asRecord(message);
  const raw = record.order_seq ?? record.turn_sequence;
  return Number(raw ?? Number.NaN);
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

export function buildWorkbenchThreadViewModel(
  turns: SessionTurn[],
  messages: Message[],
  toolsByTurnId: Record<string, SessionTurnTool[]>,
  events: SessionEvent[],
  assistantStreamingOrAnswers:
    | Record<string, AssistantStreamingState>
    | Map<string, AskUserQuestionAnswerState> = {},
  askUserQuestionAnswers?: Map<string, AskUserQuestionAnswerState>,
): WorkbenchThreadView {
  const assistantStreamingByTurnId =
    assistantStreamingOrAnswers instanceof Map ? {} : assistantStreamingOrAnswers;
  const answers =
    assistantStreamingOrAnswers instanceof Map
      ? assistantStreamingOrAnswers
      : askUserQuestionAnswers ??
        collectAskUserQuestionAnswers(events, {});
  if (turns.length > 0) {
    return buildWorkbenchThreadViewModelFromTurns(
      turns,
      messages,
      toolsByTurnId,
      events,
      assistantStreamingByTurnId,
      answers,
    );
  }
  recordThreadInvariantCounter("managed_no_turns");
  return { groups: mergeGroupsWithSystemMessages([], messages), debugEvents: [] };
}

type ActivityEntry = {
  item: ThreadItem;
  created_at: string;
  kind: "tool" | "thought" | "ask_user_question" | "message";
  order_seq?: number;
};

type ToolLocation = Extract<ThreadItem, { kind: "tool" }>["locations"][number];

function resolveToolUpdateRecord(payload: Record<string, unknown>): Record<string, unknown> {
  const nested = asRecord(payload.update);
  return Object.keys(nested).length > 0 ? nested : payload;
}

function readToolCallId(payload: Record<string, unknown>, update: Record<string, unknown>): string {
  const rawInputRecord = asRecord(update.rawInput);
  const rawInputLegacyRecord = asRecord(update.raw_input);
  const toolCall = asRecord(update.toolCall);
  const toolCallRawInput = asRecord(toolCall.rawInput);
  return String(
    payload.tool_call_id ??
      update.toolCallId ??
      update.tool_call_id ??
      rawInputRecord.call_id ??
      rawInputLegacyRecord.call_id ??
      toolCallRawInput.call_id ??
      "",
  ).trim();
}

function normalizeToolLocations(locations: unknown): ToolLocation[] {
  const locs = Array.isArray(locations) ? locations : [];
  const out: ToolLocation[] = [];
  for (const loc of locs) {
    const locRecord = asRecord(loc);
    const pathValue = locRecord.path;
    const rangeValue = locRecord.range;
    if (typeof pathValue !== "string" && rangeValue === undefined) continue;
    out.push({
      path: typeof pathValue === "string" ? pathValue : undefined,
      range: rangeValue,
    });
  }
  return out;
}

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
    provider_tool_name: "",
    title: "Tool",
    subtitle: "",
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
  update: unknown,
) {
  const updateRecord = asRecord(update);
  const toolCall = asRecord(updateRecord.toolCall);
  const rawInput = updateRecord.rawInput;
  const toolCallRawInput = toolCall.rawInput;
  tool.updated_at = ev.created_at;
  tool.updates_seen += 1;
  tool.raw = ev.payload_json ?? tool.raw;

  const nextKind = String(updateRecord.kind ?? toolCall?.kind ?? "").trim();
  if (nextKind) tool.tool_kind = nextKind;
  const nextProviderToolName = String(
    updateRecord.tool_name ?? updateRecord.toolName ?? updateRecord.name ?? toolCall?.name ?? "",
  ).trim();
  if (nextProviderToolName) tool.provider_tool_name = nextProviderToolName;

  const nextTitle = toolDisplayTitleFromPayload(updateRecord);
  if (nextTitle) tool.title = nextTitle;
  else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);
  const nextSubtitle = String(updateRecord.subtitle ?? "").trim();
  if (nextSubtitle) tool.subtitle = nextSubtitle;

  const nextStatus = String(updateRecord.status ?? toolCall?.status ?? "").trim();
  if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
  else if (ev.event_type === "tool_result") tool.status = "completed";

  tool.locations = normalizeToolLocations(updateRecord.locations);

  const input =
    rawInput ?? toolCallRawInput ?? toolCall.input ?? updateRecord.input ?? updateRecord.input_preview ?? null;
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
  const explicitTimelineMessage = payload?.display_in_timeline === true;
  if (!explicitTimelineMessage && code !== "context.compacted" && code !== "context_compacted") {
    return null;
  }
  const unknownToolNotice = code === "crp_unknown_event" ? buildUnknownToolNoticeMessage(payload) : null;
  const message = explicitTimelineMessage
    ? (
      unknownToolNotice ??
      pickFirstString(payload?.message, payload?.text, payload?.summary, payload?.content) ??
      (() => {
        const originalType = pickFirstString(payload?.original_type, payload?.originalType);
        return originalType ? `Unknown runtime event: ${originalType}` : "Unknown runtime event.";
      })()
    )
    : (
      pickFirstString(payload?.message, payload?.text, payload?.summary, payload?.content) ??
      "Context compacted. Earlier turns were summarized."
    );
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

function buildUnknownToolNoticeMessage(payload: Record<string, unknown>): string | null {
  const raw = asRecord(payload.raw);
  const toolName = normalizeDisplayToolLabel(
    toolDisplayTitleFromPayload(payload) || toolDisplayTitleFromPayload(raw),
  );
  if (!toolName || isPlaceholderToolLabel(toolName)) return null;
  const preview = pickFirstString(
    payload.tool_preview,
    payload.toolPreview,
    raw.command,
    raw.description,
    raw.file_path,
    raw.filePath,
    raw.path,
    raw.query,
    raw.pattern,
    raw.regex,
    raw.message,
    raw.text,
    raw.summary,
    raw.title,
  );
  if (preview && preview !== toolName) {
    return `Unknown tool event: ${toolName} · ${preview}`;
  }
  return `Unknown tool event: ${toolName}`;
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

  const thoughtBlocks = collectThoughtBlocks(opts.events, { onInvariant: logViewModelInvariant });

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
      const noticeItem = buildNoticeMessageItem(ev, opts.turnId);
      if (noticeItem) {
        activity.push({
          item: noticeItem,
          created_at: noticeItem.created_at,
          kind: "message",
          order_seq: orderSeq as number,
        });
      }
    }

    if (ev.event_type === "tool_call" || ev.event_type === "tool_call_update" || ev.event_type === "tool_result") {
      const orderSeq = readEventOrderSeq(ev);
      if (!Number.isFinite(orderSeq)) continue;
      const payload = asRecord(ev.payload_json);
      const update = resolveToolUpdateRecord(payload);
      const toolCallId = readToolCallId(payload, update);
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
    thoughtBlocks.forEach((block) => {
      if (!Number.isFinite(block.orderSeq)) return;
      const thoughtText = block.text ?? "";
      if (!thoughtText.trim()) return;
      if (!block.idKey) {
        logViewModelInvariant("thought block missing idKey", {
          turn_id: opts.turnId,
          order_seq: block.orderSeq ?? null,
        });
        return;
      }
      const thoughtItem: Extract<ThreadItem, { kind: "thought" }> = {
        kind: "thought",
        // Avoid index/text-based IDs; those break MessageList prepend invariants and streaming updates.
        id: `thought-${opts.turnId}-${block.idKey}`,
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

  for (const tool of toolById.values()) {
    if (toolInserted.has(tool.tool_call_id)) continue;
    const raw = asRecord(tool.raw);
    const orderSeq = Number(raw.order_seq ?? Number.NaN);
    if (!Number.isFinite(orderSeq)) continue;
    activity.push({
      item: tool,
      created_at: tool.created_at,
      kind: "tool",
      order_seq: orderSeq as number,
    });
    toolInserted.add(tool.tool_call_id);
  }

  return { activity, tools: Array.from(toolById.values()) };
}

export function buildWorkbenchThreadViewModelFromTurns(
  turns: SessionTurn[],
  messages: Message[],
  toolsByTurnId: Record<string, SessionTurnTool[]>,
  events: SessionEvent[],
  assistantStreamingOrAnswers:
    | Record<string, AssistantStreamingState>
    | Map<string, AskUserQuestionAnswerState> = {},
  askUserQuestionAnswers: Map<string, AskUserQuestionAnswerState> = new Map(),
): WorkbenchThreadView {
  const assistantStreamingByTurnId =
    assistantStreamingOrAnswers instanceof Map ? {} : assistantStreamingOrAnswers;
  const answers =
    assistantStreamingOrAnswers instanceof Map
      ? assistantStreamingOrAnswers
      : askUserQuestionAnswers;
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
    const aStart = String(a.started_at ?? "");
    const bStart = String(b.started_at ?? "");
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
          updated_at: turn.updated_at ?? null,
        });
      }
      continue;
    }
    const userMessageId = turn.user_message_id ? idToString(turn.user_message_id) : "";

    const userMessage = userMessageId ? messageById.get(userMessageId) : undefined;
    if (userMessageId && !userMessage) {
      recordThreadInvariantCounter("missing_user_message_anchor", { turn_id: turnId });
    }
    const headerId = turnId;

    const header: WorkbenchTurnHeader | null = userMessage
      ? {
        id: headerId,
        content: userMessage.content ?? "",
        content_revision: userMessage.id,
        attachments: Array.isArray(userMessage.attachments)
          ? userMessage.attachments
          : [],
        created_at: userMessage.created_at,
      }
      : null;
    const headerOrderSeq = readMessageOrderSeq(userMessage);

    let tools = (toolsByTurnId[turnId] ?? []).map((tool) => {
      const toolKind = String(tool.tool_kind ?? "tool");
      const title = String(tool.title ?? humanToolKind(toolKind));
      const summaryOnly = asRecord(tool).summary_only === true;
      const hasDetails =
        !summaryOnly && (tool.input_json != null || String(tool.output_text ?? "").trim().length > 0);
      return {
        kind: "tool",
        id: `tool-${turnId}-${tool.tool_call_id}`,
        tool_call_id: tool.tool_call_id,
        created_at: tool.created_at,
        updated_at: tool.updated_at ?? tool.created_at,
        tool_kind: toolKind,
        provider_tool_name: String(tool.provider_tool_name ?? ""),
        title,
        subtitle: String(tool.subtitle ?? ""),
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
      askUserQuestionAnswers: answers,
    });
    const assistantOrderSeq = collectAssistantOrderSeq(eventsForTurn);

    const assistantMessages = (messagesByTurnId.get(turnId) ?? [])
      .filter((m) => m.role === "assistant")
      .slice()
      .sort((a, b) => {
        const sa = readMessageOrderSeq(a);
        const sb = readMessageOrderSeq(b);
        if (Number.isFinite(sa) && Number.isFinite(sb) && sa !== sb) return sa - sb;
        if (Number.isFinite(sa) && !Number.isFinite(sb)) return -1;
        if (!Number.isFinite(sa) && Number.isFinite(sb)) return 1;
        return String(a.created_at).localeCompare(String(b.created_at));
      });

    type TimelineEntry = {
      item: ThreadItem;
      created_at: string;
      kind: "assistant" | "tool" | "thought" | "ask_user_question" | "message";
      order_seq?: number;
      turn_sequence?: number;
    };

    const timeline: TimelineEntry[] = [];
    for (const m of assistantMessages) {
      const orderSeq = readMessageOrderSeq(m);
      if (!Number.isFinite(orderSeq)) continue;
      const messageId = idToString(m.id);
      if (!messageId) {
        // Missing message ids break MessageList identity invariants; treat this as a bug.
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.error("[WorkbenchThreadViewModel] assistant message missing id", {
            turnId,
            created_at: m.created_at,
            order_seq: asRecord(m).order_seq ?? null,
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
        turn_sequence: orderSeq as number,
        order_seq: orderSeq as number,
      });
    }

    const statusText =
      turn.status === "running" || turn.status === "starting" || turn.status === "queued"
        ? customStatusByTurnId.get(turnId) ?? null
        : null;
    const pendingState = assistantStreamingByTurnId[turnId] ?? null;
    const pendingContent = String(pendingState?.content ?? "");
    const pendingTrimmed = pendingContent.trim();
    const statusTrimmed = statusText?.trim() ?? "";
    const pendingProviderId = pendingState?.providerMessageId ?? null;
    const persistedAssistantDuplicate =
      pendingTrimmed.length > 0
        ? assistantMessages.find((message) => String(message.content ?? "").trim() === pendingTrimmed) ?? null
        : null;
    if (persistedAssistantDuplicate) {
      const reason = isTerminalTurnStatus(turn.status)
        ? "stale_pending_after_terminal_assistant_message"
        : "stale_pending_duplicate_assistant_message";
      recordThreadInvariantCounter(reason, { turn_status: String(turn.status ?? "") || "unknown" });
      logViewModelInvariant(reason, {
        turnId,
        turnStatus: turn.status ?? null,
        pendingProviderId,
        pendingLength: pendingTrimmed.length,
        messageId: idToString(persistedAssistantDuplicate.id),
      });
    }
    if (
      pendingTrimmed.length > 0 &&
      pendingTrimmed !== statusTrimmed &&
      !persistedAssistantDuplicate
    ) {
      const pendingOrderSeq = Number.isFinite(pendingState?.orderSeq)
        ? pendingState?.orderSeq
        : pendingProviderId
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
      custom_status: statusText ?? undefined,
      assistant_messages_content: assistantMessagesContent,
    });

    const timelineOrderSeq = timeline
      .map((entry) => entry.order_seq)
      .filter((seq): seq is number => Number.isFinite(seq));
    const turnStartOrderSeq = Number(turn.start_seq ?? Number.NaN);
    const turnEndOrderSeq = Number(turn.end_seq ?? Number.NaN);
    const groupOrderSeq = Number.isFinite(headerOrderSeq)
      ? (headerOrderSeq as number)
      : Number.isFinite(turnStartOrderSeq)
        ? turnStartOrderSeq
      : timelineOrderSeq.length > 0
        ? Math.min(...timelineOrderSeq)
        : Number.isFinite(turnEndOrderSeq)
          ? turnEndOrderSeq
        : Number.NaN;
    if (!Number.isFinite(groupOrderSeq)) {
      recordThreadInvariantCounter("missing_order_seq_anchor", { turn_id: turnId });
      if (import.meta.env.DEV) {
        // eslint-disable-next-line no-console
        console.error("[WorkbenchThreadViewModel] turn missing order_seq anchor", {
          turn_id: turnId,
          user_message_id: userMessageId || null,
          status: turn.status ?? null,
        });
      }
      continue;
    }
    groups.push({
      sort_seq: groupOrderSeq as number,
      group: { key: `turn-${turnId}`, header, items },
    });
  }

  return { groups: mergeGroupsWithSystemMessages(groups, messages), debugEvents };
}

function extractToolOutputText(update: unknown): string {
  const updateRecord = asRecord(update);
  const rawOutput = asRecord(updateRecord.rawOutput);
  const toolCall = asRecord(updateRecord.toolCall);
  const toolCallRawOutput = asRecord(toolCall?.rawOutput);
  const direct =
    updateRecord.outputText ??
    updateRecord.output_text ??
    updateRecord.output_preview ??
    updateRecord.result ??
    rawOutput?.aggregated_output ??
    rawOutput?.output ??
    toolCall?.outputText ??
    toolCall?.output_text ??
    toolCallRawOutput?.aggregated_output ??
    toolCallRawOutput?.output ??
    null;
  if (typeof direct === "string" && direct.trim()) return direct.trim();

  const blocks = Array.isArray(updateRecord.content) ? updateRecord.content : [];
  const parts: string[] = [];
  for (const b of blocks) {
    const blockRecord = asRecord(b);
    const contentRecord = asRecord(blockRecord?.content ?? b);
    const t = contentRecord?.text;
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

function pickFirstString(...values: unknown[]): string | null {
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

function isStatusUpdateMeta(meta: unknown): boolean {
  const metaRecord = asRecord(meta);
  if (!metaRecord) return false;
  const codexMeta = asRecord(metaRecord.codex);
  const reasoningKind = codexMeta.reasoning_kind ?? codexMeta.reasoningKind;
  if (reasoningKind === "status") return true;

  const statusText = pickFirstString(
    metaRecord.status_text,
    metaRecord.statusText,
    metaRecord.status_string,
    metaRecord.statusString,
    codexMeta.status_text,
    codexMeta.statusText,
    codexMeta.status_string,
    codexMeta.statusString,
  );
  if (statusText) return true;

  const statusValue =
    typeof metaRecord.status === "string"
      ? metaRecord.status
      : typeof codexMeta.status === "string"
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
