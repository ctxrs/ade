import { layoutNextLine, type LayoutCursor, type LayoutLine, type PreparedTextWithSegments } from "@chenglou/pretext";
import type { SessionMarkdownInlineRun } from "./sessionMarkdownContract";
import { browserAllowsInlineCodeLeadingHang } from "./sessionMarkdownBrowserProfile";
import {
  createInlineCodeFitPlanner,
  isShortExtensionPathLikeFragment,
  resolveInlineCodeContinuationFitSlackPx,
  resolveInlineCodeProseStartSeamGuardPx,
  resolveInlineCodeWhitespaceSeparatedFragmentSlackPx,
  resolveInlineCodeWrapChromeWidth,
  shouldApplyInlineCodeSoftBreakTextStartGuard,
  shouldBreakBeforePartialDottedStemPathTailContinuation,
  shouldBreakBeforePartialSealedDottedPathContinuation,
  shouldBreakBeforeWhitespaceSeparatedInlineCodeFragment,
} from "./sessionMarkdownInlineCodeFit";
import { prepareInlineLayoutItems } from "./sessionMarkdownInlineLayout";
import { resolveInlineCodeStartDecision } from "./sessionMarkdownInlineStartDecisions";
import {
  LINE_START_CURSOR,
  clampHeight,
  cursorsMatch,
  segmentGraphemes,
  type SessionMarkdownDebugWindow,
  type TextBlockTypography,
} from "./sessionMarkdownMeasurementCore";

const INLINE_CODE_FRAGMENT_FIT_SLACK_PX = 0;
const INLINE_CODE_WHOLE_GROUP_FIT_SLACK_PX = 0;
const INLINE_CODE_CURRENT_LINE_START_RATIO_THRESHOLD = 0.35;
const INLINE_CODE_TAIL_TEXT_SEAM_GUARD_PX = 4;
const INLINE_CODE_SOFT_BREAK_TEXT_START_GUARD_RATIO = 1;
const INLINE_CODE_SOFT_BREAK_TEXT_START_GUARD_MAX_PX = 36;
const INLINE_CODE_WHITESPACE_CONTINUATION_GUARD_PX = 1;
const INLINE_CODE_HYPHEN_CONTINUATION_GUARD_PX = 2;
const INLINE_CODE_WEAK_PROSE_START_CONTINUATION_GUARD_PX = 1;
const STYLED_TEXT_SEAM_GUARD_PX = 2;
const STYLED_TEXT_BODY_START_GUARD_PX = 4;
const STYLED_TEXT_BODY_START_CURRENT_LINE_RATIO_THRESHOLD = 0.2;
const STYLED_TEXT_AFTER_INLINE_CODE_CLUSTER_GUARD_PX = 2;

function isPunctuationOnlySeamText(text: string): boolean {
  const trimmed = text.trim();
  return trimmed.length > 0 && /^[\p{P}\p{S}]+$/u.test(trimmed);
}

function isAtomicNonCodeTextSegment(text: string): boolean {
  const trimmed = text.trim();
  return trimmed.length > 0 && !/\s/.test(trimmed) && !isPunctuationOnlySeamText(trimmed);
}

function isWhitespaceOnlyLineSlice(text: string): boolean {
  return text.trim().length === 0;
}

function resolveInlineCodeSoftBreakTextStartGuardPx(minStartTextWidth: number): number {
  return Math.min(
    INLINE_CODE_SOFT_BREAK_TEXT_START_GUARD_MAX_PX,
    Math.max(INLINE_CODE_TAIL_TEXT_SEAM_GUARD_PX, minStartTextWidth * INLINE_CODE_SOFT_BREAK_TEXT_START_GUARD_RATIO),
  );
}

function advancePreparedCursorOneGrapheme(
  prepared: PreparedTextWithSegments,
  cursor: LayoutCursor,
): LayoutCursor | null {
  const segment = prepared.segments[cursor.segmentIndex];
  if (segment == null) {
    return null;
  }
  const graphemeCount = segmentGraphemes(segment).length;
  if (cursor.graphemeIndex + 1 < graphemeCount) {
    return {
      segmentIndex: cursor.segmentIndex,
      graphemeIndex: cursor.graphemeIndex + 1,
    };
  }
  const nextSegmentIndex = cursor.segmentIndex + 1;
  if (nextSegmentIndex < prepared.segments.length) {
    return {
      segmentIndex: nextSegmentIndex,
      graphemeIndex: 0,
    };
  }
  return null;
}

