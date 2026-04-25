import {
  SESSION_TEXT_MEASUREMENT_CACHE_LIMIT,
  buildPreparedContentKey,
  measureCollapsedSpaceWidth,
  normalizeHeight,
  pruneCache,
  segmentImplicitWordBreaks,
} from "./sessionTextMeasurement";
import {
  SOFT_HYPHEN,
  SOFT_HYPHEN_CURRENT_LINE_FIT_GUARD_PX,
  ZERO_WIDTH_SPACE,
  acceptsAbsolutePathContinuationOnCurrentLine,
  acceptsUrlContinuationOnCurrentLine,
  findLargestCollapsedPlainTextPrefixThatFits,
  isAbsolutePathLikeDelimitedWord,
  isPathLikeDelimitedWord,
  lineEndsWithUrlQueryPair,
  measureImplicitWordBreakFit,
  measurePlainTextDelimitedTokenFit,
  measureSingleLineTextWidth,
  measureSoftHyphenBreakFit,
  measureZeroWidthSpaceBreakFit,
  normalizeCollapsedPlainTextLineText,
  resolvePlainTextDelimitedStartRatioThreshold,
  snapDelimitedUrlContinuationFit,
  splitPlainTextWrapFragments,
  stripDiscretionaryBreakMarkers,
} from "./sessionPlainTextMeasurementWrap";

type SessionPlainTextDebugWindow = Window & {
  __ctxForcePlainTextDebug?: boolean;
  __ctxPlainTextDebugTarget?: string;
  __ctxPlainTextDebugWidth?: number;
  __ctxPlainTextDebug?: {
    lineCount: number;
    lines: string[];
    lineWidths: number[];
    text: string;
    width: number;
  };
};

