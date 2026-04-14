import {
  layout,
  layoutNextLine,
  prepare,
  prepareWithSegments,
  type LayoutCursor,
  type PreparedText,
  type PreparedTextWithSegments,
} from "@chenglou/pretext";
import { hashPretextPerfValue, incrementPretextPerfCounter } from "../../utils/pretextPerfDiagnostics";
import {
  createSessionMarkdownDocument,
  type SessionMarkdownDocument,
  type SessionMarkdownInlineRun,
} from "./sessionMarkdownContract";
import {
  SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY,
  SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX,
  SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY,
} from "./sessionThreadLayoutTokens";

export const PREPARED_CACHE_LIMIT = 4000;
const AST_CACHE_LIMIT = 1000;

export const BODY_LINE_HEIGHT_PX = SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX;
const HEADING_FONT_SIZE_BY_DEPTH = {
  1: 18,
  2: 16,
  3: 14,
  4: 13,
} as const;
export const MONO_FONT = `${SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY}`;
export const MONO_LINE_HEIGHT_PX = SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX;
export const CODE_BLOCK_VERTICAL_PADDING_PX =
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX +
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX;
export const LINE_START_CURSOR: LayoutCursor = { segmentIndex: 0, graphemeIndex: 0 };
const UNBOUNDED_WIDTH_PX = 100_000;
const BODY_STRONG_FONT_WEIGHT = 700;
const HEADING_FONT_WEIGHT = 600;
const HEADING_STRONG_FONT_WEIGHT = 700;
const TABLE_HEADER_FONT_WEIGHT = 600;
const TABLE_HEADER_STRONG_FONT_WEIGHT = 700;

export type TextWhiteSpace = "normal" | "pre-wrap";

export type TextBlockTypography = {
  body: string;
  strong: string;
  emphasis: string;
  strongEmphasis: string;
  lineHeight: number;
};

export type SessionMarkdownDebugWindow = Window & {
  __ctxForceInlineCodeDebug?: boolean;
  __ctxInlineCodeDebugTarget?: string;
  __ctxInlineCodeDebugWidth?: number;
  __ctxInlineCodeDebug?: {
    lines: string[];
    startDecisions?: Array<{
      pendingSpaceWidth: number;
      preferredStartWidth: number;
      dottedPathClusterWidth: number;
      wholeCodeGroupWidth: number;
      remainingWidth: number;
      shouldBreak: boolean;
      text: string;
    }>;
    items: Array<{
      kind: "hardBreak" | "space" | "segment";
      text: string;
      chromeWidth?: number;
      fullWidth?: number;
      minStartTextWidth?: number;
    }>;
    width: number;
  };
  __ctxForcePlainTextDebug?: boolean;
  __ctxPlainTextDebugTarget?: string;
  __ctxPlainTextDebugWidth?: number;
  __ctxPlainTextDebug?: {
    lineCount: number;
    lines: string[];
    text: string;
    width: number;
  };
};

const preparedCache = new Map<string, PreparedText>();
const preparedSegmentsCache = new Map<string, PreparedTextWithSegments>();
const markdownDocumentCache = new Map<string, SessionMarkdownDocument>();
const collapsedSpaceWidthCache = new Map<string, number>();
export const plainTextBlockHeightCache = new Map<string, number>();

export const clampHeight = (value: number): number =>
  Number.isFinite(value) && value > 0 ? Math.max(1, value) : 1;

export const normalizeHeight = (value: number): number =>
  Math.round(clampHeight(value) * 16) / 16;

const graphemeSegmenter =
  typeof Intl !== "undefined" && typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;

const buildBodyFont = (weight: number, italic = false): string =>
  `${italic ? "italic " : ""}${weight} ${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`;

const buildHeadingFont = (
  depth: keyof typeof HEADING_FONT_SIZE_BY_DEPTH,
  weight: number,
  italic = false,
): string =>
  `${italic ? "italic " : ""}${weight} ${HEADING_FONT_SIZE_BY_DEPTH[depth]}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`;

