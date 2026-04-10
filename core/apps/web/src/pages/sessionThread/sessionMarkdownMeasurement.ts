import {
  layout,
  layoutNextLine,
  prepare,
  prepareWithSegments,
  type LayoutCursor,
  type PreparedText,
  type PreparedTextWithSegments,
} from "@chenglou/pretext";
import {
  addPretextPerfBucket,
  hashPretextPerfValue,
  incrementPretextPerfCounter,
} from "../../utils/pretextPerfDiagnostics";
import {
  createSessionMarkdownDocument,
  resolveSessionMarkdownBlockEntryGapPx,
  resolveSessionMarkdownBlockGapPx,
  type SessionMarkdownBlock,
  type SessionMarkdownBlockContext,
  type SessionMarkdownDocument,
  type SessionMarkdownInlineRun,
} from "./sessionMarkdownContract";
import {
  SESSION_THREAD_LAYOUT_STYLE,
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
  SESSION_THREAD_MARKDOWN_LIST_MARKER_GAP_PX,
  SESSION_THREAD_MARKDOWN_TABLE_BORDER_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_BLOCK_PX,
  SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_INLINE_PX,
} from "./sessionThreadLayoutTokens";

const PREPARED_CACHE_LIMIT = 4000;
const AST_CACHE_LIMIT = 1000;

