import type { PretextVirtualizerPlannedLayout } from "@pretext-virtualizer/core";
import type { MessageAttachment } from "../../api/client";
import { resolveAskUserQuestionShellLayout } from "../../components/askUserQuestionLayout";
import {
  addPretextPerfBucket,
  incrementPretextPerfCounter,
} from "../../utils/pretextPerfDiagnostics";
import type { WorkbenchListItem, WorkbenchTurnHeader } from "../sessionView";
import { resolveWorkbenchMessageExpanded } from "../sessionMessageListItemIdentity";
import {
  clearSessionMarkdownMeasurementCaches,
  measureSessionMarkdownDocument,
  measureSessionPlainTextBlockHeight,
  measureSessionTextHeight,
} from "./sessionMarkdownMeasurement";
import {
  resolveSessionMarkdownBlockEntryGapPx,
  resolveSessionMarkdownBlockGapPx,
} from "./sessionMarkdownContract";
import {
  clearSessionStreamingMarkdownCaches,
  resolveSessionStreamingMarkdownLayout,
} from "./sessionStreamingMarkdown";
import {
  SESSION_THREAD_INDENT_LEFT_PX,
  SESSION_THREAD_MESSAGE_ATTACHMENT_GAP_PX,
  SESSION_THREAD_MESSAGE_ATTACHMENT_HEIGHT_PX,
  SESSION_THREAD_MESSAGE_ATTACHMENT_MARGIN_TOP_PX,
  SESSION_THREAD_MESSAGE_ATTACHMENT_WIDTH_PX,
  SESSION_THREAD_MESSAGE_BUBBLE_BORDER_WIDTH_PX,
  SESSION_THREAD_MESSAGE_BUBBLE_PADDING_BLOCK_PX,
  SESSION_THREAD_MESSAGE_BUBBLE_PADDING_INLINE_PX,
  SESSION_THREAD_MESSAGE_ROLE_LINE_HEIGHT_PX,
  SESSION_THREAD_MESSAGE_ROW_PADDING_BLOCK_PX,
  SESSION_THREAD_MESSAGE_TOGGLE_LINE_HEIGHT_PX,
  SESSION_THREAD_MESSAGE_TOGGLE_MARGIN_TOP_PX,
  SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY,
  SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY,
  SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_INLINE_PX,
  resolveSessionThreadAssistantTextWidth,
  resolveSessionThreadContentWidth,
  resolveSessionThreadIndentedContentWidth,
  resolveSessionThreadMessageTextWidth,
  resolveSessionThreadTurnHeaderTextWidth,
} from "./sessionThreadLayoutTokens";
import {
  getWorkbenchMessageLayoutState,
  getWorkbenchTurnHeaderDisplayPlainText,
  getWorkbenchTurnHeaderLayoutState,
  isExpandableMessageContent,
} from "./transcriptRowLayoutModel";

export type PretextVirtualizerRowLayoutContext = {
  expandedTurnHeaders?: Readonly<Record<string, boolean>>;
  expandedTurnDetailsById?: Readonly<Record<string, boolean>>;
  expandedMessageById?: Readonly<Record<string, boolean>>;
  turnToolsLoading?: readonly string[];
};

const BODY_FONT = `${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`;
const BODY_LINE_HEIGHT_PX = SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX;
const SMALL_LINE_HEIGHT_PX = 16;
const MONO_FONT = `${SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY}`;
const MONO_LINE_HEIGHT_PX = SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX;

const SPACER_HEIGHT_PX = 1;
const TURN_STATUS_HEIGHT_PX = 24;
const TOOL_ROW_HEIGHT_PX = 18;
const TOOL_GROUP_GAP_PX = 6;
const TOOL_THOUGHT_TITLE_HEIGHT_PX = 18;
const TOOL_THOUGHT_PRE_PADDING_PX = 18;

const THOUGHT_HORIZONTAL_PADDING_PX = 8;
const THOUGHT_VERTICAL_PADDING_PX = 16;

const TURN_HEADER_OUTER_VERTICAL_PX = 14;
const TURN_HEADER_BUBBLE_VERTICAL_PX = 18;
const TURN_HEADER_COLLAPSED_MAX_HEIGHT_PX = 66;
const TURN_HEADER_ATTACHMENT_SIZE_PX = 44;
const TURN_HEADER_ATTACHMENT_GAP_PX = 6;
const TURN_HEADER_ATTACHMENT_MARGIN_TOP_PX = 8;

