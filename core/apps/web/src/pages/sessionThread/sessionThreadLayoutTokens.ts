import type { CSSProperties } from "react";

export const SESSION_THREAD_ROW_MAX_WIDTH_PX = 820;
export const SESSION_THREAD_HORIZONTAL_INSET_PX = 12;
export const SESSION_THREAD_INDENT_LEFT_PX = 4;
export const SESSION_THREAD_CONTENT_MAX_WIDTH_PX =
  SESSION_THREAD_ROW_MAX_WIDTH_PX - SESSION_THREAD_HORIZONTAL_INSET_PX * 2;
export const SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX = 13;
export const SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX = 20;
export const SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY =
  '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';
export const SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH = {
  1: 22,
  2: 20,
  3: 18,
  4: 16,
} as const;
export const SESSION_THREAD_MARKDOWN_BLOCK_MARGIN_BOTTOM_PX = 12;
export const SESSION_THREAD_MARKDOWN_HEADING_MARGIN_TOP_PX = 16;
export const SESSION_THREAD_MARKDOWN_HEADING_MARGIN_BOTTOM_PX = 8;
export const SESSION_THREAD_MARKDOWN_LIST_INDENT_PX = 16.25;
export const SESSION_THREAD_MARKDOWN_LIST_GAP_PX = 4;
export const SESSION_THREAD_MARKDOWN_LIST_MARKER_MIN_WIDTH_PX = 18;
export const SESSION_THREAD_MARKDOWN_LIST_MARKER_GAP_PX = 6;
export const SESSION_THREAD_MARKDOWN_LIST_MARKER_ADVANCE_PX = 7.25;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_BLOCK_PX = 1;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_INLINE_PX = 6;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_BORDER_WIDTH_PX = 1;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_BORDER_RADIUS_PX = 6;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX =
  SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY =
  '"SF Mono", Menlo, Monaco, Consolas, "Courier New", monospace';
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_EDGE_BLOCK_PX =
  SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_BLOCK_PX + SESSION_THREAD_MARKDOWN_INLINE_CODE_BORDER_WIDTH_PX;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_EDGE_PX =
  SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_INLINE_PX + SESSION_THREAD_MARKDOWN_INLINE_CODE_BORDER_WIDTH_PX;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_HEIGHT_PX =
  SESSION_THREAD_MARKDOWN_INLINE_CODE_EDGE_BLOCK_PX * 2;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX =
  SESSION_THREAD_MARKDOWN_INLINE_CODE_EDGE_PX * 2;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX = 12;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX = 17;
export const SESSION_THREAD_MARKDOWN_BLOCKQUOTE_BORDER_WIDTH_PX = 2;
export const SESSION_THREAD_MARKDOWN_BLOCKQUOTE_PADDING_INLINE_START_PX = 12;
export const SESSION_THREAD_MARKDOWN_BLOCKQUOTE_INSET_PX =
  SESSION_THREAD_MARKDOWN_BLOCKQUOTE_BORDER_WIDTH_PX + SESSION_THREAD_MARKDOWN_BLOCKQUOTE_PADDING_INLINE_START_PX;
