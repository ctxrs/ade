import { layout, prepare, prepareWithSegments, type PreparedText, type PreparedTextWithSegments } from "@chenglou/pretext";
import { stripCitationMarkers } from "../../utils/citationMarkers";
import {
  addPretextPerfBucket,
  hashPretextPerfValue,
  incrementPretextPerfCounter,
} from "../../utils/pretextPerfDiagnostics";
import {
  parseSessionMarkdown,
} from "./sessionMarkdownShared";
import {
  nodeChildren,
  normalizeSessionMarkdownBlocks,
  resolveSessionMarkdownBlockEntryGapPx,
  resolveSessionMarkdownBlockGapPx,
  type SessionMarkdownBlock,
  type SessionMarkdownBlockContext,
  type SessionMarkdownInlineNode,
} from "./sessionMarkdownContract";
import {
  SESSION_THREAD_MARKDOWN_BLOCKQUOTE_INSET_PX,
  SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY,
  SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_BORDER_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX,
  SESSION_THREAD_MARKDOWN_IMAGE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_BLOCK_PX,
  SESSION_THREAD_MARKDOWN_LIST_GAP_PX,
  SESSION_THREAD_MARKDOWN_LIST_INDENT_PX,
  SESSION_THREAD_MARKDOWN_TABLE_BORDER_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_BLOCK_PX,
  SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_INLINE_PX,
} from "./sessionThreadLayoutTokens";

const PREPARED_CACHE_LIMIT = 4000;
const AST_CACHE_LIMIT = 1000;

const BODY_FONT = `${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`;
const BODY_LINE_HEIGHT_PX = SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX;
const HEADING_FONT_BY_DEPTH = {
  1: `600 18px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`,
  2: `600 16px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`,
  3: `600 14px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`,
  4: `600 13px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`,
} as const;
const HEADING_LINE_HEIGHT_BY_DEPTH = {
  1: 22.5,
  2: 20,
  3: 18,
  4: 16.25,
} as const;
const MONO_FONT = `${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY}`;
const MONO_LINE_HEIGHT_PX = 17.4;
const CODE_BLOCK_VERTICAL_PADDING_PX =
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX +
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX;
const CHECKBOX_GUTTER_PX = 18;
export const SESSION_TRANSCRIPT_LAYOUT_ENGINE_REVISION = "2026-04-08-1";

type TextWhiteSpace = "normal" | "pre-wrap";

type InlineRun =
  | {
      kind: "hardBreak";
    }
  | {
      kind: "text";
      text: string;
    }
  | {
      kind: "inlineCode";
      text: string;
    };

type TextInlineRun = Extract<InlineRun, { kind: "text" }>;

const preparedCache = new Map<string, PreparedText>();
const preparedSegmentsCache = new Map<string, PreparedTextWithSegments>();
const markdownBlocksCache = new Map<string, SessionMarkdownBlock[]>();
const textWidthCache = new Map<string, number>();

const clampHeight = (value: number): number =>
  Number.isFinite(value) && value > 0 ? Math.max(1, value) : 1;

const normalizeHeight = (value: number): number =>
  Math.round(clampHeight(value) * 16) / 16;

function pruneCache<T>(cache: Map<string, T>, limit: number) {
  while (cache.size > limit) {
    const oldestKey = cache.keys().next().value;
    if (typeof oldestKey !== "string") break;
    cache.delete(oldestKey);
  }
}

function getPreparedText(
  cacheKey: string,
  text: string,
  font: string,
  whiteSpace: TextWhiteSpace,
): PreparedText {
  const cached = preparedCache.get(cacheKey);
  if (cached) {
    incrementPretextPerfCounter("pretext_markdown_prepared_text_hit");
    return cached;
  }
  incrementPretextPerfCounter("pretext_markdown_prepared_text_miss");
  const prepared = prepare(text, font, whiteSpace === "pre-wrap" ? { whiteSpace } : undefined);
  preparedCache.set(cacheKey, prepared);
  pruneCache(preparedCache, PREPARED_CACHE_LIMIT);
  return prepared;
}

