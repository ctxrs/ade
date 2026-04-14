import {
  addPretextPerfBucket,
  hashPretextPerfValue,
  incrementPretextPerfCounter,
} from "../../utils/pretextPerfDiagnostics";
import {
  resolveSessionMarkdownBlockEntryGapPx,
  resolveSessionMarkdownBlockGapPx,
  type SessionMarkdownBlock,
  type SessionMarkdownBlockContext,
  type SessionMarkdownInlineRun,
} from "./sessionMarkdownContract";
import { measureInlineRunsHeight } from "./sessionMarkdownInlineMeasurement";
import {
  BODY_LINE_HEIGHT_PX,
  BODY_TYPOGRAPHY,
  CODE_BLOCK_VERTICAL_PADDING_PX,
  MONO_LINE_HEIGHT_PX,
  TABLE_HEADER_TYPOGRAPHY,
  buildHeadingTypography,
  clampHeight,
  measureTextHeight,
  normalizeHeight,
  parseMarkdown,
  type TextBlockTypography,
} from "./sessionMarkdownMeasurementCore";
import {
  SESSION_THREAD_MARKDOWN_BLOCKQUOTE_INSET_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_BORDER_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_IMAGE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_LIST_GAP_PX,
  SESSION_THREAD_MARKDOWN_LIST_MARKER_GAP_PX,
  SESSION_THREAD_MARKDOWN_TABLE_BORDER_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_BLOCK_PX,
  SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_INLINE_PX,
} from "./sessionThreadLayoutTokens";

const CHECKBOX_GUTTER_PX = 18;
const LIST_ITEM_BODY_WIDTH_FIT_BIAS_PX = 0;

function measureTextBlock(params: {
  text: {
    plainText: string;
    runs: readonly SessionMarkdownInlineRun[];
    hasInlineCode: boolean;
    hasHardBreak: boolean;
    hasStyledText: boolean;
  };
  width: number;
  typography: TextBlockTypography;
  cacheKeyPrefix: string;
}): number {
  const text = params.text.plainText.trim();
  if (!text) {
    return 0;
  }
  if (params.text.hasHardBreak && !params.text.hasInlineCode && !params.text.hasStyledText) {
    return normalizeHeight(
      params.text.plainText
        .split("\n")
        .reduce(
          (sum, line) =>
            sum +
            (line.length === 0
              ? params.typography.lineHeight
              : measureTextHeight({
                  cacheKey: `${params.cacheKeyPrefix}:line:${line}`,
                  text: line,
                  font: params.typography.body,
                  width: params.width,
                  lineHeight: params.typography.lineHeight,
                })),
          0,
        ),
    );
  }
  if (!params.text.hasInlineCode && !params.text.hasHardBreak && !params.text.hasStyledText) {
    return measureTextHeight({
      cacheKey: `${params.cacheKeyPrefix}:${text}`,
      text,
      font: params.typography.body,
      width: params.width,
      lineHeight: params.typography.lineHeight,
    });
  }
  return measureInlineRunsHeight({
    runs: params.text.runs,
    width: params.width,
    typography: params.typography,
    cacheKeyPrefix: params.cacheKeyPrefix,
  });
}

function measureParagraph(block: Extract<SessionMarkdownBlock, { kind: "paragraph" }>, width: number): number {
  return measureTextBlock({
    text: block.text,
    width,
    typography: BODY_TYPOGRAPHY,
    cacheKeyPrefix: "paragraph-inline",
  });
}

function measureHeading(block: Extract<SessionMarkdownBlock, { kind: "heading" }>, width: number): number {
  return measureTextBlock({
    text: block.text,
    width,
    typography: buildHeadingTypography(block.depth),
    cacheKeyPrefix: `heading-inline:${block.depth}`,
  });
}

function measureCodeBlock(block: Extract<SessionMarkdownBlock, { kind: "code" }>): number {
  const lineCount = Math.max(1, block.code.replace(/\n$/, "").split("\n").length);
  const textHeight = clampHeight(lineCount * MONO_LINE_HEIGHT_PX);
  return (
    CODE_BLOCK_VERTICAL_PADDING_PX +
    textHeight +
    SESSION_THREAD_MARKDOWN_CODE_BLOCK_BORDER_WIDTH_PX * 2
  );
}

function measureListItem(
  item: Extract<SessionMarkdownBlock, { kind: "list" }>["items"][number],
  width: number,
  bulletInsetPx: number,
): number {
  const bodyInsetPx = item.checked != null ? bulletInsetPx : bulletInsetPx + LIST_ITEM_BODY_WIDTH_FIT_BIAS_PX;
  const childWidth = Math.max(1, width - bodyInsetPx);
  if (item.blocks.length === 0) {
    return clampHeight(BODY_LINE_HEIGHT_PX);
  }
  return Math.max(clampHeight(BODY_LINE_HEIGHT_PX), measureBlockChildren(item.blocks, childWidth, "listItem"));
}

function measureList(block: Extract<SessionMarkdownBlock, { kind: "list" }>, width: number): number {
  let total = 0;
  for (let index = 0; index < block.items.length; index += 1) {
    const item = block.items[index]!;
    const markerInsetPx =
      item.checked != null
        ? CHECKBOX_GUTTER_PX
        : block.markerColumnWidthPx + SESSION_THREAD_MARKDOWN_LIST_MARKER_GAP_PX;
    total += measureListItem(item, width, markerInsetPx);
    if (index < block.items.length - 1) total += SESSION_THREAD_MARKDOWN_LIST_GAP_PX;
  }
  return total;
}

