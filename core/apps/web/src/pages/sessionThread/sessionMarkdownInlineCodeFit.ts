import { layoutNextLine } from "@chenglou/pretext";
import { isSealedInlineCodeFragment } from "../../utils/inlineCodeFragments";
import { browserAllowsInlineCodeLeadingHang } from "./sessionMarkdownBrowserProfile";
import type { PreparedInlineLayoutItem } from "./sessionMarkdownInlineLayout";
import { LINE_START_CURSOR, cursorsMatch } from "./sessionMarkdownMeasurementCore";

type InlineSegmentItem = Extract<PreparedInlineLayoutItem, { kind: "segment" }>;
type InlineContinuationSlackItem = Pick<
  InlineSegmentItem,
  | "codeGroupHasDottedPath"
  | "codeGroupHasTrailingText"
  | "codeGroupStartsAfterText"
  | "codeGroupStartsAfterStyledTextSeam"
  | "isPathTailFragment"
  | "isSealedInlineCodeFragment"
  | "text"
>;

export type InlineCodeBoundaryFit = {
  consumedWidth: number;
  endedAtGroupEnd: boolean;
  endedInsideFragment: boolean;
  lastFragmentText: string | null;
  nextFragmentText: string | null;
  nextStartsAfterCodeWhitespace: boolean;
};

export type InlineCodeTrailingPlainInfo = {
  width: number;
  text: string;
  hasFollowingInlineCode: boolean;
  isDecoratedText: boolean;
  startsAfterCollapsedSoftBreak: boolean;
};

export function isShortExtensionPathLikeFragment(text: string | null | undefined): boolean {
  if (typeof text !== "string") {
    return false;
  }
  if (!/[\\/]$/.test(text) || text.includes(".") || /\s/.test(text)) {
    return false;
  }
  return Array.from(text).length <= 16;
}

const INLINE_CODE_ENGINE_PROSE_START_FOLLOWING_FRAGMENT_SLACK_PX = 16;
const INLINE_CODE_CONTINUATION_FIT_SLACK_PX = 5;
const INLINE_CODE_PATH_TAIL_CONTINUATION_FIT_SLACK_PX = 1;
export const INLINE_CODE_PATH_DELIMITER_CONTINUATION_MIN_SPARE_PX = 1;
export const INLINE_CODE_DOTTED_CALL_CONTINUATION_MIN_SPARE_PX = 4;
const INLINE_CODE_COLON_COMMAND_FRAGMENT_SLACK_PX = 1;
const INLINE_CODE_PROSE_START_SEAM_GUARD_PX = 4;
const INLINE_CODE_STANDALONE_HYPHEN_FRAGMENT_SLACK_PX = 2;
const DOTTED_CALL_SPARE_MIN_STEM_GRAPHEMES = 12;

export function allowsChromiumDottedBoundaryHang(params: {
  boundaryRemainingWidth: number;
  chromeWidth: number;
  fullWidth: number;
}): boolean {
  // Chromium's cloned inline-code decoration can effectively hang one chip edge
  // at a dotted path boundary, but not arbitrary text. Keep the allowance capped
  // to the code chrome so this cannot mask real wrapping pressure.
  return params.fullWidth <= params.boundaryRemainingWidth + params.chromeWidth + 0.01;
}

export function shouldApplyInlineCodeSoftBreakTextStartGuard(params: {
  text: string;
  startsAfterCollapsedSoftBreak: boolean;
  startsAfterPathLikeInlineCodeSeam: boolean;
  startsAfterInlineCodeSeam: boolean;
  startsStyledTextAfterInlineCodeSeam: boolean;
  lastFragmentEndedWithPathDelimiter: boolean;
  lastFragmentEndedWithHyphen: boolean;
}): boolean {
  const startsAtSoftBreakPathSeam =
    params.startsAfterCollapsedSoftBreak &&
    params.startsAfterPathLikeInlineCodeSeam &&
    (params.startsAfterInlineCodeSeam || params.startsStyledTextAfterInlineCodeSeam);
  if (!startsAtSoftBreakPathSeam) {
    return false;
  }
  return params.lastFragmentEndedWithPathDelimiter || params.lastFragmentEndedWithHyphen;
}

