import type { ContextWindowInfo } from "../../components/WorkbenchComposer";
import type { WorkbenchListItem } from "../SessionPage.types";

const getEstimateBucket = (length: number): string => {
  if (length > 8000) return "xxl";
  if (length > 4000) return "xl";
  if (length > 2000) return "l";
  if (length > 800) return "m";
  if (length > 200) return "s";
  return "xs";
};

const estimateTextHeight = (text: string, attachments = 0): number => {
  const lineBreaks = text.split("\n").length;
  const lengthLines = Math.ceil(text.length / 80);
  const lines = Math.max(1, Math.max(lineBreaks, lengthLines));
  const base = 28;
  const lineHeight = 18;
  const attachmentExtra = attachments > 0 ? 120 * attachments : 0;
  return base + lines * lineHeight + attachmentExtra;
};

export const estimateItemHeight = (item: WorkbenchListItem): number => {
  const kind = item.kind ?? "unknown";
  switch (kind) {
    case "message": {
      const msg = item as Extract<WorkbenchListItem, { kind: "message" }>;
      return estimateTextHeight(msg.content ?? "", msg.attachments?.length ?? 0);
    }
    case "assistant":
    case "thought": {
      const text = (item as Extract<WorkbenchListItem, { kind: "assistant" | "thought" }>).content ?? "";
      return estimateTextHeight(text);
    }
    case "turn_header": {
      const header = (item as Extract<WorkbenchListItem, { kind: "turn_header" }>).header;
      const text = header?.plain_text ?? header?.content ?? "";
      return estimateTextHeight(text, header?.attachments?.length ?? 0);
    }
    case "tool": {
      const toolItem = item as Extract<WorkbenchListItem, { kind: "tool" }>;
      const text = `${toolItem.title ?? ""}\n${toolItem.output_text ?? ""}`;
      return Math.min(900, estimateTextHeight(text));
    }
    case "tool_group": {
      const group = item as Extract<WorkbenchListItem, { kind: "tool_group" }>;
      const text = String(group.thought ?? "");
      const toolCount = group.tool_total ?? group.tools?.length ?? 0;
      return estimateTextHeight(text) + toolCount * 28;
    }
    case "turn_status":
      return 44;
    case "ask_user_question": {
      const input = String((item as Extract<WorkbenchListItem, { kind: "ask_user_question" }>).input ?? "");
      return estimateTextHeight(input) + 60;
    }
    case "spacer":
      return 24;
    default:
      return 56;
  }
};

export const shouldLockItem = (item: WorkbenchListItem): boolean => {
  if (!item) return false;
  if (item.kind === "turn_header" || item.kind === "spacer" || item.kind === "message") return true;
  if (item.kind === "assistant") return item.is_complete;
  if (item.kind === "tool_group") return item.tool_pending === 0 && item.tool_running === 0;
  if (item.kind === "tool") {
    const status = String(item.status ?? "").toLowerCase();
    if (!status) return true;
    return !(
      status.includes("running") ||
      status.includes("pending") ||
      status.includes("queued") ||
      status.includes("starting")
    );
  }
  if (item.kind === "turn_status") {
    const status = String(item.status ?? "").toLowerCase();
    if (!status) return true;
    return !(status.includes("running") || status.includes("queued"));
  }
  if (item.kind === "ask_user_question") return item.answered;
  return true;
};

export const getItemEstimateKey = (item: WorkbenchListItem): string => {
  const kind = item.kind ?? "unknown";
  switch (kind) {
    case "message":
    case "assistant":
    case "thought": {
      const text = (item as Extract<WorkbenchListItem, { kind: "message" | "assistant" | "thought" }>).content ?? "";
      return `${kind}:${getEstimateBucket(text.length)}`;
    }
    case "tool": {
      const toolItem = item as Extract<WorkbenchListItem, { kind: "tool" }>;
      const text = String(toolItem.output_text ?? "").length + String(toolItem.title ?? "").length * 2;
      return `${kind}:${getEstimateBucket(text)}`;
    }
    case "tool_group": {
      const group = item as Extract<WorkbenchListItem, { kind: "tool_group" }>;
      const text = String(group.thought ?? "");
      const toolCount = group.tool_total ?? group.tools?.length ?? 0;
      return `${kind}:${getEstimateBucket(text.length + toolCount * 60)}`;
    }
    case "ask_user_question": {
      const input = String((item as Extract<WorkbenchListItem, { kind: "ask_user_question" }>).input ?? "");
      return `${kind}:${getEstimateBucket(input.length)}`;
    }
    case "turn_header": {
      const header = (item as Extract<WorkbenchListItem, { kind: "turn_header" }>).header;
      const text = header?.plain_text ?? header?.content ?? "";
      return `${kind}:${getEstimateBucket(text.length)}`;
    }
    default:
      return `${kind}:base`;
  }
};

export function isSameContextWindow(a: ContextWindowInfo | null, b: ContextWindowInfo | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.windowTokens === b.windowTokens &&
    a.usedTokens === b.usedTokens &&
    a.remainingTokens === b.remainingTokens &&
    a.remainingFraction === b.remainingFraction
  );
}
