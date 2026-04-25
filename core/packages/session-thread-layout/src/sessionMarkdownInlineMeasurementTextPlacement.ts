import { layoutNextLine, type LayoutCursor, type LayoutLine } from "@chenglou/pretext";
import { browserAllowsInlineCodeLeadingHang } from "./sessionMarkdownBrowserProfile";
import { shouldApplyInlineCodeSoftBreakTextStartGuard } from "./sessionMarkdownInlineCodeFit";
import type { PreparedInlineLayoutItem } from "./sessionMarkdownInlineLayout";
import type { InlineMeasurementLineState } from "./sessionMarkdownInlineMeasurementState";
import type { InlineTextPlacementDebug, InlineTextPlacementResult } from "./sessionMarkdownInlineMeasurementTextPlacementTypes";
export type { InlineTextPlacementResult } from "./sessionMarkdownInlineMeasurementTextPlacementTypes";
import {
  STYLED_TEXT_BODY_START_CURRENT_LINE_RATIO_THRESHOLD,
  advancePreparedCursorOneGrapheme,
  isAtomicNonCodeTextSegment,
  isPunctuationOnlySeamText,
  isWhitespaceOnlyLineSlice,
  resolveInlineCodeSoftBreakTextStartGuardPx,
  slicePreparedTextBetweenCursors,
} from "./sessionMarkdownInlineMeasurementContext";
import {
  LINE_START_CURSOR,
  cursorsMatch,
} from "./sessionMarkdownMeasurementCore";

import {
  backtrackBreakWordTokenContinuation,
  measureBreakWordLine,
  resolveInlineCodeTailWholeSegmentFitAllowancePx,
  shouldBreakBeforePunctuationOnlyContinuationTail,
  shouldEmergencyBreakAtomicTextSegment,
} from "./sessionMarkdownInlineMeasurementTextPlacementHelpers";

export {
  backtrackBreakWordTokenContinuation,
  measureBreakWordLine,
  resolveInlineCodeTailWholeSegmentFitAllowancePx,
  shouldBreakBeforePunctuationOnlyContinuationTail,
  shouldEmergencyBreakAtomicTextSegment,
} from "./sessionMarkdownInlineMeasurementTextPlacementHelpers";

const INLINE_CODE_TAIL_WHOLE_SEGMENT_FIT_TOLERANCE_PX = 0;