export const SESSION_THREAD_MARKDOWN_IMAGE_WIDTH_PX = 240;
export const SESSION_THREAD_MARKDOWN_IMAGE_HEIGHT_PX = 180;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_BORDER_WIDTH_PX = 1;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX = 28;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX = 12;
export const SESSION_THREAD_MARKDOWN_TABLE_BORDER_WIDTH_PX = 1;
export const SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_BLOCK_PX = 8;
export const SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_INLINE_PX = 12;
export const SESSION_THREAD_MESSAGE_ROW_PADDING_BLOCK_PX = 6;
export const SESSION_THREAD_MESSAGE_BUBBLE_PADDING_BLOCK_PX = 10;
export const SESSION_THREAD_MESSAGE_BUBBLE_PADDING_INLINE_PX = 12;
export const SESSION_THREAD_MESSAGE_BUBBLE_BORDER_WIDTH_PX = 1;
export const SESSION_THREAD_MESSAGE_MAX_WIDTH_RATIO = 0.92;
export const SESSION_THREAD_MESSAGE_ROLE_FONT_SIZE_PX = 11;
export const SESSION_THREAD_MESSAGE_ROLE_LINE_HEIGHT_PX = 11;
export const SESSION_THREAD_MESSAGE_TOGGLE_MARGIN_TOP_PX = 6;
export const SESSION_THREAD_MESSAGE_TOGGLE_FONT_SIZE_PX = SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX;
export const SESSION_THREAD_MESSAGE_TOGGLE_LINE_HEIGHT_PX = 16;
export const SESSION_THREAD_MESSAGE_ATTACHMENT_WIDTH_PX = 240;
export const SESSION_THREAD_MESSAGE_ATTACHMENT_HEIGHT_PX = 180;
export const SESSION_THREAD_MESSAGE_ATTACHMENT_GAP_PX = 8;
export const SESSION_THREAD_MESSAGE_ATTACHMENT_MARGIN_TOP_PX = 8;
export const SESSION_THREAD_ASSISTANT_ENTRY_PADDING_INLINE_PX = 2;
export const SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_INLINE_PX = 10;
export const SESSION_THREAD_TURN_HEADER_COPY_GUTTER_PX = 24;
export const SESSION_THREAD_ASK_USER_MARGIN_VERTICAL_PX = 16;
export const SESSION_THREAD_ASK_USER_CARD_MAX_WIDTH_PX = 680;
export const SESSION_THREAD_ASK_USER_CARD_MIN_WIDTH_PX = 280;
export const SESSION_THREAD_ASK_USER_CARD_PADDING_PX = 12;
export const SESSION_THREAD_ASK_USER_CARD_GAP_PX = 12;
export const SESSION_THREAD_ASK_USER_TABS_HEIGHT_PX = 32;
export const SESSION_THREAD_ASK_USER_PANEL_HEIGHT_PX = 208;
export const SESSION_THREAD_ASK_USER_STATUS_HEIGHT_PX = 16;
export const SESSION_THREAD_ASK_USER_ACTIONS_HEIGHT_PX = 34;
export const SESSION_THREAD_ASK_USER_HINT_HEIGHT_PX = 14;
export const SESSION_THREAD_ASK_USER_SHELL_HEIGHT_PX =
  SESSION_THREAD_ASK_USER_CARD_PADDING_PX * 2 +
  SESSION_THREAD_ASK_USER_TABS_HEIGHT_PX +
  SESSION_THREAD_ASK_USER_CARD_GAP_PX +
  SESSION_THREAD_ASK_USER_PANEL_HEIGHT_PX +
  SESSION_THREAD_ASK_USER_CARD_GAP_PX +
  SESSION_THREAD_ASK_USER_STATUS_HEIGHT_PX +
  SESSION_THREAD_ASK_USER_CARD_GAP_PX +
  SESSION_THREAD_ASK_USER_ACTIONS_HEIGHT_PX +
  SESSION_THREAD_ASK_USER_CARD_GAP_PX +
  SESSION_THREAD_ASK_USER_HINT_HEIGHT_PX;