const MESSAGE_ROLE_HEIGHT_PX = SESSION_THREAD_MESSAGE_ROLE_LINE_HEIGHT_PX;
const MESSAGE_BUBBLE_HORIZONTAL_PX =
  SESSION_THREAD_MESSAGE_BUBBLE_PADDING_INLINE_PX * 2 +
  SESSION_THREAD_MESSAGE_BUBBLE_BORDER_WIDTH_PX * 2;
const MESSAGE_BUBBLE_VERTICAL_PX =
  SESSION_THREAD_MESSAGE_BUBBLE_PADDING_BLOCK_PX * 2 +
  SESSION_THREAD_MESSAGE_BUBBLE_BORDER_WIDTH_PX * 2;
const MESSAGE_TOGGLE_HEIGHT_PX =
  SESSION_THREAD_MESSAGE_TOGGLE_MARGIN_TOP_PX + SESSION_THREAD_MESSAGE_TOGGLE_LINE_HEIGHT_PX;
const MESSAGE_ROW_VERTICAL_PX = SESSION_THREAD_MESSAGE_ROW_PADDING_BLOCK_PX * 2;

const ASSISTANT_VERTICAL_PADDING_PX = 20;

const PERF_WIDTH_BUCKET_SIZE = 64;
const normalizeHeight = (value: number): number =>
  Number.isFinite(value) && value > 0 ? Math.max(1, Math.round(value * 16) / 16) : 1;

const resolveThoughtTextWidth = (viewportWidth: number): number =>
  Math.max(1, resolveSessionThreadIndentedContentWidth(viewportWidth) - THOUGHT_HORIZONTAL_PADDING_PX);

function measureTextHeight(params: {
  cacheKey: string;
  text: string;
  font: string;
  width: number;
  lineHeight: number;
  whiteSpace?: "normal" | "pre-wrap";
}): number {
  return measureSessionTextHeight(params);
}

const countImageAttachments = (attachments: readonly MessageAttachment[]): number =>
  attachments.filter((attachment) => attachment.kind === "image" || attachment.kind === "image_ref").length;

function countAttachmentRows(attachmentCount: number, width: number): number {
  if (attachmentCount <= 0) return 0;
  const perRow = Math.max(
    1,
    Math.floor(
      (width + SESSION_THREAD_MESSAGE_ATTACHMENT_GAP_PX) /
        (SESSION_THREAD_MESSAGE_ATTACHMENT_WIDTH_PX + SESSION_THREAD_MESSAGE_ATTACHMENT_GAP_PX),
    ),
  );
  return Math.ceil(attachmentCount / perRow);
}

function measureTurnHeaderHeight(
  header: WorkbenchTurnHeader,
  viewportWidth: number,
  context: PretextVirtualizerRowLayoutContext,
): number {
  const displayPlainText = getWorkbenchTurnHeaderDisplayPlainText(header);
  const expanded = getWorkbenchTurnHeaderLayoutState(
    { kind: "turn_header", id: `turn-header-${header.id}`, header },
    context.expandedTurnHeaders ?? {},
  ).expanded;
  const textHeight = measureSessionPlainTextBlockHeight({
    cacheKey: `turn-header:${header.id}:${displayPlainText}`,
    text: displayPlainText,
    font: BODY_FONT,
    width: resolveSessionThreadTurnHeaderTextWidth(viewportWidth),
    lineHeight: BODY_LINE_HEIGHT_PX,
  });
  const collapsedTextHeight = expanded ? textHeight : Math.min(textHeight, TURN_HEADER_COLLAPSED_MAX_HEIGHT_PX);
  const imageCount = expanded ? countImageAttachments(header.attachments) : 0;
  const perRow = Math.max(
    1,
    Math.floor(
      (resolveSessionThreadContentWidth(viewportWidth) -
        SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_INLINE_PX * 2 +
        TURN_HEADER_ATTACHMENT_GAP_PX) /
        (TURN_HEADER_ATTACHMENT_SIZE_PX + TURN_HEADER_ATTACHMENT_GAP_PX),
    ),
  );
  const attachmentRows = imageCount > 0 ? Math.ceil(imageCount / perRow) : 0;
  const attachmentsHeight =
    attachmentRows > 0
      ? TURN_HEADER_ATTACHMENT_MARGIN_TOP_PX +
        attachmentRows * TURN_HEADER_ATTACHMENT_SIZE_PX +
        Math.max(0, attachmentRows - 1) * TURN_HEADER_ATTACHMENT_GAP_PX
      : 0;
  return normalizeHeight(
    TURN_HEADER_OUTER_VERTICAL_PX + TURN_HEADER_BUBBLE_VERTICAL_PX + collapsedTextHeight + attachmentsHeight,
  );
}