function slicePreparedTextBetweenCursors(
  prepared: PreparedTextWithSegments,
  start: LayoutCursor,
  end: LayoutCursor,
): string {
  if (
    end.segmentIndex < start.segmentIndex ||
    (end.segmentIndex === start.segmentIndex && end.graphemeIndex <= start.graphemeIndex)
  ) {
    return "";
  }
  const parts: string[] = [];
  for (let segmentIndex = start.segmentIndex; segmentIndex <= end.segmentIndex; segmentIndex += 1) {
    const segment = prepared.segments[segmentIndex] ?? "";
    const graphemes = segmentGraphemes(segment);
    const startGraphemeIndex = segmentIndex === start.segmentIndex ? start.graphemeIndex : 0;
    const endGraphemeIndex = segmentIndex === end.segmentIndex ? end.graphemeIndex : graphemes.length;
    if (endGraphemeIndex > startGraphemeIndex) {
      parts.push(graphemes.slice(startGraphemeIndex, endGraphemeIndex).join(""));
    }
  }
  return parts.join("");
}

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
  const debugInlineCodeWidthMatches =
    inlineCodeDebugWindow?.__ctxInlineCodeDebugWidth == null ||
    inlineCodeDebugWindow.__ctxInlineCodeDebugWidth === params.width;
  const debugInlineCode =
    debugInlineCodeWidthMatches &&
    (forceInlineCodeDebug ||
      (inlineCodeDebugTarget != null &&
        params.runs.some(
          (run) =>
            run.kind === "inlineCode" &&
            (inlineCodeDebugTarget === "*" ||
              run.text === inlineCodeDebugTarget ||
              run.text.includes(inlineCodeDebugTarget ?? "")),
        )));
  const debugLines: string[] = [];
  const debugStartDecisions: Array<{
    pendingSpaceWidth: number;
    preferredStartWidth: number;
    dottedPathClusterWidth: number;
    wholeCodeGroupWidth: number;
    remainingWidth: number;
    currentLineConsumedWidth: number;
    currentLineStartFitRatio: number;
    currentLineCodeStartFitIsReadable: boolean;
    currentLineCodeStartFitIsStrong: boolean;
    shouldBreakForSoftBreakProseCodeStart: boolean;
    shouldBreakForInlineTailPunctuationCodeStart: boolean;
    shouldBreakForStyledTailCodeStart: boolean;
    shouldLimitCurrentCodeGroupToFirstFragment: boolean;
    shouldBreakForAttachedTrailingPlainStart: boolean;
    shouldBreak: boolean;
    text: string;
  }> = [];
  const debugWhitespaceDecisions: Array<{
    lineHasContent: boolean;
    reservedWidth: number;
    remainingWidth: number;
    guardedRemainingWidth: number;
    fragmentWidth: number;
    slackPx?: number;
    shouldBreak: boolean;
    text: string;
  }> = [];
  const debugSealedContinuationDecisions: Array<{
    canRelaxChromiumDottedPathBoundary: boolean;
    currentCodeGroupStartFragmentText: string | null;
    fullWidth: number;
    guardedRemainingWidth: number;
    remainingWidth?: number;
    continuationSlackPx?: number;
    lastFragmentIsShortExtensionPath: boolean;
    lastFragmentText: string | null;
    sameCodeGroupContinuation: boolean;
    sealedBoundaryOverflow: boolean;
    acceptedFragment?: boolean;
    overflowedAcceptedFragment?: boolean;
    shouldBreakBeforePartialSealedDottedPathFragment: boolean;
    text: string;
  }> = [];
  const debugContinuationDecisions: Array<{
    text: string;
    reservedWidth: number;
    remainingWidth: number;
    guardedRemainingWidth: number;
    availableLineWidth?: number;
    lineWidth?: number;
    fullWidth: number;
    currentLineFitSlackPx: number;
    lineLastCodeFragmentText: string | null;
    lineLastCodeFragmentEndedWithPathDelimiter: boolean;
    lineLastCodeFragmentEndedWithHyphen: boolean;
    acceptedWholeFragment?: boolean;
    brokeBeforeFragment?: boolean;
  }> = [];
  const debugSegmentSeamAdjustments: Array<{
    type: "no-progress-advance" | "no-progress-drop" | "whitespace-only-break" | "whitespace-only-advance";
    lineHasContent: boolean;
    text: string;
  }> = [];
  const {
    codeGroupFitEndsAtFriendlyBoundary,
    dottedPathCodeGroupStartClusterWidths,
    measureAttachedTrailingPlainWidth,
    measureCodeGroupTrailingPlainInfo,
    measureCodeGroupTrailingPlainWidth,
    measureCodeGroupFitWithinWidth,
    preferredCodeGroupStartWidths,
    wholeCodeGroupInlineWidths,
  } = createInlineCodeFitPlanner({ items, maxWidth });
  let totalHeight = 0;
  let itemIndex = 0;
  let cursor: LayoutCursor | null = null;
  let forcedFreshWholeCodeGroupIndex: number | null = null;

  while (itemIndex < items.length) {
    let lineHasContent = false;
    let lineOnlyCodeGroupId: number | null = null;
    let lineCurrentCodeGroupStartFragmentText: string | null = null;
    let lineCurrentCodeGroupStartedNearFresh = false;
    let lineCurrentCodeGroupLimitToFirstFragment = false;
    let lineLastCodeFragmentText: string | null = null;
    let lineLastCodeFragmentEndedWithHyphen = false;
    let lineLastCodeFragmentEndedWithPathDelimiter = false;
    let lastAcceptedCodeGroupId: number | null = null;
    let lineDecoratedTextSegmentCount = 0;
    let lineSawInlineCode = false;
    let lineTailAfterInlineCodeIsPunctuationOnly = false;
    let lineStartedWithContinuedCode = false;
    let lineStartedWithCollapsedSoftBreakPlainText = false;
    let lineAcceptedPlainAfterContinuedCode = false;
    let lineAcceptedSoftBreakProseAfterInlineCode = false;
    let lineSoftBreakProseAfterInlineCodeGuardPx = 0;
    let lineSoftBreakProseGuardCodeGroupId: number | null = null;
    let lineForceSoftBreakWeakProseContinuationCodeGroupId: number | null = null;
    let lineWeakProseStartCodeGroupId: number | null = null;
    let lineUsedChromiumDottedPathBoundaryContinuation = false;
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
      const preferredStartWidth = preferredCodeGroupStartWidths.get(itemIndex) ?? 0;
      const dottedPathClusterWidth = dottedPathCodeGroupStartClusterWidths.get(itemIndex) ?? 0;
      const wholeCodeGroupWidth = wholeCodeGroupInlineWidths.get(itemIndex) ?? 0;
      const allowCurrentLineLeadingHang =
        !lineSawInlineCode || lineAcceptedPlainAfterContinuedCode || item.startsAfterCollapsedSoftBreak;
      const allowLeadingHangForCurrentLine: boolean =
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        item.isFirstCodeGroupFragment &&
        item.codeGroupStartsAfterText &&
        !lineStartedWithCollapsedSoftBreakPlainText &&
        wholeCodeGroupWidth > remainingWidth + 0.01;
      const chromeWidth: number =
        codeGroupId != null
          ? resolveInlineCodeWrapChromeWidth({
              allowLeadingHang: allowCurrentLineLeadingHang && allowLeadingHangForCurrentLine,
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
      const reservedWidth: number = (lineHasContent ? pendingSpaceWidth : 0) + chromeWidth;
      let guardedRemainingWidth = remainingWidth;
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
      const currentLineCodeContinuationSlackPx = resolveInlineCodeContinuationFitSlackPx({
        lineHasContent,
        atLineBreakBoundary: cursor === null,
        sameCodeGroupContinuation: codeGroupId != null && lastAcceptedCodeGroupId === codeGroupId,
        startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
        lastFragmentEndedWithHyphen: lineLastCodeFragmentEndedWithHyphen,
        lastFragmentEndedWithPathDelimiter: lineLastCodeFragmentEndedWithPathDelimiter,
        item,
      });
      const currentLineCodeStartSeamGuardPx = resolveInlineCodeProseStartSeamGuardPx({
        startsAtLineStart: !lineHasContent,
        item,
      });
      const currentLineFitSlackPx = Math.max(
        currentLineStartSlackPx,
        currentLineWhitespaceContinuationSlackPx,
        currentLineCodeContinuationSlackPx,
      );
      const currentLineWeakProseStartContinuationGuardPx =
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        lineWeakProseStartCodeGroupId === codeGroupId &&
        lastAcceptedCodeGroupId === codeGroupId
          ? currentLineCodeContinuationSlackPx +
            currentLineCodeStartSeamGuardPx +
            lineSoftBreakProseAfterInlineCodeGuardPx +
            INLINE_CODE_WEAK_PROSE_START_CONTINUATION_GUARD_PX
          : 0;
      const currentLineNearFitLeadingHangPx: number =
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        item.isFirstCodeGroupFragment &&
        item.codeGroupStartsAfterText &&
        (item.prefersFreshLineStart || item.codeGroupHasWhitespace) &&
        allowLeadingHangForCurrentLine &&
        allowCurrentLineLeadingHang
          ? item.chromeWidth / 2
          : 0;
      const sliceStartsAtItemStart = cursor === null;
      const currentLineWhitespaceContinuationGuardPx: number =
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        item.startsAfterCodeWhitespace &&
        item.codeGroupStartsAfterText
          ? INLINE_CODE_WHITESPACE_CONTINUATION_GUARD_PX
          : 0;
      const currentLineStyledSeamGuardPx: number =
        lineHasContent &&
        cursor === null &&
        codeGroupId == null &&
        item.startsAfterStyledTextSeam &&
        !item.hasTrailingStyledText &&
        !isPunctuationOnlySeamText(item.text)
          ? lineDecoratedTextSegmentCount >= 3
            ? STYLED_TEXT_SEAM_GUARD_PX * 2
            : STYLED_TEXT_SEAM_GUARD_PX
          : 0;
      const currentLineStyledAfterInlineCodeClusterGuardPx: number =
        lineHasContent &&
        cursor === null &&
        codeGroupId == null &&
        item.isDecoratedText &&
        lineSawInlineCode &&
        lineDecoratedTextSegmentCount > 0
          ? STYLED_TEXT_AFTER_INLINE_CODE_CLUSTER_GUARD_PX
          : 0;
      const currentLineInlineCodeTailTextSeamGuardPx: number =
        lineHasContent &&
        cursor === null &&
        codeGroupId == null &&
        item.startsAfterInlineCodeSeam &&
        !item.startsAfterCollapsedSoftBreak
          ? !browserAllowsInlineCodeLeadingHang() && item.startsAfterPathLikeInlineCodeSeam
            ? 0
            : INLINE_CODE_TAIL_TEXT_SEAM_GUARD_PX
          : 0;
      const currentLineInlineCodeSoftBreakTextStartGuardPx: number =
        lineHasContent &&
        cursor === null &&
        codeGroupId == null &&
        sliceStartsAtItemStart &&
        shouldApplyInlineCodeSoftBreakTextStartGuard({
          text: item.text,
          startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
          startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
          startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
          startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
          lastFragmentEndedWithPathDelimiter: lineLastCodeFragmentEndedWithPathDelimiter,
          lastFragmentEndedWithHyphen: lineLastCodeFragmentEndedWithHyphen,
        })
          ? resolveInlineCodeSoftBreakTextStartGuardPx(item.minStartTextWidth)
          : 0;
      const currentLineStyledBodyStartGuardPx: number =
        lineHasContent && cursor === null && codeGroupId == null && item.startsStyledTextAfterBodySeam
          ? STYLED_TEXT_BODY_START_GUARD_PX
          : 0;
      const canDropLeadingCollapsedSpaceAtWrap: boolean =
        codeGroupId == null &&
        lineHasContent &&
        cursor === null &&
        pendingSpaceWidth > 0 &&
        !item.startsAfterInlineCodeSeam;
      const startCursor: LayoutCursor = cursor ?? LINE_START_CURSOR;
      const trailingPlainWidthAfterCodeGroupStart = measureCodeGroupTrailingPlainWidth(itemIndex);
      const trailingPlainAfterCodeGroupStart = measureCodeGroupTrailingPlainInfo(itemIndex);
      const currentLineCodeFit =
        lineHasContent && cursor === null && codeGroupId != null && item.isFirstCodeGroupFragment
          ? measureCodeGroupFitWithinWidth(
              itemIndex,
              Math.max(
                1,
                remainingWidth - pendingSpaceWidth - currentLineCodeStartSeamGuardPx + currentLineFitSlackPx,
              ),
              false,
              allowCurrentLineLeadingHang,
            )
          : null;
      const codeFitEndsAtFriendlyBoundary =
        currentLineCodeFit != null && codeGroupFitEndsAtFriendlyBoundary(currentLineCodeFit);
      const freshLineCodeFit =
        lineHasContent && cursor === null && codeGroupId != null && item.isFirstCodeGroupFragment
          ? measureCodeGroupFitWithinWidth(itemIndex, maxWidth, true)
          : null;
      const effectiveTrailingPlainWidthAfterCodeGroupStart =
        trailingPlainAfterCodeGroupStart.startsAfterCollapsedSoftBreak
          ? 0
          : trailingPlainWidthAfterCodeGroupStart;
      const effectiveTrailingPlainHasFollowingInlineCode =
        trailingPlainAfterCodeGroupStart.startsAfterCollapsedSoftBreak
          ? false
          : trailingPlainAfterCodeGroupStart.hasFollowingInlineCode;
      const freshLineFitEndsAtFriendlyBoundary =
        freshLineCodeFit != null && codeGroupFitEndsAtFriendlyBoundary(freshLineCodeFit);
      const prefersFreshLineStart = item.prefersFreshLineStart;
      const {
        currentLineStartFitRatio,
        currentLineCodeStartFitIsReadable,
        currentLineCodeStartFitIsStrong,
        shouldTrackWeakProseStartCodeGroup,
        shouldForceSoftBreakWeakProseContinuationWrap,
        shouldBreakForPreferredStart,
        shouldBreakForSoftBreakProseCodeStart,
        shouldBreakForInlineTailPunctuationCodeStart,
        shouldBreakForStyledTailCodeStart,
        shouldLimitCurrentCodeGroupToFirstFragment,
        shouldAllowChromiumPartialDottedPathStart,
        shouldAllowChromiumAttachedTrailingPlainPartialFit,
        shouldAllowEnginePathLikeTrailingPlainWrap,
        shouldBreakForAttachedTrailingPlainStart,
        shouldBreakForWeakFirstSliceFreshLineStart,
      } = resolveInlineCodeStartDecision({
        codeGroupId,
        currentLineStartFragmentWidth: chromeWidth + item.fullWidth,
        lineAcceptedPlainAfterContinuedCode,
        lineAcceptedSoftBreakProseAfterInlineCode,
        lineHasContent,
        lineSawInlineCode,
        lineSoftBreakProseGuardCodeGroupId,
        lineTailAfterInlineCodeIsPunctuationOnly,
        item,
        maxWidth,
        preferredStartWidth,
        remainingWidth,
        trailingPlainAfterCodeGroupStart,
        wholeCodeGroupWidth,
        currentLineCodeFit,
        freshLineCodeFit,
        codeFitEndsAtFriendlyBoundary,
        freshLineFitEndsAtFriendlyBoundary,
      });
      if (
        debugInlineCode &&
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        item.isFirstCodeGroupFragment &&
        prefersFreshLineStart
      ) {
        debugStartDecisions.push({
          pendingSpaceWidth,
          preferredStartWidth,
          dottedPathClusterWidth,
          wholeCodeGroupWidth,
          remainingWidth,
          currentLineConsumedWidth: currentLineCodeFit?.consumedWidth ?? 0,
          currentLineStartFitRatio,
          currentLineCodeStartFitIsReadable,
          currentLineCodeStartFitIsStrong,
          shouldBreakForSoftBreakProseCodeStart,
          shouldBreakForInlineTailPunctuationCodeStart,
          shouldBreakForStyledTailCodeStart,
          shouldLimitCurrentCodeGroupToFirstFragment,
          shouldBreakForAttachedTrailingPlainStart,
          shouldBreak:
            shouldBreakForSoftBreakProseCodeStart ||
            shouldBreakForPreferredStart ||
            shouldBreakForInlineTailPunctuationCodeStart ||
            shouldBreakForStyledTailCodeStart ||
            shouldBreakForAttachedTrailingPlainStart ||
            shouldBreakForWeakFirstSliceFreshLineStart,
          text: item.text,
        });
      }
      if (
        shouldBreakForSoftBreakProseCodeStart ||
        shouldBreakForPreferredStart ||
        shouldBreakForInlineTailPunctuationCodeStart ||
        shouldBreakForStyledTailCodeStart ||
        shouldBreakForAttachedTrailingPlainStart ||
        shouldBreakForWeakFirstSliceFreshLineStart
      ) {
        if (
          shouldBreakForInlineTailPunctuationCodeStart &&
          item.isFirstCodeGroupFragment &&
          codeGroupId != null &&
          !item.codeGroupHasTrailingText &&
          wholeCodeGroupWidth > 0 &&
          wholeCodeGroupWidth <= maxWidth + 0.01
        ) {
          forcedFreshWholeCodeGroupIndex = itemIndex;
        }
        cursor = null;
        break;
      }
      if (
        lineHasContent &&
        cursor === null &&
        codeGroupId != null &&
        item.isFirstCodeGroupFragment &&
        prefersFreshLineStart &&
        (currentLineCodeFit == null || currentLineCodeFit.consumedWidth <= 0) &&
        reservedWidth + item.fullWidth > remainingWidth + currentLineNearFitLeadingHangPx + 0.01
      ) {
        cursor = null;
        break;
      }

      if (lineHasContent && cursor === null && codeGroupId != null) {
        const fullWidth = reservedWidth + item.fullWidth;
        if (
          fullWidth > remainingWidth + 0.01 &&
          (prefersFreshLineStart || item.codeGroupHasDottedPath) &&
          remainingWidth < reservedWidth + item.minStartTextWidth - 0.01 &&
          !currentLineCodeStartFitIsReadable
        ) {
          cursor = null;
          break;
        }
      }

      if (cursor === null && codeGroupId != null) {
        if (
          forcedFreshWholeCodeGroupIndex === itemIndex &&
          item.isFirstCodeGroupFragment &&
          wholeCodeGroupWidth > 0 &&
          wholeCodeGroupWidth <= maxWidth + 0.01
        ) {
          let scanIndex = itemIndex;
          let lastCodeFragmentText = item.text;
          while (scanIndex < items.length) {
            const candidate = items[scanIndex]!;
            if (candidate.kind === "hardBreak") {
              break;
            }
            if (candidate.kind === "space") {
              if (candidate.codeGroupId !== codeGroupId) {
                break;
              }
              if (debugInlineCode) {
                debugLine += candidate.text;
              }
              scanIndex += 1;
              continue;
            }
            if (candidate.codeGroupId !== codeGroupId) {
              break;
            }
            if (debugInlineCode) {
              debugLine += candidate.text;
            }
            lastCodeFragmentText = candidate.text;
            scanIndex += 1;
          }
          const remainingWidthBeforeWholeGroupAccept = remainingWidth;
          remainingWidth = Math.max(0, remainingWidth - wholeCodeGroupWidth);
          if (!lineHasContent) {
            lineOnlyCodeGroupId = codeGroupId;
            lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
          } else if (lineOnlyCodeGroupId !== codeGroupId) {
            lineOnlyCodeGroupId = null;
          }
          if (lastAcceptedCodeGroupId !== codeGroupId) {
            lineCurrentCodeGroupStartFragmentText = item.text;
            lineCurrentCodeGroupStartedNearFresh =
              lineHasContent && remainingWidthBeforeWholeGroupAccept > maxWidth * 0.85;
            lineCurrentCodeGroupLimitToFirstFragment = shouldLimitCurrentCodeGroupToFirstFragment;
          }
          lineLastCodeFragmentText = lastCodeFragmentText;
          lineLastCodeFragmentEndedWithHyphen = lastCodeFragmentText.endsWith("-");
          lineLastCodeFragmentEndedWithPathDelimiter = /[\\/]+$/.test(lastCodeFragmentText);
          lastAcceptedCodeGroupId = codeGroupId;
          lineTailAfterInlineCodeIsPunctuationOnly = false;
          lineHasContent = true;
          chargedCodeGroups.add(codeGroupId);
          if (
            lineAcceptedSoftBreakProseAfterInlineCode &&
            lineSoftBreakProseGuardCodeGroupId == null
          ) {
            lineSoftBreakProseGuardCodeGroupId = codeGroupId;
          }
          itemIndex = scanIndex;
          pendingSpaceWidth = 0;
          forcedFreshWholeCodeGroupIndex = null;
          continue;
        }
        const fullWidth = reservedWidth + item.fullWidth;
        guardedRemainingWidth =
          remainingWidth -
          currentLineCodeStartSeamGuardPx -
          currentLineWhitespaceContinuationGuardPx +
          -currentLineWeakProseStartContinuationGuardPx +
          currentLineFitSlackPx +
          currentLineNearFitLeadingHangPx;
        const attachedTrailingPlainWidth = measureAttachedTrailingPlainWidth(itemIndex);
        const hyphenContinuationGuardPx =
          !browserAllowsInlineCodeLeadingHang() &&
          lineLastCodeFragmentEndedWithHyphen &&
          !item.text.includes("/") &&
          !item.text.includes("\\") &&
          !item.isPathTailFragment
            ? INLINE_CODE_HYPHEN_CONTINUATION_GUARD_PX
            : 0;
        const whitespaceSlackPx = resolveInlineCodeWhitespaceSeparatedFragmentSlackPx({
          lineHasContent,
          startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
          fragmentText: item.text,
        });
        const shouldBreakBeforeWhitespaceSeparatedFragment =
          shouldBreakBeforeWhitespaceSeparatedInlineCodeFragment({
            lineHasContent,
            startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
            reservedWidth,
            remainingWidth: guardedRemainingWidth,
            fragmentWidth: item.fullWidth,
            slackPx: whitespaceSlackPx,
          });
        const wholeFragmentFitTolerancePx =
          item.startsAfterCodeWhitespace ? whitespaceSlackPx : 0;
        if (debugInlineCode && item.startsAfterCodeWhitespace) {
          debugWhitespaceDecisions.push({
            lineHasContent,
            reservedWidth,
            remainingWidth,
            guardedRemainingWidth,
            fragmentWidth: item.fullWidth,
            slackPx: whitespaceSlackPx,
            shouldBreak: shouldBreakBeforeWhitespaceSeparatedFragment,
            text: item.text,
          });
        }
        if (
          shouldBreakBeforeWhitespaceSeparatedFragment
        ) {
          cursor = null;
          break;
        }
        if (
          lineHasContent &&
          lineCurrentCodeGroupLimitToFirstFragment &&
          lastAcceptedCodeGroupId === codeGroupId &&
          lineLastCodeFragmentEndedWithPathDelimiter
        ) {
          cursor = null;
          break;
        }
        if (
          lineHasContent &&
          lineForceSoftBreakWeakProseContinuationCodeGroupId === codeGroupId &&
          lastAcceptedCodeGroupId === codeGroupId
        ) {
          cursor = null;
          break;
        }
        const sealedBoundary =
          lineHasContent &&
          lastAcceptedCodeGroupId === codeGroupId &&
          (lineLastCodeFragmentEndedWithHyphen || lineLastCodeFragmentEndedWithPathDelimiter);
        const boundaryRemainingWidth =
          remainingWidth +
          currentLineFitSlackPx -
          hyphenContinuationGuardPx -
          currentLineWeakProseStartContinuationGuardPx;
        const shouldAcceptChromiumPathTailContinuation =
          browserAllowsInlineCodeLeadingHang() &&
          lineHasContent &&
          lastAcceptedCodeGroupId === codeGroupId &&
          lineLastCodeFragmentEndedWithPathDelimiter &&
          !item.isSealedInlineCodeFragment &&
          !item.startsAfterCodeWhitespace &&
          reservedWidth + item.fullWidth <= boundaryRemainingWidth + 0.01;
        const nextSameCodeGroupItem = items[itemIndex + 1];
        const splitDottedStemTailWouldOverflowCurrentLine =
          lineCurrentCodeGroupStartedNearFresh &&
          lineHasContent &&
          lastAcceptedCodeGroupId === codeGroupId &&
          lineLastCodeFragmentEndedWithPathDelimiter &&
          item.codeGroupStartsAfterText &&
          item.codeGroupHasTrailingText &&
          lineCurrentCodeGroupStartFragmentText != null &&
          !lineCurrentCodeGroupStartFragmentText.includes(".") &&
          item.text.endsWith(".") &&
          nextSameCodeGroupItem?.kind === "segment" &&
          nextSameCodeGroupItem.codeGroupId === codeGroupId &&
          (nextSameCodeGroupItem.text.includes("/") ||
            nextSameCodeGroupItem.text.includes("\\") ||
            nextSameCodeGroupItem.isPathTailFragment) &&
          reservedWidth + item.fullWidth + nextSameCodeGroupItem.fullWidth > boundaryRemainingWidth + 0.01;
        if (splitDottedStemTailWouldOverflowCurrentLine) {
          cursor = null;
          break;
        }
        const sealedBoundaryOverflow =
          sealedBoundary && reservedWidth + item.fullWidth > boundaryRemainingWidth + 0.01;
        const shouldBreakBeforePartialSealedDottedPathFragment =
          shouldBreakBeforePartialSealedDottedPathContinuation({
            currentCodeGroupStartFragmentText: lineCurrentCodeGroupStartFragmentText,
            fullWidth,
            guardedRemainingWidth,
            item,
            lastFragmentText: lineLastCodeFragmentText,
            sameCodeGroupContinuation: lineHasContent && lastAcceptedCodeGroupId === codeGroupId,
          });
        const shouldBreakBeforePartialDottedStemPathTailFragment =
          shouldBreakBeforePartialDottedStemPathTailContinuation({
            fullWidth,
            guardedRemainingWidth,
            item,
            lastFragmentText: lineLastCodeFragmentText,
            sameCodeGroupContinuation: lineHasContent && lastAcceptedCodeGroupId === codeGroupId,
          });
        const lastFragmentIsShortExtensionPath = isShortExtensionPathLikeFragment(lineLastCodeFragmentText);
        const canRelaxChromiumDottedPathBoundary =
          sealedBoundaryOverflow &&
          !lineUsedChromiumDottedPathBoundaryContinuation &&
          browserAllowsInlineCodeLeadingHang() &&
          item.codeGroupStartsAfterText &&
          lineCurrentCodeGroupStartFragmentText != null &&
          !lineCurrentCodeGroupStartFragmentText.includes(".") &&
          !lastFragmentIsShortExtensionPath &&
          item.codeGroupHasDottedPath &&
          !item.text.includes("/") &&
          !item.text.includes("\\") &&
          (item.text.includes(".") || item.isPathTailFragment);
        const effectiveSealedBoundaryOverflow =
          sealedBoundaryOverflow && !shouldAcceptChromiumPathTailContinuation;
        if (shouldBreakBeforePartialDottedStemPathTailFragment) {
          cursor = null;
          break;
        }
        if (
          debugInlineCode &&
          lineHasContent &&
          item.isSealedInlineCodeFragment &&
          lastAcceptedCodeGroupId === codeGroupId
        ) {
          debugSealedContinuationDecisions.push({
            canRelaxChromiumDottedPathBoundary,
            currentCodeGroupStartFragmentText: lineCurrentCodeGroupStartFragmentText,
            fullWidth,
            guardedRemainingWidth,
            remainingWidth,
            continuationSlackPx: currentLineFitSlackPx,
            lastFragmentIsShortExtensionPath,
            lastFragmentText: lineLastCodeFragmentText,
            sameCodeGroupContinuation: lineHasContent && lastAcceptedCodeGroupId === codeGroupId,
            sealedBoundaryOverflow: effectiveSealedBoundaryOverflow,
            shouldBreakBeforePartialSealedDottedPathFragment,
            text: item.text,
          });
        }
        if (effectiveSealedBoundaryOverflow && !canRelaxChromiumDottedPathBoundary) {
          cursor = null;
          break;
        }
        if (shouldBreakBeforePartialSealedDottedPathFragment) {
          cursor = null;
          break;
        }
        if (canRelaxChromiumDottedPathBoundary) {
          lineUsedChromiumDottedPathBoundaryContinuation = true;
        }
        if (shouldAcceptChromiumPathTailContinuation) {
          const remainingWidthBeforeFragmentAccept = remainingWidth;
          remainingWidth = Math.max(0, remainingWidth - reservedWidth - item.fullWidth);
          if (!lineHasContent) {
            lineOnlyCodeGroupId = codeGroupId;
            lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
            lineLastCodeFragmentEndedWithPathDelimiter = /[\\/]+$/.test(item.text);
            lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
          } else if (lineOnlyCodeGroupId !== codeGroupId) {
            lineOnlyCodeGroupId = null;
          }
          if (lastAcceptedCodeGroupId !== codeGroupId) {
            lineCurrentCodeGroupStartFragmentText = item.text;
            lineCurrentCodeGroupStartedNearFresh =
              lineHasContent && remainingWidthBeforeFragmentAccept > maxWidth * 0.85;
            lineCurrentCodeGroupLimitToFirstFragment = shouldLimitCurrentCodeGroupToFirstFragment;
          }
          lineLastCodeFragmentText = item.text;
          lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
          lineLastCodeFragmentEndedWithPathDelimiter = /[\\/]+$/.test(item.text);
          lastAcceptedCodeGroupId = codeGroupId;
          lineTailAfterInlineCodeIsPunctuationOnly = false;
          lineHasContent = true;
          chargedCodeGroups.add(codeGroupId);
          if (
            lineAcceptedSoftBreakProseAfterInlineCode &&
            lineSoftBreakProseGuardCodeGroupId == null
          ) {
            lineSoftBreakProseGuardCodeGroupId = codeGroupId;
          }
          if (shouldTrackWeakProseStartCodeGroup) {
            lineWeakProseStartCodeGroupId = codeGroupId;
            if (shouldForceSoftBreakWeakProseContinuationWrap) {
              lineForceSoftBreakWeakProseContinuationCodeGroupId = codeGroupId;
            }
          }
          itemIndex += 1;
          pendingSpaceWidth = 0;
          if (debugInlineCode) {
            debugContinuationDecisions.push({
              text: item.text,
              reservedWidth,
              remainingWidth: remainingWidthBeforeFragmentAccept,
              guardedRemainingWidth,
              fullWidth,
              currentLineFitSlackPx,
              lineLastCodeFragmentText,
              lineLastCodeFragmentEndedWithPathDelimiter,
              lineLastCodeFragmentEndedWithHyphen,
              acceptedWholeFragment: true,
            });
            debugLine += item.text;
          }
          continue;
        }
        if (item.isSealedInlineCodeFragment) {
          if (lineHasContent && fullWidth > guardedRemainingWidth + 0.01) {
            const shouldBreakBeforeEnginePartialSealedPathContinuation =
              !browserAllowsInlineCodeLeadingHang() &&
              (item.text.includes("/") || item.text.includes("\\") || item.isPathTailFragment);
            if (shouldBreakBeforeEnginePartialSealedPathContinuation) {
              cursor = null;
              break;
            }
            const sealedContinuationLine: LayoutLine | null =
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
            const sealedContinuationWouldOverflowCurrentLine =
              sealedContinuationLine != null &&
              !cursorsMatch(LINE_START_CURSOR, sealedContinuationLine.end) &&
              reservedWidth + sealedContinuationLine.width > guardedRemainingWidth + 0.01;
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
              const remainingWidthBeforeSealedContinuationAccept = remainingWidth;
              remainingWidth = Math.max(
                0,
                remainingWidth - reservedWidth - sealedContinuationLine.width,
              );
              if (!lineHasContent) {
                lineOnlyCodeGroupId = codeGroupId;
                lineLastCodeFragmentEndedWithHyphen = false;
                lineLastCodeFragmentEndedWithPathDelimiter = false;
                lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
              } else if (lineOnlyCodeGroupId !== codeGroupId) {
                lineOnlyCodeGroupId = null;
              }
              if (lastAcceptedCodeGroupId !== codeGroupId) {
                lineCurrentCodeGroupStartFragmentText = item.text;
                lineCurrentCodeGroupStartedNearFresh =
                  lineHasContent && remainingWidthBeforeSealedContinuationAccept > maxWidth * 0.85;
                lineCurrentCodeGroupLimitToFirstFragment = shouldLimitCurrentCodeGroupToFirstFragment;
              }
              lineLastCodeFragmentText = item.text;
              lineLastCodeFragmentEndedWithHyphen = false;
              lineLastCodeFragmentEndedWithPathDelimiter = false;
              lastAcceptedCodeGroupId = codeGroupId;
              lineTailAfterInlineCodeIsPunctuationOnly = false;
              lineHasContent = true;
              chargedCodeGroups.add(codeGroupId);
              if (
                lineAcceptedSoftBreakProseAfterInlineCode &&
                lineSoftBreakProseGuardCodeGroupId == null
              ) {
                lineSoftBreakProseGuardCodeGroupId = codeGroupId;
              }
              if (shouldTrackWeakProseStartCodeGroup) {
                lineWeakProseStartCodeGroupId = codeGroupId;
                if (shouldForceSoftBreakWeakProseContinuationWrap) {
                  lineForceSoftBreakWeakProseContinuationCodeGroupId = codeGroupId;
                }
              }
              pendingSpaceWidth = 0;
              if (debugInlineCode) {
                debugSealedContinuationDecisions.push({
                  canRelaxChromiumDottedPathBoundary,
                  currentCodeGroupStartFragmentText: lineCurrentCodeGroupStartFragmentText,
                  fullWidth,
                  guardedRemainingWidth,
                  remainingWidth: remainingWidthBeforeSealedContinuationAccept,
                  continuationSlackPx: currentLineFitSlackPx,
                  lastFragmentIsShortExtensionPath,
                  lastFragmentText: lineLastCodeFragmentText,
                  sameCodeGroupContinuation: lineHasContent,
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
                debugLine += segmentText;
              }
              if (sealedContinuationConsumedWholeItem) {
                itemIndex += 1;
                cursor = null;
                continue;
              }
              cursor = sealedContinuationLine.end;
              break;
            }
            cursor = null;
            break;
          }
          const overflowed = fullWidth > guardedRemainingWidth + 0.01;
          const remainingWidthBeforeSealedFragmentAccept = remainingWidth;
          remainingWidth = overflowed ? 0 : Math.max(0, remainingWidth - fullWidth);
          if (!lineHasContent) {
            lineOnlyCodeGroupId = codeGroupId;
            lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
            lineLastCodeFragmentEndedWithPathDelimiter = /[\\/]+$/.test(item.text);
            lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
          } else if (lineOnlyCodeGroupId !== codeGroupId) {
            lineOnlyCodeGroupId = null;
          }
          if (lastAcceptedCodeGroupId !== codeGroupId) {
            lineCurrentCodeGroupStartFragmentText = item.text;
            lineCurrentCodeGroupStartedNearFresh =
              lineHasContent && remainingWidthBeforeSealedFragmentAccept > maxWidth * 0.85;
            lineCurrentCodeGroupLimitToFirstFragment = shouldLimitCurrentCodeGroupToFirstFragment;
          }
          lineLastCodeFragmentText = item.text;
          lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
          lineLastCodeFragmentEndedWithPathDelimiter = /[\\/]+$/.test(item.text);
          lastAcceptedCodeGroupId = codeGroupId;
          lineTailAfterInlineCodeIsPunctuationOnly = false;
          lineHasContent = true;
          chargedCodeGroups.add(codeGroupId);
          if (
            lineAcceptedSoftBreakProseAfterInlineCode &&
            lineSoftBreakProseGuardCodeGroupId == null
          ) {
            lineSoftBreakProseGuardCodeGroupId = codeGroupId;
          }
          if (shouldTrackWeakProseStartCodeGroup) {
            lineWeakProseStartCodeGroupId = codeGroupId;
            if (shouldForceSoftBreakWeakProseContinuationWrap) {
              lineForceSoftBreakWeakProseContinuationCodeGroupId = codeGroupId;
            }
          }
          itemIndex += 1;
          pendingSpaceWidth = 0;
          if (debugInlineCode) {
            debugSealedContinuationDecisions.push({
              canRelaxChromiumDottedPathBoundary,
              currentCodeGroupStartFragmentText: lineCurrentCodeGroupStartFragmentText,
              fullWidth,
              guardedRemainingWidth,
              remainingWidth: remainingWidthBeforeSealedFragmentAccept,
              continuationSlackPx: currentLineFitSlackPx,
              lastFragmentIsShortExtensionPath,
              lastFragmentText: lineLastCodeFragmentText,
              sameCodeGroupContinuation: lineHasContent,
              sealedBoundaryOverflow: effectiveSealedBoundaryOverflow,
              acceptedFragment: true,
              overflowedAcceptedFragment: overflowed,
              shouldBreakBeforePartialSealedDottedPathFragment,
              text: item.text,
            });
            debugLine += item.text;
          }
          if (overflowed) {
            cursor = null;
            break;
          }
          continue;
        }
        if (fullWidth <= guardedRemainingWidth + wholeFragmentFitTolerancePx + 0.01) {
          if (debugInlineCode && lineHasContent && lastAcceptedCodeGroupId === codeGroupId) {
            debugContinuationDecisions.push({
              text: item.text,
              reservedWidth,
              remainingWidth,
              guardedRemainingWidth,
              fullWidth,
              currentLineFitSlackPx,
              lineLastCodeFragmentText,
              lineLastCodeFragmentEndedWithPathDelimiter,
              lineLastCodeFragmentEndedWithHyphen,
              acceptedWholeFragment: true,
            });
          }
          const remainingWidthBeforeFragmentAccept = remainingWidth;
          remainingWidth = Math.max(0, remainingWidth - fullWidth);
          if (!lineHasContent) {
            lineOnlyCodeGroupId = codeGroupId;
            lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
            lineLastCodeFragmentEndedWithPathDelimiter = /[\\/]+$/.test(item.text);
            lineStartedWithContinuedCode = !item.isFirstCodeGroupFragment;
          } else if (lineOnlyCodeGroupId !== codeGroupId) {
            lineOnlyCodeGroupId = null;
          }
          if (lastAcceptedCodeGroupId !== codeGroupId) {
            lineCurrentCodeGroupStartFragmentText = item.text;
            lineCurrentCodeGroupStartedNearFresh =
              lineHasContent && remainingWidthBeforeFragmentAccept > maxWidth * 0.85;
            lineCurrentCodeGroupLimitToFirstFragment = shouldLimitCurrentCodeGroupToFirstFragment;
          }
          lineLastCodeFragmentText = item.text;
          lineLastCodeFragmentEndedWithHyphen = item.text.endsWith("-");
          lineLastCodeFragmentEndedWithPathDelimiter = /[\\/]+$/.test(item.text);
          lastAcceptedCodeGroupId = codeGroupId;
          lineTailAfterInlineCodeIsPunctuationOnly = false;
          lineHasContent = true;
          chargedCodeGroups.add(codeGroupId);
          if (
            lineAcceptedSoftBreakProseAfterInlineCode &&
            lineSoftBreakProseGuardCodeGroupId == null
          ) {
            lineSoftBreakProseGuardCodeGroupId = codeGroupId;
          }
          if (shouldTrackWeakProseStartCodeGroup) {
            lineWeakProseStartCodeGroupId = codeGroupId;
            if (shouldForceSoftBreakWeakProseContinuationWrap) {
              lineForceSoftBreakWeakProseContinuationCodeGroupId = codeGroupId;
            }
          }
          itemIndex += 1;
          pendingSpaceWidth = 0;
          if (debugInlineCode) {
            debugLine += item.text;
          }
          continue;
        }
        if (debugInlineCode && lineHasContent && lastAcceptedCodeGroupId === codeGroupId) {
          debugContinuationDecisions.push({
            text: item.text,
            reservedWidth,
            remainingWidth,
            guardedRemainingWidth,
            fullWidth,
            currentLineFitSlackPx,
            lineLastCodeFragmentText,
            lineLastCodeFragmentEndedWithPathDelimiter,
            lineLastCodeFragmentEndedWithHyphen,
            brokeBeforeFragment: item.codePartStartsAfterWhitespace && item.text.endsWith("-"),
          });
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

      if (lineHasContent && remainingWidth < reservedWidth - 0.01 && !canDropLeadingCollapsedSpaceAtWrap) {
        cursor = null;
        break;
      }

      const availableWidth = Math.max(
        1,
          remainingWidth -
          reservedWidth -
          currentLineWhitespaceContinuationGuardPx -
          currentLineStyledSeamGuardPx -
          currentLineStyledAfterInlineCodeClusterGuardPx -
          currentLineStyledBodyStartGuardPx -
          currentLineInlineCodeTailTextSeamGuardPx -
          currentLineInlineCodeSoftBreakTextStartGuardPx,
      );
      const availableWidthWithoutLeadingSpace: number = canDropLeadingCollapsedSpaceAtWrap
        ? Math.max(
            1,
            remainingWidth -
              currentLineWhitespaceContinuationGuardPx -
              currentLineStyledSeamGuardPx -
              currentLineStyledAfterInlineCodeClusterGuardPx -
              currentLineStyledBodyStartGuardPx -
              currentLineInlineCodeTailTextSeamGuardPx -
              currentLineInlineCodeSoftBreakTextStartGuardPx,
          )
        : availableWidth;
      const codeSegmentAvailableWidth =
        codeGroupId != null && !item.isSealedInlineCodeFragment
          ? Math.max(1, availableWidth + currentLineFitSlackPx + currentLineNearFitLeadingHangPx)
          : availableWidth;
      if (
        debugInlineCode &&
        codeGroupId != null &&
        !item.isSealedInlineCodeFragment &&
        cursor === null &&
        lineHasContent &&
        lastAcceptedCodeGroupId === codeGroupId
      ) {
        debugContinuationDecisions.push({
          text: item.text,
          reservedWidth,
          remainingWidth,
          guardedRemainingWidth,
          availableLineWidth: codeSegmentAvailableWidth,
          fullWidth: item.fullWidth,
          currentLineFitSlackPx,
          lineLastCodeFragmentText,
          lineLastCodeFragmentEndedWithPathDelimiter,
          lineLastCodeFragmentEndedWithHyphen,
        });
      }
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
        item.startsAfterInlineCodeSeam &&
        !lineStartedWithContinuedCode &&
        item.minStartTextWidth > availableWidth + 0.01 &&
        item.minStartTextWidth <= maxWidth + 0.01
      ) {
        cursor = null;
        break;
      }
      if (
        codeGroupId == null &&
        lineHasContent &&
        cursor === null &&
        item.startsAfterStyledTextSeam &&
        item.minStartTextWidth > availableWidth + 0.01 &&
        item.minStartTextWidth <= maxWidth + 0.01
      ) {
        cursor = null;
        break;
      }
      if (
        codeGroupId == null &&
        lineHasContent &&
        cursor === null &&
        pendingSpaceWidth > 0 &&
        item.minStartTextWidth > availableWidthWithoutLeadingSpace + 0.01 &&
        item.minStartTextWidth <= maxWidth + 0.01
      ) {
        cursor = null;
        break;
      }
      if (
        codeGroupId == null &&
        lineHasContent &&
        cursor === null &&
        item.startsStyledTextAfterInlineCodeSeam &&
        item.hasTrailingInlineCode &&
        item.minStartTextWidth > availableWidth + 0.01 &&
        item.minStartTextWidth <= maxWidth + 0.01
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
          styledStartLine.width / Math.max(1, item.fullWidth) <
            STYLED_TEXT_BODY_START_CURRENT_LINE_RATIO_THRESHOLD)
      ) {
        cursor = null;
        break;
      }
      const allowWholeSegmentFastPath =
        codeGroupId == null &&
        cursor === null &&
        !item.startsAfterStyledTextSeam &&
        !item.startsAfterInlineCodeSeam;
      if (allowWholeSegmentFastPath && item.fullWidth <= availableWidth + 0.01) {
        const lineWasEmpty = !lineHasContent;
        remainingWidth = Math.max(0, remainingWidth - reservedWidth - item.fullWidth);
        lineOnlyCodeGroupId = null;
        lastAcceptedCodeGroupId = null;
        lineCurrentCodeGroupStartFragmentText = null;
        lineCurrentCodeGroupStartedNearFresh = false;
        lineCurrentCodeGroupLimitToFirstFragment = false;
        lineLastCodeFragmentText = null;
        lineLastCodeFragmentEndedWithHyphen = false;
        lineLastCodeFragmentEndedWithPathDelimiter = false;
        lineHasContent = true;
        if (item.isDecoratedText) {
          lineDecoratedTextSegmentCount += 1;
        }
        lineStartedWithCollapsedSoftBreakPlainText ||= lineWasEmpty && item.startsAfterCollapsedSoftBreak;
        lineTailAfterInlineCodeIsPunctuationOnly =
          item.startsAfterInlineCodeSeam && isPunctuationOnlySeamText(item.text);
        lineAcceptedPlainAfterContinuedCode ||= codeGroupId == null && lineStartedWithContinuedCode;
        lineAcceptedSoftBreakProseAfterInlineCode ||=
          sliceStartsAtItemStart &&
          shouldApplyInlineCodeSoftBreakTextStartGuard({
            text: item.text,
            startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
            startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
            startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
            startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
            lastFragmentEndedWithPathDelimiter: lineLastCodeFragmentEndedWithPathDelimiter,
            lastFragmentEndedWithHyphen: lineLastCodeFragmentEndedWithHyphen,
          });
        if (
          sliceStartsAtItemStart &&
          shouldApplyInlineCodeSoftBreakTextStartGuard({
            text: item.text,
            startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
            startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
            startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
            startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
            lastFragmentEndedWithPathDelimiter: lineLastCodeFragmentEndedWithPathDelimiter,
            lastFragmentEndedWithHyphen: lineLastCodeFragmentEndedWithHyphen,
          })
        ) {
          lineSoftBreakProseAfterInlineCodeGuardPx = Math.max(
            lineSoftBreakProseAfterInlineCodeGuardPx,
            resolveInlineCodeSoftBreakTextStartGuardPx(item.minStartTextWidth),
          );
        }
        pendingSpaceWidth = 0;
        if (debugInlineCode) {
          debugLine += item.text;
        }
        itemIndex += 1;
        continue;
      }
      const lineWithReservedSpace: LayoutLine | null =
        styledStartLine ?? layoutNextLine(item.prepared, startCursor, codeSegmentAvailableWidth);
      const lineWithoutLeadingSpace: LayoutLine | null =
        canDropLeadingCollapsedSpaceAtWrap && availableWidthWithoutLeadingSpace > availableWidth + 0.01
          ? layoutNextLine(item.prepared, startCursor, availableWidthWithoutLeadingSpace)
          : null;
      const useLineWithoutLeadingSpace: boolean =
        lineWithoutLeadingSpace != null &&
        !cursorsMatch(startCursor, lineWithoutLeadingSpace.end) &&
        (lineWithReservedSpace == null ||
          cursorsMatch(startCursor, lineWithReservedSpace.end) ||
          lineWithoutLeadingSpace.width > lineWithReservedSpace.width + 0.01);
      const line: LayoutLine | null = useLineWithoutLeadingSpace ? lineWithoutLeadingSpace : lineWithReservedSpace;
      if (
        debugInlineCode &&
        codeGroupId != null &&
        !item.isSealedInlineCodeFragment &&
        cursor === null &&
        lineHasContent &&
        lastAcceptedCodeGroupId === codeGroupId
      ) {
        debugContinuationDecisions.push({
          text: item.text,
          reservedWidth,
          remainingWidth,
          guardedRemainingWidth,
          availableLineWidth: codeSegmentAvailableWidth,
          lineWidth: line?.width,
          fullWidth: item.fullWidth,
          currentLineFitSlackPx,
          lineLastCodeFragmentText,
          lineLastCodeFragmentEndedWithPathDelimiter,
          lineLastCodeFragmentEndedWithHyphen,
          acceptedWholeFragment: line != null && cursorsMatch(line.end, item.endCursor),
          brokeBeforeFragment: line == null || cursorsMatch(startCursor, line.end),
        });
      }
      if (line == null || cursorsMatch(startCursor, line.end)) {
        if (!lineHasContent) {
          if (codeGroupId == null && item.startsAfterCollapsedSoftBreak) {
            const advancedCursor = advancePreparedCursorOneGrapheme(item.prepared, startCursor);
            if (advancedCursor != null) {
              if (debugInlineCode) {
                debugSegmentSeamAdjustments.push({
                  type: "no-progress-advance",
                  lineHasContent,
                  text: item.text,
                });
              }
              cursor = advancedCursor;
              continue;
            }
          }
          if (debugInlineCode) {
            debugSegmentSeamAdjustments.push({
              type: "no-progress-drop",
              lineHasContent,
              text: item.text,
            });
          }
          itemIndex += 1;
        }
        cursor = null;
        break;
      }
      const lineSegmentText = slicePreparedTextBetweenCursors(item.prepared, startCursor, line.end);
      if (
        codeGroupId == null &&
        item.startsAfterCollapsedSoftBreak &&
        lineHasContent &&
        !cursorsMatch(line.end, item.endCursor) &&
        isWhitespaceOnlyLineSlice(lineSegmentText)
      ) {
        if (debugInlineCode) {
          debugSegmentSeamAdjustments.push({
            type: "whitespace-only-break",
            lineHasContent,
            text: item.text,
          });
        }
        cursor = null;
        break;
      }
      if (
        codeGroupId == null &&
        item.startsAfterCollapsedSoftBreak &&
        !lineHasContent &&
        !cursorsMatch(line.end, item.endCursor) &&
        isWhitespaceOnlyLineSlice(lineSegmentText)
      ) {
        if (debugInlineCode) {
          debugSegmentSeamAdjustments.push({
            type: "whitespace-only-advance",
            lineHasContent,
            text: item.text,
          });
        }
        cursor = line.end;
        continue;
      }
      if (
        codeGroupId == null &&
        lineHasContent &&
        cursor === null &&
        !cursorsMatch(line.end, item.endCursor) &&
        isAtomicNonCodeTextSegment(item.text) &&
        item.fullWidth <= maxWidth + 0.01
      ) {
        cursor = null;
        break;
      }

      const consumedReservedWidth = useLineWithoutLeadingSpace ? 0 : reservedWidth;
      const lineWasEmpty = !lineHasContent;
      remainingWidth = Math.max(0, remainingWidth - consumedReservedWidth - line.width);
      if (!lineHasContent) {
        lineOnlyCodeGroupId = codeGroupId;
        lineLastCodeFragmentEndedWithHyphen =
          codeGroupId != null && cursorsMatch(line.end, item.endCursor) && item.text.endsWith("-");
        lineLastCodeFragmentEndedWithPathDelimiter =
          codeGroupId != null && cursorsMatch(line.end, item.endCursor) && /[\\/]+$/.test(item.text);
        lineStartedWithContinuedCode =
          codeGroupId != null && (!item.isFirstCodeGroupFragment || !cursorsMatch(startCursor, LINE_START_CURSOR));
        lineSawInlineCode = codeGroupId != null;
      } else if (lineOnlyCodeGroupId !== codeGroupId) {
        lineOnlyCodeGroupId = null;
      }
      lineLastCodeFragmentEndedWithHyphen =
        codeGroupId != null && cursorsMatch(line.end, item.endCursor) && item.text.endsWith("-");
      lineLastCodeFragmentEndedWithPathDelimiter =
        codeGroupId != null && cursorsMatch(line.end, item.endCursor) && /[\\/]+$/.test(item.text);
      if (codeGroupId != null && lastAcceptedCodeGroupId !== codeGroupId) {
        lineCurrentCodeGroupStartFragmentText = lineSegmentText;
      }
      lineLastCodeFragmentText = codeGroupId != null ? lineSegmentText : null;
      lastAcceptedCodeGroupId = codeGroupId;
      lineHasContent = true;
      if (codeGroupId != null) {
        chargedCodeGroups.add(codeGroupId);
        if (
          lineAcceptedSoftBreakProseAfterInlineCode &&
          lineSoftBreakProseGuardCodeGroupId == null
        ) {
          lineSoftBreakProseGuardCodeGroupId = codeGroupId;
        }
        lineSawInlineCode = true;
        lineTailAfterInlineCodeIsPunctuationOnly = false;
      } else {
        lastAcceptedCodeGroupId = null;
        lineLastCodeFragmentEndedWithHyphen = false;
        lineLastCodeFragmentEndedWithPathDelimiter = false;
        lineStartedWithCollapsedSoftBreakPlainText ||= lineWasEmpty && item.startsAfterCollapsedSoftBreak;
        if (item.isDecoratedText) {
          lineDecoratedTextSegmentCount += 1;
        }
        lineTailAfterInlineCodeIsPunctuationOnly =
          item.startsAfterInlineCodeSeam && isPunctuationOnlySeamText(item.text);
        lineAcceptedPlainAfterContinuedCode ||= lineStartedWithContinuedCode;
        lineAcceptedSoftBreakProseAfterInlineCode ||=
          sliceStartsAtItemStart &&
          shouldApplyInlineCodeSoftBreakTextStartGuard({
            text: item.text,
            startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
            startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
            startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
            startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
            lastFragmentEndedWithPathDelimiter: lineLastCodeFragmentEndedWithPathDelimiter,
            lastFragmentEndedWithHyphen: lineLastCodeFragmentEndedWithHyphen,
          });
        if (
          sliceStartsAtItemStart &&
          shouldApplyInlineCodeSoftBreakTextStartGuard({
            text: item.text,
            startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
            startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
            startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
            startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
            lastFragmentEndedWithPathDelimiter: lineLastCodeFragmentEndedWithPathDelimiter,
            lastFragmentEndedWithHyphen: lineLastCodeFragmentEndedWithHyphen,
          })
        ) {
          lineSoftBreakProseAfterInlineCodeGuardPx = Math.max(
            lineSoftBreakProseAfterInlineCodeGuardPx,
            resolveInlineCodeSoftBreakTextStartGuardPx(item.minStartTextWidth),
          );
        }
      }
      pendingSpaceWidth = 0;
      if (debugInlineCode) {
        const segmentText = lineSegmentText;
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

    if (lineHasContent && cursor === null && items[itemIndex]?.kind === "hardBreak") {
      itemIndex += 1;
      forcedBreak = true;
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
      whitespaceDecisions: debugWhitespaceDecisions,
      continuationDecisions: debugContinuationDecisions,
      sealedContinuationDecisions: debugSealedContinuationDecisions,
      segmentSeamAdjustments: debugSegmentSeamAdjustments,
      items: items.map((item) => {
        if (item.kind === "segment") {
          return {
            kind: item.kind,
            codeGroupHasDottedPath: item.codeGroupHasDottedPath,
            text: item.text,
            chromeWidth: item.chromeWidth,
            fullWidth: item.fullWidth,
            minStartTextWidth: item.minStartTextWidth,
            codeGroupHasTrailingText: item.codeGroupHasTrailingText,
            codeGroupStartsAfterText: item.codeGroupStartsAfterText,
            codeGroupStartsAfterStyledTextSeam: item.codeGroupStartsAfterStyledTextSeam,
            isFirstCodeGroupFragment: item.isFirstCodeGroupFragment,
            isSealedInlineCodeFragment: item.isSealedInlineCodeFragment,
            startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
            startsAfterStyledTextSeam: item.startsAfterStyledTextSeam,
            startsAfterInlineCodeSeam: item.startsAfterInlineCodeSeam,
            startsStyledTextAfterBodySeam: item.startsStyledTextAfterBodySeam,
            startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
            startsAfterPathLikeInlineCodeSeam: item.startsAfterPathLikeInlineCodeSeam,
            startsStyledTextAfterInlineCodeSeam: item.startsStyledTextAfterInlineCodeSeam,
            hasTrailingInlineCode: item.hasTrailingInlineCode,
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