export function shouldBreakBeforeWhitespaceSeparatedInlineCodeFragment(params: {
  lineHasContent: boolean;
  startsAfterCodeWhitespace: boolean;
  reservedWidth: number;
  remainingWidth: number;
  fragmentWidth: number;
  slackPx?: number;
}): boolean {
  return (
    params.lineHasContent &&
    params.startsAfterCodeWhitespace &&
    params.reservedWidth + params.fragmentWidth > params.remainingWidth + (params.slackPx ?? 0) + 0.01
  );
}

export function shouldBreakBeforePathDelimiterNearFitContinuation(params: {
  lineHasContent: boolean;
  sameCodeGroupContinuation: boolean;
  lastFragmentEndedWithPathDelimiter: boolean;
  startsAfterCodeWhitespace: boolean;
  isSealedInlineCodeFragment: boolean;
  reservedWidth: number;
  remainingWidth: number;
  fragmentWidth: number;
  slackPx?: number;
}): boolean {
  return (
    params.lineHasContent &&
    params.sameCodeGroupContinuation &&
    params.lastFragmentEndedWithPathDelimiter &&
    !params.startsAfterCodeWhitespace &&
    !params.isSealedInlineCodeFragment &&
    params.reservedWidth + params.fragmentWidth + INLINE_CODE_PATH_DELIMITER_CONTINUATION_MIN_SPARE_PX >
      params.remainingWidth + (params.slackPx ?? 0) + 0.01
  );
}

export function resolveInlineCodeWhitespaceSeparatedFragmentSlackPx(params: {
  lineHasContent: boolean;
  startsAfterCodeWhitespace: boolean;
  fragmentText: string;
}): number {
  if (
    browserAllowsInlineCodeLeadingHang() &&
    params.lineHasContent &&
    params.startsAfterCodeWhitespace &&
    params.fragmentText !== "-" &&
    params.fragmentText.endsWith("-") &&
    !params.fragmentText.includes("/") &&
    !params.fragmentText.includes("\\") &&
    !/\s/.test(params.fragmentText)
  ) {
    return INLINE_CODE_CONTINUATION_FIT_SLACK_PX;
  }
  if (
    !browserAllowsInlineCodeLeadingHang() &&
    params.lineHasContent &&
    params.startsAfterCodeWhitespace &&
    params.fragmentText.includes(":") &&
    !params.fragmentText.includes("/") &&
    !/\s/.test(params.fragmentText)
  ) {
    return INLINE_CODE_COLON_COMMAND_FRAGMENT_SLACK_PX;
  }
  return browserAllowsInlineCodeLeadingHang() &&
    params.lineHasContent &&
    params.startsAfterCodeWhitespace &&
    params.fragmentText === "-"
    ? INLINE_CODE_STANDALONE_HYPHEN_FRAGMENT_SLACK_PX
    : 0;
}