const BODY_LINE_HEIGHT_PX = SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX;
const HEADING_FONT_SIZE_BY_DEPTH = {
  1: 18,
  2: 16,
  3: 14,
  4: 13,
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
export const SESSION_TRANSCRIPT_LAYOUT_ENGINE_REVISION = "2026-04-08-2";
const LINE_START_CURSOR: LayoutCursor = { segmentIndex: 0, graphemeIndex: 0 };
const UNBOUNDED_WIDTH_PX = 100_000;

type TextWhiteSpace = "normal" | "pre-wrap";
type TextBlockTypography = {
  body: string;
  strong: string;
  emphasis: string;
  strongEmphasis: string;
  lineHeight: number;
};
type PreparedInlineLayoutItem =
  | { kind: "hardBreak" }
  | { kind: "space"; width: number; codeGroupId: number | null }
  | {
      kind: "segment";
      codeGroupId: number | null;
      chromeWidth: number;
      endCursor: LayoutCursor;
      fullWidth: number;
      lineHeight: number;
      prepared: PreparedTextWithSegments;
    };

const preparedCache = new Map<string, PreparedText>();
const preparedSegmentsCache = new Map<string, PreparedTextWithSegments>();
const markdownDocumentCache = new Map<string, SessionMarkdownDocument>();
const collapsedSpaceWidthCache = new Map<string, number>();
const plainTextBlockHeightCache = new Map<string, number>();

const clampHeight = (value: number): number =>
  Number.isFinite(value) && value > 0 ? Math.max(1, value) : 1;

const normalizeHeight = (value: number): number =>
  Math.round(clampHeight(value) * 16) / 16;

const buildBodyFont = (weight: number, italic = false): string =>
  `${italic ? "italic " : ""}${weight} ${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`;

const buildHeadingFont = (depth: keyof typeof HEADING_FONT_SIZE_BY_DEPTH, weight: number, italic = false): string =>
  `${italic ? "italic " : ""}${weight} ${HEADING_FONT_SIZE_BY_DEPTH[depth]}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`;

const BODY_TYPOGRAPHY: TextBlockTypography = {
  body: buildBodyFont(400),
  strong: buildBodyFont(600),
  emphasis: buildBodyFont(400, true),
  strongEmphasis: buildBodyFont(600, true),
  lineHeight: BODY_LINE_HEIGHT_PX,
};

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

function measureCollapsedPlainTextLineHeight(params: {
  cacheKey: string;
  text: string;
  font: string;
  width: number;
  lineHeight: number;
}): number {
  const typography: TextBlockTypography = {
    body: params.font,
    strong: params.font,
    emphasis: params.font,
    strongEmphasis: params.font,
    lineHeight: params.lineHeight,
  };
  return measureInlineRunsHeight({
    runs: [{ kind: "text", text: params.text, style: "body" }],
    width: params.width,
    typography,
    cacheKeyPrefix: params.cacheKey,
  });
}

function measureRenderedPlainTextBlockHeight(params: {
  text: string;
  width: number;
}): number | null {
  if (typeof document === "undefined" || !document.body) {
    return null;
  }

  const host = document.createElement("div");
  host.className = "wb-turn-header-content";
  host.style.position = "fixed";
  host.style.left = "-10000px";
  host.style.top = "0";
  host.style.width = `${Math.max(1, params.width)}px`;
  host.style.margin = "0";
  host.style.padding = "0";
  host.style.border = "0";
  host.style.boxSizing = "border-box";
  host.style.visibility = "hidden";
  for (const [key, value] of Object.entries(SESSION_THREAD_LAYOUT_STYLE)) {
    host.style.setProperty(key, String(value));
  }

  const fragment = document.createDocumentFragment();
  const lines = params.text.split("\n");
  lines.forEach((line, index) => {
    const span = document.createElement("span");
    span.textContent = line;
    fragment.appendChild(span);
    if (index < lines.length - 1) {
      fragment.appendChild(document.createElement("br"));
    }
  });
  host.appendChild(fragment);
  document.body.appendChild(host);
  const height = host.getBoundingClientRect().height;
  host.remove();
  return height > 0 ? normalizeHeight(height) : null;
}

export function measureSessionPlainTextBlockHeight(params: {
  cacheKey: string;
  text: string;
  font: string;
  width: number;
  lineHeight: number;
}): number {
  const normalizedText = String(params.text ?? "")
    .replace(/\r\n/g, "\n")
    .replace(/[\r\f]/g, "\n");

  const measurementKey = `${params.cacheKey}:plain-text:${params.font}:${params.width}:${params.lineHeight}`;
  const cached = plainTextBlockHeightCache.get(measurementKey);
  if (cached != null) {
    return cached;
  }

  const renderedHeight = measureRenderedPlainTextBlockHeight({
    text: normalizedText,
    width: params.width,
  });
  if (renderedHeight != null) {
    plainTextBlockHeightCache.set(measurementKey, renderedHeight);
    pruneCache(plainTextBlockHeightCache, PREPARED_CACHE_LIMIT);
    return renderedHeight;
  }

  // EXCEPTION: jsdom has no real layout engine, so browser-faithful DOM measurement
  // returns 0 there. Keep a deterministic token estimate for test environments.
  const totalHeight = normalizedText.split("\n").reduce((sum, line, index) => {
    if (line.length === 0) {
      return sum + params.lineHeight;
    }
    return (
      sum +
      measureCollapsedPlainTextLineHeight({
        cacheKey: `${params.cacheKey}:line:${index}`,
        text: line,
        font: params.font,
        width: params.width,
        lineHeight: params.lineHeight,
      })
    );
  }, 0);
  const fallbackHeight = normalizeHeight(totalHeight);
  plainTextBlockHeightCache.set(measurementKey, fallbackHeight);
  pruneCache(plainTextBlockHeightCache, PREPARED_CACHE_LIMIT);
  return fallbackHeight;
}

function parseMarkdown(content: string): SessionMarkdownDocument {
  const cached = markdownDocumentCache.get(content);
  if (cached) {
    incrementPretextPerfCounter("pretext_markdown_ast_hit");
    return cached;
  }
  incrementPretextPerfCounter("pretext_markdown_ast_miss");
  const parsed = createSessionMarkdownDocument(content);
  markdownDocumentCache.set(content, parsed);
  pruneCache(markdownDocumentCache, AST_CACHE_LIMIT);
  return parsed;
}

function resolveInlineCodeFont(textFont: string): string {
  void textFont;
  return `${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY}`;
}

function resolveTextRunFont(run: Extract<SessionMarkdownInlineRun, { kind: "text" }>, typography: TextBlockTypography): string {
  switch (run.style) {
    case "strong":
      return typography.strong;
    case "emphasis":
      return typography.emphasis;
    case "strongEmphasis":
      return typography.strongEmphasis;
    case "body":
    default:
      return typography.body;
  }
}

function cursorsMatch(a: LayoutCursor, b: LayoutCursor): boolean {
  return a.segmentIndex === b.segmentIndex && a.graphemeIndex === b.graphemeIndex;
}

function measureSingleLineLayout(prepared: PreparedTextWithSegments) {
  return layoutNextLine(prepared, LINE_START_CURSOR, UNBOUNDED_WIDTH_PX);
}

function measureCollapsedSpaceWidth(font: string): number {
  const cached = collapsedSpaceWidthCache.get(font);
  if (cached != null) {
    return cached;
  }
  const joined = measureSingleLineLayout(getPreparedTextWithSegments(`collapsed-space:${font}:joined`, "A A", font, "normal"));
  const compact = measureSingleLineLayout(getPreparedTextWithSegments(`collapsed-space:${font}:compact`, "AA", font, "normal"));
  const width = Math.max(0, (joined?.width ?? 0) - (compact?.width ?? 0));
  collapsedSpaceWidthCache.set(font, width);
  return width;
}

function buildPreparedContentKey(prefix: string, text: string): string {
  return `${prefix}:${text.length}:${hashPretextPerfValue(text)}`;
}

function measureInlineSpaceWidth(cacheKey: string, text: string, font: string, whiteSpace: TextWhiteSpace): number {
  const prepared = getPreparedTextWithSegments(cacheKey, text, font, whiteSpace);
  return Math.max(0, measureSingleLineLayout(prepared)?.width ?? 0);
}

function pushTextRunItems(
  items: PreparedInlineLayoutItem[],
  params: {
    text: string;
    font: string;
    lineHeight: number;
    cacheKeyPrefix: string;
    collapsedSpaceWidth: number;
  },
): void {
  const normalized = params.text.replace(/\u00a0/g, " ");
  if (normalized.length === 0) {
    return;
  }
  const leadingWhitespace = normalized.match(/^\s+/)?.[0] ?? "";
  const trailingWhitespace = normalized.match(/\s+$/)?.[0] ?? "";
  const core = normalized.slice(leadingWhitespace.length, normalized.length - trailingWhitespace.length);

  if (leadingWhitespace.length > 0) {
    items.push({ kind: "space", width: params.collapsedSpaceWidth, codeGroupId: null });
  }

  if (core.length > 0) {
    const prepared = getPreparedTextWithSegments(
      buildPreparedContentKey(params.cacheKeyPrefix, core),
      core,
      params.font,
      "normal",
    );
    const wholeLine = measureSingleLineLayout(prepared);
    if (wholeLine != null) {
      items.push({
        kind: "segment",
        codeGroupId: null,
        chromeWidth: 0,
        endCursor: wholeLine.end,
        fullWidth: wholeLine.width,
        lineHeight: params.lineHeight,
        prepared,
      });
    }
  }

  if (trailingWhitespace.length > 0) {
    items.push({ kind: "space", width: params.collapsedSpaceWidth, codeGroupId: null });
  }
}

function pushInlineCodeWhitespaceItems(
  items: PreparedInlineLayoutItem[],
  params: {
    text: string;
    font: string;
    codeGroupId: number;
    cacheKeyPrefix: string;
  },
): void {
  const normalized = params.text.replace(/\r\n/g, "\n");
  let spaces = "";
  let partIndex = 0;

  const flushSpaces = () => {
    if (spaces.length === 0) {
      return;
    }
    items.push({
      kind: "space",
      width: measureInlineSpaceWidth(
        buildPreparedContentKey(`${params.cacheKeyPrefix}:space:${partIndex}`, spaces),
        spaces,
        params.font,
        "pre-wrap",
      ),
      codeGroupId: params.codeGroupId,
    });
    spaces = "";
    partIndex += 1;
  };

  for (let index = 0; index < normalized.length; index += 1) {
    const character = normalized[index]!;
    if (character === "\n") {
      flushSpaces();
      items.push({ kind: "hardBreak" });
      continue;
    }
    spaces += character;
  }
  flushSpaces();
}

function prepareInlineLayoutItems(params: {
  runs: readonly SessionMarkdownInlineRun[];
  typography: TextBlockTypography;
  cacheKeyPrefix: string;
}): PreparedInlineLayoutItem[] {
  const items: PreparedInlineLayoutItem[] = [];
  const inlineCodeFont = resolveInlineCodeFont(params.typography.body);
  const inlineCodeLineHeight =
    Math.max(params.typography.lineHeight, MONO_LINE_HEIGHT_PX) + SESSION_THREAD_MARKDOWN_INLINE_CODE_PADDING_BLOCK_PX;
  const preserveBodyTextRuns =
    params.runs.some((run) => run.kind === "inlineCode") &&
    params.runs.every((run) => run.kind !== "text" || run.style === "body");

  for (let index = 0; index < params.runs.length; index += 1) {
    const run = params.runs[index]!;
    if (run.kind === "hardBreak") {
      items.push({ kind: "hardBreak" });
      continue;
    }

    if (run.kind === "inlineCode") {
      if (run.text.length === 0) {
        continue;
      }
      const codeGroupId = index;
      for (let partIndex = 0; partIndex < run.parts.length; partIndex += 1) {
        const part = run.parts[partIndex]!;
        if (part.length === 0) {
          continue;
        }
        if (/^\s+$/.test(part)) {
          pushInlineCodeWhitespaceItems(items, {
            text: part,
            font: inlineCodeFont,
            codeGroupId,
            cacheKeyPrefix: `${params.cacheKeyPrefix}:${run.kind}:${index}:${partIndex}`,
          });
          continue;
        }
        const prepared = getPreparedTextWithSegments(
          buildPreparedContentKey(`${params.cacheKeyPrefix}:${run.kind}:${index}:${partIndex}`, part),
          part,
          inlineCodeFont,
          "pre-wrap",
        );
        const wholeLine = measureSingleLineLayout(prepared);
        if (wholeLine == null) {
          continue;
        }
        items.push({
          kind: "segment",
          codeGroupId,
          chromeWidth: SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX,
          endCursor: wholeLine.end,
          fullWidth: wholeLine.width,
          lineHeight: inlineCodeLineHeight,
          prepared,
        });
      }
      continue;
    }

    const font = resolveTextRunFont(run, params.typography);
    const collapsedSpaceWidth = measureCollapsedSpaceWidth(font);
    if (preserveBodyTextRuns) {
      pushTextRunItems(items, {
        text: run.text,
        font,
        lineHeight: params.typography.lineHeight,
        cacheKeyPrefix: `${params.cacheKeyPrefix}:${run.kind}:${index}`,
        collapsedSpaceWidth,
      });
      continue;
    }
    const tokens = run.text.replace(/\u00a0/g, " ").match(/\s+|\S+/g) ?? [];
    for (let tokenIndex = 0; tokenIndex < tokens.length; tokenIndex += 1) {
      const token = tokens[tokenIndex]!;
      if (/^\s+$/.test(token)) {
        items.push({ kind: "space", width: collapsedSpaceWidth, codeGroupId: null });
        continue;
      }

      const prepared = getPreparedTextWithSegments(
        buildPreparedContentKey(`${params.cacheKeyPrefix}:${run.kind}:${index}:${tokenIndex}`, token),
        token,
        font,
        "normal",
      );
      const wholeLine = measureSingleLineLayout(prepared);
      if (wholeLine == null) {
        continue;
      }

      items.push({
        kind: "segment",
        codeGroupId: null,
        chromeWidth: 0,
        endCursor: wholeLine.end,
        fullWidth: wholeLine.width,
        lineHeight: params.typography.lineHeight,
        prepared,
      });
    }
  }

  return items;
}

function measureInlineRunsHeight(params: {
  runs: readonly SessionMarkdownInlineRun[];
  width: number;
  typography: TextBlockTypography;
  cacheKeyPrefix: string;
}): number {
  const maxWidth = Math.max(1, params.width);
  const items = prepareInlineLayoutItems(params);
  let totalHeight = 0;
  let itemIndex = 0;
  let cursor: LayoutCursor | null = null;

  while (itemIndex < items.length) {
    let lineHeight = params.typography.lineHeight;
    let lineHasContent = false;
    let forcedBreak = false;
    let remainingWidth = maxWidth;
    let pendingSpaceWidth = 0;
    const chargedCodeGroups = new Set<number>();

    while (itemIndex < items.length) {
      const item = items[itemIndex]!;
      if (item.kind === "hardBreak") {
        itemIndex += 1;
        cursor = null;
        forcedBreak = true;
        break;
      }
      if (item.kind === "space") {
        itemIndex += 1;
        if (lineHasContent) {
          pendingSpaceWidth = item.width;
        }
        continue;
      }

      const codeGroupId = item.codeGroupId;
      const chromeWidth =
        codeGroupId != null && !chargedCodeGroups.has(codeGroupId) ? item.chromeWidth : 0;
      const reservedWidth = (lineHasContent ? pendingSpaceWidth : 0) + chromeWidth;
      const startCursor = cursor ?? LINE_START_CURSOR;

      if (cursor === null && codeGroupId != null) {
        const fullWidth = reservedWidth + item.fullWidth;
        if (fullWidth <= remainingWidth + 0.01) {
          remainingWidth = Math.max(0, remainingWidth - fullWidth);
          lineHasContent = true;
          lineHeight = Math.max(lineHeight, item.lineHeight);
          chargedCodeGroups.add(codeGroupId);
          itemIndex += 1;
          pendingSpaceWidth = 0;
          continue;
        }
      }

      if (lineHasContent && remainingWidth < reservedWidth - 0.01) {
        cursor = null;
        break;
      }

      const availableWidth = Math.max(1, remainingWidth - reservedWidth);
      const line = layoutNextLine(item.prepared, startCursor, availableWidth);
      if (line == null || cursorsMatch(startCursor, line.end)) {
        if (!lineHasContent) {
          itemIndex += 1;
        }
        cursor = null;
        break;
      }

      remainingWidth = Math.max(0, remainingWidth - reservedWidth - line.width);
      lineHasContent = true;
      lineHeight = Math.max(lineHeight, item.lineHeight);
      if (codeGroupId != null) {
        chargedCodeGroups.add(codeGroupId);
      }
      pendingSpaceWidth = 0;

      if (cursorsMatch(line.end, item.endCursor)) {
        itemIndex += 1;
        cursor = null;
        continue;
      }

      cursor = line.end;
      break;
    }

    if (!lineHasContent && !forcedBreak) {
      break;
    }

    totalHeight += lineHeight;
  }

  return clampHeight(totalHeight);
}

function headingTypography(depth: number): TextBlockTypography {
  const normalizedDepth = Math.max(1, Math.min(4, depth));
  const normalizedKey = normalizedDepth as keyof typeof HEADING_FONT_SIZE_BY_DEPTH;
  return {
    body: buildHeadingFont(normalizedKey, 600),
    strong: buildHeadingFont(normalizedKey, 600),
    emphasis: buildHeadingFont(normalizedKey, 600, true),
    strongEmphasis: buildHeadingFont(normalizedKey, 600, true),
    lineHeight: HEADING_LINE_HEIGHT_BY_DEPTH[normalizedDepth as keyof typeof HEADING_LINE_HEIGHT_BY_DEPTH],
  };
}

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
  const typography = headingTypography(block.depth);
  return measureTextBlock({
    text: block.text,
    width,
    typography,
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
  const childWidth = Math.max(1, width - bulletInsetPx);
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
  markdownDocumentCache.clear();
  collapsedSpaceWidthCache.clear();
  plainTextBlockHeightCache.clear();
}

export function measureSessionMarkdownDocument(markdown: string, width: number): number {
  const normalizedWidth = Math.max(1, Math.round(width));
  incrementPretextPerfCounter("pretext_markdown_document_calls");
  addPretextPerfBucket(
    "pretext_markdown_document_key",
    `w${normalizedWidth}:${markdown.length}:${hashPretextPerfValue(markdown)}`,
  );
  const parsed = parseMarkdown(markdown);
  return measureBlockChildren(parsed.blocks, normalizedWidth, "root");
}