function measureBlockQuote(block: Extract<SessionMarkdownBlock, { kind: "blockquote" }>, width: number): number {
  return measureBlockChildren(
    block.blocks,
    Math.max(1, width - SESSION_THREAD_MARKDOWN_BLOCKQUOTE_INSET_PX),
    "root",
  );
}

function measureTableCellHeight(
  cell: Extract<SessionMarkdownBlock, { kind: "table" }>["rows"][number]["cells"][number] | null | undefined,
  width: number,
  isHeader: boolean,
): number {
  if (!cell) {
    return clampHeight(BODY_LINE_HEIGHT_PX);
  }
  if (cell.blocks.length === 0) {
    return clampHeight(BODY_LINE_HEIGHT_PX);
  }
  return cell.blocks.reduce((total, block, index) => {
    const marginTopPx =
      index === 0
        ? resolveSessionMarkdownBlockEntryGapPx(block.kind, "root")
        : resolveSessionMarkdownBlockGapPx(cell.blocks[index - 1]!.kind, block.kind, "root");
    let blockHeight: number;
    switch (block.kind) {
      case "paragraph":
        blockHeight = measureTextBlock({
          text: block.text,
          width,
          typography: isHeader ? TABLE_HEADER_TYPOGRAPHY : BODY_TYPOGRAPHY,
          cacheKeyPrefix: isHeader ? "table-header-inline" : "table-cell-inline",
        });
        break;
      case "heading":
        blockHeight = measureHeading(block, width);
        break;
      case "image":
        blockHeight = SESSION_THREAD_MARKDOWN_IMAGE_HEIGHT_PX;
        break;
      case "code":
        blockHeight = measureCodeBlock(block);
        break;
      case "thematicBreak":
        blockHeight = measureThematicBreak();
        break;
      default:
        blockHeight = measureBlock(block, width);
        break;
    }
    return total + marginTopPx + blockHeight;
  }, 0);
}

function measureTable(block: Extract<SessionMarkdownBlock, { kind: "table" }>, width: number): number {
  if (block.rows.length === 0) {
    return 0;
  }
  const columnCount = block.rows.reduce((max, row) => Math.max(max, row.cells.length), 0);
  if (columnCount <= 0) {
    return 0;
  }
  const borderWidthPx = SESSION_THREAD_MARKDOWN_TABLE_BORDER_WIDTH_PX;
  const cellPaddingInlinePx = SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_INLINE_PX;
  const cellPaddingBlockPx = SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_BLOCK_PX;
  const totalBorderWidth = borderWidthPx * (columnCount + 1);
  const totalCellPaddingInlineWidth = columnCount * cellPaddingInlinePx * 2;
  const availableContentWidth = Math.max(
    columnCount,
    Math.floor(Math.max(1, width) - totalBorderWidth - totalCellPaddingInlineWidth),
  );
  const baseColumnWidth = Math.floor(availableContentWidth / columnCount);
  let remainderWidth = availableContentWidth - baseColumnWidth * columnCount;
  const columnContentWidths = Array.from({ length: columnCount }, () => {
    const widthPx = Math.max(1, baseColumnWidth + (remainderWidth > 0 ? 1 : 0));
    remainderWidth = Math.max(0, remainderWidth - 1);
    return widthPx;
  });

  let height = borderWidthPx;
  for (let rowIndex = 0; rowIndex < block.rows.length; rowIndex += 1) {
    const row = block.rows[rowIndex]!;
    let rowHeight = clampHeight(BODY_LINE_HEIGHT_PX) + cellPaddingBlockPx * 2;
    for (let columnIndex = 0; columnIndex < columnCount; columnIndex += 1) {
      const cell = row.cells[columnIndex];
      const cellHeight =
        measureTableCellHeight(cell, columnContentWidths[columnIndex] ?? 1, rowIndex === 0) + cellPaddingBlockPx * 2;
      rowHeight = Math.max(rowHeight, cellHeight);
    }
    height += rowHeight + borderWidthPx;
  }
  return normalizeHeight(height);
}

function measureThematicBreak(): number {
  return 1;
}

function measureBlock(block: SessionMarkdownBlock, width: number): number {
  switch (block.kind) {
    case "heading":
      return measureHeading(block, width);
    case "list":
      return measureList(block, width);
    case "code":
      return measureCodeBlock(block);
    case "blockquote":
      return measureBlockQuote(block, width);
    case "table":
      return measureTable(block, width);
    case "thematicBreak":
      return measureThematicBreak();
    case "image":
      return SESSION_THREAD_MARKDOWN_IMAGE_HEIGHT_PX;
    case "paragraph":
    default:
      return measureParagraph(block, width);
  }
}

function measureBlockChildren(
  blocks: readonly SessionMarkdownBlock[],
  width: number,
  context: SessionMarkdownBlockContext,
): number {
  let total = 0;
  for (let index = 0; index < blocks.length; index += 1) {
    const block = blocks[index]!;
    total +=
      index === 0
        ? resolveSessionMarkdownBlockEntryGapPx(block.kind, context)
        : resolveSessionMarkdownBlockGapPx(blocks[index - 1]!.kind, block.kind, context);
    total += measureBlock(block, width);
  }
  return normalizeHeight(total);
}

export function measureSessionMarkdownDocument(markdown: string, width: number): number {
  const normalizedWidth = Math.max(1, width);
  const widthBucket = Math.round(normalizedWidth);
  incrementPretextPerfCounter("pretext_markdown_document_calls");
  addPretextPerfBucket(
    "pretext_markdown_document_key",
    `w${widthBucket}:${markdown.length}:${hashPretextPerfValue(markdown)}`,
  );
  const parsed = parseMarkdown(markdown);
  return measureBlockChildren(parsed.blocks, normalizedWidth, "root");
}
