import {
  layout,
  layoutNextLine,
  type LayoutCursor,
  type PreparedTextWithSegments,
} from "@chenglou/pretext";
import {
  addPretextPerfBucket,
  hashPretextPerfValue,
  incrementPretextPerfCounter,
} from "../../utils/pretextPerfDiagnostics";
import { isSealedInlineCodeFragment, splitInlineCodeFragments } from "../../utils/inlineCodeFragments";
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
  SESSION_THREAD_MARKDOWN_BLOCKQUOTE_INSET_PX,
  SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY,
  SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_BORDER_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX,
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX,
  SESSION_THREAD_MARKDOWN_HEADING_LINE_HEIGHT_PX_BY_DEPTH,
  SESSION_THREAD_MARKDOWN_IMAGE_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_HEIGHT_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_LINE_HEIGHT_PREMIUM_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_LIST_GAP_PX,
  SESSION_THREAD_MARKDOWN_LIST_MARKER_GAP_PX,
  SESSION_THREAD_MARKDOWN_TABLE_BORDER_WIDTH_PX,
  SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_BLOCK_PX,
  SESSION_THREAD_MARKDOWN_TABLE_CELL_PADDING_INLINE_PX,
} from "./sessionThreadLayoutTokens";
import { clearSessionPlainTextMeasurementCaches } from "./sessionPlainTextMeasurement";
import {
  buildPreparedContentKey,
  clampHeight,
  clearSessionTextMeasurementCaches,
  getPreparedText,
  getPreparedTextWithSegments,
  measureCollapsedSpaceWidth,
  measureSingleLineLayout,
  normalizeHeight,
  pruneCache,
  segmentGraphemes,
  type TextWhiteSpace,
} from "./sessionTextMeasurement";

const AST_CACHE_LIMIT = 1000;

const BODY_LINE_HEIGHT_PX = SESSION_THREAD_MARKDOWN_BODY_LINE_HEIGHT_PX;
const HEADING_FONT_SIZE_BY_DEPTH = {
  1: 18,
  2: 16,
  3: 14,
  4: 13,
} as const;
const MONO_FONT = `${SESSION_THREAD_MARKDOWN_CODE_BLOCK_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY}`;
const MONO_LINE_HEIGHT_PX = SESSION_THREAD_MARKDOWN_CODE_BLOCK_LINE_HEIGHT_PX;
const CODE_BLOCK_VERTICAL_PADDING_PX =
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_TOP_PX +
  SESSION_THREAD_MARKDOWN_CODE_BLOCK_PADDING_BOTTOM_PX;
const CHECKBOX_GUTTER_PX = 18;
export const SESSION_TRANSCRIPT_LAYOUT_ENGINE_REVISION = "2026-04-13-1";
const LINE_START_CURSOR: LayoutCursor = { segmentIndex: 0, graphemeIndex: 0 };
// Chromium/WebKit grid track rounding leaves list bodies 1px narrower than the
// naive marker-column-plus-gap subtraction for bullet rows.
const LIST_ITEM_BODY_WIDTH_FIT_BIAS_PX = 0;
// Inline code chips need enough visible prefix when they start after prose;
// otherwise browsers often push the chip to the next line instead.
const INLINE_CODE_MIN_START_GRAPHEMES = 4;
const INLINE_CODE_FRAGMENT_FIT_SLACK_PX = 0;
const INLINE_CODE_WHOLE_GROUP_FIT_SLACK_PX = 8;
const INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD = 0.2;
const BODY_STRONG_FONT_WEIGHT = 700;
const HEADING_FONT_WEIGHT = 600;
const HEADING_STRONG_FONT_WEIGHT = 700;
const TABLE_HEADER_FONT_WEIGHT = 600;
const TABLE_HEADER_STRONG_FONT_WEIGHT = 700;

type TextBlockTypography = {
  body: string;
  strong: string;
  emphasis: string;
  strongEmphasis: string;
  lineHeight: number;
};
type PreparedInlineLayoutItem =
  | { kind: "hardBreak" }
  | { kind: "space"; width: number; codeGroupId: number | null; text: string }
  | {
      kind: "segment";
      codeGroupId: number | null;
      codeGroupHasDottedPath: boolean;
      codeGroupHasTrailingText: boolean;
      codeGroupIsOnlyInlineCodeInSegment: boolean;
      codeGroupStartsAfterText: boolean;
      chromeWidth: number;
      endCursor: LayoutCursor;
      fullWidth: number;
      isFirstCodeGroupFragment: boolean;
      startsAfterCodeWhitespace: boolean;
      isFirstPathFragmentAfterHyphenRun: boolean;
      isPathTailFragment: boolean;
      isSealedInlineCodeFragment: boolean;
      lineHeight: number;
      minStartTextWidth: number;
      prefersFreshLineStart: boolean;
      startsStyledTextAfterInlineCodeSeam: boolean;
      startsAfterStyledTextSeam: boolean;
      startsStyledTextAfterBodySeam: boolean;
      hasTrailingInlineCode: boolean;
      prepared: PreparedTextWithSegments;
      text: string;
    };

type SessionMarkdownDebugWindow = Window & {
  __ctxForceInlineCodeDebug?: boolean;
  __ctxInlineCodeDebugTarget?: string;
  __ctxInlineCodeDebugWidth?: number;
  __ctxInlineCodeDebug?: {
    lines: string[];
    startDecisions?: Array<{
      pendingSpaceWidth: number;
      preferredStartWidth: number;
      dottedPathClusterWidth: number;
      remainingWidth: number;
      shouldBreak: boolean;
      text: string;
    }>;
    items: Array<{
      kind: PreparedInlineLayoutItem["kind"];
      text: string;
      chromeWidth?: number;
      fullWidth?: number;
      minStartTextWidth?: number;
    }>;
    width: number;
  };
};

