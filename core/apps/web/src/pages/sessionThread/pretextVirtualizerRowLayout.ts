import { layout, prepare, type PreparedText } from "@chenglou/pretext";
import type { PretextVirtualizerPlannedLayout } from "@pretext-virtualizer/core";
import type { MessageAttachment } from "../../api/client";
import {
  normalizeAskUserQuestions,
  type AskUserQuestionItem,
} from "../../components/askUserQuestionShared";
import type { WorkbenchListItem, WorkbenchTurnHeader } from "../SessionPage.types";
import { isExpandableMessageContent, resolveWorkbenchMessageExpanded } from "../sessionMessageListItemIdentity";
import { measureSessionMarkdownDocument, clearSessionMarkdownMeasurementCaches } from "./sessionMarkdownMeasurement";
import {
  SESSION_THREAD_CONTENT_MAX_WIDTH_PX,
  SESSION_THREAD_HORIZONTAL_INSET_PX,
  SESSION_THREAD_INDENT_LEFT_PX,
  resolveSessionThreadContentWidth,
} from "./sessionThreadLayoutTokens";

export type PretextVirtualizerRowLayoutContext = {
  expandedTurnHeaders?: Readonly<Record<string, boolean>>;
  expandedTurnDetailsById?: Readonly<Record<string, boolean>>;
  expandedMessageById?: Readonly<Record<string, boolean>>;
  turnToolsLoading?: readonly string[];
};

const BODY_FONT = "13px system-ui, sans-serif";
const BODY_LINE_HEIGHT_PX = 20.15;
const SMALL_FONT = "12px system-ui, sans-serif";
const SMALL_LINE_HEIGHT_PX = 16;
const MONO_FONT =
  '12px ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace';
const MONO_LINE_HEIGHT_PX = 17.4;

const SPACER_HEIGHT_PX = 1;
const TURN_STATUS_HEIGHT_PX = 24;
const TOOL_ROW_HEIGHT_PX = 18;
const TOOL_GROUP_GAP_PX = 6;
const TOOL_THOUGHT_TITLE_HEIGHT_PX = 18;
const TOOL_THOUGHT_PRE_PADDING_PX = 18;

const THOUGHT_HORIZONTAL_PADDING_PX = 8;
const THOUGHT_VERTICAL_PADDING_PX = 16;

const TURN_HEADER_OUTER_VERTICAL_PX = 14;
const TURN_HEADER_BUBBLE_HORIZONTAL_PX = 22;
const TURN_HEADER_BUBBLE_VERTICAL_PX = 18;
const TURN_HEADER_COPY_GUTTER_PX = 24;
const TURN_HEADER_COLLAPSED_MAX_HEIGHT_PX = 66;
const TURN_HEADER_ATTACHMENT_SIZE_PX = 44;
const TURN_HEADER_ATTACHMENT_GAP_PX = 6;
const TURN_HEADER_ATTACHMENT_MARGIN_TOP_PX = 8;

const MESSAGE_ROLE_HEIGHT_PX = 12;
const MESSAGE_BUBBLE_HORIZONTAL_PX = 24;
const MESSAGE_BUBBLE_VERTICAL_PX = 20;
const MESSAGE_TOGGLE_HEIGHT_PX = 22;
const MESSAGE_MAX_WIDTH_RATIO = 0.92;
const MESSAGE_BUBBLE_MARGIN_VERTICAL_PX = 12;
const FIXED_ATTACHMENT_WIDTH_PX = 240;
const FIXED_ATTACHMENT_HEIGHT_PX = 180;
const ATTACHMENT_GAP_PX = 8;
const ATTACHMENT_MARGIN_TOP_PX = 8;

const ASSISTANT_HORIZONTAL_PADDING_PX = 4;
const ASSISTANT_VERTICAL_PADDING_PX = 20;

const ASK_CARD_MARGIN_VERTICAL_PX = 16;
const ASK_CARD_PADDING_PX = 24;
const ASK_CARD_GAP_PX = 12;
const ASK_TAB_HEIGHT_PX = 28;
const ASK_BODY_GAP_PX = 12;
const ASK_OPTION_PADDING_VERTICAL_PX = 12;
const ASK_OPTION_PADDING_HORIZONTAL_PX = 16;
const ASK_OPTION_LABEL_LINE_HEIGHT_PX = 18;
const ASK_OPTION_DESC_LINE_HEIGHT_PX = 16;
const ASK_ACTIONS_HEIGHT_PX = 34;
const ASK_HINT_HEIGHT_PX = 14;
const ASK_OTHER_LABEL_HEIGHT_PX = 16;
const ASK_OTHER_INPUT_HEIGHT_PX = 34;

const PREPARED_CACHE_LIMIT = 2000;
const preparedCache = new Map<string, PreparedText>();

const normalizeHeight = (value: number): number =>
  Number.isFinite(value) && value > 0 ? Math.max(1, Math.round(value * 16) / 16) : 1;

