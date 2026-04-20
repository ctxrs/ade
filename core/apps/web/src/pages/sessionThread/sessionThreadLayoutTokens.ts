import type { CSSProperties } from "react";
import {
  SESSION_THREAD_GEOMETRY_SPEC,
  resolveSessionThreadAskUserShellHeightPx,
  resolveSessionThreadContentMaxWidthPx,
  resolveSessionThreadMarkdownBlockquoteInsetPx,
  resolveSessionThreadMarkdownInlineCodeEdgeBlockPx,
  resolveSessionThreadMarkdownInlineCodeEdgeInlinePx,
  resolveSessionThreadMarkdownInlineCodeFragmentChromeHeightPx,
  resolveSessionThreadMarkdownInlineCodeFragmentChromeWidthPx,
} from "./sessionThreadGeometrySpec";

const spec = SESSION_THREAD_GEOMETRY_SPEC;

export const SESSION_THREAD_ROW_MAX_WIDTH_PX = spec.viewport.rowMaxWidthPx;
export const SESSION_THREAD_HORIZONTAL_INSET_PX = spec.viewport.horizontalInsetPx;
export const SESSION_THREAD_INDENT_LEFT_PX = spec.viewport.indentLeftPx;
export const SESSION_THREAD_CONTENT_MAX_WIDTH_PX = resolveSessionThreadContentMaxWidthPx(spec);
export const SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX = spec.markdown.typography.bodyFontSizePx;
export const SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX = spec.markdown.typography.bodyLineHeightPx;
export const SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY = spec.markdown.typography.bodyFontFamily;
export const SESSION_THREAD_MARKDOWN_HEADING_FONT_SIZE_PX_BY_DEPTH =
  spec.markdown.typography.headingFontSizePxByDepth;
export const SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH =
  spec.markdown.typography.headingLineHeightPxByDepth;
export const SESSION_THREAD_MARKDOWN_FONT_WEIGHT = spec.markdown.typography.fontWeight;
export const SESSION_THREAD_MARKDOWN_BLOCK_MARGIN_BOTTOM_PX = spec.markdown.blockSpacing.blockMarginBottomPx;
export const SESSION_THREAD_MARKDOWN_HEADING_MARGIN_TOP_PX = spec.markdown.blockSpacing.headingMarginTopPx;
export const SESSION_THREAD_MARKDOWN_HEADING_MARGIN_BOTTOM_PX =
  spec.markdown.blockSpacing.headingMarginBottomPx;
export const SESSION_THREAD_MARKDOWN_LIST_INDENT_PX = spec.markdown.list.indentPx;
export const SESSION_THREAD_MARKDOWN_LIST_GAP_PX = spec.markdown.list.gapPx;
export const SESSION_THREAD_MARKDOWN_LIST_MARKER_MIN_WIDTH_PX = spec.markdown.list.markerMinWidthPx;
export const SESSION_THREAD_MARKDOWN_LIST_MARKER_GAP_PX = spec.markdown.list.markerGapPx;
export const SESSION_THREAD_MARKDOWN_LIST_MARKER_ADVANCE_PX = spec.markdown.list.markerAdvancePx;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_BLOCK_PX = spec.markdown.inlineCode.paddingBlockPx;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_INLINE_PX = spec.markdown.inlineCode.paddingInlinePx;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_BORDER_WIDTH_PX = spec.markdown.inlineCode.borderWidthPx;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_BORDER_RADIUS_PX = spec.markdown.inlineCode.borderRadiusPx;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX =
  spec.markdown.typography.inlineCodeFontSizePx;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY =
  spec.markdown.typography.inlineCodeFontFamily;
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_EDGE_BLOCK_PX =
  resolveSessionThreadMarkdownInlineCodeEdgeBlockPx(spec);
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_EDGE_PX =
  resolveSessionThreadMarkdownInlineCodeEdgeInlinePx(spec);
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_HEIGHT_PX =
  resolveSessionThreadMarkdownInlineCodeFragmentChromeHeightPx(spec);
export const SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX =
  resolveSessionThreadMarkdownInlineCodeFragmentChromeWidthPx(spec);
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX =
  spec.markdown.typography.codeBlockFontSizePx;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX =
  spec.markdown.typography.codeBlockLineHeightPx;