export function resolveInlineCodeContinuationFitSlackPx(params: {
  lineHasContent: boolean;
  atLineBreakBoundary: boolean;
  sameCodeGroupContinuation: boolean;
  startsAfterCodeWhitespace: boolean;
  lastFragmentEndedWithDot: boolean;
  lastFragmentEndedWithHyphen: boolean;
  lastFragmentEndedWithPathDelimiter: boolean;
  item: InlineContinuationSlackItem;
}): number {
  const shouldDisableChromiumNonDelimitedPathTailSlack =
    browserAllowsInlineCodeLeadingHang() &&
    params.item.isPathTailFragment &&
    !params.lastFragmentEndedWithPathDelimiter;
  const shouldDisableForEngine =
    shouldDisableChromiumNonDelimitedPathTailSlack ||
    !browserAllowsInlineCodeLeadingHang() &&
    (params.item.isPathTailFragment ||
      params.item.codeGroupHasDottedPath ||
      params.item.isSealedInlineCodeFragment ||
      params.item.text.includes("/") ||
      params.item.text.includes("\\"));
  if (
    !params.lineHasContent ||
    !params.atLineBreakBoundary ||
    !params.sameCodeGroupContinuation ||
    params.startsAfterCodeWhitespace ||
    (params.lastFragmentEndedWithDot &&
      !params.item.text.includes(".") &&
      !params.item.text.includes("/") &&
      !params.item.text.includes("\\") &&
      !params.item.isPathTailFragment) ||
    params.lastFragmentEndedWithHyphen ||
    (params.lastFragmentEndedWithPathDelimiter && params.item.codeGroupStartsAfterStyledTextSeam) ||
    (params.lastFragmentEndedWithPathDelimiter &&
      params.item.codeGroupStartsAfterText &&
      params.item.codeGroupHasTrailingText) ||
    (params.lastFragmentEndedWithPathDelimiter && !browserAllowsInlineCodeLeadingHang()) ||
    shouldDisableForEngine
  ) {
    return 0;
  }
  const chromiumPathTailSlackPx =
    browserAllowsInlineCodeLeadingHang() &&
    params.lastFragmentEndedWithPathDelimiter &&
    !params.item.isSealedInlineCodeFragment
      ? INLINE_CODE_PATH_TAIL_CONTINUATION_FIT_SLACK_PX
      : 0;
  // Chromium sometimes keeps a short terminal path tail on the current line
  // after a slash boundary, but that tolerance is much smaller than the
  // generic same-group continuation slack.
  if (
    chromiumPathTailSlackPx > 0 &&
    !params.item.isSealedInlineCodeFragment
  ) {
    return chromiumPathTailSlackPx;
  }
  return INLINE_CODE_CONTINUATION_FIT_SLACK_PX + chromiumPathTailSlackPx;
}

export function resolveInlineCodeProseStartSeamGuardPx(params: {
  startsAtLineStart: boolean;
  item: InlineContinuationSlackItem & Pick<InlineSegmentItem, "codeGroupHasWhitespace" | "codeGroupStartsAfterText" | "isFirstCodeGroupFragment" | "prefersFreshLineStart">;
}): number {
  if (
    params.startsAtLineStart ||
    !browserAllowsInlineCodeLeadingHang() ||
    !params.item.isFirstCodeGroupFragment ||
    !params.item.codeGroupStartsAfterText ||
    !params.item.prefersFreshLineStart ||
    params.item.codeGroupHasWhitespace
  ) {
    return 0;
  }
  return INLINE_CODE_PROSE_START_SEAM_GUARD_PX;
}

export function resolveInlineCodeWrapChromeWidth(params: {
  allowLeadingHang?: boolean;
  chromeWidth: number;
  codeGroupHasWhitespace: boolean;
  codeGroupStartsAfterText: boolean;
  chargedChrome: boolean;
  isFirstCodeGroupFragment: boolean;
  prefersFreshLineStart: boolean;
  lineHasContent: boolean;
  startsAtLineStart: boolean;
}): number {
  if (params.chargedChrome) {
    return 0;
  }
  // Chromium continuation lines of path-like inline code still keep a visible
  // chip edge, but they do not consume the full leading chrome width of a
  // brand-new chip. Charging half the edge matches the wrapped DOM more
  // closely than treating every continuation line like a fresh code start.
  if (
    browserAllowsInlineCodeLeadingHang() &&
    params.startsAtLineStart &&
    !params.isFirstCodeGroupFragment &&
    params.codeGroupStartsAfterText &&
    !params.codeGroupHasWhitespace
  ) {
    return params.chromeWidth / 2;
  }
  // Chromium lets one edge of the outer <code> chip hang on the first visual
  // slice of a continuous path-like code group when that slice starts after
  // prose on the same line. We only opt into that discount from the current-
  // line fit path when the full code group does not fit inline; preferred-start
  // and whole-group widths still charge full chrome so near-threshold prose
  // math stays conservative.
  if (
    (params.allowLeadingHang ?? false) &&
    browserAllowsInlineCodeLeadingHang() &&
    !params.startsAtLineStart &&
    params.codeGroupStartsAfterText &&
    params.isFirstCodeGroupFragment &&
    (params.prefersFreshLineStart || params.codeGroupHasWhitespace)
  ) {
    return params.chromeWidth / 2;
  }
  return params.chromeWidth;
}