const resolveThoughtTextWidth = (viewportWidth: number): number =>
  Math.max(1, resolveSessionThreadContentWidth(viewportWidth) - SESSION_THREAD_INDENT_LEFT_PX - THOUGHT_HORIZONTAL_PADDING_PX);

const resolveTurnHeaderTextWidth = (viewportWidth: number): number =>
  Math.max(
    1,
    resolveSessionThreadContentWidth(viewportWidth) -
      TURN_HEADER_BUBBLE_HORIZONTAL_PX -
      TURN_HEADER_COPY_GUTTER_PX,
  );

const resolveMessageOuterWidth = (viewportWidth: number): number =>
  Math.max(
    1,
    Math.floor((resolveSessionThreadContentWidth(viewportWidth) - SESSION_THREAD_INDENT_LEFT_PX) * MESSAGE_MAX_WIDTH_RATIO),
  );

const resolveMessageTextWidth = (viewportWidth: number): number =>
  Math.max(1, resolveMessageOuterWidth(viewportWidth) - MESSAGE_BUBBLE_HORIZONTAL_PX);

const resolveAssistantTextWidth = (viewportWidth: number): number =>
  Math.max(
    1,
    resolveSessionThreadContentWidth(viewportWidth) -
      SESSION_THREAD_INDENT_LEFT_PX -
      ASSISTANT_HORIZONTAL_PADDING_PX,
  );

const resolveAskCardWidth = (viewportWidth: number): number =>
  Math.max(280, Math.min(resolveSessionThreadContentWidth(viewportWidth) - SESSION_THREAD_INDENT_LEFT_PX, 680));

function getPreparedText(
  cacheKey: string,
  text: string,
  font: string,
  whiteSpace: "normal" | "pre-wrap",
): PreparedText {
  const cached = preparedCache.get(cacheKey);
  if (cached) return cached;
  const prepared = prepare(text, font, whiteSpace === "pre-wrap" ? { whiteSpace } : undefined);
  preparedCache.set(cacheKey, prepared);
  while (preparedCache.size > PREPARED_CACHE_LIMIT) {
    const oldestKey = preparedCache.keys().next().value;
    if (typeof oldestKey !== "string") break;
    preparedCache.delete(oldestKey);
  }
  return prepared;
}

function measureTextHeight(params: {
  cacheKey: string;
  text: string;
  font: string;
  width: number;
  lineHeight: number;
  whiteSpace?: "normal" | "pre-wrap";
}): number {
  const whiteSpace = params.whiteSpace ?? "normal";
  const prepared = getPreparedText(params.cacheKey, params.text, params.font, whiteSpace);
  return normalizeHeight(layout(prepared, Math.max(1, params.width), params.lineHeight).height);
}

const isExpandedTurnHeader = (
  header: WorkbenchTurnHeader,
  plainText: string,
  context: PretextVirtualizerRowLayoutContext,
): boolean => {
  const isLong = plainText.split("\n").length > 4 || plainText.length > 280;
  return context.expandedTurnHeaders?.[header.id] ?? !isLong;
};

const countImageAttachments = (attachments: readonly MessageAttachment[]): number =>
  attachments.filter((attachment) => attachment.kind === "image" || attachment.kind === "image_ref").length;

function countAttachmentRows(attachmentCount: number, width: number): number {
  if (attachmentCount <= 0) return 0;
  const perRow = Math.max(1, Math.floor((width + ATTACHMENT_GAP_PX) / (FIXED_ATTACHMENT_WIDTH_PX + ATTACHMENT_GAP_PX)));
  return Math.ceil(attachmentCount / perRow);
}