export const SESSION_THREAD_MARKDOWN_BLOCKQUOTE_BORDER_WIDTH_PX = spec.markdown.blockquote.borderWidthPx;
export const SESSION_THREAD_MARKDOWN_BLOCKQUOTE_PADDING_INLINE_START_PX =
  spec.markdown.blockquote.paddingInlineStartPx;
export const SESSION_THREAD_MARKDOWN_BLOCKQUOTE_INSET_PX =
  resolveSessionThreadMarkdownBlockquoteInsetPx(spec);
export const SESSION_THREAD_MARKDOWN_IMAGE_WIDTH_PX = spec.markdown.image.widthPx;
export const SESSION_THREAD_MARKDOWN_IMAGE_HEIGHT_PX = spec.markdown.image.heightPx;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_BORDER_WIDTH_PX = spec.markdown.codeBlock.borderWidthPx;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX = spec.markdown.codeBlock.paddingTopPx;
export const SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX =
  spec.markdown.codeBlock.paddingBottomPx;
export const SESSION_THREAD_MARKDOWN_TABLE_BORDER_WIDTH_PX = spec.markdown.table.borderWidthPx;
export const SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_BLOCK_PX =
  spec.markdown.table.cellPaddingBlockPx;
export const SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_INLINE_PX =
  spec.markdown.table.cellPaddingInlinePx;
export const SESSION_THREAD_MESSAGE_ROW_PADDING_BLOCK_PX = spec.rows.message.rowPaddingBlockPx;
export const SESSION_THREAD_MESSAGE_BUBBLE_PADDING_BLOCK_PX = spec.rows.message.bubblePaddingBlockPx;
export const SESSION_THREAD_MESSAGE_BUBBLE_PADDING_INLINE_PX =
  spec.rows.message.bubblePaddingInlinePx;
export const SESSION_THREAD_MESSAGE_BUBBLE_BORDER_WIDTH_PX =
  spec.rows.message.bubbleBorderWidthPx;
export const SESSION_THREAD_MESSAGE_MAX_WIDTH_RATIO = spec.rows.message.maxWidthRatio;
export const SESSION_THREAD_MESSAGE_ROLE_FONT_SIZE_PX = spec.rows.message.roleFontSizePx;
export const SESSION_THREAD_MESSAGE_ROLE_LINE_HEIGHT_PX = spec.rows.message.roleLineHeightPx;
export const SESSION_THREAD_MESSAGE_TOGGLE_MARGIN_TOP_PX = spec.rows.message.toggleMarginTopPx;
export const SESSION_THREAD_MESSAGE_TOGGLE_FONT_SIZE_PX = spec.rows.message.toggleFontSizePx;
export const SESSION_THREAD_MESSAGE_TOGGLE_LINE_HEIGHT_PX = spec.rows.message.toggleLineHeightPx;
export const SESSION_THREAD_MESSAGE_ATTACHMENT_WIDTH_PX = spec.rows.message.attachments.widthPx;
export const SESSION_THREAD_MESSAGE_ATTACHMENT_HEIGHT_PX = spec.rows.message.attachments.heightPx;
export const SESSION_THREAD_MESSAGE_ATTACHMENT_GAP_PX = spec.rows.message.attachments.gapPx;
export const SESSION_THREAD_MESSAGE_ATTACHMENT_MARGIN_TOP_PX =
  spec.rows.message.attachments.marginTopPx;
export const SESSION_THREAD_ASSISTANT_ENTRY_PADDING_INLINE_PX =
  spec.rows.assistant.entryPaddingInlinePx;
export const SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_BLOCK_PX =
  spec.rows.turnHeader.bubblePaddingBlockPx;
export const SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_INLINE_PX =
  spec.rows.turnHeader.bubblePaddingInlinePx;
export const SESSION_THREAD_TURN_HEADER_BUBBLE_BORDER_WIDTH_PX =
  spec.rows.turnHeader.bubbleBorderWidthPx;
export const SESSION_THREAD_TURN_HEADER_COPY_GUTTER_PX = spec.rows.turnHeader.copyGutterPx;
export const SESSION_THREAD_TURN_HEADER_COLLAPSED_MAX_HEIGHT_PX =
  spec.rows.turnHeader.collapsedMaxHeightPx;
