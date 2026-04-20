import { layoutNextLine, type LayoutLine } from "@chenglou/pretext";
import { browserAllowsInlineCodeLeadingHang } from "./sessionMarkdownBrowserProfile";
import {
  isShortExtensionPathLikeFragment,
  resolveInlineCodeWhitespaceSeparatedFragmentSlackPx,
  shouldBreakBeforePartialDottedStemPathTailContinuation,
  shouldBreakBeforePartialSealedDottedPathContinuation,
  shouldBreakBeforeWhitespaceSeparatedInlineCodeFragment,
} from "./sessionMarkdownInlineCodeFit";
import type { PreparedInlineLayoutItem } from "./sessionMarkdownInlineLayout";
import type {
  SessionMarkdownInlineCodeContinuationDecision,
  SessionMarkdownInlineCodeSealedContinuationDecision,
  SessionMarkdownInlineCodeWhitespaceDecision,
} from "./sessionMarkdownInlineMeasurementDebug";
import type { InlineMeasurementLineState } from "./sessionMarkdownInlineMeasurementState";
import { slicePreparedTextBetweenCursors } from "./sessionMarkdownInlineMeasurementContext";
import { LINE_START_CURSOR, cursorsMatch } from "./sessionMarkdownMeasurementCore";

const INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD = 0.35;
const INLINE_CODE_HYPHEN_CONTINUATION_GUARD_PX = 2;

type InlineCodePlacementDebug = {
  enabled: boolean;
  whitespaceDecisions: SessionMarkdownInlineCodeWhitespaceDecision[];
  continuationDecisions: SessionMarkdownInlineCodeContinuationDecision[];
  sealedContinuationDecisions: SessionMarkdownInlineCodeSealedContinuationDecision[];
  appendLineText: (text: string) => void;
};

export type InlineCodePlacementResult = {
  action: "continue" | "break" | "pass";
  state: InlineMeasurementLineState;
  forcedFreshWholeCodeGroupIndex: number | null;
};

function acceptCodeFragmentState(params: {
  state: InlineMeasurementLineState;
  item: Extract<PreparedInlineLayoutItem, { kind: "segment" }>;
  codeGroupId: number;
  remainingWidthBeforeAccept: number;
  remainingWidthAfterAccept: number;
  lastFragmentText: string;
  lastFragmentEndedWithHyphen: boolean;
  lastFragmentEndedWithPathDelimiter: boolean;
  shouldLimitCurrentCodeGroupToFirstFragment: boolean;
  shouldTrackWeakProseStartCodeGroup: boolean;
  shouldForceSoftBreakWeakProseContinuationWrap: boolean;
  maxWidth: number;
}): void {
  const {
    state,
    item,
    codeGroupId,
    remainingWidthBeforeAccept,
    remainingWidthAfterAccept,
    lastFragmentText,
    lastFragmentEndedWithHyphen,
    lastFragmentEndedWithPathDelimiter,
    shouldLimitCurrentCodeGroupToFirstFragment,
    shouldTrackWeakProseStartCodeGroup,
    shouldForceSoftBreakWeakProseContinuationWrap,
    maxWidth,
  } = params;
  const lineHasContentBeforeAccept = state.lineHasContent;

  state.remainingWidth = remainingWidthAfterAccept;
  if (!lineHasContentBeforeAccept) {
    state.lineOnlyCodeGroupId = codeGroupId;
    state.lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
  } else if (state.lineOnlyCodeGroupId !== codeGroupId) {
    state.lineOnlyCodeGroupId = null;
  }
  if (state.lastAcceptedCodeGroupId !== codeGroupId) {
    state.lineCurrentCodeGroupStartFragmentText = item.text;
    state.lineCurrentCodeGroupStartedNearFresh =
      lineHasContentBeforeAccept && remainingWidthBeforeAccept > maxWidth * 0.85;
    state.lineCurrentCodeGroupLimitToFirstFragment = shouldLimitCurrentCodeGroupToFirstFragment;
  }
  state.lineLastCodeFragmentText = lastFragmentText;
  state.lineLastCodeFragmentEndedWithHyphen = lastFragmentEndedWithHyphen;
  state.lineLastCodeFragmentEndedWithPathDelimiter = lastFragmentEndedWithPathDelimiter;
  state.lastAcceptedCodeGroupId = codeGroupId;
  state.lineTailAfterInlineCodeIsPunctuationOnly = false;
  state.lineHasContent = true;
  state.chargedCodeGroups.add(codeGroupId);
  if (state.lineAcceptedSoftBreakProseAfterInlineCode && state.lineSoftBreakProseGuardCodeGroupId == null) {
    state.lineSoftBreakProseGuardCodeGroupId = codeGroupId;
  }
  if (shouldTrackWeakProseStartCodeGroup) {
    state.lineWeakProseStartCodeGroupId = codeGroupId;
    if (shouldForceSoftBreakWeakProseContinuationWrap) {
      state.lineForceSoftBreakWeakProseContinuationCodeGroupId = codeGroupId;
    }
  }
}