const plainTextBlockHeightCache = new Map<string, number>();
function measureCollapsedPlainTextLineHeight(params: {
  cacheKey: string;
  text: string;
  font: string;
  width: number;
  lineHeight: number;
}): number {
  const normalizedText = normalizeCollapsedPlainTextLineText(params.text);
  const words = normalizedText.match(/\S+/g) ?? [];
  if (words.length === 0) {
    return params.lineHeight;
  }
  const plainTextDebugWindow =
    typeof window !== "undefined" ? (window as SessionPlainTextDebugWindow) : null;
  const debugPlainText =
    (plainTextDebugWindow?.__ctxForcePlainTextDebug === true ||
      plainTextDebugWindow?.__ctxPlainTextDebugTarget === "*" ||
      plainTextDebugWindow?.__ctxPlainTextDebugTarget === normalizedText) &&
    (plainTextDebugWindow?.__ctxPlainTextDebugWidth == null ||
      plainTextDebugWindow.__ctxPlainTextDebugWidth === params.width);
  const debugLines: string[] = [];
  const debugLineWidths: number[] = [];
  let currentLineText = "";
  const appendLineText = (text: string, prefixSpace: boolean) => {
    if (prefixSpace && currentLineText.length > 0) {
      currentLineText += " ";
    }
    currentLineText += text;
  };
  const flushLine = () => {
    if (debugPlainText && currentLineText.length > 0) {
      debugLines.push(currentLineText);
      debugLineWidths.push(
        measureSingleLineTextWidth({
          cacheKey: buildPreparedContentKey(`${params.cacheKey}:debug-line:${debugLines.length}`, currentLineText),
          text: currentLineText,
          font: params.font,
        }),
      );
    }
    currentLineText = "";
  };

  const maxWidth = Math.max(1, params.width);
  const collapsedSpaceWidth = measureCollapsedSpaceWidth(params.font);
  let lineCount = 1;
  let lineHasContent = false;
  let remainingWidth = maxWidth;
  let wordIndex = 0;
  let remainder: string | null = null;
  let remainderFragments: string[] | null = null;

  while (wordIndex < words.length || remainder != null) {
    const word: string = remainder ?? words[wordIndex]!;
    const displayWord = stripDiscretionaryBreakMarkers(word);
    const wordContainsSoftHyphen = word.includes(SOFT_HYPHEN);
    const wordContainsZeroWidthBreak = word.includes(ZERO_WIDTH_SPACE);
    const wordFragments = remainderFragments ?? splitPlainTextWrapFragments(word);
    const usesDelimitedWrapping = wordFragments.length > 1;
    const implicitWordBreakSegments =
      !usesDelimitedWrapping && !wordContainsSoftHyphen && !wordContainsZeroWidthBreak
        ? segmentImplicitWordBreaks(displayWord)
        : [displayWord];
    const usesImplicitWordBreaking = implicitWordBreakSegments.length > 1;
    const wordWidth = measureSingleLineTextWidth({
      cacheKey: buildPreparedContentKey(`${params.cacheKey}:word:${wordIndex}`, displayWord),
      text: displayWord,
      font: params.font,
    });
    const reservedWidth = lineHasContent ? collapsedSpaceWidth : 0;

    if (lineHasContent && reservedWidth + wordWidth <= remainingWidth + 0.01) {
      remainingWidth = Math.max(0, remainingWidth - reservedWidth - wordWidth);
      appendLineText(displayWord, true);
      remainder = null;
      remainderFragments = null;
      wordIndex += 1;
      continue;
    }

    if (lineHasContent) {
      const availableWidth = Math.max(0, remainingWidth - reservedWidth);
      const wordFitsFreshLine = wordWidth <= maxWidth + 0.01;
      const isUrlLikeWord = word.includes("://");
      const isAbsolutePathLikeWord = isAbsolutePathLikeDelimitedWord(word);
      if (usesDelimitedWrapping) {
        if ((isUrlLikeWord || isAbsolutePathLikeWord || !wordFitsFreshLine) && availableWidth > 0.01) {
          const currentFit = snapDelimitedUrlContinuationFit({
            cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued`,
            word,
            font: params.font,
            fit: measurePlainTextDelimitedTokenFit({
              cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued`,
              text: word,
              fragments: wordFragments,
              font: params.font,
              width: availableWidth,
              allowPartialFragment: true,
              allowPartialAfterConsumedText: true,
            }),
          });
          const freshFit = measurePlainTextDelimitedTokenFit({
            cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:fresh`,
            text: word,
            fragments: wordFragments,
            font: params.font,
            width: maxWidth,
            allowPartialFragment: true,
            allowPartialAfterConsumedText: word.includes("://"),
          });
          const currentFitRatio =
            freshFit.consumedWidth > 0 ? currentFit.consumedWidth / freshFit.consumedWidth : 1;
          const startRatioThreshold = resolvePlainTextDelimitedStartRatioThreshold(word);
          const blocksPathContinuationAfterQueryTail =
            lineEndsWithUrlQueryPair(currentLineText) && isPathLikeDelimitedWord(word);
          const acceptsCurrentDelimitedContinuation = blocksPathContinuationAfterQueryTail
            ? false
            : isUrlLikeWord
              ? acceptsUrlContinuationOnCurrentLine({
                  word,
                  consumedText: currentFit.consumedText,
                  currentFitRatio,
                })
              : isAbsolutePathLikeWord
                ? acceptsAbsolutePathContinuationOnCurrentLine(currentFit.consumedText)
                : currentFitRatio >= startRatioThreshold;
          if (
            currentFit.consumedText.length > 0 &&
            acceptsCurrentDelimitedContinuation
          ) {
            remainingWidth = Math.max(0, availableWidth - currentFit.consumedWidth);
            lineHasContent = true;
            appendLineText(currentFit.consumedText, true);
            remainder = currentFit.remainder.length > 0 ? currentFit.remainder : null;
            remainderFragments = remainder != null ? splitPlainTextWrapFragments(remainder) : null;
            if (remainder == null) {
              wordIndex += 1;
            }
            if (wordIndex < words.length || remainder != null) {
              flushLine();
              lineCount += 1;
              lineHasContent = false;
              remainingWidth = maxWidth;
            }
            continue;
          }
        }
      }
      if (
        !usesDelimitedWrapping &&
        wordWidth > availableWidth + 0.01 &&
        availableWidth > 0.01
      ) {
        const softHyphenFit =
          wordContainsSoftHyphen
            ? measureSoftHyphenBreakFit({
                cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued`,
                text: word,
                font: params.font,
                width: Math.max(1, availableWidth - SOFT_HYPHEN_CURRENT_LINE_FIT_GUARD_PX),
              })
            : null;
        if (softHyphenFit != null) {
          remainingWidth = Math.max(0, availableWidth - softHyphenFit.consumedWidth);
          lineHasContent = true;
          appendLineText(softHyphenFit.consumedText, true);
          remainder = softHyphenFit.remainder.length > 0 ? softHyphenFit.remainder : null;
          remainderFragments = null;
          if (remainder == null) {
            wordIndex += 1;
          }
          if (wordIndex < words.length || remainder != null) {
            flushLine();
            lineCount += 1;
            lineHasContent = false;
            remainingWidth = maxWidth;
          }
          continue;
        }
        const zeroWidthBreakFit =
          wordContainsZeroWidthBreak
            ? measureZeroWidthSpaceBreakFit({
                cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued`,
                text: word,
                font: params.font,
                width: availableWidth,
              })
            : null;
        if (zeroWidthBreakFit != null) {
          remainingWidth = Math.max(0, availableWidth - zeroWidthBreakFit.consumedWidth);
          lineHasContent = true;
          appendLineText(zeroWidthBreakFit.consumedText, true);
          remainder = zeroWidthBreakFit.remainder.length > 0 ? zeroWidthBreakFit.remainder : null;
          remainderFragments = null;
          if (remainder == null) {
            wordIndex += 1;
          }
          if (wordIndex < words.length || remainder != null) {
            flushLine();
            lineCount += 1;
            lineHasContent = false;
            remainingWidth = maxWidth;
          }
          continue;
        }
        if (!wordFitsFreshLine) {
          const implicitWordBreakFit =
            usesImplicitWordBreaking
              ? measureImplicitWordBreakFit({
                  cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued`,
                  segments: implicitWordBreakSegments,
                  font: params.font,
                  width: availableWidth,
                })
              : null;
          if (implicitWordBreakFit != null) {
            remainingWidth = Math.max(0, availableWidth - implicitWordBreakFit.consumedWidth);
            lineHasContent = true;
            appendLineText(implicitWordBreakFit.consumedText, true);
            remainder = implicitWordBreakFit.remainder.length > 0 ? implicitWordBreakFit.remainder : null;
            remainderFragments = null;
            if (remainder == null) {
              wordIndex += 1;
            }
            if (wordIndex < words.length || remainder != null) {
              flushLine();
              lineCount += 1;
              lineHasContent = false;
              remainingWidth = maxWidth;
            }
            continue;
          }
          const fittingPrefix = findLargestCollapsedPlainTextPrefixThatFits({
            cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued`,
            text: displayWord,
            font: params.font,
            width: availableWidth,
          });
          if (fittingPrefix.prefixWidth > 0) {
            remainingWidth = Math.max(0, availableWidth - fittingPrefix.prefixWidth);
            lineHasContent = true;
            appendLineText(
              displayWord.slice(0, displayWord.length - fittingPrefix.remainder.length),
              true,
            );
            remainder = fittingPrefix.remainder.length > 0 ? fittingPrefix.remainder : null;
            remainderFragments = null;
            if (remainder == null) {
              wordIndex += 1;
            }
            if (wordIndex < words.length || remainder != null) {
              flushLine();
              lineCount += 1;
              lineHasContent = false;
              remainingWidth = maxWidth;
            }
            continue;
          }
        }
      }
      flushLine();
      lineCount += 1;
      lineHasContent = false;
      remainingWidth = maxWidth;
      continue;
    }

    if (usesDelimitedWrapping) {
      const fittingPrefix = measurePlainTextDelimitedTokenFit({
        cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:fresh`,
        text: word,
        fragments: wordFragments,
        font: params.font,
        width: remainingWidth,
        allowPartialFragment: true,
      });
      remainingWidth = Math.max(0, remainingWidth - fittingPrefix.consumedWidth);
      lineHasContent = true;
      appendLineText(fittingPrefix.consumedText, false);
      remainder = fittingPrefix.remainder.length > 0 ? fittingPrefix.remainder : null;
      remainderFragments = remainder != null ? splitPlainTextWrapFragments(remainder) : null;
      if (remainder == null) {
        wordIndex += 1;
      }

      const nextWord = remainder == null ? words[wordIndex] ?? null : null;
      const nextWordWidth =
        nextWord != null
          ? measureSingleLineTextWidth({
              cacheKey: buildPreparedContentKey(`${params.cacheKey}:word:${wordIndex}`, nextWord),
              text: nextWord,
              font: params.font,
            })
          : 0;

      if (remainder != null) {
        flushLine();
        lineCount += 1;
        lineHasContent = false;
        remainingWidth = maxWidth;
      }
      continue;
    }

    if (wordWidth <= remainingWidth + 0.01) {
      remainingWidth = Math.max(0, remainingWidth - wordWidth);
      lineHasContent = true;
      appendLineText(displayWord, false);
      remainder = null;
      remainderFragments = null;
      wordIndex += 1;
      continue;
    }

    const softHyphenFreshFit =
      wordContainsSoftHyphen
        ? measureSoftHyphenBreakFit({
            cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:fresh`,
            text: word,
            font: params.font,
            width: remainingWidth,
          })
        : null;
    if (softHyphenFreshFit != null) {
      remainingWidth = Math.max(0, remainingWidth - softHyphenFreshFit.consumedWidth);
      lineHasContent = true;
      appendLineText(softHyphenFreshFit.consumedText, false);
      remainder = softHyphenFreshFit.remainder.length > 0 ? softHyphenFreshFit.remainder : null;
      remainderFragments = null;
      if (remainder == null) {
        wordIndex += 1;
      }

      if (wordIndex < words.length || remainder != null) {
        flushLine();
        lineCount += 1;
        lineHasContent = false;
        remainingWidth = maxWidth;
      }
      continue;
    }
    const zeroWidthFreshFit =
      wordContainsZeroWidthBreak
        ? measureZeroWidthSpaceBreakFit({
            cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:fresh`,
            text: word,
            font: params.font,
            width: remainingWidth,
          })
        : null;
    if (zeroWidthFreshFit != null) {
      remainingWidth = Math.max(0, remainingWidth - zeroWidthFreshFit.consumedWidth);
      lineHasContent = true;
      appendLineText(zeroWidthFreshFit.consumedText, false);
      remainder = zeroWidthFreshFit.remainder.length > 0 ? zeroWidthFreshFit.remainder : null;
      remainderFragments = null;
      if (remainder == null) {
        wordIndex += 1;
      }

      if (wordIndex < words.length || remainder != null) {
        flushLine();
        lineCount += 1;
        lineHasContent = false;
        remainingWidth = maxWidth;
      }
      continue;
    }
    const implicitWordFreshFit =
      usesImplicitWordBreaking
        ? measureImplicitWordBreakFit({
            cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:fresh`,
            segments: implicitWordBreakSegments,
            font: params.font,
            width: remainingWidth,
          })
        : null;
    if (implicitWordFreshFit != null) {
      remainingWidth = Math.max(0, remainingWidth - implicitWordFreshFit.consumedWidth);
      lineHasContent = true;
      appendLineText(implicitWordFreshFit.consumedText, false);
      remainder = implicitWordFreshFit.remainder.length > 0 ? implicitWordFreshFit.remainder : null;
      remainderFragments = null;
      if (remainder == null) {
        wordIndex += 1;
      }

      if (wordIndex < words.length || remainder != null) {
        flushLine();
        lineCount += 1;
        lineHasContent = false;
        remainingWidth = maxWidth;
      }
      continue;
    }

    const fittingPrefix = findLargestCollapsedPlainTextPrefixThatFits({
      cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}`,
      text: displayWord,
      font: params.font,
      width: remainingWidth,
    });

    remainingWidth = Math.max(0, remainingWidth - fittingPrefix.prefixWidth);
    lineHasContent = true;
    appendLineText(
      displayWord.slice(0, displayWord.length - fittingPrefix.remainder.length),
      false,
    );
    remainder = fittingPrefix.remainder.length > 0 ? fittingPrefix.remainder : null;
    remainderFragments = null;
    if (remainder == null) {
      wordIndex += 1;
    }

    if (wordIndex < words.length || remainder != null) {
      flushLine();
      lineCount += 1;
      lineHasContent = false;
      remainingWidth = maxWidth;
    }
  }

  flushLine();
  if (debugPlainText && plainTextDebugWindow) {
    plainTextDebugWindow.__ctxPlainTextDebug = {
      lineCount,
      lines: debugLines,
      lineWidths: debugLineWidths,
      text: normalizedText,
      width: maxWidth,
    };
  }

  return lineCount * params.lineHeight;
}

