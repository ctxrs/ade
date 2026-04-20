import {
  SESSION_THREAD_GEOMETRY_REVISION,
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

export const SESSION_THREAD_MEASUREMENT_GEOMETRY_REVISION = SESSION_THREAD_GEOMETRY_REVISION;

export const SESSION_MARKDOWN_MEASUREMENT_CONTRACT = {
  geometryRevision: SESSION_THREAD_MEASUREMENT_GEOMETRY_REVISION,
  typography: {
    bodyFontFamily: spec.markdown.typography.bodyFontFamily,
    bodyFontSizePx: spec.markdown.typography.bodyFontSizePx,
    bodyLineHeightPx: spec.markdown.typography.bodyLineHeightPx,
    headingFontSizePxByDepth: spec.markdown.typography.headingFontSizePxByDepth,
    inlineCodeFontFamily: spec.markdown.typography.inlineCodeFontFamily,
    inlineCodeFontSizePx: spec.markdown.typography.inlineCodeFontSizePx,
    codeBlockFontSizePx: spec.markdown.typography.codeBlockFontSizePx,
    codeBlockLineHeightPx: spec.markdown.typography.codeBlockLineHeightPx,
    headingLineHeightPxByDepth: spec.markdown.typography.headingLineHeightPxByDepth,
    fontWeight: spec.markdown.typography.fontWeight,
  },
  inlineCode: {
    paddingBlockPx: spec.markdown.inlineCode.paddingBlockPx,
    paddingInlinePx: spec.markdown.inlineCode.paddingInlinePx,
    borderWidthPx: spec.markdown.inlineCode.borderWidthPx,
    borderRadiusPx: spec.markdown.inlineCode.borderRadiusPx,
    edgeBlockPx: resolveSessionThreadMarkdownInlineCodeEdgeBlockPx(spec),
    edgeInlinePx: resolveSessionThreadMarkdownInlineCodeEdgeInlinePx(spec),
    fragmentChromeHeightPx: resolveSessionThreadMarkdownInlineCodeFragmentChromeHeightPx(spec),
    fragmentChromeWidthPx: resolveSessionThreadMarkdownInlineCodeFragmentChromeWidthPx(spec),
  },
  blockSpacing: {
    blockMarginBottomPx: spec.markdown.blockSpacing.blockMarginBottomPx,
    headingMarginTopPx: spec.markdown.blockSpacing.headingMarginTopPx,
    headingMarginBottomPx: spec.markdown.blockSpacing.headingMarginBottomPx,
    entryGapPxByContext: spec.markdown.blockSpacing.entryGapPxByContext,
    exitGapPxByContext: spec.markdown.blockSpacing.exitGapPxByContext,
  },
  list: {
    indentPx: spec.markdown.list.indentPx,
    gapPx: spec.markdown.list.gapPx,
    markerMinWidthPx: spec.markdown.list.markerMinWidthPx,
    markerGapPx: spec.markdown.list.markerGapPx,
    markerAdvancePx: spec.markdown.list.markerAdvancePx,
    checkboxGutterPx: spec.markdown.list.checkboxGutterPx,
  },
  blockquote: {
    borderWidthPx: spec.markdown.blockquote.borderWidthPx,
    paddingInlineStartPx: spec.markdown.blockquote.paddingInlineStartPx,
    insetPx: resolveSessionThreadMarkdownBlockquoteInsetPx(spec),
  },
  codeBlock: {
    borderWidthPx: spec.markdown.codeBlock.borderWidthPx,
    paddingTopPx: spec.markdown.codeBlock.paddingTopPx,
    paddingBottomPx: spec.markdown.codeBlock.paddingBottomPx,
  },
  image: {
    widthPx: spec.markdown.image.widthPx,
    heightPx: spec.markdown.image.heightPx,
  },
  table: {
    borderWidthPx: spec.markdown.table.borderWidthPx,
    cellPaddingBlockPx: spec.markdown.table.cellPaddingBlockPx,
    cellPaddingInlinePx: spec.markdown.table.cellPaddingInlinePx,
  },
} as const;

export const SESSION_THREAD_ROW_MEASUREMENT_CONTRACT = {
  geometryRevision: SESSION_THREAD_MEASUREMENT_GEOMETRY_REVISION,
  viewport: {
    rowMaxWidthPx: spec.viewport.rowMaxWidthPx,
    contentMaxWidthPx: resolveSessionThreadContentMaxWidthPx(spec),
    horizontalInsetPx: spec.viewport.horizontalInsetPx,
    indentLeftPx: spec.viewport.indentLeftPx,
  },
  assistant: {
    entryPaddingInlinePx: spec.rows.assistant.entryPaddingInlinePx,
    verticalPaddingPx: spec.rows.assistant.verticalPaddingPx,
  },
  message: {
    rowPaddingBlockPx: spec.rows.message.rowPaddingBlockPx,
    bubblePaddingBlockPx: spec.rows.message.bubblePaddingBlockPx,
    bubblePaddingInlinePx: spec.rows.message.bubblePaddingInlinePx,
    bubbleBorderWidthPx: spec.rows.message.bubbleBorderWidthPx,
    maxWidthRatio: spec.rows.message.maxWidthRatio,
    roleFontSizePx: spec.rows.message.roleFontSizePx,
    roleLineHeightPx: spec.rows.message.roleLineHeightPx,
    toggleMarginTopPx: spec.rows.message.toggleMarginTopPx,
    toggleFontSizePx: spec.rows.message.toggleFontSizePx,
    toggleLineHeightPx: spec.rows.message.toggleLineHeightPx,
    attachments: {
      widthPx: spec.rows.message.attachments.widthPx,
      heightPx: spec.rows.message.attachments.heightPx,
      gapPx: spec.rows.message.attachments.gapPx,
      marginTopPx: spec.rows.message.attachments.marginTopPx,
    },
  },
  turnHeader: {
    bubblePaddingBlockPx: spec.rows.turnHeader.bubblePaddingBlockPx,
    bubblePaddingInlinePx: spec.rows.turnHeader.bubblePaddingInlinePx,
    bubbleBorderWidthPx: spec.rows.turnHeader.bubbleBorderWidthPx,
    collapsedMaxHeightPx: spec.rows.turnHeader.collapsedMaxHeightPx,
    copyGutterPx: spec.rows.turnHeader.copyGutterPx,
    outerVerticalPx: spec.rows.turnHeader.outerVerticalPx,
    attachments: {
      sizePx: spec.rows.turnHeader.attachments.sizePx,
      gapPx: spec.rows.turnHeader.attachments.gapPx,
      marginTopPx: spec.rows.turnHeader.attachments.marginTopPx,
    },
  },
  askUser: {
    marginVerticalPx: spec.rows.askUser.marginVerticalPx,
    cardMinWidthPx: spec.rows.askUser.cardMinWidthPx,
    cardMaxWidthPx: spec.rows.askUser.cardMaxWidthPx,
    cardPaddingPx: spec.rows.askUser.cardPaddingPx,
    cardGapPx: spec.rows.askUser.cardGapPx,
    tabsHeightPx: spec.rows.askUser.tabsHeightPx,
    panelHeightPx: spec.rows.askUser.panelHeightPx,
    statusHeightPx: spec.rows.askUser.statusHeightPx,
    actionsHeightPx: spec.rows.askUser.actionsHeightPx,
    hintHeightPx: spec.rows.askUser.hintHeightPx,
    shellHeightPx: resolveSessionThreadAskUserShellHeightPx(spec),
    outerHeightPx: spec.rows.askUser.marginVerticalPx + resolveSessionThreadAskUserShellHeightPx(spec),
  },
  thought: {
    horizontalPaddingPx: spec.rows.thought.horizontalPaddingPx,
    verticalPaddingPx: spec.rows.thought.verticalPaddingPx,
  },
  tools: {
    rowHeightPx: spec.rows.tools.rowHeightPx,
    itemGapPx: spec.rows.tools.itemGapPx,
    groupGapPx: spec.rows.tools.groupGapPx,
    loadingLineHeightPx: spec.rows.tools.loadingLineHeightPx,
    thoughtTitleHeightPx: spec.rows.tools.thoughtTitleHeightPx,
    thoughtPrePaddingPx: spec.rows.tools.thoughtPrePaddingPx,
  },
  fixed: {
    spacerHeightPx: spec.rows.fixed.spacerHeightPx,
    turnStatusHeightPx: spec.rows.fixed.turnStatusHeightPx,
  },
} as const;
