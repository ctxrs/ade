import { layoutNextLine, type LayoutCursor, type LayoutLine } from "@chenglou/pretext";
import { isSealedInlineCodeFragment } from "../../utils/inlineCodeFragments";
import type { SessionMarkdownInlineRun } from "./sessionMarkdownContract";
import { prepareInlineLayoutItems, type PreparedInlineLayoutItem } from "./sessionMarkdownInlineLayout";
import {
  LINE_START_CURSOR,
  clampHeight,
  cursorsMatch,
  segmentGraphemes,
  type SessionMarkdownDebugWindow,
  type TextBlockTypography,
} from "./sessionMarkdownMeasurementCore";

const INLINE_CODE_FRAGMENT_FIT_SLACK_PX = 0;
const INLINE_CODE_WHOLE_GROUP_FIT_SLACK_PX = 8;
const INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD = 0.2;

export function measureInlineRunsHeight(params: {
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
      const startCursor: LayoutCursor = cursor ?? LINE_START_CURSOR;

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
        (!codeFitEndsAtFriendlyBoundary ||
          currentLineStartFitRatio < INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD);
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
      if (shouldBreakForPreferredStart) {
        cursor = null;
        break;
      }

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
          ((lineLastCodeFragmentEndedWithHyphen &&
            (item.isFirstPathFragmentAfterHyphenRun || item.prefersFreshLineStart || item.isPathTailFragment)) ||
            (lineLastCodeFragmentEndedWithPathDelimiter && item.isPathTailFragment))
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
          ((item.isSealedInlineCodeFragment && (item.text.includes("/") || item.text.includes("\\"))) ||
            item.text.endsWith("-") ||
            item.isPathTailFragment)
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
      const styledStartLine: LayoutLine | null =
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
        (styledStartLine == null ||
          cursorsMatch(startCursor, styledStartLine.end) ||
          styledStartLine.width / Math.max(1, item.fullWidth) < 0.25)
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
      const line: LayoutLine | null =
        styledStartLine ?? layoutNextLine(item.prepared, startCursor, availableWidth);
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
      items: items.map((item) => {
        if (item.kind === "segment") {
          return {
            kind: item.kind,
            text: item.text,
            chromeWidth: item.chromeWidth,
            fullWidth: item.fullWidth,
            minStartTextWidth: item.minStartTextWidth,
          };
        }
        if (item.kind === "space") {
          return {
            kind: item.kind,
            text: item.text,
          };
        }
        return {
          kind: item.kind,
          text: "",
        };
      }),
      width: params.width,
    };
  }

  return clampHeight(totalHeight);
}