export function clearSessionPlainTextMeasurementCaches(): void {
  plainTextBlockHeightCache.clear();
}

export function measureSessionPlainTextBlockHeight(params: {
  cacheKey: string;
  text: string;
  font: string;
  width: number;
  lineHeight: number;
}): number {
  const normalizedText = String(params.text ?? "")
    .replace(/\r\n/g, "\n")
    .replace(/[\r\f]/g, "\n");

  const measurementKey = `${params.cacheKey}:plain-text:${params.font}:${params.width}:${params.lineHeight}`;
  const cached = plainTextBlockHeightCache.get(measurementKey);
  if (cached != null) {
    return cached;
  }

  const measuredHeight = normalizeHeight(
    normalizedText.split("\n").reduce((sum, line, index) => {
      if (line.length === 0) {
        return sum + params.lineHeight;
      }
      return (
        sum +
        measureCollapsedPlainTextLineHeight({
          cacheKey: `${measurementKey}:line:${index}`,
          text: line,
          font: params.font,
          width: params.width,
          lineHeight: params.lineHeight,
        })
      );
    }, 0),
  );
  plainTextBlockHeightCache.set(measurementKey, measuredHeight);
  pruneCache(plainTextBlockHeightCache, SESSION_TEXT_MEASUREMENT_CACHE_LIMIT);
  return measuredHeight;
}