const markdownDocumentCache = new Map<string, SessionMarkdownDocument>();

const buildBodyFont = (weight: number, italic = false): string =>
  `${italic ? "italic " : ""}${weight} ${SESSION_THREAD_MARKDOWN_BODY_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`;

const buildHeadingFont = (depth: keyof typeof HEADING_FONT_SIZE_BY_DEPTH, weight: number, italic = false): string =>
  `${italic ? "italic " : ""}${weight} ${HEADING_FONT_SIZE_BY_DEPTH[depth]}px ${SESSION_THREAD_MARKDOWN_BODY_FONT_FAMILY}`;

const BODY_TYPOGRAPHY: TextBlockTypography = {
  body: buildBodyFont(400),
  strong: buildBodyFont(BODY_STRONG_FONT_WEIGHT),
  emphasis: buildBodyFont(400, true),
  strongEmphasis: buildBodyFont(BODY_STRONG_FONT_WEIGHT, true),
  lineHeight: BODY_LINE_HEIGHT_PX,
};

const TABLE_HEADER_TYPOGRAPHY: TextBlockTypography = {
  body: buildBodyFont(TABLE_HEADER_FONT_WEIGHT),
  strong: buildBodyFont(TABLE_HEADER_STRONG_FONT_WEIGHT),
  emphasis: buildBodyFont(TABLE_HEADER_FONT_WEIGHT, true),
  strongEmphasis: buildBodyFont(TABLE_HEADER_STRONG_FONT_WEIGHT, true),
  lineHeight: BODY_LINE_HEIGHT_PX,
};

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