export function placeInlineCodeSegment(params: {
  items: readonly PreparedInlineLayoutItem[];
  item: Extract<PreparedInlineLayoutItem, { kind: "segment" }>;
  state: InlineMeasurementLineState;
  codeGroupId: number;
  maxWidth: number;
  reservedWidth: number;
  wholeCodeGroupWidth: number;
  forcedFreshWholeCodeGroupIndex: number | null;
  currentLineCodeStartSeamGuardPx: number;
  currentLineFitSlackPx: number;
  currentLineWhitespaceContinuationGuardPx: number;
  currentLineWeakProseStartContinuationGuardPx: number;
  currentLineNearFitLeadingHangPx: number;
  shouldLimitCurrentCodeGroupToFirstFragment: boolean;
  shouldTrackWeakProseStartCodeGroup: boolean;
  shouldForceSoftBreakWeakProseContinuationWrap: boolean;
  debug: InlineCodePlacementDebug;
}): InlineCodePlacementResult {
  const state = { ...params.state };
  const { item, codeGroupId } = params;

  if (
    params.forcedFreshWholeCodeGroupIndex === state.itemIndex &&
    item.isFirstCodeGroupFragment &&
    params.wholeCodeGroupWidth > 0 &&
    params.wholeCodeGroupWidth <= params.maxWidth + 0.01
  ) {
    let scanIndex = state.itemIndex;
    let lastCodeFragmentText = item.text;
    while (scanIndex < params.items.length) {
      const candidate = params.items[scanIndex]!;
      if (candidate.kind === "hardBreak") {
        break;
      }
      if (candidate.kind === "space") {
        if (candidate.codeGroupId !== codeGroupId) {
          break;
        }
        if (params.debug.enabled) {
          params.debug.appendLineText(candidate.text);
        }
        scanIndex += 1;
        continue;
      }
      if (candidate.codeGroupId !== codeGroupId) {
        break;
      }
      if (params.debug.enabled) {
        params.debug.appendLineText(candidate.text);
      }
      lastCodeFragmentText = candidate.text;
      scanIndex += 1;
    }
    const remainingWidthBeforeWholeGroupAccept = state.remainingWidth;
    acceptCodeFragmentState({
      state,
      item,
      codeGroupId,
      remainingWidthBeforeAccept: remainingWidthBeforeWholeGroupAccept,
      remainingWidthAfterAccept: Math.max(0, state.remainingWidth - params.wholeCodeGroupWidth),
      lastFragmentText: lastCodeFragmentText,
      lastFragmentEndedWithHyphen: lastCodeFragmentText.endsWith("-"),
      lastFragmentEndedWithPathDelimiter: /[\\/]+$/.test(lastCodeFragmentText),
      shouldLimitCurrentCodeGroupToFirstFragment: params.shouldLimitCurrentCodeGroupToFirstFragment,
      shouldTrackWeakProseStartCodeGroup: false,
      shouldForceSoftBreakWeakProseContinuationWrap: false,
      maxWidth: params.maxWidth,
    });
    state.itemIndex = scanIndex;
    state.pendingSpaceWidth = 0;
    return {
      action: "continue",
      state,
      forcedFreshWholeCodeGroupIndex: null,
    };
  }

  const fullWidth = params.reservedWidth + item.fullWidth;
  const guardedRemainingWidth =
    state.remainingWidth -
    params.currentLineCodeStartSeamGuardPx -
    params.currentLineWhitespaceContinuationGuardPx -
    params.currentLineWeakProseStartContinuationGuardPx +
    params.currentLineFitSlackPx +
    params.currentLineNearFitLeadingHangPx;
  const hyphenContinuationGuardPx =
    !browserAllowsInlineCodeLeadingHang() &&
    state.lineLastCodeFragmentEndedWithHyphen &&
    !item.text.includes("/") &&
    !item.text.includes("\\") &&
    !item.isPathTailFragment
      ? INLINE_CODE_HYPHEN_CONTINUATION_GUARD_PX
      : 0;
  const whitespaceSlackPx = resolveInlineCodeWhitespaceSeparatedFragmentSlackPx({
    lineHasContent: state.lineHasContent,
    startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
    fragmentText: item.text,
  });
  const shouldBreakBeforeWhitespaceSeparatedFragment =
    shouldBreakBeforeWhitespaceSeparatedInlineCodeFragment({
      lineHasContent: state.lineHasContent,
      startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
      reservedWidth: params.reservedWidth,
      remainingWidth: guardedRemainingWidth,
      fragmentWidth: item.fullWidth,
      slackPx: whitespaceSlackPx,
    });
  const wholeFragmentFitTolerancePx = item.startsAfterCodeWhitespace ? whitespaceSlackPx : 0;

  if (params.debug.enabled && item.startsAfterCodeWhitespace) {
    params.debug.whitespaceDecisions.push({
      lineHasContent: state.lineHasContent,
      reservedWidth: params.reservedWidth,
      remainingWidth: state.remainingWidth,
      guardedRemainingWidth,
      fragmentWidth: item.fullWidth,
      slackPx: whitespaceSlackPx,
      shouldBreak: shouldBreakBeforeWhitespaceSeparatedFragment,
      text: item.text,
    });
  }
  if (shouldBreakBeforeWhitespaceSeparatedFragment) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }
  if (
    state.lineHasContent &&
    state.lineCurrentCodeGroupLimitToFirstFragment &&
    state.lastAcceptedCodeGroupId === codeGroupId &&
    state.lineLastCodeFragmentEndedWithPathDelimiter
  ) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }
  if (
    state.lineHasContent &&
    state.lineForceSoftBreakWeakProseContinuationCodeGroupId === codeGroupId &&
    state.lastAcceptedCodeGroupId === codeGroupId
  ) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }

  const sealedBoundary =
    state.lineHasContent &&
    state.lastAcceptedCodeGroupId === codeGroupId &&
    (state.lineLastCodeFragmentEndedWithHyphen || state.lineLastCodeFragmentEndedWithPathDelimiter);
  const boundaryRemainingWidth =
    state.remainingWidth +
    params.currentLineFitSlackPx -
    hyphenContinuationGuardPx -
    params.currentLineWeakProseStartContinuationGuardPx;
  const shouldAcceptChromiumPathTailContinuation =
    browserAllowsInlineCodeLeadingHang() &&
    state.lineHasContent &&
    state.lastAcceptedCodeGroupId === codeGroupId &&
    state.lineLastCodeFragmentEndedWithPathDelimiter &&
    !item.isSealedInlineCodeFragment &&
    !item.startsAfterCodeWhitespace &&
    params.reservedWidth + item.fullWidth <= boundaryRemainingWidth + 0.01;
  const nextSameCodeGroupItem = params.items[state.itemIndex + 1];
  const splitDottedStemTailWouldOverflowCurrentLine =
    state.lineCurrentCodeGroupStartedNearFresh &&
    state.lineHasContent &&
    state.lastAcceptedCodeGroupId === codeGroupId &&
    state.lineLastCodeFragmentEndedWithPathDelimiter &&
    item.codeGroupStartsAfterText &&
    item.codeGroupHasTrailingText &&
    state.lineCurrentCodeGroupStartFragmentText != null &&
    !state.lineCurrentCodeGroupStartFragmentText.includes(".") &&
    item.text.endsWith(".") &&
    nextSameCodeGroupItem?.kind === "segment" &&
    nextSameCodeGroupItem.codeGroupId === codeGroupId &&
    (nextSameCodeGroupItem.text.includes("/") ||
      nextSameCodeGroupItem.text.includes("\\") ||
      nextSameCodeGroupItem.isPathTailFragment) &&
    params.reservedWidth + item.fullWidth + nextSameCodeGroupItem.fullWidth > boundaryRemainingWidth + 0.01;
  if (splitDottedStemTailWouldOverflowCurrentLine) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }

  const sealedBoundaryOverflow =
    sealedBoundary && params.reservedWidth + item.fullWidth > boundaryRemainingWidth + 0.01;
  const shouldBreakBeforePartialSealedDottedPathFragment =
    shouldBreakBeforePartialSealedDottedPathContinuation({
      currentCodeGroupStartFragmentText: state.lineCurrentCodeGroupStartFragmentText,
      fullWidth,
      guardedRemainingWidth,
      item,
      lastFragmentText: state.lineLastCodeFragmentText,
      sameCodeGroupContinuation: state.lineHasContent && state.lastAcceptedCodeGroupId === codeGroupId,
    });
  const shouldBreakBeforePartialDottedStemPathTailFragment =
    shouldBreakBeforePartialDottedStemPathTailContinuation({
      fullWidth,
      guardedRemainingWidth,
      item,
      lastFragmentText: state.lineLastCodeFragmentText,
      sameCodeGroupContinuation: state.lineHasContent && state.lastAcceptedCodeGroupId === codeGroupId,
    });
  const lastFragmentIsShortExtensionPath = isShortExtensionPathLikeFragment(state.lineLastCodeFragmentText);
  const canRelaxChromiumDottedPathBoundary =
    sealedBoundaryOverflow &&
    !state.lineUsedChromiumDottedPathBoundaryContinuation &&
    browserAllowsInlineCodeLeadingHang() &&
    item.codeGroupStartsAfterText &&
    state.lineCurrentCodeGroupStartFragmentText != null &&
    !state.lineCurrentCodeGroupStartFragmentText.includes(".") &&
    !lastFragmentIsShortExtensionPath &&
    item.codeGroupHasDottedPath &&
    !item.text.includes("/") &&
    !item.text.includes("\\") &&
    (item.text.includes(".") || item.isPathTailFragment);
  const effectiveSealedBoundaryOverflow =
    sealedBoundaryOverflow && !shouldAcceptChromiumPathTailContinuation;

  if (shouldBreakBeforePartialDottedStemPathTailFragment) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }

  if (params.debug.enabled && state.lineHasContent && item.isSealedInlineCodeFragment && state.lastAcceptedCodeGroupId === codeGroupId) {
    params.debug.sealedContinuationDecisions.push({
      canRelaxChromiumDottedPathBoundary,
      currentCodeGroupStartFragmentText: state.lineCurrentCodeGroupStartFragmentText,
      fullWidth,
      guardedRemainingWidth,
      remainingWidth: state.remainingWidth,
      continuationSlackPx: params.currentLineFitSlackPx,
      lastFragmentIsShortExtensionPath,
      lastFragmentText: state.lineLastCodeFragmentText,
      sameCodeGroupContinuation: state.lineHasContent && state.lastAcceptedCodeGroupId === codeGroupId,
      sealedBoundaryOverflow: effectiveSealedBoundaryOverflow,
      shouldBreakBeforePartialSealedDottedPathFragment,
      text: item.text,
    });
  }
  if (effectiveSealedBoundaryOverflow && !canRelaxChromiumDottedPathBoundary) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }
  if (shouldBreakBeforePartialSealedDottedPathFragment) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }
  if (canRelaxChromiumDottedPathBoundary) {
    state.lineUsedChromiumDottedPathBoundaryContinuation = true;
  }

  if (shouldAcceptChromiumPathTailContinuation) {
    const remainingWidthBeforeFragmentAccept = state.remainingWidth;
    acceptCodeFragmentState({
      state,
      item,
      codeGroupId,
      remainingWidthBeforeAccept: remainingWidthBeforeFragmentAccept,
      remainingWidthAfterAccept: Math.max(
        0,
        state.remainingWidth - params.reservedWidth - item.fullWidth,
      ),
      lastFragmentText: item.text,
      lastFragmentEndedWithHyphen: item.text.endsWith("-"),
      lastFragmentEndedWithPathDelimiter: /[\\/]+$/.test(item.text),
      shouldLimitCurrentCodeGroupToFirstFragment: params.shouldLimitCurrentCodeGroupToFirstFragment,
      shouldTrackWeakProseStartCodeGroup: params.shouldTrackWeakProseStartCodeGroup,
      shouldForceSoftBreakWeakProseContinuationWrap: params.shouldForceSoftBreakWeakProseContinuationWrap,
      maxWidth: params.maxWidth,
    });
    state.itemIndex += 1;
    state.pendingSpaceWidth = 0;
    if (params.debug.enabled) {
      params.debug.continuationDecisions.push({
        text: item.text,
        reservedWidth: params.reservedWidth,
        remainingWidth: remainingWidthBeforeFragmentAccept,
        guardedRemainingWidth,
        fullWidth,
        currentLineFitSlackPx: params.currentLineFitSlackPx,
        lineLastCodeFragmentText: params.state.lineLastCodeFragmentText,
        lineLastCodeFragmentEndedWithPathDelimiter: params.state.lineLastCodeFragmentEndedWithPathDelimiter,
        lineLastCodeFragmentEndedWithHyphen: params.state.lineLastCodeFragmentEndedWithHyphen,
        acceptedWholeFragment: true,
      });
      params.debug.appendLineText(item.text);
    }
    return {
      action: "continue",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }

  if (item.isSealedInlineCodeFragment) {
    if (state.lineHasContent && fullWidth > guardedRemainingWidth + 0.01) {
      const shouldBreakBeforeEnginePartialSealedPathContinuation =
        !browserAllowsInlineCodeLeadingHang() &&
        (item.text.includes("/") || item.text.includes("\\") || item.isPathTailFragment);
      if (shouldBreakBeforeEnginePartialSealedPathContinuation) {
        state.cursor = null;
        return {
          action: "break",
          state,
          forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
        };
      }
      const sealedContinuationLine: LayoutLine | null =
        state.chargedCodeGroups.has(codeGroupId)
          ? layoutNextLine(
              item.prepared,
              LINE_START_CURSOR,
              Math.max(1, state.remainingWidth - params.reservedWidth - params.currentLineWhitespaceContinuationGuardPx),
            )
          : null;
      const sealedContinuationFitRatio =
        sealedContinuationLine != null && item.fullWidth > 0
          ? sealedContinuationLine.width / item.fullWidth
          : 0;
      const sealedContinuationWouldOverflowCurrentLine =
        sealedContinuationLine != null &&
        !cursorsMatch(LINE_START_CURSOR, sealedContinuationLine.end) &&
        params.reservedWidth + sealedContinuationLine.width > guardedRemainingWidth + 0.01;
      const shouldBreakForEngineSealedOverflow =
        sealedContinuationWouldOverflowCurrentLine &&
        !browserAllowsInlineCodeLeadingHang() &&
        (item.text.includes("/") || item.text.includes("\\") || item.isPathTailFragment);
      if (
        sealedContinuationLine != null &&
        !cursorsMatch(LINE_START_CURSOR, sealedContinuationLine.end) &&
        !shouldBreakForEngineSealedOverflow &&
        sealedContinuationFitRatio >= INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD
      ) {
        const sealedContinuationConsumedWholeItem = cursorsMatch(
          sealedContinuationLine.end,
          item.endCursor,
        );
        const remainingWidthBeforeSealedContinuationAccept = state.remainingWidth;
        acceptCodeFragmentState({
          state,
          item,
          codeGroupId,
          remainingWidthBeforeAccept: remainingWidthBeforeSealedContinuationAccept,
          remainingWidthAfterAccept: Math.max(
            0,
            state.remainingWidth - params.reservedWidth - sealedContinuationLine.width,
          ),
          lastFragmentText: item.text,
          lastFragmentEndedWithHyphen: false,
          lastFragmentEndedWithPathDelimiter: false,
          shouldLimitCurrentCodeGroupToFirstFragment: params.shouldLimitCurrentCodeGroupToFirstFragment,
          shouldTrackWeakProseStartCodeGroup: params.shouldTrackWeakProseStartCodeGroup,
          shouldForceSoftBreakWeakProseContinuationWrap: params.shouldForceSoftBreakWeakProseContinuationWrap,
          maxWidth: params.maxWidth,
        });
        state.pendingSpaceWidth = 0;
        if (params.debug.enabled) {
          params.debug.sealedContinuationDecisions.push({
            canRelaxChromiumDottedPathBoundary,
            currentCodeGroupStartFragmentText: state.lineCurrentCodeGroupStartFragmentText,
            fullWidth,
            guardedRemainingWidth,
            remainingWidth: remainingWidthBeforeSealedContinuationAccept,
            continuationSlackPx: params.currentLineFitSlackPx,
            lastFragmentIsShortExtensionPath,
            lastFragmentText: state.lineLastCodeFragmentText,
            sameCodeGroupContinuation: state.lineHasContent,
            sealedBoundaryOverflow: effectiveSealedBoundaryOverflow,
            acceptedFragment: true,
            overflowedAcceptedFragment: false,
            shouldBreakBeforePartialSealedDottedPathFragment,
            text: item.text,
          });
          const segmentText = slicePreparedTextBetweenCursors(
            item.prepared,
            LINE_START_CURSOR,
            sealedContinuationLine.end,
          );
          params.debug.appendLineText(segmentText);
        }
        if (sealedContinuationConsumedWholeItem) {
          state.itemIndex += 1;
          state.cursor = null;
          return {
            action: "continue",
            state,
            forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
          };
        }
        state.cursor = sealedContinuationLine.end;
        return {
          action: "break",
          state,
          forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
        };
      }
      state.cursor = null;
      return {
        action: "break",
        state,
        forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
      };
    }

    const overflowed = fullWidth > guardedRemainingWidth + 0.01;
    const remainingWidthBeforeSealedFragmentAccept = state.remainingWidth;
    acceptCodeFragmentState({
      state,
      item,
      codeGroupId,
      remainingWidthBeforeAccept: remainingWidthBeforeSealedFragmentAccept,
      remainingWidthAfterAccept: overflowed ? 0 : Math.max(0, state.remainingWidth - fullWidth),
      lastFragmentText: item.text,
      lastFragmentEndedWithHyphen: item.text.endsWith("-"),
      lastFragmentEndedWithPathDelimiter: /[\\/]+$/.test(item.text),
      shouldLimitCurrentCodeGroupToFirstFragment: params.shouldLimitCurrentCodeGroupToFirstFragment,
      shouldTrackWeakProseStartCodeGroup: params.shouldTrackWeakProseStartCodeGroup,
      shouldForceSoftBreakWeakProseContinuationWrap: params.shouldForceSoftBreakWeakProseContinuationWrap,
      maxWidth: params.maxWidth,
    });
    state.itemIndex += 1;
    state.pendingSpaceWidth = 0;
    if (params.debug.enabled) {
      params.debug.sealedContinuationDecisions.push({
        canRelaxChromiumDottedPathBoundary,
        currentCodeGroupStartFragmentText: state.lineCurrentCodeGroupStartFragmentText,
        fullWidth,
        guardedRemainingWidth,
        remainingWidth: remainingWidthBeforeSealedFragmentAccept,
        continuationSlackPx: params.currentLineFitSlackPx,
        lastFragmentIsShortExtensionPath,
        lastFragmentText: state.lineLastCodeFragmentText,
        sameCodeGroupContinuation: state.lineHasContent,
        sealedBoundaryOverflow: effectiveSealedBoundaryOverflow,
        acceptedFragment: true,
        overflowedAcceptedFragment: overflowed,
        shouldBreakBeforePartialSealedDottedPathFragment,
        text: item.text,
      });
      params.debug.appendLineText(item.text);
    }
    if (overflowed) {
      state.cursor = null;
      return {
        action: "break",
        state,
        forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
      };
    }
    return {
      action: "continue",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }

  if (fullWidth <= guardedRemainingWidth + wholeFragmentFitTolerancePx + 0.01) {
    if (params.debug.enabled && state.lineHasContent && state.lastAcceptedCodeGroupId === codeGroupId) {
      params.debug.continuationDecisions.push({
        text: item.text,
        reservedWidth: params.reservedWidth,
        remainingWidth: state.remainingWidth,
        guardedRemainingWidth,
        fullWidth,
        currentLineFitSlackPx: params.currentLineFitSlackPx,
        lineLastCodeFragmentText: state.lineLastCodeFragmentText,
        lineLastCodeFragmentEndedWithPathDelimiter: state.lineLastCodeFragmentEndedWithPathDelimiter,
        lineLastCodeFragmentEndedWithHyphen: state.lineLastCodeFragmentEndedWithHyphen,
        acceptedWholeFragment: true,
      });
    }
    const remainingWidthBeforeFragmentAccept = state.remainingWidth;
    acceptCodeFragmentState({
      state,
      item,
      codeGroupId,
      remainingWidthBeforeAccept: remainingWidthBeforeFragmentAccept,
      remainingWidthAfterAccept: Math.max(0, state.remainingWidth - fullWidth),
      lastFragmentText: item.text,
      lastFragmentEndedWithHyphen: item.text.endsWith("-"),
      lastFragmentEndedWithPathDelimiter: /[\\/]+$/.test(item.text),
      shouldLimitCurrentCodeGroupToFirstFragment: params.shouldLimitCurrentCodeGroupToFirstFragment,
      shouldTrackWeakProseStartCodeGroup: params.shouldTrackWeakProseStartCodeGroup,
      shouldForceSoftBreakWeakProseContinuationWrap: params.shouldForceSoftBreakWeakProseContinuationWrap,
      maxWidth: params.maxWidth,
    });
    state.itemIndex += 1;
    state.pendingSpaceWidth = 0;
    if (params.debug.enabled) {
      params.debug.appendLineText(item.text);
    }
    return {
      action: "continue",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }

  const shouldBreakBeforePartialFreshLineFittingCodeGroup =
    state.lineHasContent &&
    state.pendingSpaceWidth > 0 &&
    item.isFirstCodeGroupFragment &&
    item.codeGroupStartsAfterText &&
    !state.chargedCodeGroups.has(codeGroupId) &&
    params.wholeCodeGroupWidth > 0 &&
    params.wholeCodeGroupWidth <= params.maxWidth + 0.01;

  if (shouldBreakBeforePartialFreshLineFittingCodeGroup) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }

  if (params.debug.enabled && state.lineHasContent && state.lastAcceptedCodeGroupId === codeGroupId) {
    params.debug.continuationDecisions.push({
      text: item.text,
      reservedWidth: params.reservedWidth,
      remainingWidth: state.remainingWidth,
      guardedRemainingWidth,
      fullWidth,
      currentLineFitSlackPx: params.currentLineFitSlackPx,
      lineLastCodeFragmentText: state.lineLastCodeFragmentText,
      lineLastCodeFragmentEndedWithPathDelimiter: state.lineLastCodeFragmentEndedWithPathDelimiter,
      lineLastCodeFragmentEndedWithHyphen: state.lineLastCodeFragmentEndedWithHyphen,
      brokeBeforeFragment: item.codePartStartsAfterWhitespace && item.text.endsWith("-"),
    });
  }
  if (
    state.lineHasContent &&
    ((!item.codePartStartsAfterWhitespace && item.text.endsWith("-")) ||
      (!item.codePartStartsAfterWhitespace &&
        item.isSealedInlineCodeFragment &&
        (item.text.includes("/") || item.text.includes("\\"))))
  ) {
    state.cursor = null;
    return {
      action: "break",
      state,
      forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
    };
  }

  return {
    action: "pass",
    state,
    forcedFreshWholeCodeGroupIndex: params.forcedFreshWholeCodeGroupIndex,
  };
}