function measureThoughtHeight(item: Extract<WorkbenchListItem, { kind: "thought" }>, viewportWidth: number): number {
  return normalizeHeight(
    THOUGHT_VERTICAL_PADDING_PX +
      measureTextHeight({
        cacheKey: `thought:${item.id}:${item.content}`,
        text: item.content ?? "",
        font: MONO_FONT,
        width: resolveThoughtTextWidth(viewportWidth),
        lineHeight: MONO_LINE_HEIGHT_PX,
        whiteSpace: "pre-wrap",
      }),
  );
}

function measureMessageAttachmentsHeight(item: Extract<WorkbenchListItem, { kind: "message" }>, viewportWidth: number): number {
  const imageCount = countImageAttachments(item.attachments ?? []);
  if (imageCount === 0) return 0;
  const rows = countAttachmentRows(imageCount, resolveSessionThreadMessageTextWidth(viewportWidth));
  return (
    SESSION_THREAD_MESSAGE_ATTACHMENT_MARGIN_TOP_PX +
    rows * SESSION_THREAD_MESSAGE_ATTACHMENT_HEIGHT_PX +
    Math.max(0, rows - 1) * SESSION_THREAD_MESSAGE_ATTACHMENT_GAP_PX
  );
}

function measureMessageHeight(
  item: Extract<WorkbenchListItem, { kind: "message" }>,
  viewportWidth: number,
  context: PretextVirtualizerRowLayoutContext,
): number {
  const layout = getWorkbenchMessageLayoutState(item, context.expandedMessageById ?? {});
  const textHeight = measureSessionMarkdownDocument(layout.shownContent, resolveSessionThreadMessageTextWidth(viewportWidth));
  const attachmentsHeight = measureMessageAttachmentsHeight(item, viewportWidth);
  const toggleHeight = layout.expandable ? MESSAGE_TOGGLE_HEIGHT_PX : 0;
  return normalizeHeight(
    MESSAGE_ROW_VERTICAL_PX +
      MESSAGE_ROLE_HEIGHT_PX +
      MESSAGE_BUBBLE_VERTICAL_PX +
      textHeight +
      attachmentsHeight +
      toggleHeight,
  );
}

function measureAssistantHeight(
  item: Extract<WorkbenchListItem, { kind: "assistant" }>,
  viewportWidth: number,
): number {
  if (!item.is_complete && item.content.trim().length === 0) {
    return SPACER_HEIGHT_PX;
  }
  const textWidth = resolveSessionThreadAssistantTextWidth(viewportWidth);
  const textHeight = item.is_complete
    ? measureSessionMarkdownDocument(item.content, textWidth)
    : (() => {
        const layout = resolveSessionStreamingMarkdownLayout(item.content);
        const stableHeight =
          layout.stableMarkdown.length > 0
            ? measureSessionMarkdownDocument(layout.stableMarkdown, textWidth)
            : 0;
        if (layout.trailingTail.length === 0) {
          return stableHeight;
        }
        const tailHeight = measureTextHeight({
          cacheKey: `assistant-tail:${item.id}:${layout.trailingTail}`,
          text: layout.trailingTail,
          font: BODY_FONT,
          width: textWidth,
          lineHeight: BODY_LINE_HEIGHT_PX,
          whiteSpace: "pre-wrap",
        });
        const tailGap =
          layout.stableBlocks.length === 0
            ? resolveSessionMarkdownBlockEntryGapPx("paragraph", "root")
            : resolveSessionMarkdownBlockGapPx(
                layout.stableBlocks[layout.stableBlocks.length - 1]!.kind,
                "paragraph",
                "root",
              );
        return stableHeight + tailGap + tailHeight;
      })();
  return normalizeHeight(ASSISTANT_VERTICAL_PADDING_PX + textHeight);
}

