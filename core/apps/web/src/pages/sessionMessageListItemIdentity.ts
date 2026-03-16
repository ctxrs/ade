import type { WorkbenchListItem } from "./SessionPage.types";
import { humanTurnStatus } from "./SessionPage.helpers";

const MESSAGE_COLLAPSE_LINE_THRESHOLD = 20;
const MESSAGE_COLLAPSE_CHAR_THRESHOLD = 1500;
const TURN_HEADER_COLLAPSE_LINE_THRESHOLD = 1;
const TURN_HEADER_COLLAPSE_CHAR_THRESHOLD = 140;

export type WorkbenchMessageListUiState = {
  expandedTurnHeaders: Record<string, boolean>;
  expandedTurnDetailsById: Record<string, boolean>;
  expandedToolById: Record<string, boolean>;
  expandedMessageById: Record<string, boolean>;
  turnToolsLoading: readonly string[];
};

type HeightRevisionOptions = {
  verbosity?: string;
};

function fingerprintAttachmentLayout(
  attachments: ReadonlyArray<{
    kind?: string;
    name?: string | null;
    mime_type?: string | null;
  }>,
): string {
  return fingerprintUnknown(
    attachments.map((attachment) => ({
      kind: attachment.kind ?? "",
      name: attachment.name ?? "",
      mimeType: attachment.mime_type ?? "",
    })),
  );
}

function stableTrueKeys(record: Record<string, boolean>): string[] {
  return Object.entries(record)
    .filter(([, value]) => value)
    .map(([key]) => key)
    .sort();
}