export const SESSION_THREAD_ASK_USER_MARGIN_VERTICAL_PX = spec.rows.askUser.marginVerticalPx;
export const SESSION_THREAD_ASK_USER_CARD_MAX_WIDTH_PX = spec.rows.askUser.cardMaxWidthPx;
export const SESSION_THREAD_ASK_USER_CARD_MIN_WIDTH_PX = spec.rows.askUser.cardMinWidthPx;
export const SESSION_THREAD_ASK_USER_CARD_PADDING_PX = spec.rows.askUser.cardPaddingPx;
export const SESSION_THREAD_ASK_USER_CARD_GAP_PX = spec.rows.askUser.cardGapPx;
export const SESSION_THREAD_ASK_USER_TABS_HEIGHT_PX = spec.rows.askUser.tabsHeightPx;
export const SESSION_THREAD_ASK_USER_PANEL_HEIGHT_PX = spec.rows.askUser.panelHeightPx;
export const SESSION_THREAD_ASK_USER_STATUS_HEIGHT_PX = spec.rows.askUser.statusHeightPx;
export const SESSION_THREAD_ASK_USER_ACTIONS_HEIGHT_PX = spec.rows.askUser.actionsHeightPx;
export const SESSION_THREAD_ASK_USER_HINT_HEIGHT_PX = spec.rows.askUser.hintHeightPx;
export const SESSION_THREAD_ASK_USER_SHELL_HEIGHT_PX = resolveSessionThreadAskUserShellHeightPx(spec);

export const SESSION_THREAD_LAYOUT_STYLE = {
  "--wb-thread-max-width": `${SESSION_THREAD_ROW_MAX_WIDTH_PX}px`,
  "--wb-session-padding-inline": `${SESSION_THREAD_HORIZONTAL_INSET_PX}px`,
  "--wb-transcript-max-width": `${SESSION_THREAD_ROW_MAX_WIDTH_PX}px`,
  "--wb-markdown-body-font-size": `${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px`,
  "--wb-markdown-body-line-height": `${SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX}px`,
  "--wb-markdown-body-font-family": SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY,
  "--wb-markdown-heading-1-font-size": `${SESSION_THREAD_MARKDOWN_HEADING_FONT_SIZE_PX_BY_DEPTH[1]}px`,
  "--wb-markdown-heading-2-font-size": `${SESSION_THREAD_MARKDOWN_HEADING_FONT_SIZE_PX_BY_DEPTH[2]}px`,
  "--wb-markdown-heading-3-font-size": `${SESSION_THREAD_MARKDOWN_HEADING_FONT_SIZE_PX_BY_DEPTH[3]}px`,
  "--wb-markdown-heading-4-font-size": `${SESSION_THREAD_MARKDOWN_HEADING_FONT_SIZE_PX_BY_DEPTH[4]}px`,
  "--wb-markdown-heading-1-line-height": `${SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[1]}px`,
  "--wb-markdown-heading-2-line-height": `${SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[2]}px`,
  "--wb-markdown-heading-3-line-height": `${SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[3]}px`,
  "--wb-markdown-heading-4-line-height": `${SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[4]}px`,
  "--wb-markdown-heading-font-weight": String(SESSION_THREAD_MARKDOWN_FONT_WEIGHT.heading),
  "--wb-markdown-table-header-font-weight": String(SESSION_THREAD_MARKDOWN_FONT_WEIGHT.tableHeader),
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
  "--wb-turn-header-bubble-padding-block": `${SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_BLOCK_PX}px`,
  "--wb-turn-header-bubble-padding-inline": `${SESSION_THREAD_TURN_HEADER_BUBBLE_PADDING_INLINE_PX}px`,
  "--wb-turn-header-bubble-border-width": `${SESSION_THREAD_TURN_HEADER_BUBBLE_BORDER_WIDTH_PX}px`,
  "--wb-turn-header-copy-gutter": `${SESSION_THREAD_TURN_HEADER_COPY_GUTTER_PX}px`,
  "--wb-turn-header-collapsed-max-height": `${SESSION_THREAD_TURN_HEADER_COLLAPSED_MAX_HEIGHT_PX}px`,
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
  return Math.max(
    1,
    resolveSessionThreadIndentedContentWidth(viewportWidth) -
      SESSION_THREAD_ASSISTANT_ENTRY_PADDING_INLINE_PX * 2,
  );
}

export function resolveSessionThreadTurnHeaderTextWidth(viewportWidth: number): number {
  return Math.max(
    1,
    resolveSessionThreadContentWidth(viewportWidth) -
      SESSION_THREAD_TURN_HEADER_BUBBLE_BORDER_WIDTH_PX * 2 -
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
