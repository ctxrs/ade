import type { WorkbenchListItem, WorkbenchTurnHeader } from "../SessionPage.types";
import { markdownToPlainText } from "../SessionPage.helpers";
import type { WorkbenchMessageListUiState } from "../sessionMessageListItemIdentity";

const MESSAGE_COLLAPSE_LINE_THRESHOLD = 20;
const MESSAGE_COLLAPSE_CHAR_THRESHOLD = 1500;
const TURN_HEADER_COLLAPSE_LINE_THRESHOLD = 1;
const TURN_HEADER_COLLAPSE_CHAR_THRESHOLD = 140;

export function isExpandableMessageContent(content: string): boolean {
  const normalized = String(content ?? "");
  const lines = normalized.split("\n").length;
  return lines > MESSAGE_COLLAPSE_LINE_THRESHOLD || normalized.length > MESSAGE_COLLAPSE_CHAR_THRESHOLD;
}

export function isExpandableTurnHeaderPlainText(plainText: string): boolean {
  const normalized = String(plainText ?? "");
  const lines = normalized.split("\n").length;
  return lines > TURN_HEADER_COLLAPSE_LINE_THRESHOLD || normalized.length > TURN_HEADER_COLLAPSE_CHAR_THRESHOLD;
}

export function getWorkbenchTurnHeaderDisplayPlainText(header: WorkbenchTurnHeader): string {
  const explicitPlainText = typeof header.plain_text === "string" ? header.plain_text : "";
  if (explicitPlainText.length > 0) {
    return explicitPlainText;
  }
  return markdownToPlainText(header.content ?? "");
}

export function resolveWorkbenchTurnHeaderExpandedFromPlainText(
  header: WorkbenchTurnHeader,
  displayPlainText: string,
  expandedTurnHeaders: Record<string, boolean>,
): boolean {
  if (!isExpandableTurnHeaderPlainText(displayPlainText)) return true;
  return expandedTurnHeaders[header.id] ?? false;
}

export function getWorkbenchTurnHeaderLayoutState(
  item: Extract<WorkbenchListItem, { kind: "turn_header" }>,
  expandedTurnHeaders: Record<string, boolean>,
) {
  const displayPlainText = getWorkbenchTurnHeaderDisplayPlainText(item.header);
  const expanded = resolveWorkbenchTurnHeaderExpandedFromPlainText(
    item.header,
    displayPlainText,
    expandedTurnHeaders,
  );
  return {
    displayPlainText,
    expanded,
    expandable: isExpandableTurnHeaderPlainText(displayPlainText),
  };
}

export function resolveWorkbenchMessageExpandedFromContent(
  item: Extract<WorkbenchListItem, { kind: "message" }>,
  expandedMessageById: Record<string, boolean>,
): boolean {
  if (!isExpandableMessageContent(item.content)) return true;
  return expandedMessageById[item.id] ?? false;
}

export function getWorkbenchMessageLayoutState(
  item: Extract<WorkbenchListItem, { kind: "message" }>,
  expandedMessageById: Record<string, boolean>,
) {
  const expanded = resolveWorkbenchMessageExpandedFromContent(item, expandedMessageById);
  return {
    expanded,
    expandable: isExpandableMessageContent(item.content),
    shownContent: expanded ? item.content : item.content.split("\n").slice(0, 20).join("\n"),
  };
}

export function getWorkbenchListItemLayoutState(
  item: WorkbenchListItem,
  uiState: Pick<WorkbenchMessageListUiState, "expandedTurnHeaders" | "expandedMessageById">,
) {
  switch (item.kind) {
    case "turn_header":
      return getWorkbenchTurnHeaderLayoutState(item, uiState.expandedTurnHeaders);
    case "message":
      return getWorkbenchMessageLayoutState(item, uiState.expandedMessageById);
    default:
      return null;
  }
}