function measureInlineCodeMinStartTextWidth(text: string, font: string): number {
  const sample = segmentGraphemes(text).slice(0, INLINE_CODE_MIN_START_GRAPHEMES).join("");
  if (sample.length === 0) {
    return 0;
  }
  const prepared = getPreparedTextWithSegments(
    buildPreparedContentKey(`inline-code-min-start:${font}`, sample),
    sample,
    font,
    "pre-wrap",
  );
  const wholeLine = measureSingleLineLayout(prepared);
  return wholeLine?.width ?? 0;
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
    startsStyledTextAfterInlineCodeSeam: boolean;
    startsAfterStyledTextSeam: boolean;
    startsStyledTextAfterBodySeam: boolean;
    hasTrailingInlineCode: boolean;
  },
): void {
  const pushCollapsedSpace = () => {
    const previous = items[items.length - 1];
    if (previous?.kind === "space" && previous.codeGroupId == null) {
      return;
    }
    items.push({ kind: "space", width: params.collapsedSpaceWidth, codeGroupId: null, text: " " });
  };
  const normalized = params.text.replace(/\u00a0/g, " ").replace(/\r\n?/g, "\n").replace(/\n/g, " ");
  if (normalized.length === 0) {
    return;
  }
  const leadingWhitespace = normalized.match(/^\s+/)?.[0] ?? "";
  const trailingWhitespace = normalized.match(/\s+$/)?.[0] ?? "";
  const core = normalized.slice(leadingWhitespace.length, normalized.length - trailingWhitespace.length);

  if (leadingWhitespace.length > 0) {
    pushCollapsedSpace();
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
        codeGroupHasDottedPath: false,
        codeGroupHasTrailingText: false,
        codeGroupIsOnlyInlineCodeInSegment: false,
        codeGroupStartsAfterText: false,
        chromeWidth: 0,
        endCursor: wholeLine.end,
        fullWidth: wholeLine.width,
        isFirstCodeGroupFragment: false,
        startsAfterCodeWhitespace: false,
        isFirstPathFragmentAfterHyphenRun: false,
        isPathTailFragment: false,
        isSealedInlineCodeFragment: false,
        lineHeight: params.lineHeight,
        minStartTextWidth: 0,
        prefersFreshLineStart: false,
        startsStyledTextAfterInlineCodeSeam: params.startsStyledTextAfterInlineCodeSeam,
        startsAfterStyledTextSeam: params.startsAfterStyledTextSeam,
        startsStyledTextAfterBodySeam: params.startsStyledTextAfterBodySeam,
        hasTrailingInlineCode: params.hasTrailingInlineCode,
        prepared,
        text: core,
      });
    }
  }

  if (trailingWhitespace.length > 0) {
    pushCollapsedSpace();
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
      text: spaces,
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
    Math.max(params.typography.lineHeight, MONO_LINE_HEIGHT_PX) +
    SESSION_THREAD_MARKDOWN_INLINE_CODE_LINE_HEIGHT_PREMIUM_PX;
  const runHasRenderableText = (run: SessionMarkdownInlineRun): boolean =>
    run.kind === "text" && /\S/.test(run.text);
  const textRunStartsAfterStyledTextSeam = (
    runIndex: number,
    run: Extract<SessionMarkdownInlineRun, { kind: "text" }>,
  ): boolean => {
    if (run.style !== "body" || !runHasRenderableText(run)) {
      return false;
    }
    for (let candidateIndex = runIndex - 1; candidateIndex >= 0; candidateIndex -= 1) {
      const candidate = params.runs[candidateIndex]!;
      if (candidate.kind === "hardBreak") {
        break;
      }
      if (candidate.kind === "inlineCode") {
        return false;
      }
      if (!runHasRenderableText(candidate)) {
        continue;
      }
      return candidate.style !== "body";
    }
    return false;
  };
  const textRunStartsStyledTextAfterBodySeam = (
    runIndex: number,
    run: Extract<SessionMarkdownInlineRun, { kind: "text" }>,
  ): boolean => {
    if (run.style === "body" || !runHasRenderableText(run)) {
      return false;
    }
    for (let candidateIndex = runIndex - 1; candidateIndex >= 0; candidateIndex -= 1) {
      const candidate = params.runs[candidateIndex]!;
      if (candidate.kind === "hardBreak") {
        break;
      }
      if (candidate.kind === "inlineCode") {
        return false;
      }
      if (!runHasRenderableText(candidate)) {
        continue;
      }
      return candidate.style === "body";
    }
    return false;
  };
  const textRunStartsStyledTextAfterInlineCodeSeam = (
    runIndex: number,
    run: Extract<SessionMarkdownInlineRun, { kind: "text" }>,
  ): boolean => {
    if (run.style === "body" || !runHasRenderableText(run)) {
      return false;
    }
    for (let candidateIndex = runIndex - 1; candidateIndex >= 0; candidateIndex -= 1) {
      const candidate = params.runs[candidateIndex]!;
      if (candidate.kind === "hardBreak") {
        break;
      }
      if (candidate.kind === "inlineCode") {
        return true;
      }
      if (runHasRenderableText(candidate)) {
        return false;
      }
    }
    return false;
  };
  const textRunHasTrailingInlineCode = (runIndex: number): boolean => {
    for (let candidateIndex = runIndex + 1; candidateIndex < params.runs.length; candidateIndex += 1) {
      const candidate = params.runs[candidateIndex]!;
      if (candidate.kind === "hardBreak") {
        return false;
      }
      if (candidate.kind === "inlineCode") {
        return true;
      }
    }
    return false;
  };
  const codeGroupStartsAfterText = (runIndex: number): boolean => {
    for (let candidateIndex = runIndex - 1; candidateIndex >= 0; candidateIndex -= 1) {
      const candidate = params.runs[candidateIndex]!;
      if (candidate.kind === "hardBreak") {
        return false;
      }
      if (runHasRenderableText(candidate)) {
        return true;
      }
    }
    return false;
  };
  const codeGroupHasTrailingText = (runIndex: number): boolean => {
    for (let candidateIndex = runIndex + 1; candidateIndex < params.runs.length; candidateIndex += 1) {
      const candidate = params.runs[candidateIndex]!;
      if (candidate.kind === "hardBreak") {
        return false;
      }
      if (runHasRenderableText(candidate)) {
        return true;
      }
    }
    return false;
  };
  const codeGroupIsOnlyInlineCodeInSegment = (runIndex: number): boolean => {
    for (let candidateIndex = runIndex - 1; candidateIndex >= 0; candidateIndex -= 1) {
      const candidate = params.runs[candidateIndex]!;
      if (candidate.kind === "hardBreak") {
        break;
      }
      if (candidate.kind === "inlineCode") {
        return false;
      }
    }
    for (let candidateIndex = runIndex + 1; candidateIndex < params.runs.length; candidateIndex += 1) {
      const candidate = params.runs[candidateIndex]!;
      if (candidate.kind === "hardBreak") {
        break;
      }
      if (candidate.kind === "inlineCode") {
        return false;
      }
    }
    return true;
  };

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
      const codeGroupHasDottedPath =
        (run.text.includes("/") || run.text.includes("\\")) && run.text.includes(".");
      const startsAfterText = codeGroupStartsAfterText(index);
      const hasTrailingText = codeGroupHasTrailingText(index);
      const isOnlyInlineCodeInSegment = codeGroupIsOnlyInlineCodeInSegment(index);
      let firstCodeGroupFragment = true;
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
        const fragments = splitInlineCodeFragments(part);
        const startsAfterCodeWhitespace = partIndex > 0 && /^\s+$/.test(run.parts[partIndex - 1] ?? "");
        let sawHyphenFragment = false;
        let sawPathFragment = false;
        for (let fragmentIndex = 0; fragmentIndex < fragments.length; fragmentIndex += 1) {
          const fragment = fragments[fragmentIndex]!;
          const isPathFragment = fragment.includes("/") || fragment.includes("\\");
          const isFirstPathFragmentAfterHyphenRun =
            isPathFragment && sawHyphenFragment && !sawPathFragment;
          const prepared = getPreparedTextWithSegments(
            buildPreparedContentKey(
              `${params.cacheKeyPrefix}:${run.kind}:${index}:${partIndex}:${fragmentIndex}`,
              fragment,
            ),
            fragment,
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
            codeGroupHasDottedPath,
            codeGroupHasTrailingText: hasTrailingText,
            codeGroupIsOnlyInlineCodeInSegment: isOnlyInlineCodeInSegment,
            codeGroupStartsAfterText: startsAfterText,
            chromeWidth: SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX,
            endCursor: wholeLine.end,
            fullWidth: wholeLine.width,
            isFirstCodeGroupFragment: firstCodeGroupFragment,
            startsAfterCodeWhitespace: startsAfterCodeWhitespace && fragmentIndex === 0,
            isFirstPathFragmentAfterHyphenRun,
            isPathTailFragment:
              sawPathFragment &&
              !isPathFragment &&
              !fragment.endsWith(".") &&
              !fragment.endsWith("-"),
            isSealedInlineCodeFragment: isSealedInlineCodeFragment(fragment),
            lineHeight: inlineCodeLineHeight,
            minStartTextWidth:
              firstCodeGroupFragment ? measureInlineCodeMinStartTextWidth(part, inlineCodeFont) : 0,
            prefersFreshLineStart: fragment.includes("/") || fragment.includes("\\") || fragment.endsWith("."),
            startsStyledTextAfterInlineCodeSeam: false,
            startsAfterStyledTextSeam: false,
            startsStyledTextAfterBodySeam: false,
            hasTrailingInlineCode: false,
            prepared,
            text: fragment,
          });
          firstCodeGroupFragment = false;
          sawHyphenFragment ||= fragment.endsWith("-");
          sawPathFragment ||= isPathFragment;
        }
      }
      continue;
    }

    const font = resolveTextRunFont(run, params.typography);
    const collapsedSpaceWidth = measureCollapsedSpaceWidth(font);
    pushTextRunItems(items, {
      text: run.text,
      font,
      lineHeight: params.typography.lineHeight,
      cacheKeyPrefix: `${params.cacheKeyPrefix}:${run.kind}:${index}`,
      collapsedSpaceWidth,
      startsStyledTextAfterInlineCodeSeam: textRunStartsStyledTextAfterInlineCodeSeam(index, run),
      startsAfterStyledTextSeam: textRunStartsAfterStyledTextSeam(index, run),
      startsStyledTextAfterBodySeam: textRunStartsStyledTextAfterBodySeam(index, run),
      hasTrailingInlineCode: textRunHasTrailingInlineCode(index),
    });
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
  const inlineCodeDebugWindow =
    typeof window !== "undefined" ? (window as SessionMarkdownDebugWindow) : null;
  const forceInlineCodeDebug = inlineCodeDebugWindow?.__ctxForceInlineCodeDebug === true;
  const inlineCodeDebugTarget = inlineCodeDebugWindow?.__ctxInlineCodeDebugTarget ?? null;
  const debugInlineCode =
    (forceInlineCodeDebug || inlineCodeDebugTarget != null) &&
    (inlineCodeDebugWindow?.__ctxInlineCodeDebugWidth == null ||
      inlineCodeDebugWindow.__ctxInlineCodeDebugWidth === params.width) &&
    params.runs.some(
      (run) =>
        run.kind === "inlineCode" &&
        (forceInlineCodeDebug ||
          inlineCodeDebugTarget === "*" ||
          run.text === inlineCodeDebugTarget ||
          run.text.includes(inlineCodeDebugTarget ?? "")),
    );
  const debugLines: string[] = [];
  const debugStartDecisions: Array<{
    pendingSpaceWidth: number;
    preferredStartWidth: number;
    dottedPathClusterWidth: number;
    wholeCodeGroupWidth: number;
    remainingWidth: number;
    shouldBreak: boolean;
    text: string;
  }> = [];
  const preferredCodeGroupStartWidths = new Map<number, number>();
  const dottedPathCodeGroupStartClusterWidths = new Map<number, number>();
  const wholeCodeGroupInlineWidths = new Map<number, number>();
  const isPathLikeContinuationItem = (
    item: PreparedInlineLayoutItem & { kind: "segment" },
  ): boolean => item.text.includes("/") || item.text.includes("\\") || item.isPathTailFragment;
  const measurePreferredCodeGroupStartWidth = (startIndex: number): number => {
    const firstItem = items[startIndex];
    if (firstItem?.kind !== "segment" || firstItem.codeGroupId == null) {
      return 0;
    }
    const codeGroupId = firstItem.codeGroupId;
    let lineWidth = 0;
    let remainingWidth = maxWidth;
    let lineHasContent = false;
    let pendingSpaceWidth = 0;
    let chargedChrome = false;

    for (let index = startIndex; index < items.length; index += 1) {
      const item = items[index]!;
      if (item.kind === "hardBreak") {
        break;
      }
      if (item.kind === "space") {
        if (item.codeGroupId !== codeGroupId) {
          break;
        }
        if (lineHasContent) {
          pendingSpaceWidth = item.width;
        }
        continue;
      }
      if (item.codeGroupId !== codeGroupId) {
        break;
      }

      const reservedWidth = (lineHasContent ? pendingSpaceWidth : 0) + (chargedChrome ? 0 : item.chromeWidth);
      const availableWidth = Math.max(1, remainingWidth - reservedWidth);
      if (item.isSealedInlineCodeFragment) {
        const fullWidth = reservedWidth + item.fullWidth;
        if (lineHasContent && fullWidth > remainingWidth + 0.01) {
          break;
        }
        const overflowed = fullWidth > remainingWidth + 0.01;
        lineWidth += fullWidth;
        remainingWidth = overflowed ? 0 : Math.max(0, remainingWidth - fullWidth);
        chargedChrome = true;
        lineHasContent = true;
        pendingSpaceWidth = 0;
        if (overflowed) {
          break;
        }
        continue;
      }

      const line = layoutNextLine(item.prepared, LINE_START_CURSOR, availableWidth);
      if (line == null || cursorsMatch(LINE_START_CURSOR, line.end)) {
        break;
      }

      lineWidth += reservedWidth + line.width;
      remainingWidth = Math.max(0, remainingWidth - reservedWidth - line.width);
      chargedChrome = true;
      lineHasContent = true;
      pendingSpaceWidth = 0;
      if (!cursorsMatch(line.end, item.endCursor)) {
        break;
      }
    }

    return lineWidth;
  };
  const measureDottedPathCodeGroupStartClusterWidth = (startIndex: number): number => {
    const firstItem = items[startIndex];
    if (firstItem?.kind !== "segment" || firstItem.codeGroupId == null) {
      return 0;
    }

    let lineWidth = 0;
    let pendingSpaceWidth = 0;
    let lineHasContent = false;
    let chargedChrome = false;
    let sawPathLikeFragment = false;
    let sawDottedStem = false;

    for (let index = startIndex; index < items.length; index += 1) {
      const item = items[index]!;
      if (item.kind === "hardBreak") {
        break;
      }
      if (item.kind === "space") {
        break;
      }
      if (item.codeGroupId !== firstItem.codeGroupId) {
        break;
      }

      lineWidth +=
        (lineHasContent ? pendingSpaceWidth : 0) + (chargedChrome ? 0 : item.chromeWidth) + item.fullWidth;
      lineHasContent = true;
      chargedChrome = true;
      pendingSpaceWidth = 0;
      sawPathLikeFragment ||= item.text.includes("/") || item.text.includes("\\") || item.isPathTailFragment;
      sawDottedStem ||= item.text.endsWith(".");
    }

    return sawPathLikeFragment && sawDottedStem ? lineWidth : 0;
  };
  const measureWholeCodeGroupInlineWidth = (startIndex: number): number => {
    const firstItem = items[startIndex];
    if (firstItem?.kind !== "segment" || firstItem.codeGroupId == null) {
      return 0;
    }
    const codeGroupId = firstItem.codeGroupId;
    let lineWidth = 0;
    let pendingSpaceWidth = 0;
    let lineHasContent = false;
    let chargedChrome = false;

    for (let index = startIndex; index < items.length; index += 1) {
      const item = items[index]!;
      if (item.kind === "hardBreak") {
        break;
      }
      if (item.kind === "space") {
        if (item.codeGroupId !== codeGroupId) {
          break;
        }
        if (lineHasContent) {
          pendingSpaceWidth = item.width;
        }
        continue;
      }
      if (item.codeGroupId !== codeGroupId) {
        break;
      }

      lineWidth +=
        (lineHasContent ? pendingSpaceWidth : 0) + (chargedChrome ? 0 : item.chromeWidth) + item.fullWidth;
      lineHasContent = true;
      chargedChrome = true;
      pendingSpaceWidth = 0;
    }

    return lineWidth;
  };
  const shouldPreserveSealedInlineCodeBoundary = (params: {
    sealedBoundary: boolean;
    reservedWidth: number;
    remainingWidth: number;
    item: PreparedInlineLayoutItem & { kind: "segment" };
  }): boolean =>
    params.sealedBoundary &&
    params.reservedWidth + params.item.fullWidth > params.remainingWidth + 0.01;
  const measureCodeGroupFitWithinWidth = (
    startIndex: number,
    availableWidth: number,
  ): {
    consumedWidth: number;
    endedAtGroupEnd: boolean;
    endedInsideFragment: boolean;
    lastFragmentText: string | null;
    nextFragmentText: string | null;
    nextStartsAfterCodeWhitespace: boolean;
  } => {
    const firstItem = items[startIndex];
    if (firstItem?.kind !== "segment" || firstItem.codeGroupId == null) {
      return {
        consumedWidth: 0,
        endedAtGroupEnd: false,
        endedInsideFragment: false,
        lastFragmentText: null,
        nextFragmentText: null,
        nextStartsAfterCodeWhitespace: false,
      };
    }
    const codeGroupId = firstItem.codeGroupId;
    let remainingWidth = Math.max(1, availableWidth);
    let lineHasContent = false;
    let pendingSpaceWidth = 0;
    let chargedChrome = false;
    let lastFragmentText: string | null = null;
    let consumedWidth = 0;

    for (let index = startIndex; index < items.length; index += 1) {
      const item = items[index]!;
      if (item.kind === "hardBreak") {
        return {
          consumedWidth,
          endedAtGroupEnd: true,
          endedInsideFragment: false,
          lastFragmentText,
          nextFragmentText: null,
          nextStartsAfterCodeWhitespace: false,
        };
      }
      if (item.kind === "space") {
        if (item.codeGroupId !== codeGroupId) {
          return {
            consumedWidth,
            endedAtGroupEnd: true,
            endedInsideFragment: false,
            lastFragmentText,
            nextFragmentText: null,
            nextStartsAfterCodeWhitespace: false,
          };
        }
        if (lineHasContent) {
          pendingSpaceWidth = item.width;
        }
        continue;
      }
      if (item.codeGroupId !== codeGroupId) {
        return {
          consumedWidth,
          endedAtGroupEnd: true,
          endedInsideFragment: false,
          lastFragmentText,
          nextFragmentText: null,
          nextStartsAfterCodeWhitespace: false,
        };
      }

      if (
        lineHasContent &&
        firstItem.codeGroupHasDottedPath &&
        lastFragmentText?.endsWith("-") &&
        isPathLikeContinuationItem(item)
      ) {
        const continuationFit = measureCodeGroupFitWithinWidth(index, remainingWidth);
        if (!codeGroupFitEndsAtFriendlyBoundary(continuationFit)) {
          return {
            consumedWidth,
            endedAtGroupEnd: false,
            endedInsideFragment: false,
            lastFragmentText,
            nextFragmentText: item.text,
            nextStartsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
          };
        }
      }

      const reservedWidth = (lineHasContent ? pendingSpaceWidth : 0) + (chargedChrome ? 0 : item.chromeWidth);
      if (
        lineHasContent &&
        shouldPreserveSealedInlineCodeBoundary({
          sealedBoundary:
            lastFragmentText != null && isSealedInlineCodeFragment(lastFragmentText),
          reservedWidth,
          remainingWidth,
          item,
        })
      ) {
        return {
          consumedWidth,
          endedAtGroupEnd: false,
          endedInsideFragment: false,
          lastFragmentText,
          nextFragmentText: item.text,
          nextStartsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
        };
      }

      if (item.isSealedInlineCodeFragment) {
        const fullWidth = reservedWidth + item.fullWidth;
        if (fullWidth > remainingWidth + 0.01) {
          return {
            consumedWidth,
            endedAtGroupEnd: false,
            endedInsideFragment: false,
            lastFragmentText,
            nextFragmentText: item.text,
            nextStartsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
          };
        }
        remainingWidth = Math.max(0, remainingWidth - fullWidth);
        consumedWidth += fullWidth;
      } else {
        const availableLineWidth = Math.max(1, remainingWidth - reservedWidth);
        const line = layoutNextLine(item.prepared, LINE_START_CURSOR, availableLineWidth);
        if (line == null || cursorsMatch(LINE_START_CURSOR, line.end)) {
          return {
            consumedWidth,
            endedAtGroupEnd: false,
            endedInsideFragment: true,
            lastFragmentText,
            nextFragmentText: item.text,
            nextStartsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
          };
        }
        remainingWidth = Math.max(0, remainingWidth - reservedWidth - line.width);
        consumedWidth += reservedWidth + line.width;
        if (!cursorsMatch(line.end, item.endCursor)) {
          return {
            consumedWidth,
            endedAtGroupEnd: false,
            endedInsideFragment: true,
            lastFragmentText,
            nextFragmentText: item.text,
            nextStartsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
          };
        }
      }

      lineHasContent = true;
      chargedChrome = true;
      pendingSpaceWidth = 0;
      lastFragmentText = item.text;
    }

    return {
      consumedWidth,
      endedAtGroupEnd: true,
      endedInsideFragment: false,
      lastFragmentText,
      nextFragmentText: null,
      nextStartsAfterCodeWhitespace: false,
    };
  };
  const codeGroupFitEndsAtFriendlyBoundary = (fit: {
    endedAtGroupEnd: boolean;
    endedInsideFragment: boolean;
    lastFragmentText: string | null;
    nextFragmentText: string | null;
    nextStartsAfterCodeWhitespace: boolean;
  }): boolean => {
    if (fit.endedInsideFragment) {
      return false;
    }
    if (fit.endedAtGroupEnd) {
      return true;
    }
    if (fit.nextStartsAfterCodeWhitespace) {
      return true;
    }
    const lastFragmentText = fit.lastFragmentText ?? "";
    return isSealedInlineCodeFragment(lastFragmentText) && fit.nextFragmentText != null;
  };

  for (let index = 0; index < items.length; index += 1) {
    const item = items[index]!;
    if (item.kind === "segment" && item.codeGroupId != null && item.isFirstCodeGroupFragment) {
      preferredCodeGroupStartWidths.set(index, measurePreferredCodeGroupStartWidth(index));
      dottedPathCodeGroupStartClusterWidths.set(index, measureDottedPathCodeGroupStartClusterWidth(index));
      wholeCodeGroupInlineWidths.set(index, measureWholeCodeGroupInlineWidth(index));
    }
  }
  let totalHeight = 0;
  let itemIndex = 0;
  let cursor: LayoutCursor | null = null;

  while (itemIndex < items.length) {
    let lineHeight = params.typography.lineHeight;
    let lineHasContent = false;
    let lineOnlyCodeGroupId: number | null = null;
    let lineLastCodeFragmentEndedWithHyphen = false;
    let lineLastCodeFragmentEndedWithPathDelimiter = false;
    let lastAcceptedCodeGroupId: number | null = null;
    let lineStartedWithContinuedCode = false;
    let lineAcceptedPlainAfterContinuedCode = false;
    let forcedBreak = false;
    let remainingWidth = maxWidth;
    let pendingSpaceWidth = 0;
    const chargedCodeGroups = new Set<number>();
    let debugLine = "";

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
          if (debugInlineCode) {
            debugLine += item.text;
          }
        }
        continue;
      }

      const codeGroupId = item.codeGroupId;
      const chromeWidth =
        codeGroupId != null && !chargedCodeGroups.has(codeGroupId) ? item.chromeWidth : 0;
      const reservedWidth = (lineHasContent ? pendingSpaceWidth : 0) + chromeWidth;
      const currentLineStartSlackPx =
        lineHasContent && cursor === null && item.isFirstCodeGroupFragment && item.codeGroupStartsAfterText
          ? INLINE_CODE_WHOLE_GROUP_FIT_SLACK_PX
          : INLINE_CODE_FRAGMENT_FIT_SLACK_PX;
      const currentLineWhitespaceContinuationSlackPx =
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        item.startsAfterCodeWhitespace &&
        item.codeGroupStartsAfterText &&
        chargedCodeGroups.has(codeGroupId)
          ? INLINE_CODE_WHOLE_GROUP_FIT_SLACK_PX
          : 0;
      const currentLineFitSlackPx = Math.max(currentLineStartSlackPx, currentLineWhitespaceContinuationSlackPx);
      const startCursor = cursor ?? LINE_START_CURSOR;

      const preferredStartWidth = preferredCodeGroupStartWidths.get(itemIndex) ?? 0;
      const dottedPathClusterWidth = dottedPathCodeGroupStartClusterWidths.get(itemIndex) ?? 0;
      const wholeCodeGroupWidth = wholeCodeGroupInlineWidths.get(itemIndex) ?? 0;
      const currentLineCodeFit =
        lineHasContent && cursor === null && codeGroupId != null && item.isFirstCodeGroupFragment
          ? measureCodeGroupFitWithinWidth(
              itemIndex,
              Math.max(1, remainingWidth - pendingSpaceWidth + currentLineFitSlackPx),
            )
          : null;
      const codeFitEndsAtFriendlyBoundary =
        currentLineCodeFit != null && codeGroupFitEndsAtFriendlyBoundary(currentLineCodeFit);
      const freshLineCodeFit =
        lineHasContent && cursor === null && codeGroupId != null && item.isFirstCodeGroupFragment
          ? measureCodeGroupFitWithinWidth(itemIndex, maxWidth)
          : null;
      const freshLineFitEndsAtFriendlyBoundary =
        freshLineCodeFit != null && codeGroupFitEndsAtFriendlyBoundary(freshLineCodeFit);
      const currentLineStartFitRatio =
        currentLineCodeFit != null && freshLineCodeFit != null && freshLineCodeFit.consumedWidth > 0
          ? currentLineCodeFit.consumedWidth / freshLineCodeFit.consumedWidth
          : 1;
      const shouldBreakForPreferredStart =
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        item.isFirstCodeGroupFragment &&
        item.prefersFreshLineStart &&
        currentLineCodeFit != null &&
        freshLineCodeFit != null &&
        freshLineCodeFit.consumedWidth > 0 &&
        (
          !codeFitEndsAtFriendlyBoundary ||
          currentLineStartFitRatio < INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD
        );
      if (
        debugInlineCode &&
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        item.isFirstCodeGroupFragment &&
        item.prefersFreshLineStart
      ) {
        debugStartDecisions.push({
          pendingSpaceWidth,
          preferredStartWidth,
          dottedPathClusterWidth,
          wholeCodeGroupWidth,
          remainingWidth,
          shouldBreak: shouldBreakForPreferredStart,
          text: item.text,
        });
      }
      if (
        shouldBreakForPreferredStart
      ) {
        cursor = null;
        break;
      }

      // When inline code begins after prose, prefer the natural wrap opportunity
      // before the code span only if the remaining room is too narrow to start
      // a meaningful chip fragment.
      if (lineHasContent && cursor === null && codeGroupId != null) {
        const fullWidth = reservedWidth + item.fullWidth;
        if (
          fullWidth > remainingWidth + 0.01 &&
          remainingWidth < reservedWidth + item.minStartTextWidth - 0.01
        ) {
          cursor = null;
          break;
        }
      }

      if (cursor === null && codeGroupId != null) {
        const fullWidth = reservedWidth + item.fullWidth;
        if (
          lineHasContent &&
          lastAcceptedCodeGroupId === codeGroupId &&
          shouldPreserveSealedInlineCodeBoundary({
            sealedBoundary: lineLastCodeFragmentEndedWithHyphen || lineLastCodeFragmentEndedWithPathDelimiter,
            reservedWidth,
            remainingWidth: remainingWidth + currentLineFitSlackPx,
            item,
          })
        ) {
          cursor = null;
          break;
        }
        if (
          lineHasContent &&
          item.codeGroupHasDottedPath &&
          lastAcceptedCodeGroupId === codeGroupId &&
          (
            (
              lineLastCodeFragmentEndedWithHyphen &&
              (item.isFirstPathFragmentAfterHyphenRun || item.prefersFreshLineStart || item.isPathTailFragment)
            ) ||
            (lineLastCodeFragmentEndedWithPathDelimiter && item.isPathTailFragment)
          )
        ) {
          const continuationFit = measureCodeGroupFitWithinWidth(itemIndex, remainingWidth);
          const freshLineContinuationFit = measureCodeGroupFitWithinWidth(itemIndex, maxWidth);
          const continuationFitRatio =
            freshLineContinuationFit.consumedWidth > 0
              ? continuationFit.consumedWidth / freshLineContinuationFit.consumedWidth
              : 1;
          if (
            !codeGroupFitEndsAtFriendlyBoundary(continuationFit) ||
            continuationFitRatio < INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD
          ) {
            cursor = null;
            break;
          }
        }
        if (item.isSealedInlineCodeFragment) {
          if (lineHasContent && fullWidth > remainingWidth + currentLineFitSlackPx + 0.01) {
            cursor = null;
            break;
          }
          const overflowed = fullWidth > remainingWidth + currentLineFitSlackPx + 0.01;
          remainingWidth = overflowed ? 0 : Math.max(0, remainingWidth - fullWidth);
          if (!lineHasContent) {
            lineOnlyCodeGroupId = codeGroupId;
            lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
            lineLastCodeFragmentEndedWithPathDelimiter = /[./\\]$/.test(item.text);
            lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
          } else if (lineOnlyCodeGroupId !== codeGroupId) {
            lineOnlyCodeGroupId = null;
          }
          lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
          lineLastCodeFragmentEndedWithPathDelimiter = /[./\\]$/.test(item.text);
          lastAcceptedCodeGroupId = codeGroupId;
          lineHasContent = true;
          lineHeight = Math.max(lineHeight, item.lineHeight);
          chargedCodeGroups.add(codeGroupId);
          itemIndex += 1;
          pendingSpaceWidth = 0;
          if (debugInlineCode) {
            debugLine += item.text;
          }
          if (overflowed) {
            cursor = null;
            break;
          }
          continue;
        }
        if (fullWidth <= remainingWidth + currentLineFitSlackPx) {
          remainingWidth = Math.max(0, remainingWidth - fullWidth);
          if (!lineHasContent) {
            lineOnlyCodeGroupId = codeGroupId;
            lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
            lineLastCodeFragmentEndedWithPathDelimiter = /[./\\]$/.test(item.text);
            lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
          } else if (lineOnlyCodeGroupId !== codeGroupId) {
            lineOnlyCodeGroupId = null;
          }
          lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
          lineLastCodeFragmentEndedWithPathDelimiter = /[./\\]$/.test(item.text);
          lastAcceptedCodeGroupId = codeGroupId;
          lineHasContent = true;
          lineHeight = Math.max(lineHeight, item.lineHeight);
          chargedCodeGroups.add(codeGroupId);
          itemIndex += 1;
          pendingSpaceWidth = 0;
          if (debugInlineCode) {
            debugLine += item.text;
          }
          continue;
        }
        if (
          lineHasContent &&
          item.startsAfterCodeWhitespace &&
          fullWidth > remainingWidth + currentLineFitSlackPx + 0.01
        ) {
          cursor = null;
          break;
        }
        if (
          lineHasContent &&
          (
            (item.isSealedInlineCodeFragment && (item.text.includes("/") || item.text.includes("\\"))) ||
            item.text.endsWith("-") ||
            item.isPathTailFragment
          )
        ) {
          cursor = null;
          break;
        }
      }

      if (lineHasContent && remainingWidth < reservedWidth - 0.01) {
        cursor = null;
        break;
      }

      const plainAfterContinuedCodeSlackPx =
        codeGroupId == null && lineStartedWithContinuedCode && !lineAcceptedPlainAfterContinuedCode
          ? INLINE_CODE_WHOLE_GROUP_FIT_SLACK_PX
          : 0;
      const availableWidth = Math.max(1, remainingWidth - reservedWidth);
      const styledStartLine =
        codeGroupId == null &&
        lineHasContent &&
        cursor === null &&
        item.startsStyledTextAfterBodySeam &&
        item.fullWidth > availableWidth + 0.01
          ? layoutNextLine(item.prepared, startCursor, availableWidth)
          : null;
      if (
        codeGroupId == null &&
        lineHasContent &&
        cursor === null &&
        item.startsStyledTextAfterInlineCodeSeam &&
        item.hasTrailingInlineCode &&
        item.fullWidth > availableWidth + 0.01
      ) {
        cursor = null;
        break;
      }
      if (
        codeGroupId == null &&
        lineHasContent &&
        cursor === null &&
        item.startsStyledTextAfterBodySeam &&
        item.fullWidth > availableWidth + 0.01 &&
        (
          styledStartLine == null ||
          cursorsMatch(startCursor, styledStartLine.end) ||
          styledStartLine.width / Math.max(1, item.fullWidth) < 0.25
        )
      ) {
        cursor = null;
        break;
      }
      const allowWholeSegmentFastPath =
        codeGroupId == null &&
        cursor === null &&
        !item.startsAfterStyledTextSeam;
      if (
        allowWholeSegmentFastPath &&
        item.fullWidth <= availableWidth + plainAfterContinuedCodeSlackPx + 0.01
      ) {
        remainingWidth = Math.max(0, remainingWidth - reservedWidth - item.fullWidth);
        lineOnlyCodeGroupId = null;
        lastAcceptedCodeGroupId = null;
        lineLastCodeFragmentEndedWithHyphen = false;
        lineLastCodeFragmentEndedWithPathDelimiter = false;
        lineHasContent = true;
        lineHeight = Math.max(lineHeight, item.lineHeight);
        lineAcceptedPlainAfterContinuedCode ||= codeGroupId == null && lineStartedWithContinuedCode;
        pendingSpaceWidth = 0;
        if (debugInlineCode) {
          debugLine += item.text;
        }
        itemIndex += 1;
        continue;
      }
      const line = styledStartLine ?? layoutNextLine(item.prepared, startCursor, availableWidth);
      if (line == null || cursorsMatch(startCursor, line.end)) {
        if (!lineHasContent) {
          itemIndex += 1;
        }
        cursor = null;
        break;
      }

      remainingWidth = Math.max(0, remainingWidth - reservedWidth - line.width);
      if (!lineHasContent) {
        lineOnlyCodeGroupId = codeGroupId;
        lineLastCodeFragmentEndedWithHyphen =
          codeGroupId != null && cursorsMatch(line.end, item.endCursor) && item.text.endsWith("-");
        lineLastCodeFragmentEndedWithPathDelimiter =
          codeGroupId != null && cursorsMatch(line.end, item.endCursor) && /[./\\]$/.test(item.text);
        lineStartedWithContinuedCode =
          codeGroupId != null && (!item.isFirstCodeGroupFragment || !cursorsMatch(startCursor, LINE_START_CURSOR));
      } else if (lineOnlyCodeGroupId !== codeGroupId) {
        lineOnlyCodeGroupId = null;
      }
      lineLastCodeFragmentEndedWithHyphen =
        codeGroupId != null && cursorsMatch(line.end, item.endCursor) && item.text.endsWith("-");
      lineLastCodeFragmentEndedWithPathDelimiter =
        codeGroupId != null && cursorsMatch(line.end, item.endCursor) && /[./\\]$/.test(item.text);
      lastAcceptedCodeGroupId = codeGroupId;
      lineHasContent = true;
      lineHeight = Math.max(lineHeight, item.lineHeight);
      if (codeGroupId != null) {
        chargedCodeGroups.add(codeGroupId);
      } else {
        lastAcceptedCodeGroupId = null;
        lineLastCodeFragmentEndedWithHyphen = false;
        lineLastCodeFragmentEndedWithPathDelimiter = false;
        lineAcceptedPlainAfterContinuedCode ||= lineStartedWithContinuedCode;
      }
      pendingSpaceWidth = 0;
      if (debugInlineCode) {
        const segmentText = segmentGraphemes(item.text)
          .slice(startCursor.graphemeIndex, line.end.graphemeIndex)
          .join("");
        debugLine += segmentText;
      }

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
    if (debugInlineCode) {
      debugLines.push(debugLine);
    }
  }

  if (debugInlineCode && inlineCodeDebugWindow) {
    inlineCodeDebugWindow.__ctxInlineCodeDebug = {
      lines: debugLines,
      startDecisions: debugStartDecisions,
      items: items.map((item) =>
        item.kind === "segment"
          ? {
              kind: item.kind,
              text: item.text,
              chromeWidth: item.chromeWidth,
              fullWidth: item.fullWidth,
              minStartTextWidth: item.minStartTextWidth,
            }
          : {
              kind: item.kind,
              text: item.text,
            },
      ),
      width: params.width,
    };
  }

  return clampHeight(totalHeight);
}

function headingTypography(depth: number): TextBlockTypography {
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

export function clearSessionMarkdownMeasurementCaches(): void {
  markdownDocumentCache.clear();
  clearSessionTextMeasurementCaches();
  clearSessionPlainTextMeasurementCaches();
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