export function placeInlineTextSegment(params: {
  item: Extract<PreparedInlineLayoutItem, { kind: "segment" }>;
  state: InlineMeasurementLineState;
  startCursor: LayoutCursor;
  sliceStartsAtItemStart: boolean;
  reservedWidth: number;
  guardedRemainingWidth: number;
  currentLineWhitespaceContinuationGuardPx: number;
  currentLineStyledSeamGuardPx: number;
  currentLineStyledAfterInlineCodeClusterGuardPx: number;
  currentLineStyledBodyStartGuardPx: number;
  currentLineInlineCodeTailTextSeamGuardPx: number;
  currentLineInlineCodeSoftBreakTextStartGuardPx: number;
  currentLineFitSlackPx: number;
  currentLineNearFitLeadingHangPx: number;
  canDropLeadingCollapsedSpaceAtWrap: boolean;
  maxWidth: number;
  debug: InlineTextPlacementDebug;
}): InlineTextPlacementResult {
  const state = { ...params.state };
  const { item } = params;
  const codeGroupId = item.codeGroupId;
  const allowsEmergencyBreakWord = shouldEmergencyBreakAtomicTextSegment({
    item,
    maxWidth: params.maxWidth,
  });
  const allowsBreakWord =
    item.allowsBreakWord || allowsEmergencyBreakWord;

  if (
    state.lineHasContent &&
    state.remainingWidth < params.reservedWidth - 0.01 &&
    !params.canDropLeadingCollapsedSpaceAtWrap
  ) {
    state.cursor = null;
    return { action: "break", state };
  }

  const availableWidth = Math.max(
    1,
    state.remainingWidth -
      params.reservedWidth -
      params.currentLineWhitespaceContinuationGuardPx -
      params.currentLineStyledSeamGuardPx -
      params.currentLineStyledAfterInlineCodeClusterGuardPx -
      params.currentLineStyledBodyStartGuardPx -
      params.currentLineInlineCodeTailTextSeamGuardPx -
      params.currentLineInlineCodeSoftBreakTextStartGuardPx,
  );
  const availableWidthWithoutLeadingSpace = params.canDropLeadingCollapsedSpaceAtWrap
    ? Math.max(
        1,
        state.remainingWidth -
          params.currentLineWhitespaceContinuationGuardPx -
          params.currentLineStyledSeamGuardPx -
          params.currentLineStyledAfterInlineCodeClusterGuardPx -
          params.currentLineStyledBodyStartGuardPx -
          params.currentLineInlineCodeTailTextSeamGuardPx -
          params.currentLineInlineCodeSoftBreakTextStartGuardPx,
      )
    : availableWidth;
  const inlineCodeTailWholeSegmentFitAllowancePx = resolveInlineCodeTailWholeSegmentFitAllowancePx({
    lineStartedWithContinuedCode: state.lineStartedWithContinuedCode,
    seamGuardPx: params.currentLineInlineCodeTailTextSeamGuardPx,
  });
  const wholeSegmentAvailableWidth = Math.max(
    1,
    availableWidth + inlineCodeTailWholeSegmentFitAllowancePx,
  );
  const wholeSegmentFitsCurrentLine =
    item.fullWidth <= wholeSegmentAvailableWidth + INLINE_CODE_TAIL_WHOLE_SEGMENT_FIT_TOLERANCE_PX + 0.01;
  const codeSegmentAvailableWidth =
    codeGroupId != null && !item.isSealedInlineCodeFragment
      ? Math.max(1, availableWidth + params.currentLineFitSlackPx + params.currentLineNearFitLeadingHangPx)
      : availableWidth;

  if (
    params.debug.enabled &&
    codeGroupId != null &&
    !item.isSealedInlineCodeFragment &&
    state.cursor === null &&
    state.lineHasContent &&
    state.lastAcceptedCodeGroupId === codeGroupId
  ) {
    params.debug.continuationDecisions.push({
      text: item.text,
      reservedWidth: params.reservedWidth,
      remainingWidth: state.remainingWidth,
      guardedRemainingWidth: params.guardedRemainingWidth,
      availableLineWidth: codeSegmentAvailableWidth,
      fullWidth: item.fullWidth,
      currentLineFitSlackPx: params.currentLineFitSlackPx,
      lineLastCodeFragmentText: state.lineLastCodeFragmentText,
      lineLastCodeFragmentEndedWithPathDelimiter: state.lineLastCodeFragmentEndedWithPathDelimiter,
      lineLastCodeFragmentEndedWithHyphen: state.lineLastCodeFragmentEndedWithHyphen,
    });
  }

  const styledStartLine: LayoutLine | null =
    codeGroupId == null &&
    state.lineHasContent &&
    state.cursor === null &&
    item.startsStyledTextAfterBodySeam &&
    item.fullWidth > availableWidth + 0.01
      ? layoutNextLine(item.prepared, params.startCursor, availableWidth)
      : null;

  if (
    codeGroupId == null &&
    state.lineHasContent &&
    state.cursor === null &&
    allowsEmergencyBreakWord
  ) {
    state.cursor = null;
    return { action: "break", state };
  }
  if (
    codeGroupId == null &&
    state.lineHasContent &&
    state.cursor === null &&
    !allowsBreakWord &&
    item.startsAfterInlineCodeSeam &&
    !state.lineStartedWithContinuedCode &&
    item.minStartTextWidth > availableWidth + 0.01 &&
    item.minStartTextWidth <= params.maxWidth + 0.01
  ) {
    state.cursor = null;
    return { action: "break", state };
  }
  if (
    codeGroupId == null &&
    state.lineHasContent &&
    state.cursor === null &&
    !allowsBreakWord &&
    item.startsAfterStyledTextSeam &&
    item.minStartTextWidth > availableWidth + 0.01 &&
    item.minStartTextWidth <= params.maxWidth + 0.01
  ) {
    state.cursor = null;
    return { action: "break", state };
  }
  if (
    codeGroupId == null &&
    state.lineHasContent &&
    state.cursor === null &&
    state.pendingSpaceWidth > 0 &&
    !allowsBreakWord &&
    item.minStartTextWidth > availableWidthWithoutLeadingSpace + 0.01 &&
    item.minStartTextWidth <= params.maxWidth + 0.01
  ) {
    state.cursor = null;
    return { action: "break", state };
  }
  if (
    codeGroupId == null &&
    state.lineHasContent &&
    state.cursor === null &&
    !allowsBreakWord &&
    item.startsStyledTextAfterInlineCodeSeam &&
    item.hasTrailingInlineCode &&
    item.minStartTextWidth > availableWidth + 0.01 &&
    item.minStartTextWidth <= params.maxWidth + 0.01
  ) {
    state.cursor = null;
    return { action: "break", state };
  }
  if (
    codeGroupId == null &&
    state.lineHasContent &&
    state.cursor === null &&
    item.startsStyledTextAfterBodySeam &&
    item.fullWidth > availableWidth + 0.01 &&
    (styledStartLine == null ||
      cursorsMatch(params.startCursor, styledStartLine.end) ||
      styledStartLine.width / Math.max(1, item.fullWidth) <
        STYLED_TEXT_BODY_START_CURRENT_LINE_RATIO_THRESHOLD)
  ) {
    state.cursor = null;
    return { action: "break", state };
  }

  const allowWholeSegmentAfterContinuedInlineCode =
    browserAllowsInlineCodeLeadingHang() &&
    state.lineStartedWithContinuedCode &&
    item.startsAfterPathLikeInlineCodeSeam &&
    !item.startsAfterCollapsedSoftBreak &&
    !item.startsStyledTextAfterInlineCodeSeam &&
    wholeSegmentFitsCurrentLine;
  const allowWholeSegmentFastPath =
    codeGroupId == null &&
    state.cursor === null &&
    !item.startsAfterStyledTextSeam &&
    (!item.startsAfterInlineCodeSeam ||
      ((!state.lineStartedWithContinuedCode || allowWholeSegmentAfterContinuedInlineCode) &&
        !item.startsAfterCollapsedSoftBreak &&
        !item.startsStyledTextAfterInlineCodeSeam &&
        wholeSegmentFitsCurrentLine));

  if (
    params.debug.enabled &&
    codeGroupId == null &&
    state.cursor === null &&
    item.startsAfterInlineCodeSeam
  ) {
    params.debug.segmentSeamAdjustments.push({
      type: "inline-code-whole-fit",
      lineHasContent: state.lineHasContent,
      text: item.text,
      reservedWidth: params.reservedWidth,
      availableWidth,
      wholeSegmentAvailableWidth,
      inlineCodeTailWholeSegmentFitAllowancePx,
      fullWidth: item.fullWidth,
      lineStartedWithContinuedCode: state.lineStartedWithContinuedCode,
      startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
      startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
      allowed: allowWholeSegmentFastPath,
    });
  }

  if (allowWholeSegmentFastPath && wholeSegmentFitsCurrentLine) {
    const lineWasEmpty = !state.lineHasContent;
    state.remainingWidth = Math.max(0, state.remainingWidth - params.reservedWidth - item.fullWidth);
    state.lineOnlyCodeGroupId = null;
    state.lastAcceptedCodeGroupId = null;
    state.lineCurrentCodeGroupStartFragmentText = null;
    state.lineCurrentCodeGroupStartedNearFresh = false;
    state.lineCurrentCodeGroupLimitToFirstFragment = false;
    state.lineLastCodeFragmentText = null;
    state.lineLastCodeFragmentEndedWithHyphen = false;
    state.lineLastCodeFragmentEndedWithPathDelimiter = false;
    state.lineHasContent = true;
    if (item.isDecoratedText) {
      state.lineDecoratedTextSegmentCount += 1;
    }
    state.lineHasSoftHyphenText ||= item.text.includes("\u00ad");
    state.lineStartedWithCollapsedSoftBreakPlainText ||=
      lineWasEmpty && item.startsAfterCollapsedSoftBreak;
    state.lineTailAfterInlineCodeIsPunctuationOnly =
      item.startsAfterInlineCodeSeam && isPunctuationOnlySeamText(item.text);
    state.lineAcceptedPlainAfterContinuedCode ||= state.lineStartedWithContinuedCode;
    state.lineAcceptedSoftBreakProseAfterInlineCode ||=
      params.sliceStartsAtItemStart &&
      shouldApplyInlineCodeSoftBreakTextStartGuard({
        text: item.text,
        startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
        startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
        startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
        startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
        lastFragmentEndedWithPathDelimiter: state.lineLastCodeFragmentEndedWithPathDelimiter,
        lastFragmentEndedWithHyphen: state.lineLastCodeFragmentEndedWithHyphen,
      });
    if (
      params.sliceStartsAtItemStart &&
      shouldApplyInlineCodeSoftBreakTextStartGuard({
        text: item.text,
        startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
        startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
        startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
        startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
        lastFragmentEndedWithPathDelimiter: state.lineLastCodeFragmentEndedWithPathDelimiter,
        lastFragmentEndedWithHyphen: state.lineLastCodeFragmentEndedWithHyphen,
      })
    ) {
      state.lineSoftBreakProseAfterInlineCodeGuardPx = Math.max(
        state.lineSoftBreakProseAfterInlineCodeGuardPx,
        resolveInlineCodeSoftBreakTextStartGuardPx(item.minStartTextWidth),
      );
    }
    state.pendingSpaceWidth = 0;
    if (params.debug.enabled) {
      params.debug.appendLineText(item.text);
    }
    state.itemIndex += 1;
    return { action: "continue", state };
  }

  const regularLineWithReservedSpace: LayoutLine | null =
    styledStartLine ?? layoutNextLine(item.prepared, params.startCursor, codeSegmentAvailableWidth);
  const regularLineWithoutLeadingSpace: LayoutLine | null =
    params.canDropLeadingCollapsedSpaceAtWrap && availableWidthWithoutLeadingSpace > availableWidth + 0.01
      ? layoutNextLine(item.prepared, params.startCursor, availableWidthWithoutLeadingSpace)
      : null;
  const lineWithReservedSpace: LayoutLine | null =
    allowsBreakWord &&
    (allowsEmergencyBreakWord ||
      regularLineWithReservedSpace == null ||
      cursorsMatch(params.startCursor, regularLineWithReservedSpace.end))
      ? measureBreakWordLine({
          item,
          startCursor: params.startCursor,
          availableWidth: codeSegmentAvailableWidth,
        })
      : regularLineWithReservedSpace;
  const lineWithoutLeadingSpace: LayoutLine | null =
    allowsBreakWord &&
    params.canDropLeadingCollapsedSpaceAtWrap &&
    availableWidthWithoutLeadingSpace > availableWidth + 0.01 &&
    (allowsEmergencyBreakWord ||
      regularLineWithoutLeadingSpace == null ||
      cursorsMatch(params.startCursor, regularLineWithoutLeadingSpace.end))
      ? measureBreakWordLine({
          item,
          startCursor: params.startCursor,
          availableWidth: availableWidthWithoutLeadingSpace,
        })
      : regularLineWithoutLeadingSpace;
  const useLineWithoutLeadingSpace = false;
  const rawLine: LayoutLine | null = lineWithReservedSpace;
  const line: LayoutLine | null =
    rawLine == null || cursorsMatch(params.startCursor, rawLine.end)
      ? rawLine
      : backtrackBreakWordTokenContinuation({
          item,
          startCursor: params.startCursor,
          line: rawLine,
          lineHasContent: state.lineHasContent,
          lineTailAfterInlineCodeIsPunctuationOnly: state.lineTailAfterInlineCodeIsPunctuationOnly,
        });

  if (
    params.debug.enabled &&
    codeGroupId != null &&
    !item.isSealedInlineCodeFragment &&
    state.cursor === null &&
    state.lineHasContent &&
    state.lastAcceptedCodeGroupId === codeGroupId
  ) {
    params.debug.continuationDecisions.push({
      text: item.text,
      reservedWidth: params.reservedWidth,
      remainingWidth: state.remainingWidth,
      guardedRemainingWidth: params.guardedRemainingWidth,
      availableLineWidth: codeSegmentAvailableWidth,
      lineWidth: line?.width,
      fullWidth: item.fullWidth,
      currentLineFitSlackPx: params.currentLineFitSlackPx,
      lineLastCodeFragmentText: state.lineLastCodeFragmentText,
      lineLastCodeFragmentEndedWithPathDelimiter: state.lineLastCodeFragmentEndedWithPathDelimiter,
      lineLastCodeFragmentEndedWithHyphen: state.lineLastCodeFragmentEndedWithHyphen,
      acceptedWholeFragment: line != null && cursorsMatch(line.end, item.endCursor),
      brokeBeforeFragment: line == null || cursorsMatch(params.startCursor, line.end),
    });
  }

  if (line == null || cursorsMatch(params.startCursor, line.end)) {
    if (!state.lineHasContent) {
      if (codeGroupId == null && item.startsAfterCollapsedSoftBreak) {
        const advancedCursor = advancePreparedCursorOneGrapheme(item.prepared, params.startCursor);
        if (advancedCursor != null) {
          if (params.debug.enabled) {
            params.debug.segmentSeamAdjustments.push({
              type: "no-progress-advance",
              lineHasContent: state.lineHasContent,
              text: item.text,
            });
          }
          state.cursor = advancedCursor;
          return { action: "continue", state };
        }
      }
      if (params.debug.enabled) {
        params.debug.segmentSeamAdjustments.push({
          type: "no-progress-drop",
          lineHasContent: state.lineHasContent,
          text: item.text,
        });
      }
      state.itemIndex += 1;
    }
    state.cursor = null;
    return { action: "break", state };
  }

  const lineSegmentText = slicePreparedTextBetweenCursors(item.prepared, params.startCursor, line.end);
  if (
    shouldBreakBeforePunctuationOnlyContinuationTail({
      codeGroupId,
      startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
      lineHasContent: state.lineHasContent,
      atItemStart: state.cursor === null,
      lineStartedWithContinuedCode: state.lineStartedWithContinuedCode,
      lineEndsAtItemEnd: cursorsMatch(line.end, item.endCursor),
      lineSegmentText,
      segmentFullWidth: item.fullWidth,
      availableWidth,
    })
  ) {
    state.cursor = null;
    return { action: "break", state };
  }
  if (
    codeGroupId == null &&
    item.startsAfterCollapsedSoftBreak &&
    state.lineHasContent &&
    !cursorsMatch(line.end, item.endCursor) &&
    isWhitespaceOnlyLineSlice(lineSegmentText)
  ) {
    if (params.debug.enabled) {
      params.debug.segmentSeamAdjustments.push({
        type: "whitespace-only-break",
        lineHasContent: state.lineHasContent,
        text: item.text,
      });
    }
    state.cursor = null;
    return { action: "break", state };
  }
  if (
    codeGroupId == null &&
    item.startsAfterCollapsedSoftBreak &&
    !state.lineHasContent &&
    !cursorsMatch(line.end, item.endCursor) &&
    isWhitespaceOnlyLineSlice(lineSegmentText)
  ) {
    if (params.debug.enabled) {
      params.debug.segmentSeamAdjustments.push({
        type: "whitespace-only-advance",
        lineHasContent: state.lineHasContent,
        text: item.text,
      });
    }
    state.cursor = line.end;
    return { action: "continue", state };
  }
  if (
    codeGroupId == null &&
    state.lineHasContent &&
    state.cursor === null &&
    !cursorsMatch(line.end, item.endCursor) &&
    isAtomicNonCodeTextSegment(item.text) &&
    (item.fullWidth <= params.maxWidth + 0.01 ||
      (allowsBreakWord &&
        state.lineTailAfterInlineCodeIsPunctuationOnly &&
        state.pendingSpaceWidth > 0))
  ) {
    state.cursor = null;
    return { action: "break", state };
  }

  const consumedReservedWidth = params.reservedWidth;
  const lineWasEmpty = !state.lineHasContent;
  state.remainingWidth = Math.max(0, state.remainingWidth - consumedReservedWidth - line.width);
  if (!state.lineHasContent) {
    state.lineOnlyCodeGroupId = codeGroupId;
    state.lineLastCodeFragmentEndedWithHyphen =
      codeGroupId != null && cursorsMatch(line.end, item.endCursor) && item.text.endsWith("-");
    state.lineLastCodeFragmentEndedWithPathDelimiter =
      codeGroupId != null && cursorsMatch(line.end, item.endCursor) && /[\\/]+$/.test(item.text);
    state.lineStartedWithContinuedCode =
      codeGroupId != null &&
      (!item.isFirstCodeGroupFragment || !cursorsMatch(params.startCursor, LINE_START_CURSOR));
    state.lineSawInlineCode = codeGroupId != null;
  } else if (state.lineOnlyCodeGroupId !== codeGroupId) {
    state.lineOnlyCodeGroupId = null;
  }
  state.lineLastCodeFragmentEndedWithHyphen =
    codeGroupId != null && cursorsMatch(line.end, item.endCursor) && item.text.endsWith("-");
  state.lineLastCodeFragmentEndedWithPathDelimiter =
    codeGroupId != null && cursorsMatch(line.end, item.endCursor) && /[\\/]+$/.test(item.text);
  if (codeGroupId != null && state.lastAcceptedCodeGroupId !== codeGroupId) {
    state.lineCurrentCodeGroupStartFragmentText = lineSegmentText;
  }
  state.lineLastCodeFragmentText = codeGroupId != null ? lineSegmentText : null;
  state.lastAcceptedCodeGroupId = codeGroupId;
  state.lineHasContent = true;

  if (codeGroupId != null) {
    state.chargedCodeGroups.add(codeGroupId);
    if (state.lineAcceptedSoftBreakProseAfterInlineCode && state.lineSoftBreakProseGuardCodeGroupId == null) {
      state.lineSoftBreakProseGuardCodeGroupId = codeGroupId;
    }
    state.lineSawInlineCode = true;
    state.lineTailAfterInlineCodeIsPunctuationOnly = false;
  } else {
    state.lastAcceptedCodeGroupId = null;
    state.lineLastCodeFragmentEndedWithHyphen = false;
    state.lineLastCodeFragmentEndedWithPathDelimiter = false;
    state.lineStartedWithCollapsedSoftBreakPlainText ||=
      lineWasEmpty && item.startsAfterCollapsedSoftBreak;
    if (item.isDecoratedText) {
      state.lineDecoratedTextSegmentCount += 1;
    }
    state.lineHasSoftHyphenText ||= item.text.includes("\u00ad");
    state.lineTailAfterInlineCodeIsPunctuationOnly =
      item.startsAfterInlineCodeSeam && isPunctuationOnlySeamText(item.text);
    state.lineAcceptedPlainAfterContinuedCode ||= state.lineStartedWithContinuedCode;
    state.lineAcceptedSoftBreakProseAfterInlineCode ||=
      params.sliceStartsAtItemStart &&
      shouldApplyInlineCodeSoftBreakTextStartGuard({
        text: item.text,
        startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
        startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
        startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
        startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
        lastFragmentEndedWithPathDelimiter: state.lineLastCodeFragmentEndedWithPathDelimiter,
        lastFragmentEndedWithHyphen: state.lineLastCodeFragmentEndedWithHyphen,
      });
    if (
      params.sliceStartsAtItemStart &&
      shouldApplyInlineCodeSoftBreakTextStartGuard({
        text: item.text,
        startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
        startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
        startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
        startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
        lastFragmentEndedWithPathDelimiter: state.lineLastCodeFragmentEndedWithPathDelimiter,
        lastFragmentEndedWithHyphen: state.lineLastCodeFragmentEndedWithHyphen,
      })
    ) {
      state.lineSoftBreakProseAfterInlineCodeGuardPx = Math.max(
        state.lineSoftBreakProseAfterInlineCodeGuardPx,
        resolveInlineCodeSoftBreakTextStartGuardPx(item.minStartTextWidth),
      );
    }
  }

  state.pendingSpaceWidth = 0;
  if (params.debug.enabled) {
    params.debug.appendLineText(lineSegmentText);
  }

  if (cursorsMatch(line.end, item.endCursor)) {
    state.itemIndex += 1;
    state.cursor = null;
    return { action: "continue", state };
  }

  state.cursor = line.end;
  return { action: "break", state };
}