export const BODY_TYPOGRAPHY: TextBlockTypography = {
  body: buildBodyFont(400),
  strong: buildBodyFont(BODY_STRONG_FONT_WEIGHT),
  emphasis: buildBodyFont(400, true),
  strongEmphasis: buildBodyFont(BODY_STRONG_FONT_WEIGHT, true),
  lineHeight: BODY_LINE_HEIGHT_PX,
};

export const TABLE_HEADER_TYPOGRAPHY: TextBlockTypography = {
  body: buildBodyFont(TABLE_HEADER_FONT_WEIGHT),
  strong: buildBodyFont(TABLE_HEADER_STRONG_FONT_WEIGHT),
  emphasis: buildBodyFont(TABLE_HEADER_FONT_WEIGHT, true),
  strongEmphasis: buildBodyFont(TABLE_HEADER_STRONG_FONT_WEIGHT, true),
  lineHeight: BODY_LINE_HEIGHT_PX,
};

export function buildHeadingTypography(depth: number): TextBlockTypography {
  const normalizedDepth = Math.max(1, Math.min(4, depth));
  const normalizedKey = normalizedDepth as keyof typeof HEADING_FONT_SIZE_BY_DEPTH;
  return {
    body: buildHeadingFont(normalizedKey, HEADING_FONT_WEIGHT),
    strong: buildHeadingFont(normalizedKey, HEADING_STRONG_FONT_WEIGHT),
    emphasis: buildHeadingFont(normalizedKey, HEADING_FONT_WEIGHT, true),
    strongEmphasis: buildHeadingFont(normalizedKey, HEADING_STRONG_FONT_WEIGHT, true),
    lineHeight:
      SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH[
        normalizedDepth as keyof typeof SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH
      ],
  };
}

export function pruneCache<T>(cache: Map<string, T>, limit: number) {
  while (cache.size > limit) {
    const oldestKey = cache.keys().next().value;
    if (typeof oldestKey !== "string") break;
    cache.delete(oldestKey);
  }
}

export function getPreparedText(
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

export function getPreparedTextWithSegments(
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

export function measureTextHeight(params: {
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

export function parseMarkdown(content: string): SessionMarkdownDocument {
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

export function resolveTextRunFont(
  run: Extract<SessionMarkdownInlineRun, { kind: "text" }>,
  typography: TextBlockTypography,
): string {
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

export function cursorsMatch(a: LayoutCursor, b: LayoutCursor): boolean {
  return a.segmentIndex === b.segmentIndex && a.graphemeIndex === b.graphemeIndex;
}

export function measureSingleLineLayout(prepared: PreparedTextWithSegments) {
  return layoutNextLine(prepared, LINE_START_CURSOR, UNBOUNDED_WIDTH_PX);
}

export function measureCollapsedSpaceWidth(font: string): number {
  const cached = collapsedSpaceWidthCache.get(font);
  if (cached != null) {
    return cached;
  }
  const joined = measureSingleLineLayout(
    getPreparedTextWithSegments(`collapsed-space:${font}:joined`, "A A", font, "normal"),
  );
  const compact = measureSingleLineLayout(
    getPreparedTextWithSegments(`collapsed-space:${font}:compact`, "AA", font, "normal"),
  );
  const width = Math.max(0, (joined?.width ?? 0) - (compact?.width ?? 0));
  collapsedSpaceWidthCache.set(font, width);
  return width;
}

export function buildPreparedContentKey(prefix: string, text: string): string {
  return `${prefix}:${text.length}:${hashPretextPerfValue(text)}`;
}

export function segmentGraphemes(text: string): string[] {
  if (!text) return [];
  if (!graphemeSegmenter) return Array.from(text);
  return Array.from(graphemeSegmenter.segment(text), (segment) => segment.segment);
}

export function measureInlineSpaceWidth(
  cacheKey: string,
  text: string,
  font: string,
  whiteSpace: TextWhiteSpace,
): number {
  const prepared = getPreparedTextWithSegments(cacheKey, text, font, whiteSpace);
  return Math.max(0, measureSingleLineLayout(prepared)?.width ?? 0);
}

export function clearSessionMarkdownMeasurementCaches(): void {
  preparedCache.clear();
  preparedSegmentsCache.clear();
  markdownDocumentCache.clear();
  collapsedSpaceWidthCache.clear();
  plainTextBlockHeightCache.clear();
}