function measureToolThoughtHeight(thought: string, width: number): number {
  if (!thought.trim()) return 0;
  const thoughtTextHeight = measureTextHeight({
    cacheKey: `tool-thought:${thought}`,
    text: thought,
    font: MONO_FONT,
    width: Math.max(1, width - TOOL_THOUGHT_PRE_PADDING_PX),
    lineHeight: MONO_LINE_HEIGHT_PX,
    whiteSpace: "pre-wrap",
  });
  return TOOL_THOUGHT_TITLE_HEIGHT_PX + TOOL_THOUGHT_PRE_PADDING_PX + thoughtTextHeight;
}

function measureToolGroupHeight(
  item: Extract<WorkbenchListItem, { kind: "tool_group" }>,
  viewportWidth: number,
  context: PretextVirtualizerRowLayoutContext,
): number {
  const expanded = context.expandedTurnDetailsById?.[item.turn_id] ?? false;
  const summaryHeight = TOOL_ROW_HEIGHT_PX;
  if (!expanded) return summaryHeight;
  const loadingTools = item.tools.length === 0 && Boolean(context.turnToolsLoading?.includes(item.turn_id));
  let detailsHeight = 0;
  if (loadingTools) {
    detailsHeight += SMALL_LINE_HEIGHT_PX;
  } else if (item.tools.length > 0) {
    detailsHeight += item.tools.length * TOOL_ROW_HEIGHT_PX + Math.max(0, item.tools.length - 1) * 4;
  }
  const thoughtHeight = measureToolThoughtHeight(
    item.thought,
    resolveSessionThreadContentWidth(viewportWidth) - SESSION_THREAD_INDENT_LEFT_PX,
  );
  if (thoughtHeight > 0) {
    if (detailsHeight > 0) detailsHeight += TOOL_GROUP_GAP_PX;
    detailsHeight += thoughtHeight;
  }
  return normalizeHeight(summaryHeight + (detailsHeight > 0 ? TOOL_GROUP_GAP_PX + detailsHeight : 0));
}

function measureAskUserQuestionHeight(item: Extract<WorkbenchListItem, { kind: "ask_user_question" }>, viewportWidth: number): number {
  void item;
  return normalizeHeight(resolveAskUserQuestionShellLayout(viewportWidth).outerHeight);
}

export const clearPretextVirtualizerRowLayoutCache = (): void => {
  clearSessionMarkdownMeasurementCaches();
  clearSessionStreamingMarkdownCaches();
};

export const getPretextVirtualizerRowLayout = (
  item: WorkbenchListItem,
  viewportWidth: number,
  context: PretextVirtualizerRowLayoutContext,
): PretextVirtualizerPlannedLayout => {
  const widthBucket = `w${Math.floor(Math.max(0, viewportWidth) / PERF_WIDTH_BUCKET_SIZE)}`;
  incrementPretextPerfCounter("pretext_row_layout_calls");
  addPretextPerfBucket("pretext_row_layout_kind", item.kind);
  addPretextPerfBucket("pretext_row_layout_item", `${item.kind}:${item.id}:${widthBucket}`);
  switch (item.kind) {
    case "spacer":
      return { height: SPACER_HEIGHT_PX };
    case "turn_status":
      return { height: TURN_STATUS_HEIGHT_PX };
    case "thought":
      return { height: measureThoughtHeight(item, viewportWidth) };
    case "turn_header":
      return { height: measureTurnHeaderHeight(item.header, viewportWidth, context) };
    case "message":
      return { height: measureMessageHeight(item, viewportWidth, context) };
    case "assistant":
      return { height: measureAssistantHeight(item, viewportWidth) };
    case "tool":
      return { height: TOOL_ROW_HEIGHT_PX };
    case "tool_group":
      return { height: measureToolGroupHeight(item, viewportWidth, context) };
    case "ask_user_question":
      return { height: measureAskUserQuestionHeight(item, viewportWidth) };
    default:
      return { height: SPACER_HEIGHT_PX };
  }
};
