import { layoutNextLine } from "@chenglou/pretext";
import { isSealedInlineCodeFragment } from "../../utils/inlineCodeFragments";
import type { PreparedInlineLayoutItem } from "./sessionMarkdownInlineLayout";
import { LINE_START_CURSOR, cursorsMatch } from "./sessionMarkdownMeasurementCore";

type InlineSegmentItem = Extract<PreparedInlineLayoutItem, { kind: "segment" }>;

export type InlineCodeBoundaryFit = {
  consumedWidth: number;
  endedAtGroupEnd: boolean;
  endedInsideFragment: boolean;
  lastFragmentText: string | null;
  nextFragmentText: string | null;
  nextStartsAfterCodeWhitespace: boolean;
};

export const browserAllowsInlineCodeLeadingHang = (): boolean => {
  if (typeof navigator === "undefined") {
    return true;
  }
  const userAgent = navigator.userAgent;
  return /HeadlessChrome|Chrome\/|Chromium\/|Edg\//.test(userAgent) || /jsdom/i.test(userAgent);
};

const INLINE_CODE_ENGINE_PROSE_START_FOLLOWING_FRAGMENT_SLACK_PX = 16;

export function resolveInlineCodeWrapChromeWidth(params: {
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
  // Chromium lets one edge of the outer <code> chip hang on the first visual
  // slice of a continuous path-like code group when that slice starts after
  // prose on the same line. Continuation lines, standalone path tokens, non-
  // path code chips, and inline code with internal whitespace still consume
  // full chrome.
  if (
    browserAllowsInlineCodeLeadingHang() &&
    params.codeGroupStartsAfterText &&
    params.isFirstCodeGroupFragment &&
    params.prefersFreshLineStart &&
    !params.codeGroupHasWhitespace
  ) {
    return params.chromeWidth / 2;
  }
  return params.chromeWidth;
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

  const measureAttachedTrailingPlainWidth = (startIndex: number): number => {
    const nextItem = items[startIndex + 1];
    return nextItem?.kind === "segment" &&
      nextItem.codeGroupId == null &&
      !/\s/.test(nextItem.text)
      ? nextItem.fullWidth
      : 0;
  };

  const measureCodeGroupTrailingPlainWidth = (startIndex: number): number => {
    const firstItem = items[startIndex];
    if (firstItem?.kind !== "segment" || firstItem.codeGroupId == null) {
      return 0;
    }
    const codeGroupId = firstItem.codeGroupId;
    for (let index = startIndex + 1; index < items.length; index += 1) {
      const item = items[index]!;
      if (item.kind === "hardBreak") {
        return 0;
      }
      if (item.kind === "space") {
        if (item.codeGroupId === codeGroupId) {
          continue;
        }
        return 0;
      }
      if (item.codeGroupId === codeGroupId) {
        continue;
      }
      return item.codeGroupId == null && !/\s/.test(item.text) ? item.fullWidth : 0;
    }
    return 0;
  };

  const shouldPreserveSealedInlineCodeBoundary = (params: {
    sealedBoundary: boolean;
    reservedWidth: number;
    remainingWidth: number;
    item: InlineSegmentItem;
  }): boolean =>
    params.sealedBoundary &&
    params.reservedWidth + params.item.fullWidth > params.remainingWidth + 0.01;

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
          startsAtLineStart: true,
        });
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
          startsAtLineStart: true,
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
        (lineHasContent ? pendingSpaceWidth : 0) +
        resolveInlineCodeWrapChromeWidth({
          chromeWidth: item.chromeWidth,
          codeGroupHasWhitespace: item.codeGroupHasWhitespace,
          codeGroupStartsAfterText: item.codeGroupStartsAfterText,
          chargedChrome,
          isFirstCodeGroupFragment: item.isFirstCodeGroupFragment,
          prefersFreshLineStart: item.prefersFreshLineStart,
          lineHasContent,
          startsAtLineStart: true,
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
        const continuationFit = measureCodeGroupFitWithinWidth(index, remainingWidth, false);
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
          chromeWidth: item.chromeWidth,
          codeGroupHasWhitespace: item.codeGroupHasWhitespace,
          codeGroupStartsAfterText: item.codeGroupStartsAfterText,
          chargedChrome,
          isFirstCodeGroupFragment: item.isFirstCodeGroupFragment,
          prefersFreshLineStart: item.prefersFreshLineStart,
          lineHasContent,
          startsAtLineStart,
        });
      if (
        lineHasContent &&
        !startsAtLineStart &&
        !browserAllowsInlineCodeLeadingHang() &&
        !firstItem.codeGroupHasWhitespace &&
        firstItem.isFirstCodeGroupFragment &&
        firstItem.codeGroupStartsAfterText &&
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
      if (
        lineHasContent &&
        shouldPreserveSealedInlineCodeBoundary({
          sealedBoundary: lastFragmentText != null && isSealedInlineCodeFragment(lastFragmentText),
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
    measureCodeGroupFitWithinWidth,
    preferredCodeGroupStartWidths,
    shouldPreserveSealedInlineCodeBoundary,
    wholeCodeGroupInlineWidths,
  };
}