function measureTurnHeaderHeight(
  header: WorkbenchTurnHeader,
  viewportWidth: number,
  context: PretextVirtualizerRowLayoutContext,
): number {
  const plainText = header.plain_text ?? header.content ?? "";
  const expanded = isExpandedTurnHeader(header, plainText, context);
  const textHeight = measureTextHeight({
    cacheKey: `turn-header:${header.id}:${plainText}`,
    text: plainText,
    font: BODY_FONT,
    width: resolveTurnHeaderTextWidth(viewportWidth),
    lineHeight: BODY_LINE_HEIGHT_PX,
    whiteSpace: "pre-wrap",
  });
  const collapsedTextHeight = expanded ? textHeight : Math.min(textHeight, TURN_HEADER_COLLAPSED_MAX_HEIGHT_PX);
  const imageCount = expanded ? countImageAttachments(header.attachments) : 0;
  const perRow = Math.max(
    1,
      Math.floor(
      (resolveSessionThreadContentWidth(viewportWidth) -
        TURN_HEADER_BUBBLE_HORIZONTAL_PX +
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
  const rows = countAttachmentRows(imageCount, resolveMessageOuterWidth(viewportWidth));
  return ATTACHMENT_MARGIN_TOP_PX + rows * FIXED_ATTACHMENT_HEIGHT_PX + Math.max(0, rows - 1) * ATTACHMENT_GAP_PX;
}

function measureMessageHeight(
  item: Extract<WorkbenchListItem, { kind: "message" }>,
  viewportWidth: number,
  context: PretextVirtualizerRowLayoutContext,
): number {
  const expanded = resolveWorkbenchMessageExpanded(item, {
    ...(context.expandedMessageById ?? {}),
  });
  const shownContent = expanded ? item.content : item.content.split("\n").slice(0, 20).join("\n");
  const textHeight = measureSessionMarkdownDocument(shownContent, resolveMessageTextWidth(viewportWidth));
  const attachmentsHeight = measureMessageAttachmentsHeight(item, viewportWidth);
  const toggleHeight = isExpandableMessageContent(item.content) ? MESSAGE_TOGGLE_HEIGHT_PX : 0;
  return normalizeHeight(
    MESSAGE_BUBBLE_MARGIN_VERTICAL_PX +
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
  const textHeight = measureSessionMarkdownDocument(item.content, resolveAssistantTextWidth(viewportWidth));
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

function measureAskUserQuestionOption(option: AskUserQuestionItem["options"][number], width: number): number {
  const labelHeight = measureTextHeight({
    cacheKey: `ask-opt-label:${option.label}`,
    text: option.label,
    font: BODY_FONT,
    width,
    lineHeight: ASK_OPTION_LABEL_LINE_HEIGHT_PX,
  });
  const description = option.description?.trim() ?? "";
  const descriptionHeight =
    description.length > 0
      ? measureTextHeight({
          cacheKey: `ask-opt-desc:${description}`,
          text: description,
          font: SMALL_FONT,
          width,
          lineHeight: ASK_OPTION_DESC_LINE_HEIGHT_PX,
        }) + 2
      : 0;
  return ASK_OPTION_PADDING_VERTICAL_PX + labelHeight + descriptionHeight;
}

function measureAskUserQuestionPanel(question: AskUserQuestionItem, width: number): number {
  const questionHeight = measureTextHeight({
    cacheKey: `ask-question:${question.question}`,
    text: question.question,
    font: BODY_FONT,
    width,
    lineHeight: 18.9,
  });
  const optionWidth = Math.max(1, width - ASK_OPTION_PADDING_HORIZONTAL_PX - 38);
  const options = [...question.options];
  if (question.otherLabel) {
    options.push({ label: question.otherLabel, isOther: true });
  }
  let optionsHeight = 0;
  for (let index = 0; index < options.length; index += 1) {
    optionsHeight += measureAskUserQuestionOption(options[index]!, optionWidth);
    if (index < options.length - 1) optionsHeight += 6;
  }
  const otherHeight = question.otherLabel ? ASK_BODY_GAP_PX + ASK_OTHER_LABEL_HEIGHT_PX + 6 + ASK_OTHER_INPUT_HEIGHT_PX : 0;
  return questionHeight + ASK_BODY_GAP_PX + optionsHeight + otherHeight;
}

function measureAskUserQuestionHeight(item: Extract<WorkbenchListItem, { kind: "ask_user_question" }>, viewportWidth: number): number {
  const questions = normalizeAskUserQuestions(item.input);
  if (questions.length === 0) {
    return normalizeHeight(ASK_CARD_MARGIN_VERTICAL_PX + ASK_CARD_PADDING_PX + ASK_ACTIONS_HEIGHT_PX + ASK_HINT_HEIGHT_PX);
  }

  const cardWidth = resolveAskCardWidth(viewportWidth);
  const innerWidth = Math.max(1, cardWidth - ASK_CARD_PADDING_PX);
  const maxPanelHeight = Math.max(
    ...questions.map((question) => measureAskUserQuestionPanel(question, innerWidth)),
    questions.length * 34,
  );
  const tabsHeight = ASK_TAB_HEIGHT_PX;
  const bottomChromeHeight = item.answered
    ? ASK_HINT_HEIGHT_PX
    : ASK_ACTIONS_HEIGHT_PX + ASK_CARD_GAP_PX + ASK_HINT_HEIGHT_PX;

  return normalizeHeight(
    ASK_CARD_MARGIN_VERTICAL_PX +
      ASK_CARD_PADDING_PX +
      tabsHeight +
      ASK_CARD_GAP_PX +
      maxPanelHeight +
      ASK_CARD_GAP_PX +
      bottomChromeHeight,
  );
}

export const clearPretextVirtualizerRowLayoutCache = (): void => {
  preparedCache.clear();
  clearSessionMarkdownMeasurementCaches();
};

export const getPretextVirtualizerRowLayout = (
  item: WorkbenchListItem,
  viewportWidth: number,
  context: PretextVirtualizerRowLayoutContext,
): PretextVirtualizerPlannedLayout => {
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