function getPreparedTextWithSegments(
  cacheKey: string,
  text: string,
  font: string,
  whiteSpace: TextWhiteSpace,
): PreparedTextWithSegments {
  const cached = preparedSegmentsCache.get(cacheKey);
  if (cached) {
    incrementPretextPerfCounter("pretext_markdown_prepared_segments_hit");
    return cached;
  }
  incrementPretextPerfCounter("pretext_markdown_prepared_segments_miss");
  const prepared = prepareWithSegments(text, font, whiteSpace === "pre-wrap" ? { whiteSpace } : undefined);
  preparedSegmentsCache.set(cacheKey, prepared);
  pruneCache(preparedSegmentsCache, PREPARED_CACHE_LIMIT);
  return prepared;
}

function measureTextWidth(params: {
  cacheKey: string;
  text: string;
  font: string;
  whiteSpace?: TextWhiteSpace;
}): number {
  const whiteSpace = params.whiteSpace ?? "normal";
  const cacheKey = `${params.cacheKey}:${params.font}:${whiteSpace}:${params.text}`;
  const cached = textWidthCache.get(cacheKey);
  if (cached != null) {
    incrementPretextPerfCounter("pretext_markdown_text_width_hit");
    return cached;
  }
  incrementPretextPerfCounter("pretext_markdown_text_width_miss");
  const prepared = getPreparedTextWithSegments(cacheKey, params.text, params.font, whiteSpace);
  const width = prepared.widths.reduce((sum, segmentWidth) => sum + (segmentWidth ?? 0), 0);
  textWidthCache.set(cacheKey, width);
  pruneCache(textWidthCache, PREPARED_CACHE_LIMIT);
  return width;
}

function measureTextHeight(params: {
  cacheKey: string;
  text: string;
  font: string;
  width: number;
  lineHeight: number;
  whiteSpace?: TextWhiteSpace;
}): number {
  const whiteSpace = params.whiteSpace ?? "normal";
  const prepared = getPreparedText(params.cacheKey, params.text, params.font, whiteSpace);
  return clampHeight(layout(prepared, Math.max(1, params.width), params.lineHeight).height);
}

export function measureSessionTextHeight(params: {
  cacheKey: string;
  text: string;
  font: string;
  width: number;
  lineHeight: number;
  whiteSpace?: TextWhiteSpace;
}): number {
  return normalizeHeight(measureTextHeight(params));
}

function parseMarkdown(content: string): SessionMarkdownBlock[] {
  const normalized = stripCitationMarkers(content);
  const cached = markdownBlocksCache.get(normalized);
  if (cached) {
    incrementPretextPerfCounter("pretext_markdown_ast_hit");
    return cached;
  }
  incrementPretextPerfCounter("pretext_markdown_ast_miss");
  const parsed = parseSessionMarkdown(normalized);
  const blocks = normalizeSessionMarkdownBlocks(nodeChildren(parsed));
  markdownBlocksCache.set(normalized, blocks);
  pruneCache(markdownBlocksCache, AST_CACHE_LIMIT);
  return blocks;
}

function flattenInlineText(nodes: readonly SessionMarkdownInlineNode[]): string {
  let text = "";
  for (const node of nodes) {
    switch (node.kind) {
      case "text":
      case "inlineCode":
        text += node.text;
        break;
      case "break":
        text += "\n";
        break;
      case "image":
        text += node.alt.trim();
        break;
      default:
        text += flattenInlineText(node.children);
        break;
    }
  }
  return text.replace(/\u00a0/g, " ");
}

function containsInlineCode(nodes: readonly SessionMarkdownInlineNode[]): boolean {
  for (const node of nodes) {
    if (node.kind === "inlineCode") return true;
    if ("children" in node && containsInlineCode(node.children)) return true;
  }
  return false;
}

function appendTextRun(runs: InlineRun[], text: string) {
  if (text.length === 0) return;
  const last = runs[runs.length - 1];
  if (last?.kind === "text") {
    last.text += text;
    return;
  }
  runs.push({ kind: "text", text });
}

