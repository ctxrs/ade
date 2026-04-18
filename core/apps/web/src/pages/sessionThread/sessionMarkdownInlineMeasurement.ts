import { layoutNextLine, type LayoutCursor, type LayoutLine } from "@chenglou/pretext";
import type { SessionMarkdownInlineRun } from "./sessionMarkdownContract";
import { createInlineCodeFitPlanner, resolveInlineCodeWrapChromeWidth } from "./sessionMarkdownInlineCodeFit";
import { prepareInlineLayoutItems } from "./sessionMarkdownInlineLayout";
import {
  LINE_START_CURSOR,
  clampHeight,
  cursorsMatch,
  segmentGraphemes,
  type SessionMarkdownDebugWindow,
  type TextBlockTypography,
} from "./sessionMarkdownMeasurementCore";

const INLINE_CODE_FRAGMENT_FIT_SLACK_PX = 0;
const INLINE_CODE_CONTINUATION_FIT_SLACK_PX = 5;
const INLINE_CODE_WHOLE_GROUP_FIT_SLACK_PX = 0;
const INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD = 0.2;
const INLINE_CODE_WHITESPACE_CONTINUATION_GUARD_PX = 1;

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
  const {
    codeGroupFitEndsAtFriendlyBoundary,
    dottedPathCodeGroupStartClusterWidths,
    measureAttachedTrailingPlainWidth,
    measureCodeGroupFitWithinWidth,
    preferredCodeGroupStartWidths,
    shouldPreserveSealedInlineCodeBoundary,
    wholeCodeGroupInlineWidths,
  } = createInlineCodeFitPlanner({ items, maxWidth });
  let totalHeight = 0;
  let itemIndex = 0;
  let cursor: LayoutCursor | null = null;

  while (itemIndex < items.length) {
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
        codeGroupId != null
          ? resolveInlineCodeWrapChromeWidth({
              chromeWidth: item.chromeWidth,
              codeGroupHasWhitespace: item.codeGroupHasWhitespace,
              codeGroupStartsAfterText: item.codeGroupStartsAfterText,
              chargedChrome: chargedCodeGroups.has(codeGroupId),
              isFirstCodeGroupFragment: item.isFirstCodeGroupFragment,
              prefersFreshLineStart: item.prefersFreshLineStart,
              lineHasContent,
              startsAtLineStart: !lineHasContent,
            })
          : 0;
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
      const currentLineCodeContinuationSlackPx =
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        lastAcceptedCodeGroupId === codeGroupId &&
        !item.startsAfterCodeWhitespace
          ? INLINE_CODE_CONTINUATION_FIT_SLACK_PX
          : 0;
      const currentLineFitSlackPx = Math.max(
        currentLineStartSlackPx,
        currentLineWhitespaceContinuationSlackPx,
        currentLineCodeContinuationSlackPx,
      );
      const currentLineWhitespaceContinuationGuardPx =
        lineHasContent && cursor === null && codeGroupId != null && item.startsAfterCodeWhitespace
          ? INLINE_CODE_WHITESPACE_CONTINUATION_GUARD_PX
          : 0;
      const startCursor: LayoutCursor = cursor ?? LINE_START_CURSOR;

      const preferredStartWidth = preferredCodeGroupStartWidths.get(itemIndex) ?? 0;
      const dottedPathClusterWidth = dottedPathCodeGroupStartClusterWidths.get(itemIndex) ?? 0;
      const wholeCodeGroupWidth = wholeCodeGroupInlineWidths.get(itemIndex) ?? 0;
      const currentLineCodeFit =
        lineHasContent && cursor === null && codeGroupId != null && item.isFirstCodeGroupFragment
          ? measureCodeGroupFitWithinWidth(
              itemIndex,
              Math.max(1, remainingWidth - pendingSpaceWidth + currentLineFitSlackPx),
              false,
            )
          : null;
      const codeFitEndsAtFriendlyBoundary =
        currentLineCodeFit != null && codeGroupFitEndsAtFriendlyBoundary(currentLineCodeFit);
      const freshLineCodeFit =
        lineHasContent && cursor === null && codeGroupId != null && item.isFirstCodeGroupFragment
          ? measureCodeGroupFitWithinWidth(itemIndex, maxWidth, true)
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
          (item.prefersFreshLineStart || item.codeGroupHasDottedPath) &&
          remainingWidth < reservedWidth + item.minStartTextWidth - 0.01
        ) {
          cursor = null;
          break;
        }
      }

      if (cursor === null && codeGroupId != null) {
        const fullWidth = reservedWidth + item.fullWidth;
        const guardedRemainingWidth =
          remainingWidth - currentLineWhitespaceContinuationGuardPx + currentLineFitSlackPx;
        const attachedTrailingPlainWidth = measureAttachedTrailingPlainWidth(itemIndex);
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
          item.startsAfterCodeWhitespace &&
          fullWidth + attachedTrailingPlainWidth > guardedRemainingWidth + 0.01
        ) {
          cursor = null;
          break;
        }
        if (item.isSealedInlineCodeFragment) {
          if (lineHasContent && fullWidth > guardedRemainingWidth + 0.01) {
            const sealedContinuationLine =
              codeGroupId != null && chargedCodeGroups.has(codeGroupId)
                ? layoutNextLine(
                    item.prepared,
                    LINE_START_CURSOR,
                    Math.max(1, remainingWidth - reservedWidth - currentLineWhitespaceContinuationGuardPx),
                  )
                : null;
            const sealedContinuationFitRatio =
              sealedContinuationLine != null && item.fullWidth > 0
                ? sealedContinuationLine.width / item.fullWidth
                : 0;
            if (
              sealedContinuationLine != null &&
              !cursorsMatch(LINE_START_CURSOR, sealedContinuationLine.end) &&
              sealedContinuationFitRatio >= INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD
            ) {
              remainingWidth = Math.max(0, remainingWidth - reservedWidth - sealedContinuationLine.width);
              if (!lineHasContent) {
                lineOnlyCodeGroupId = codeGroupId;
                lineLastCodeFragmentEndedWithHyphen = false;
                lineLastCodeFragmentEndedWithPathDelimiter = false;
                lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
              } else if (lineOnlyCodeGroupId !== codeGroupId) {
                lineOnlyCodeGroupId = null;
              }
              lineLastCodeFragmentEndedWithHyphen = false;
              lineLastCodeFragmentEndedWithPathDelimiter = false;
              lastAcceptedCodeGroupId = codeGroupId;
              lineHasContent = true;
              chargedCodeGroups.add(codeGroupId);
              pendingSpaceWidth = 0;
              if (debugInlineCode) {
                const segmentText = segmentGraphemes(item.text)
                  .slice(0, sealedContinuationLine.end.graphemeIndex)
                  .join("");
                debugLine += segmentText;
              }
              cursor = sealedContinuationLine.end;
              break;
            }
            cursor = null;
            break;
          }
          const overflowed = fullWidth > guardedRemainingWidth + 0.01;
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
        if (fullWidth <= guardedRemainingWidth) {
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
          ((!item.codePartStartsAfterWhitespace && item.text.endsWith("-")) ||
            ((!item.codePartStartsAfterWhitespace &&
              item.isSealedInlineCodeFragment &&
              (item.text.includes("/") || item.text.includes("\\")))))
        ) {
          cursor = null;
          break;
        }
      }

      if (lineHasContent && remainingWidth < reservedWidth - 0.01) {
        cursor = null;
        break;
      }

      const availableWidth = Math.max(
        1,
        remainingWidth - reservedWidth - currentLineWhitespaceContinuationGuardPx,
      );
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
      if (allowWholeSegmentFastPath && item.fullWidth <= availableWidth + 0.01) {
        remainingWidth = Math.max(0, remainingWidth - reservedWidth - item.fullWidth);
        lineOnlyCodeGroupId = null;
        lastAcceptedCodeGroupId = null;
        lineLastCodeFragmentEndedWithHyphen = false;
        lineLastCodeFragmentEndedWithPathDelimiter = false;
        lineHasContent = true;
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

    totalHeight += params.typography.lineHeight;
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