export function shouldBreakBeforePartialSealedDottedPathContinuation(params: {
  currentCodeGroupStartFragmentText: string | null;
  fullWidth: number;
  guardedRemainingWidth: number;
  item: Pick<InlineSegmentItem, "codeGroupHasDottedPath" | "codeGroupStartsAfterText" | "isPathTailFragment" | "text">;
  lastFragmentText: string | null;
  sameCodeGroupContinuation: boolean;
}): boolean {
  return (
    params.sameCodeGroupContinuation &&
    params.currentCodeGroupStartFragmentText != null &&
    params.currentCodeGroupStartFragmentText === params.lastFragmentText &&
    isShortExtensionPathLikeFragment(params.lastFragmentText) &&
    params.item.codeGroupStartsAfterText &&
    params.item.codeGroupHasDottedPath &&
    !params.item.text.includes("/") &&
    !params.item.text.includes("\\") &&
    (params.item.text.includes(".") || params.item.isPathTailFragment) &&
    params.fullWidth > params.guardedRemainingWidth + 0.01
  );
}

export function shouldBreakBeforePartialDottedStemPathTailContinuation(params: {
  fullWidth: number;
  guardedRemainingWidth: number;
  item: Pick<InlineSegmentItem, "codeGroupHasDottedPath" | "codeGroupStartsAfterText" | "text">;
  lastFragmentText: string | null;
  sameCodeGroupContinuation: boolean;
}): boolean {
  return (
    params.sameCodeGroupContinuation &&
    params.item.codeGroupStartsAfterText &&
    params.item.codeGroupHasDottedPath &&
    /[\\/]/.test(params.item.text) &&
    /\.$/.test(params.lastFragmentText ?? "") &&
    params.fullWidth > params.guardedRemainingWidth + 0.01
  );
}

export function shouldBreakBeforePartialDottedCallContinuation(params: {
  fragmentWidth: number;
  fullWidth: number;
  guardedRemainingWidth: number;
  item: Pick<InlineSegmentItem, "isPathTailFragment" | "isSealedInlineCodeFragment" | "text">;
  lastFragmentText: string | null;
  maxWidth: number;
  minSparePx?: number;
  sameCodeGroupContinuation: boolean;
}): boolean {
  const dottedStem = (params.lastFragmentText ?? "").replace(/\.$/, "");
  const shouldReserveDottedSpare =
    /^[\p{L}_]/u.test(dottedStem) &&
    Array.from(dottedStem).length >= DOTTED_CALL_SPARE_MIN_STEM_GRAPHEMES;
  const minSparePx = shouldReserveDottedSpare ? Math.max(0, params.minSparePx ?? 0) : 0;
  return (
    params.sameCodeGroupContinuation &&
    /\.$/.test(params.lastFragmentText ?? "") &&
    !params.item.isSealedInlineCodeFragment &&
    !params.item.isPathTailFragment &&
    !params.item.text.includes(".") &&
    !params.item.text.includes("/") &&
    !params.item.text.includes("\\") &&
    params.fullWidth + minSparePx > params.guardedRemainingWidth + 0.01 &&
    params.fragmentWidth <= params.maxWidth + 0.01
  );
}