function collectInlineRuns(nodes: readonly SessionMarkdownInlineNode[], runs: InlineRun[] = []): InlineRun[] {
  for (const node of nodes) {
    switch (node.kind) {
      case "text":
        appendTextRun(runs, node.text.replace(/\u00a0/g, " "));
        break;
      case "inlineCode":
        runs.push({ kind: "inlineCode", text: node.text });
        break;
      case "break":
        runs.push({ kind: "hardBreak" });
        break;
      case "image":
        appendTextRun(runs, node.alt.trim());
        break;
      default:
        collectInlineRuns(node.children, runs);
        break;
    }
  }
  return runs;
}

function resolveInlineCodeFont(textFont: string): string {
  void textFont;
  return `${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY}`;
}

function splitNormalTextTokens(text: string): TextInlineRun[] {
  const normalized = text.replace(/\s+/g, " ");
  if (normalized.length === 0) {
    return [];
  }
  const parts = normalized.split(/(\s+)/).filter((part) => part.length > 0);
  return parts.map((part) => (/\s+/.test(part) ? { kind: "text" as const, text: " " } : { kind: "text" as const, text: part }));
}

function measureGraphemeWidths(
  text: string,
  font: string,
  cacheKeyPrefix: string,
): number[] {
  return Array.from(text).map((grapheme, index) =>
    measureTextWidth({
      cacheKey: `${cacheKeyPrefix}:grapheme:${index}`,
      text: grapheme,
      font,
      whiteSpace: "pre-wrap",
    }),
  );
}