function fingerprintString(value: string): string {
  const normalized = String(value ?? "");
  let hash = 2166136261;
  for (let index = 0; index < normalized.length; index += 1) {
    hash ^= normalized.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `${normalized.length}:${(hash >>> 0).toString(36)}`;
}

function fingerprintUnknown(value: unknown): string {
  try {
    return fingerprintString(JSON.stringify(value) ?? "");
  } catch {
    return fingerprintString(String(value ?? ""));
  }
}

export function getWorkbenchMessageListLayoutRevision(
  uiState: WorkbenchMessageListUiState,
  options?: { verbosity?: string },
): string {
  return JSON.stringify({
    verbosity: options?.verbosity ?? null,
    turnHeaders: stableTrueKeys(uiState.expandedTurnHeaders),
    turnDetails: stableTrueKeys(uiState.expandedTurnDetailsById),
    tools: stableTrueKeys(uiState.expandedToolById),
    messages: stableTrueKeys(uiState.expandedMessageById),
    turnToolsLoading: [...uiState.turnToolsLoading].sort(),
  });
}

export function isExpandableMessageContent(content: string): boolean {
  const normalized = String(content ?? "");
  const lines = normalized.split("\n").length;
  return lines > MESSAGE_COLLAPSE_LINE_THRESHOLD || normalized.length > MESSAGE_COLLAPSE_CHAR_THRESHOLD;
}

export function resolveWorkbenchMessageExpanded(
  item: Extract<WorkbenchListItem, { kind: "message" }>,
  expandedMessageById: Record<string, boolean>,
): boolean {
  if (!isExpandableMessageContent(item.content)) return true;
  return expandedMessageById[item.id] ?? false;
}

export function isExpandableTurnHeaderPlainText(plainText: string): boolean {
  const normalized = String(plainText ?? "");
  const lines = normalized.split("\n").length;
  return lines > TURN_HEADER_COLLAPSE_LINE_THRESHOLD || normalized.length > TURN_HEADER_COLLAPSE_CHAR_THRESHOLD;
}

export function resolveWorkbenchTurnHeaderExpanded(
  item: Extract<WorkbenchListItem, { kind: "turn_header" }>,
  expandedTurnHeaders: Record<string, boolean>,
): boolean {
  const plainText = item.header.plain_text ?? item.header.content ?? "";
  if (!isExpandableTurnHeaderPlainText(plainText)) return true;
  return expandedTurnHeaders[item.header.id] ?? false;
}

function toolGroupChildExpansionKey(
  item: Extract<WorkbenchListItem, { kind: "tool_group" }>,
  expandedToolById: Record<string, boolean>,
): string {
  if (item.tools.length === 0) return "none";
  return item.tools
    .map((tool) => `${tool.id}:${expandedToolById[tool.id] ? "open" : "closed"}`)
    .join("|");
}

function isMutableToolStatus(status: string): boolean {
  const normalized = String(status ?? "").trim().toLowerCase();
  return (
    normalized.includes("running") ||
    normalized.includes("pending") ||
    normalized.includes("queued") ||
    normalized.includes("starting")
  );
}

function isMutableTurnStatus(status: Extract<WorkbenchListItem, { kind: "turn_status" }>["status"]): boolean {
  return status === "running" || status === "queued";
}

function getTurnStatusHeightRevision(item: Extract<WorkbenchListItem, { kind: "turn_status" }>): string {
  const customStatus = item.custom_status?.trim() ?? "";
  const statusLabel = isMutableTurnStatus(item.status) && customStatus ? customStatus : humanTurnStatus(item.status);
  const showCopyButton =
    item.status === "completed" && Boolean(item.assistant_messages_content?.trim());
  return `turn-status:${fingerprintString(statusLabel)}:${showCopyButton ? "copy" : "nocopy"}`;
}

export function getWorkbenchListItemHeightRevision(
  item: WorkbenchListItem,
  uiState: WorkbenchMessageListUiState,
  options?: HeightRevisionOptions,
): string {
  switch (item.kind) {
    case "message":
      if (!isExpandableMessageContent(item.content)) {
        return `message:fixed:${fingerprintString(item.content)}:${fingerprintAttachmentLayout(item.attachments)}`;
      }
      return resolveWorkbenchMessageExpanded(item, uiState.expandedMessageById) ? "message:expanded" : "message:collapsed";
    case "turn_header":
      {
        const plainText = item.header.plain_text ?? item.header.content ?? "";
        const contentRevision = fingerprintString(plainText);
        const attachmentRevision = fingerprintAttachmentLayout(item.header.attachments);
        if (!isExpandableTurnHeaderPlainText(plainText)) {
          return `turn-header:fixed:${contentRevision}:${attachmentRevision}`;
        }
        return resolveWorkbenchTurnHeaderExpanded(item, uiState.expandedTurnHeaders)
          ? `turn-header:expanded:${contentRevision}:${attachmentRevision}`
          : `turn-header:collapsed:${contentRevision}:attachments:hidden`;
      }
    case "tool":
      return [
        uiState.expandedToolById[item.id] ? "tool:expanded" : "tool:collapsed",
        item.status,
        fingerprintString(item.title),
        fingerprintString(item.subtitle ?? ""),
        fingerprintUnknown(item.locations),
        fingerprintUnknown(item.input),
        uiState.expandedToolById[item.id] && options?.verbosity === "verbose"
          ? fingerprintString(item.output_text)
          : "output:hidden",
      ].join(":");
    case "tool_group": {
      const expanded = uiState.expandedTurnDetailsById[item.turn_id] ?? false;
      if (!expanded) return "tool-group:collapsed";
      const loading =
        item.tools.length === 0 && uiState.turnToolsLoading.includes(item.turn_id) ? "loading" : "ready";
      return `tool-group:expanded:${loading}:${fingerprintString(item.thought)}:${toolGroupChildExpansionKey(item, uiState.expandedToolById)}`;
    }
    case "assistant":
      if (!item.is_complete) {
        return `assistant:pending:${fingerprintString(item.content)}`;
      }
      return `assistant:complete:fixed:${fingerprintString(item.content)}`;
    case "thought":
      return `thought:${fingerprintString(item.content)}`;
    case "turn_status":
      return getTurnStatusHeightRevision(item);
    case "ask_user_question":
      return [
        "ask-user-question",
        item.answered ? "answered" : "pending",
        item.outcome ?? "none",
        fingerprintUnknown(item.input),
        fingerprintUnknown(item.answers ?? null),
      ].join(":");
    default:
      return "fixed";
  }
}

export function getWorkbenchListItemSizeCacheKey(
  item: WorkbenchListItem,
  uiState: WorkbenchMessageListUiState,
  options?: HeightRevisionOptions,
): string | null {
  switch (item.kind) {
    case "assistant":
      return item.is_complete ? getWorkbenchListItemHeightRevision(item, uiState, options) : null;
    case "turn_status":
      return isMutableTurnStatus(item.status) ? null : getWorkbenchListItemHeightRevision(item, uiState, options);
    case "tool":
      return isMutableToolStatus(item.status) ? null : getWorkbenchListItemHeightRevision(item, uiState, options);
    case "tool_group":
      return item.tool_pending === 0 && item.tool_running === 0
        ? getWorkbenchListItemHeightRevision(item, uiState, options)
        : null;
    case "ask_user_question":
      return item.answered ? getWorkbenchListItemHeightRevision(item, uiState, options) : null;
    case "message":
    case "turn_header":
    case "thought":
    case "spacer":
      return getWorkbenchListItemHeightRevision(item, uiState, options);
    default:
      return null;
  }
}

export function getWorkbenchListItemKey(
  item: WorkbenchListItem,
  uiState: WorkbenchMessageListUiState,
  options?: HeightRevisionOptions,
): string {
  return `${item.id}:${getWorkbenchListItemHeightRevision(item, uiState, options)}`;
}