export const SESSION_THREAD_LAYOUT_STYLE = {
  "--wb-thread-max-width": `${SESSION_THREAD_ROW_MAX_WIDTH_PX}px`,
  "--wb-session-padding-inline": `${SESSION_THREAD_HORIZONTAL_INSET_PX}px`,
  "--wb-transcript-max-width": `${SESSION_THREAD_ROW_MAX_WIDTH_PX}px`,
  "--wb-markdown-body-font-size": `${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px`,
  "--wb-markdown-body-line-height": `${SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX}px`,
  "--wb-markdown-body-font-family": SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY,
  "--wb-markdown-heading-1-line-height": `${SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[1]}px`,
  "--wb-markdown-heading-2-line-height": `${SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[2]}px`,
  "--wb-markdown-heading-3-line-height": `${SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[3]}px`,
  "--wb-markdown-heading-4-line-height": `${SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[4]}px`,
  "--wb-markdown-block-margin-bottom": `${SESSION_THREAD_MARKDOWN_BLOCK_MARGIN_BOTTOM_PX}px`,
  "--wb-markdown-heading-margin-top": `${SESSION_THREAD_MARKDOWN_HEADING_MARGIN_TOP_PX}px`,
  "--wb-markdown-heading-margin-bottom": `${SESSION_THREAD_MARKDOWN_HEADING_MARGIN_BOTTOM_PX}px`,
  "--wb-markdown-list-indent": `${SESSION_THREAD_MARKDOWN_LIST_INDENT_PX}px`,
  "--wb-markdown-list-gap": `${SESSION_THREAD_MARKDOWN_LIST_GAP_PX}px`,
  "--wb-markdown-list-marker-min-width": `${SESSION_THREAD_MARKDOWN_LIST_MARKER_MIN_WIDTH_PX}px`,
  "--wb-markdown-list-marker-gap": `${SESSION_THREAD_MARKDOWN_LIST_MARKER_GAP_PX}px`,
  "--wb-markdown-inline-code-padding-block": `${SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_BLOCK_PX}px`,
  "--wb-markdown-inline-code-padding-inline": `${SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_INLINE_PX}px`,
  "--wb-markdown-inline-code-border-width": `${SESSION_THREAD_MARKDOWN_INLINE_CODE_BORDER_WIDTH_PX}px`,
  "--wb-markdown-inline-code-border-radius": `${SESSION_THREAD_MARKDOWN_INLINE_CODE_BORDER_RADIUS_PX}px`,
  "--wb-markdown-inline-code-font-size": `${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX}px`,
  "--wb-markdown-inline-code-font-family": SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY,
  "--wb-markdown-code-block-font-size": `${SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX}px`,
  "--wb-markdown-code-block-line-height": `${SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX}px`,
  "--wb-markdown-blockquote-border-width": `${SESSION_THREAD_MARKDOWN_BLOCKQUOTE_BORDER_WIDTH_PX}px`,
  "--wb-markdown-blockquote-padding-inline-start": `${SESSION_THREAD_MARKDOWN_BLOCKQUOTE_PADDING_INLINE_START_PX}px`,
  "--wb-markdown-image-width": `${SESSION_THREAD_MARKDOWN_IMAGE_WIDTH_PX}px`,
  "--wb-markdown-image-height": `${SESSION_THREAD_MARKDOWN_IMAGE_HEIGHT_PX}px`,
  "--wb-markdown-table-border-width": `${SESSION_THREAD_MARKDOWN_TABLE_BORDER_WIDTH_PX}px`,
  "--wb-markdown-table-cell-padding-block": `${SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_BLOCK_PX}px`,
  "--wb-markdown-table-cell-padding-inline": `${SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_INLINE_PX}px`,
  "--wb-message-row-padding-block": `${SESSION_THREAD_MESSAGE_ROW_PADDING_BLOCK_PX}px`,
  "--wb-message-bubble-padding-block": `${SESSION_THREAD_MESSAGE_BUBBLE_PADDING_BLOCK_PX}px`,
  "--wb-message-bubble-padding-inline": `${SESSION_THREAD_MESSAGE_BUBBLE_PADDING_INLINE_PX}px`,
  "--wb-message-bubble-border-width": `${SESSION_THREAD_MESSAGE_BUBBLE_BORDER_WIDTH_PX}px`,
  "--wb-message-max-width": `${SESSION_THREAD_MESSAGE_MAX_WIDTH_RATIO * 100}%`,
  "--wb-message-role-font-size": `${SESSION_THREAD_MESSAGE_ROLE_FONT_SIZE_PX}px`,
  "--wb-message-role-line-height": `${SESSION_THREAD_MESSAGE_ROLE_LINE_HEIGHT_PX}px`,
  "--wb-message-toggle-margin-top": `${SESSION_THREAD_MESSAGE_TOGGLE_MARGIN_TOP_PX}px`,
  "--wb-message-toggle-font-size": `${SESSION_THREAD_MESSAGE_TOGGLE_FONT_SIZE_PX}px`,
  "--wb-message-toggle-line-height": `${SESSION_THREAD_MESSAGE_TOGGLE_LINE_HEIGHT_PX}px`,
  "--wb-message-attachment-width": `${SESSION_THREAD_MESSAGE_ATTACHMENT_WIDTH_PX}px`,
  "--wb-message-attachment-height": `${SESSION_THREAD_MESSAGE_ATTACHMENT_HEIGHT_PX}px`,
  "--wb-message-attachment-gap": `${SESSION_THREAD_MESSAGE_ATTACHMENT_GAP_PX}px`,
  "--wb-message-attachment-margin-top": `${SESSION_THREAD_MESSAGE_ATTACHMENT_MARGIN_TOP_PX}px`,
  "--wb-askq-shell-height": `${SESSION_THREAD_ASK_USER_SHELL_HEIGHT_PX}px`,
  "--wb-askq-card-gap": `${SESSION_THREAD_ASK_USER_CARD_GAP_PX}px`,
  "--wb-askq-tabs-height": `${SESSION_THREAD_ASK_USER_TABS_HEIGHT_PX}px`,
  "--wb-askq-panel-height": `${SESSION_THREAD_ASK_USER_PANEL_HEIGHT_PX}px`,
  "--wb-askq-status-height": `${SESSION_THREAD_ASK_USER_STATUS_HEIGHT_PX}px`,
  "--wb-askq-actions-height": `${SESSION_THREAD_ASK_USER_ACTIONS_HEIGHT_PX}px`,
  "--wb-askq-hint-height": `${SESSION_THREAD_ASK_USER_HINT_HEIGHT_PX}px`,
} as CSSProperties;