function measureInlineRunsHeight(params: {
  runs: readonly InlineRun[];
  width: number;
  textFont: string;
  lineHeight: number;
  cacheKeyPrefix: string;
}): number {
  const maxWidth = Math.max(1, params.width);
  const inlineCodeFont = resolveInlineCodeFont(params.textFont);
  const inlineCodeLineHeight = Math.max(params.lineHeight, MONO_LINE_HEIGHT_PX) + SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_BLOCK_PX;
  let totalHeight = 0;
  let currentLineWidth = 0;
  let pendingWhitespaceWidth = 0;
  let currentLineHeight = params.lineHeight;

  const advanceLine = () => {
    totalHeight += currentLineHeight;
    currentLineWidth = 0;
    pendingWhitespaceWidth = 0;
    currentLineHeight = params.lineHeight;
  };

  const consumePendingWhitespace = () => {
    if (currentLineWidth > 0) {
      currentLineWidth += pendingWhitespaceWidth;
    }
    pendingWhitespaceWidth = 0;
  };

  const placeBreakableToken = (tokenWidth: number, graphemeWidths: readonly number[]) => {
    if (currentLineWidth > 0) {
      advanceLine();
    }
    for (let graphemeIndex = 0; graphemeIndex < graphemeWidths.length; graphemeIndex += 1) {
      const graphemeWidth = graphemeWidths[graphemeIndex] ?? 0;
      if (currentLineWidth > 0 && currentLineWidth + graphemeWidth > maxWidth + 0.01) {
        advanceLine();
      }
      currentLineWidth += graphemeWidth;
    }
    if (tokenWidth === 0 && graphemeWidths.length === 0) {
      currentLineWidth += tokenWidth;
    }
  };

  const placeTextToken = (tokenText: string, tokenIndex: number) => {
    if (tokenText === " ") {
      if (currentLineWidth > 0) {
        pendingWhitespaceWidth = measureTextWidth({
          cacheKey: `${params.cacheKeyPrefix}:space:${tokenIndex}`,
          text: tokenText,
          font: params.textFont,
          whiteSpace: "pre-wrap",
        });
      }
      return;
    }

    const tokenWidth = measureTextWidth({
      cacheKey: `${params.cacheKeyPrefix}:text:${tokenIndex}`,
      text: tokenText,
      font: params.textFont,
    });
    const prefixWhitespaceWidth = currentLineWidth > 0 ? pendingWhitespaceWidth : 0;

    if (currentLineWidth > 0 && currentLineWidth + prefixWhitespaceWidth + tokenWidth > maxWidth + 0.01) {
      if (tokenWidth <= maxWidth + 0.01) {
        advanceLine();
      } else {
        placeBreakableToken(
          tokenWidth,
          measureGraphemeWidths(tokenText, params.textFont, `${params.cacheKeyPrefix}:text:${tokenIndex}`),
        );
        pendingWhitespaceWidth = 0;
        return;
      }
    }

    consumePendingWhitespace();
    currentLineWidth += tokenWidth;
  };

  const placeInlineCodeRun = (run: Extract<InlineRun, { kind: "inlineCode" }>, runIndex: number) => {
    const textWidth = measureTextWidth({
      cacheKey: `${params.cacheKeyPrefix}:inline-code:${runIndex}`,
      text: run.text,
      font: inlineCodeFont,
      whiteSpace: "pre-wrap",
    });
    const fullWidth = SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX + textWidth;
    const prefixWhitespaceWidth = currentLineWidth > 0 ? pendingWhitespaceWidth : 0;

    if (currentLineWidth > 0 && currentLineWidth + prefixWhitespaceWidth + fullWidth <= maxWidth + 0.01) {
      consumePendingWhitespace();
      currentLineWidth += fullWidth;
      currentLineHeight = Math.max(currentLineHeight, inlineCodeLineHeight);
      return;
    }

    if (fullWidth <= maxWidth + 0.01) {
      if (currentLineWidth > 0) {
        advanceLine();
      }
      currentLineWidth = fullWidth;
      currentLineHeight = Math.max(currentLineHeight, inlineCodeLineHeight);
      pendingWhitespaceWidth = 0;
      return;
    }

    if (currentLineWidth > 0) {
      consumePendingWhitespace();
    }
    let fragmentWidth = currentLineWidth + SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX;
    if (fragmentWidth > maxWidth + 0.01 && currentLineWidth > 0) {
      advanceLine();
      fragmentWidth = SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX;
    }
    const graphemeWidths = measureGraphemeWidths(run.text, inlineCodeFont, `${params.cacheKeyPrefix}:inline-code:${runIndex}`);
    currentLineHeight = Math.max(currentLineHeight, inlineCodeLineHeight);
    for (let graphemeIndex = 0; graphemeIndex < graphemeWidths.length; graphemeIndex += 1) {
      const graphemeWidth = graphemeWidths[graphemeIndex] ?? 0;
      if (
        fragmentWidth > currentLineWidth + SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX &&
        fragmentWidth + graphemeWidth > maxWidth + 0.01
      ) {
        currentLineWidth = fragmentWidth;
        advanceLine();
        fragmentWidth = SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX;
        currentLineHeight = Math.max(currentLineHeight, inlineCodeLineHeight);
      }
      fragmentWidth += graphemeWidth;
    }
    currentLineWidth = fragmentWidth;
    currentLineHeight = Math.max(currentLineHeight, inlineCodeLineHeight);
  };

  for (let runIndex = 0; runIndex < params.runs.length; runIndex += 1) {
    const run = params.runs[runIndex]!;
    if (run.kind === "hardBreak") {
      advanceLine();
      continue;
    }
    if (run.kind === "inlineCode") {
      placeInlineCodeRun(run, runIndex);
      continue;
    }
    if (run.kind !== "text") {
      continue;
    }
    const tokens = splitNormalTextTokens(run.text);
    for (let tokenIndex = 0; tokenIndex < tokens.length; tokenIndex += 1) {
      const token = tokens[tokenIndex]!;
      placeTextToken(token.text, runIndex * 1_000 + tokenIndex);
    }
  }

  return clampHeight(totalHeight + currentLineHeight);
}

function headingTypography(depth: number): { font: string; lineHeight: number } {
  const normalizedDepth = Math.max(1, Math.min(4, depth));
  return {
    font: HEADING_FONT_BY_DEPTH[normalizedDepth as keyof typeof HEADING_FONT_BY_DEPTH],
    lineHeight: HEADING_LINE_HEIGHT_BY_DEPTH[normalizedDepth as keyof typeof HEADING_LINE_HEIGHT_BY_DEPTH],
  };
}