export function createInlineCodeFitPlanner(params: {
  items: readonly PreparedInlineLayoutItem[];
  maxWidth: number;
}) {
  const { items, maxWidth } = params;
  const preferredCodeGroupStartWidths = new Map<number, number>();
  const dottedPathCodeGroupStartClusterWidths = new Map<number, number>();
  const wholeCodeGroupInlineWidths = new Map<number, number>();

  const isPathLikeContinuationItem = (item: InlineSegmentItem): boolean =>
    item.text.includes("/") || item.text.includes("\\") || item.isPathTailFragment;

  const findTrailingPlainSegmentInfo = (startIndex: number): InlineCodeTrailingPlainInfo => {
    const firstItem = items[startIndex];
    if (firstItem?.kind !== "segment" || firstItem.codeGroupId == null) {
      return {
        width: 0,
        text: "",
        hasFollowingInlineCode: false,
        isDecoratedText: false,
        startsAfterCollapsedSoftBreak: false,
      };
    }
    const codeGroupId = firstItem.codeGroupId;
    for (let index = startIndex + 1; index < items.length; index += 1) {
      const item = items[index]!;
      if (item.kind === "hardBreak") {
        return {
          width: 0,
          text: "",
          hasFollowingInlineCode: false,
          isDecoratedText: false,
          startsAfterCollapsedSoftBreak: false,
        };
      }
      if (item.kind === "space") {
        continue;
      }
      if (item.codeGroupId === codeGroupId) {
        continue;
      }
      if (item.codeGroupId != null) {
        return {
          width: 0,
          text: "",
          hasFollowingInlineCode: false,
          isDecoratedText: false,
          startsAfterCollapsedSoftBreak: false,
        };
      }
      if (item.text.trim().length === 0) {
        continue;
      }
      let hasFollowingInlineCode = false;
      for (let nextIndex = index + 1; nextIndex < items.length; nextIndex += 1) {
        const nextItem = items[nextIndex]!;
        if (nextItem.kind === "hardBreak") {
          break;
        }
        if (nextItem.kind === "space") {
          continue;
        }
        if (nextItem.startsAfterCollapsedSoftBreak) {
          break;
        }
        if (nextItem.codeGroupId != null) {
          hasFollowingInlineCode = true;
          break;
        }
      }
      return {
        width: item.fullWidth,
        text: item.text,
        hasFollowingInlineCode,
        isDecoratedText: item.isDecoratedText,
        startsAfterCollapsedSoftBreak: item.startsAfterCollapsedSoftBreak,
      };
    }
    return {
      width: 0,
      text: "",
      hasFollowingInlineCode: false,
      isDecoratedText: false,
      startsAfterCollapsedSoftBreak: false,
    };
  };

  const measureAttachedTrailingPlainWidth = (startIndex: number): number => {
    return findTrailingPlainSegmentInfo(startIndex).width;
  };

  const measureCodeGroupTrailingPlainWidth = (startIndex: number): number => {
    return measureCodeGroupTrailingPlainInfo(startIndex).width;
  };

  const measureCodeGroupTrailingPlainInfo = (startIndex: number): InlineCodeTrailingPlainInfo => {
    return findTrailingPlainSegmentInfo(startIndex);
  };

  const shouldPreserveSealedInlineCodeBoundary = (params: {
    sealedBoundary: boolean;
    reservedWidth: number;
    remainingWidth: number;
    item: InlineSegmentItem;
    lastFragmentText?: string | null;
  }): boolean => {
    const lastFragmentIsShortExtensionPath = isShortExtensionPathLikeFragment(params.lastFragmentText);
    const allowChromiumDottedPathBoundaryContinuation =
      browserAllowsInlineCodeLeadingHang() &&
      params.item.codeGroupStartsAfterText &&
      !lastFragmentIsShortExtensionPath &&
      params.item.codeGroupHasDottedPath &&
      (params.item.text.includes(".") || params.item.isPathTailFragment);
    return (
      params.sealedBoundary &&
      !allowChromiumDottedPathBoundaryContinuation &&
      params.reservedWidth + params.item.fullWidth > params.remainingWidth + 0.01
    );
  };

  const measurePreferredCodeGroupStartWidth = (startIndex: number): number => {
    const firstItem = items[startIndex];
    if (firstItem?.kind !== "segment" || firstItem.codeGroupId == null) {
      return 0;
    }
    const codeGroupId = firstItem.codeGroupId;
    let lineWidth = 0;
    let remainingWidth = maxWidth;
    let lineHasContent = firstItem.codeGroupStartsAfterText;
    let pendingSpaceWidth = 0;
    let chargedChrome = false;
    let lastFragmentText: string | null = null;

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

      const reservedWidth =
        (lineHasContent ? pendingSpaceWidth : 0) +
        resolveInlineCodeWrapChromeWidth({
          chromeWidth: item.chromeWidth,
          codeGroupHasWhitespace: item.codeGroupHasWhitespace,
          codeGroupStartsAfterText: item.codeGroupStartsAfterText,
          chargedChrome,
          isFirstCodeGroupFragment: item.isFirstCodeGroupFragment,
          prefersFreshLineStart: item.prefersFreshLineStart,
          lineHasContent,
          startsAtLineStart: !lineHasContent,
        });
      const continuationSlackPx = resolveInlineCodeContinuationFitSlackPx({
        lineHasContent,
        atLineBreakBoundary: true,
        sameCodeGroupContinuation: lineHasContent,
        startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
        lastFragmentEndedWithDot: lastFragmentText?.endsWith(".") ?? false,
        lastFragmentEndedWithHyphen: lastFragmentText?.endsWith("-") ?? false,
        lastFragmentEndedWithPathDelimiter: /[\\/]+$/.test(lastFragmentText ?? ""),
        item,
      });
      if (
        shouldBreakBeforeWhitespaceSeparatedInlineCodeFragment({
          lineHasContent,
          startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
          reservedWidth,
          remainingWidth,
          fragmentWidth: item.fullWidth,
          slackPx: resolveInlineCodeWhitespaceSeparatedFragmentSlackPx({
            lineHasContent,
            startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
            fragmentText: item.text,
          }),
        })
      ) {
        break;
      }
      const availableWidth = Math.max(1, remainingWidth - reservedWidth);
      if (item.isSealedInlineCodeFragment) {
        const fullWidth = reservedWidth + item.fullWidth;
        if (lineHasContent && fullWidth > remainingWidth + continuationSlackPx + 0.01) {
          break;
        }
        const overflowed = fullWidth > remainingWidth + 0.01;
        lineWidth += fullWidth;
        remainingWidth = overflowed ? 0 : Math.max(0, remainingWidth - fullWidth);
        chargedChrome = true;
        lineHasContent = true;
        pendingSpaceWidth = 0;
        lastFragmentText = item.text;
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
      lastFragmentText = item.text;
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
    let lineHasContent = firstItem.codeGroupStartsAfterText;
    let chargedChrome = false;
    let sawPathLikeFragment = false;
    let sawDottedStem = false;

    for (let index = startIndex; index < items.length; index += 1) {
      const item = items[index]!;
      if (item.kind === "hardBreak" || item.kind === "space" || item.codeGroupId !== firstItem.codeGroupId) {
        break;
      }

      lineWidth +=
        (lineHasContent ? pendingSpaceWidth : 0) +
        resolveInlineCodeWrapChromeWidth({
          chromeWidth: item.chromeWidth,
          codeGroupHasWhitespace: item.codeGroupHasWhitespace,
          codeGroupStartsAfterText: item.codeGroupStartsAfterText,
          chargedChrome,
          isFirstCodeGroupFragment: item.isFirstCodeGroupFragment,
          prefersFreshLineStart: item.prefersFreshLineStart,
          lineHasContent,
          startsAtLineStart: !lineHasContent,
        }) +
        item.fullWidth;
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
    let lineHasContent = firstItem.codeGroupStartsAfterText;
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
        (lineHasContent ? pendingSpaceWidth : 0) +
        resolveInlineCodeWrapChromeWidth({
          chromeWidth: item.chromeWidth,
          codeGroupHasWhitespace: item.codeGroupHasWhitespace,
          codeGroupStartsAfterText: item.codeGroupStartsAfterText,
          chargedChrome,
          isFirstCodeGroupFragment: item.isFirstCodeGroupFragment,
          prefersFreshLineStart: item.prefersFreshLineStart,
          lineHasContent,
          startsAtLineStart: !lineHasContent,
        }) +
        item.fullWidth;
      lineHasContent = true;
      chargedChrome = true;
      pendingSpaceWidth = 0;
    }

    return lineWidth;
  };

  const measureCodeGroupFitWithinWidth = (
    startIndex: number,
    availableWidth: number,
    startsAtLineStart: boolean,
    allowLeadingHang = true,
  ): InlineCodeBoundaryFit => {
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
    const wholeCodeGroupWidth = wholeCodeGroupInlineWidths.get(startIndex) ?? 0;
    const allowCurrentLineLeadingHang =
      allowLeadingHang &&
      browserAllowsInlineCodeLeadingHang() &&
      !startsAtLineStart &&
      firstItem.isFirstCodeGroupFragment &&
      firstItem.codeGroupStartsAfterText &&
      wholeCodeGroupWidth > availableWidth + 0.01;
    let remainingWidth = Math.max(
      1,
      availableWidth -
        resolveInlineCodeProseStartSeamGuardPx({
          startsAtLineStart,
          item: firstItem,
        }),
    );
    let lineHasContent = !startsAtLineStart;
    let pendingSpaceWidth = 0;
    let chargedChrome = false;
    let lastFragmentText: string | null = null;
    let consumedWidth = 0;
    let usedChromiumDottedPathBoundaryContinuation = false;

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
        const continuationFit = measureCodeGroupFitWithinWidth(index, remainingWidth, false, allowLeadingHang);
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
      const reservedWidth =
        (lineHasContent ? pendingSpaceWidth : 0) +
        resolveInlineCodeWrapChromeWidth({
          allowLeadingHang: allowCurrentLineLeadingHang,
          chromeWidth: item.chromeWidth,
          codeGroupHasWhitespace: item.codeGroupHasWhitespace,
          codeGroupStartsAfterText: item.codeGroupStartsAfterText,
          chargedChrome,
          isFirstCodeGroupFragment: item.isFirstCodeGroupFragment,
          prefersFreshLineStart: item.prefersFreshLineStart,
          lineHasContent,
          startsAtLineStart,
        });
      const continuationSlackPx = resolveInlineCodeContinuationFitSlackPx({
        lineHasContent,
        atLineBreakBoundary: true,
        sameCodeGroupContinuation: lineHasContent,
        startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
        lastFragmentEndedWithDot: lastFragmentText?.endsWith(".") ?? false,
        lastFragmentEndedWithHyphen: lastFragmentText?.endsWith("-") ?? false,
        lastFragmentEndedWithPathDelimiter: /[\\/]+$/.test(lastFragmentText ?? ""),
        item,
      });
      const shouldAcceptChromiumPathTailContinuation =
        browserAllowsInlineCodeLeadingHang() &&
        lineHasContent &&
        /[\\/]+$/.test(lastFragmentText ?? "") &&
        !item.isSealedInlineCodeFragment &&
        !item.startsAfterCodeWhitespace &&
        reservedWidth + item.fullWidth + INLINE_CODE_PATH_DELIMITER_CONTINUATION_MIN_SPARE_PX <=
          remainingWidth + continuationSlackPx + 0.01;
      const nextSameCodeGroupItem = items[index + 1];
      const splitDottedStemTailWouldOverflowCurrentLine =
        availableWidth > maxWidth * 0.85 &&
        lineHasContent &&
        firstItem.codeGroupStartsAfterText &&
        firstItem.codeGroupHasTrailingText &&
        lastFragmentText?.endsWith("/") === true &&
        !firstItem.text.includes(".") &&
        item.text.endsWith(".") &&
        nextSameCodeGroupItem?.kind === "segment" &&
        nextSameCodeGroupItem.codeGroupId === codeGroupId &&
        isPathLikeContinuationItem(nextSameCodeGroupItem) &&
        reservedWidth + item.fullWidth + nextSameCodeGroupItem.fullWidth >
          remainingWidth + continuationSlackPx + 0.01;
      if (splitDottedStemTailWouldOverflowCurrentLine) {
        return {
          consumedWidth,
          endedAtGroupEnd: false,
          endedInsideFragment: false,
          lastFragmentText,
          nextFragmentText: item.text,
          nextStartsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
        };
      }
      if (
        shouldBreakBeforeWhitespaceSeparatedInlineCodeFragment({
          lineHasContent,
          startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
          reservedWidth,
          remainingWidth,
          fragmentWidth: item.fullWidth,
          slackPx: resolveInlineCodeWhitespaceSeparatedFragmentSlackPx({
            lineHasContent,
            startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
            fragmentText: item.text,
          }),
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
      if (
        shouldBreakBeforePathDelimiterNearFitContinuation({
          lineHasContent,
          sameCodeGroupContinuation: lineHasContent,
          lastFragmentEndedWithPathDelimiter: /[\\/]+$/.test(lastFragmentText ?? ""),
          startsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
          isSealedInlineCodeFragment: item.isSealedInlineCodeFragment,
          reservedWidth,
          remainingWidth,
          fragmentWidth: item.fullWidth,
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
      if (
        lineHasContent &&
        !startsAtLineStart &&
        !browserAllowsInlineCodeLeadingHang() &&
        !firstItem.codeGroupHasWhitespace &&
        firstItem.isFirstCodeGroupFragment &&
        firstItem.codeGroupStartsAfterText &&
        !firstItem.codeGroupStartsAfterStyledTextSeam &&
        isPathLikeContinuationItem(item) &&
        Math.max(0, remainingWidth - reservedWidth - item.fullWidth) <
          INLINE_CODE_ENGINE_PROSE_START_FOLLOWING_FRAGMENT_SLACK_PX
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
      const sealedBoundary =
        lineHasContent && lastFragmentText != null && isSealedInlineCodeFragment(lastFragmentText);
      const boundaryRemainingWidth = remainingWidth + continuationSlackPx;
      const sealedBoundaryOverflow =
        sealedBoundary &&
        !shouldAcceptChromiumPathTailContinuation &&
        reservedWidth + item.fullWidth > boundaryRemainingWidth + 0.01;
      const lastFragmentIsShortExtensionPath = isShortExtensionPathLikeFragment(lastFragmentText);
      const canRelaxChromiumDottedPathBoundary =
        sealedBoundaryOverflow &&
        !usedChromiumDottedPathBoundaryContinuation &&
        browserAllowsInlineCodeLeadingHang() &&
        firstItem.codeGroupStartsAfterText &&
        !firstItem.text.includes(".") &&
        !lastFragmentIsShortExtensionPath &&
        item.codeGroupHasDottedPath &&
        !item.text.includes("/") &&
        !item.text.includes("\\") &&
        (item.text.includes(".") || item.isPathTailFragment) &&
        allowsChromiumDottedBoundaryHang({
          boundaryRemainingWidth,
          chromeWidth: item.chromeWidth,
          fullWidth: reservedWidth + item.fullWidth,
        });
      if (sealedBoundaryOverflow && !canRelaxChromiumDottedPathBoundary) {
        return {
          consumedWidth,
          endedAtGroupEnd: false,
          endedInsideFragment: false,
          lastFragmentText,
          nextFragmentText: item.text,
          nextStartsAfterCodeWhitespace: item.startsAfterCodeWhitespace,
        };
      }
      if (canRelaxChromiumDottedPathBoundary) {
        usedChromiumDottedPathBoundaryContinuation = true;
      }
      if (item.isSealedInlineCodeFragment && canRelaxChromiumDottedPathBoundary) {
        remainingWidth = Math.max(0, remainingWidth - reservedWidth - item.fullWidth);
        consumedWidth += reservedWidth + item.fullWidth;
        lineHasContent = true;
        chargedChrome = true;
        pendingSpaceWidth = 0;
        lastFragmentText = item.text;
        continue;
      }
      if (shouldAcceptChromiumPathTailContinuation) {
        remainingWidth = Math.max(0, remainingWidth - reservedWidth - item.fullWidth);
        consumedWidth += reservedWidth + item.fullWidth;
        lineHasContent = true;
        chargedChrome = true;
        pendingSpaceWidth = 0;
        lastFragmentText = item.text;
        continue;
      }

      if (item.isSealedInlineCodeFragment) {
        const fullWidth = reservedWidth + item.fullWidth;
        if (fullWidth > remainingWidth + continuationSlackPx + 0.01) {
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

  const codeGroupFitEndsAtFriendlyBoundary = (fit: InlineCodeBoundaryFit): boolean => {
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

  return {
    codeGroupFitEndsAtFriendlyBoundary,
    dottedPathCodeGroupStartClusterWidths,
    measureAttachedTrailingPlainWidth,
    measureCodeGroupTrailingPlainWidth,
    measureCodeGroupTrailingPlainInfo,
    measureCodeGroupFitWithinWidth,
    preferredCodeGroupStartWidths,
    shouldPreserveSealedInlineCodeBoundary,
    wholeCodeGroupInlineWidths,
  };
}
