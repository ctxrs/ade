import type { PreparedTextWithSegments } from "@chenglou/pretext";
import {
  isSealedInlineCodeFragment,
  splitInlineCodeFragments,
} from "../../utils/inlineCodeFragments";
import type { SessionMarkdownInlineRun } from "./sessionMarkdownContract";
import {
  LINE_START_CURSOR,
  buildPreparedContentKey,
  getPreparedTextWithSegments,
  measureCollapsedSpaceWidth,
  measureInlineSpaceWidth,
  measureSingleLineLayout,
  resolveTextRunFont,
  segmentGraphemes,
  type TextBlockTypography,
} from "./sessionMarkdownMeasurementCore";
import {
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX,
  SESSION_THREAD_MARKDOWN_INLINE_CODE_FRAGMENT_CHROME_WIDTH_PX,
} from "./sessionThreadLayoutTokens";

const INLINE_CODE_MIN_START_GRAPHEMES = 4;

export type PreparedInlineLayoutItem =
  | { kind: "hardBreak" }
  | { kind: "space"; width: number; codeGroupId: number | null; text: string }
  | {
      kind: "segment";
      codeGroupId: number | null;
      codeGroupHasDottedPath: boolean;
      codeGroupHasTrailingText: boolean;
      codeGroupIsOnlyInlineCodeInSegment: boolean;
      codeGroupStartsAfterText: boolean;
      codePartStartsAfterWhitespace: boolean;
      chromeWidth: number;
      endCursor: { segmentIndex: number; graphemeIndex: number };
      fullWidth: number;
      isFirstCodeGroupFragment: boolean;
      startsAfterCodeWhitespace: boolean;
      isFirstPathFragmentAfterHyphenRun: boolean;
      isPathTailFragment: boolean;
      isSealedInlineCodeFragment: boolean;
      minStartTextWidth: number;
      prefersFreshLineStart: boolean;
      startsStyledTextAfterInlineCodeSeam: boolean;
      startsAfterStyledTextSeam: boolean;
      startsStyledTextAfterBodySeam: boolean;
      hasTrailingInlineCode: boolean;
      prepared: PreparedTextWithSegments;
      text: string;
    };

const resolveInlineCodeFont = (textFont: string): string => {
  void textFont;
  return `${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_SIZE_PX}px ${SESSION_THREAD_MARKDOWN_INLINE_CODE_FONT_FAMILY}`;
};

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

function pushTextRunItems(
  items: PreparedInlineLayoutItem[],
  params: {
    text: string;
    font: string;
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
        codePartStartsAfterWhitespace: false,
        chromeWidth: 0,
        endCursor: wholeLine.end,
        fullWidth: wholeLine.width,
        isFirstCodeGroupFragment: false,
        startsAfterCodeWhitespace: false,
        isFirstPathFragmentAfterHyphenRun: false,
        isPathTailFragment: false,
        isSealedInlineCodeFragment: false,
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

export function prepareInlineLayoutItems(params: {
  runs: readonly SessionMarkdownInlineRun[];
  typography: TextBlockTypography;
  cacheKeyPrefix: string;
}): PreparedInlineLayoutItem[] {
  const items: PreparedInlineLayoutItem[] = [];
  const inlineCodeFont = resolveInlineCodeFont(params.typography.body);
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
            codePartStartsAfterWhitespace: startsAfterCodeWhitespace,
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