function measureParagraph(block: Extract<SessionMarkdownBlock, { kind: "paragraph" }>, width: number): number {
  const text = flattenInlineText(block.inlines).trim();
  if (!text) {
    return 0;
  }
  if (containsInlineCode(block.inlines)) {
    return measureInlineRunsHeight({
      runs: collectInlineRuns(block.inlines),
      width,
      textFont: BODY_FONT,
      lineHeight: BODY_LINE_HEIGHT_PX,
      cacheKeyPrefix: "paragraph-inline",
    });
  }
  return measureTextHeight({
    cacheKey: `paragraph:${text}`,
    text,
    font: BODY_FONT,
    width,
    lineHeight: BODY_LINE_HEIGHT_PX,
  });
}

function measureHeading(block: Extract<SessionMarkdownBlock, { kind: "heading" }>, width: number): number {
  const text = flattenInlineText(block.inlines).trim();
  const typography = headingTypography(block.depth);
  return containsInlineCode(block.inlines)
    ? measureInlineRunsHeight({
        runs: collectInlineRuns(block.inlines),
        width,
        textFont: typography.font,
        lineHeight: typography.lineHeight,
        cacheKeyPrefix: `heading-inline:${block.depth}`,
      })
    : measureTextHeight({
        cacheKey: `heading:${block.depth}:${text}`,
        text,
        font: typography.font,
        width,
        lineHeight: typography.lineHeight,
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
  const childWidth = Math.max(1, width - bulletInsetPx);
  if (item.blocks.length === 0) {
    return clampHeight(BODY_LINE_HEIGHT_PX);
  }
  return Math.max(clampHeight(BODY_LINE_HEIGHT_PX), measureBlockChildren(item.blocks, childWidth, "listItem"));
}

function measureList(block: Extract<SessionMarkdownBlock, { kind: "list" }>, width: number): number {
  const bulletInsetPx = SESSION_THREAD_MARKDOWN_LIST_INDENT_PX;
  let total = 0;
  for (let index = 0; index < block.items.length; index += 1) {
    const item = block.items[index]!;
    const checkedInset = item.checked != null ? CHECKBOX_GUTTER_PX : 0;
    total += measureListItem(item, width, bulletInsetPx + checkedInset);
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
): number {
  if (!cell) {
    return clampHeight(BODY_LINE_HEIGHT_PX);
  }
  if (cell.blocks.length === 0) {
    return clampHeight(BODY_LINE_HEIGHT_PX);
  }
  return measureBlockChildren(cell.blocks, width, "root");
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
  const cellContentWidth = Math.max(
    1,
    (Math.max(1, width) - totalBorderWidth - totalCellPaddingInlineWidth) / columnCount,
  );

  let height = borderWidthPx;
  for (let rowIndex = 0; rowIndex < block.rows.length; rowIndex += 1) {
    const row = block.rows[rowIndex]!;
    let rowHeight = clampHeight(BODY_LINE_HEIGHT_PX) + cellPaddingBlockPx * 2;
    for (let columnIndex = 0; columnIndex < columnCount; columnIndex += 1) {
      const cell = row.cells[columnIndex];
      const cellHeight = measureTableCellHeight(cell, cellContentWidth) + cellPaddingBlockPx * 2;
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

export function clearSessionMarkdownMeasurementCaches(): void {
  preparedCache.clear();
  preparedSegmentsCache.clear();
  markdownBlocksCache.clear();
  textWidthCache.clear();
}

export function measureSessionMarkdownDocument(markdown: string, width: number): number {
  const normalizedWidth = Math.max(1, Math.round(width));
  incrementPretextPerfCounter("pretext_markdown_document_calls");
  addPretextPerfBucket(
    "pretext_markdown_document_key",
    `w${normalizedWidth}:${markdown.length}:${hashPretextPerfValue(markdown)}`,
  );
  const parsed = parseMarkdown(markdown);
  return measureBlockChildren(parsed, normalizedWidth, "root");
}