export function resolveSessionThreadRowWidth(viewportWidth: number): number {
  return Math.max(1, Math.min(SESSION_THREAD_ROW_MAX_WIDTH_PX, Math.floor(viewportWidth)));
}

export function resolveSessionThreadContentWidth(viewportWidth: number): number {
  return Math.max(1, resolveSessionThreadRowWidth(viewportWidth) - SESSION_THREAD_HORIZONTAL_INSET_PX * 2);
}

export function resolveSessionThreadIndentedContentWidth(viewportWidth: number): number {
  return Math.max(1, resolveSessionThreadContentWidth(viewportWidth) - SESSION_THREAD_INDENT_LEFT_PX);
}

export function resolveSessionThreadMessageBubbleBorderBoxWidth(viewportWidth: number): number {
  return Math.max(1, resolveSessionThreadIndentedContentWidth(viewportWidth) * SESSION_THREAD_MESSAGE_MAX_WIDTH_RATIO);
}

export function resolveSessionThreadMessageTextWidth(viewportWidth: number): number {
  return Math.max(
    1,
    resolveSessionThreadMessageBubbleBorderBoxWidth(viewportWidth) -
      SESSION_THREAD_MESSAGE_BUBBLE_PADDING_INLINE_PX * 2 -
      SESSION_THREAD_MESSAGE_BUBBLE_BORDER_WIDTH_PX * 2,
  );
}

export function resolveSessionThreadAssistantTextWidth(viewportWidth: number): number {
  return Math.max(1, resolveSessionThreadIndentedContentWidth(viewportWidth) - SESSION_THREAD_ASSISTANT_ENTRY_PADDING_INLINE_PX * 2);
}

export function resolveSessionThreadTurnHeaderTextWidth(viewportWidth: number): number {
  return Math.max(
    1,
    resolveSessionThreadContentWidth(viewportWidth) -
      SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_INLINE_PX * 2 -
      SESSION_THREAD_TURN_HEADER_COPY_GUTTER_PX,
  );
}

export function resolveSessionThreadAskUserCardWidth(viewportWidth: number): number {
  return Math.max(
    SESSION_THREAD_ASK_USER_CARD_MIN_WIDTH_PX,
    Math.min(resolveSessionThreadIndentedContentWidth(viewportWidth), SESSION_THREAD_ASK_USER_CARD_MAX_WIDTH_PX),
  );
}

export function resolveSessionMarkdownListMarkerColumnWidthPx(markerTexts: readonly string[]): number {
  const maxTextLength = markerTexts.reduce((max, text) => Math.max(max, String(text ?? "").length), 0);
  return Math.max(
    SESSION_THREAD_MARKDOWN_LIST_MARKER_MIN_WIDTH_PX,
    Math.ceil(maxTextLength * SESSION_THREAD_MARKDOWN_LIST_MARKER_ADVANCE_PX),
  );
}
