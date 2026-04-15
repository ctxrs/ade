import type { LayoutCursor } from "@chenglou/pretext";
import { incrementPretextPerfCounter } from "../../utils/pretextPerfDiagnostics";
import {
  createSessionMarkdownDocument,
  type SessionMarkdownDocument,
  type SessionMarkdownInlineRun,
} from "./sessionMarkdownContract";
import {
  buildPreparedContentKey,
  clampHeight,
  clearSessionTextMeasurementCaches,
  getPreparedText,
  getPreparedTextWithSegments,
  measureCollapsedSpaceWidth,
  measureSingleLineLayout,
  measureTextHeight,
  normalizeHeight,
  pruneCache,
  segmentGraphemes,
  type TextWhiteSpace,
} from "./sessionTextMeasurement";
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
const BODY_STRONG_FONT_WEIGHT = 700;
const HEADING_FONT_WEIGHT = 600;
const HEADING_STRONG_FONT_WEIGHT = 700;
const TABLE_HEADER_FONT_WEIGHT = 600;
const TABLE_HEADER_STRONG_FONT_WEIGHT = 700;

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

const markdownDocumentCache = new Map<string, SessionMarkdownDocument>();

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

export {
  buildPreparedContentKey,
  clampHeight,
  getPreparedText,
  getPreparedTextWithSegments,
  measureCollapsedSpaceWidth,
  measureSingleLineLayout,
  measureTextHeight,
  normalizeHeight,
  pruneCache,
  segmentGraphemes,
};
export type { TextWhiteSpace };

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
  clearSessionTextMeasurementCaches();
  markdownDocumentCache.clear();
}
