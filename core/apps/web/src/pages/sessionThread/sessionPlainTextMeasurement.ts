import {
  SESSION_TEXT_MEASUREMENT_CACHE_LIMIT,
  buildPreparedContentKey,
  getPreparedTextWithSegments,
  measureCollapsedSpaceWidth,
  measureSingleLineLayout,
  normalizeHeight,
  pruneCache,
  segmentGraphemes,
} from "./sessionTextMeasurement";

type SessionPlainTextDebugWindow = Window & {
  __ctxForcePlainTextDebug?: boolean;
  __ctxPlainTextDebugTarget?: string;
  __ctxPlainTextDebugWidth?: number;
  __ctxPlainTextDebug?: {
    lineCount: number;
    lines: string[];
    text: string;
    width: number;
  };
};

const plainTextBlockHeightCache = new Map<string, number>();

function normalizeCollapsedPlainTextLineText(text: string): string {
  return text.replace(/\u00a0/g, " ").replace(/\r\n?/g, "\n").replace(/\n/g, " ");
}

function measureSingleLineTextWidth(params: {
  cacheKey: string;
  text: string;
  font: string;
}): number {
  const prepared = getPreparedTextWithSegments(params.cacheKey, params.text, params.font, "normal");
  return Math.max(0, measureSingleLineLayout(prepared)?.width ?? 0);
}

function findLargestCollapsedPlainTextPrefixThatFits(params: {
  cacheKeyPrefix: string;
  text: string;
  font: string;
  width: number;
}): {
  prefixWidth: number;
  remainder: string;
} {
  const graphemes = segmentGraphemes(params.text);
  if (graphemes.length === 0) {
    return { prefixWidth: 0, remainder: "" };
  }

  let bestCount = 1;
  let bestWidth = measureSingleLineTextWidth({
    cacheKey: buildPreparedContentKey(`${params.cacheKeyPrefix}:prefix:1`, graphemes[0]!),
    text: graphemes[0]!,
    font: params.font,
  });
  let low = 1;
  let high = graphemes.length;

  while (low <= high) {
    const count = Math.floor((low + high) / 2);
    const prefix = graphemes.slice(0, count).join("");
    const width = measureSingleLineTextWidth({
      cacheKey: buildPreparedContentKey(`${params.cacheKeyPrefix}:prefix:${count}`, prefix),
      text: prefix,
      font: params.font,
    });
    if (width <= params.width + 0.01) {
      bestCount = count;
      bestWidth = width;
      low = count + 1;
      continue;
    }
    high = count - 1;
  }

  return {
    prefixWidth: bestWidth,
    remainder: graphemes.slice(bestCount).join(""),
  };
}

function splitPlainTextWrapFragments(text: string): string[] {
  if (!/[./\\\-?&=]/.test(text)) {
    return [text];
  }

  const fragments: string[] = [];
  let current = "";
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index]!;
    current += character;
    if (character === ":" && text.slice(index + 1, index + 3) === "//") {
      current += "//";
      index += 2;
      fragments.push(current);
      current = "";
      continue;
    }
    if (
      character === "-" ||
      character === "." ||
      character === "/" ||
      character === "\\" ||
      character === "?" ||
      character === "&" ||
      character === "="
    ) {
      fragments.push(current);
      current = "";
    }
  }
  if (current.length > 0) {
    fragments.push(current);
  }

  return fragments.length > 0 ? fragments : [text];
}

function measurePlainTextDelimitedTokenFit(params: {
  cacheKeyPrefix: string;
  text: string;
  fragments: readonly string[];
  font: string;
  width: number;
  allowPartialFragment: boolean;
  allowPartialAfterConsumedText?: boolean;
}): {
  consumedText: string;
  consumedWidth: number;
  remainder: string;
} {
  let remainingWidth = Math.max(1, params.width);
  let consumedText = "";
  let consumedWidth = 0;

  for (let index = 0; index < params.fragments.length; index += 1) {
    const fragment = params.fragments[index]!;
    const nextFragment = params.fragments[index + 1] ?? null;
    const fragmentWidth = measureSingleLineTextWidth({
      cacheKey: buildPreparedContentKey(`${params.cacheKeyPrefix}:fragment:${index}`, fragment),
      text: fragment,
      font: params.font,
    });

    if (fragmentWidth <= remainingWidth + 0.01) {
      if (fragment.endsWith("=") && nextFragment != null) {
        const queryPairWidth = measureSingleLineTextWidth({
          cacheKey: buildPreparedContentKey(
            `${params.cacheKeyPrefix}:fragment-pair:${index}`,
            `${fragment}${nextFragment}`,
          ),
          text: `${fragment}${nextFragment}`,
          font: params.font,
        });
        if (queryPairWidth > remainingWidth + 0.01) {
          return {
            consumedText,
            consumedWidth,
            remainder: `${fragment}${params.fragments.slice(index + 1).join("")}`,
          };
        }
      }
      consumedText += fragment;
      consumedWidth += fragmentWidth;
      remainingWidth = Math.max(0, remainingWidth - fragmentWidth);
      continue;
    }

    if (
      !params.allowPartialFragment ||
      (consumedText.length > 0 && params.allowPartialAfterConsumedText !== true)
    ) {
      return {
        consumedText,
        consumedWidth,
        remainder: `${fragment}${params.fragments.slice(index + 1).join("")}`,
      };
    }

    const fittingPrefix = findLargestCollapsedPlainTextPrefixThatFits({
      cacheKeyPrefix: `${params.cacheKeyPrefix}:fragment:${index}`,
      text: fragment,
      font: params.font,
      width: remainingWidth,
    });
    const consumedFragmentText = fragment.slice(0, fragment.length - fittingPrefix.remainder.length);
    consumedText += consumedFragmentText;
    consumedWidth += fittingPrefix.prefixWidth;
    return {
      consumedText,
      consumedWidth,
      remainder: `${fittingPrefix.remainder}${params.fragments.slice(index + 1).join("")}`,
    };
  }

  return {
    consumedText,
    consumedWidth,
    remainder: "",
  };
}

function resolvePlainTextDelimitedStartRatioThreshold(text: string): number {
  return text.includes("://") ? 0.35 : 0.55;
}

function snapUrlContinuationPrefix(prefix: string): string {
  if (prefix.length === 0) {
    return prefix;
  }
  for (let index = prefix.length - 1; index >= 0; index -= 1) {
    const character = prefix[index]!;
    if (character === "-" || character === "/" || character === "?" || character === "&" || character === ".") {
      return prefix.slice(0, index + 1);
    }
  }
  return prefix;
}

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
  let debugLine = "";
  const appendDebugText = (text: string, prefixSpace: boolean) => {
    if (!debugPlainText) {
      return;
    }
    if (prefixSpace && debugLine.length > 0) {
      debugLine += " ";
    }
    debugLine += text;
  };
  const flushDebugLine = () => {
    if (!debugPlainText || debugLine.length === 0) {
      return;
    }
    debugLines.push(debugLine);
    debugLine = "";
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
    const wordFragments = remainderFragments ?? splitPlainTextWrapFragments(word);
    const usesDelimitedWrapping = wordFragments.length > 1;
    const wordWidth = measureSingleLineTextWidth({
      cacheKey: buildPreparedContentKey(`${params.cacheKey}:word:${wordIndex}`, word),
      text: word,
      font: params.font,
    });
    const reservedWidth = lineHasContent ? collapsedSpaceWidth : 0;

    if (lineHasContent && reservedWidth + wordWidth <= remainingWidth + 0.01) {
      remainingWidth = Math.max(0, remainingWidth - reservedWidth - wordWidth);
      appendDebugText(word, true);
      remainder = null;
      remainderFragments = null;
      wordIndex += 1;
      continue;
    }

    if (lineHasContent) {
      const isUrlLikeWord = word.includes("://");
      const availableWidth = Math.max(0, remainingWidth - reservedWidth);
      if (isUrlLikeWord && availableWidth > 0.01) {
        const currentPrefix = findLargestCollapsedPlainTextPrefixThatFits({
          cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued-url`,
          text: word,
          font: params.font,
          width: availableWidth,
        });
        const freshPrefix = findLargestCollapsedPlainTextPrefixThatFits({
          cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:fresh-url`,
          text: word,
          font: params.font,
          width: maxWidth,
        });
        const rawPrefixText = word.slice(0, word.length - currentPrefix.remainder.length);
        const snappedPrefixText = snapUrlContinuationPrefix(rawPrefixText);
        const snappedPrefixWidth =
          snappedPrefixText.length === rawPrefixText.length
            ? currentPrefix.prefixWidth
            : measureSingleLineTextWidth({
                cacheKey: buildPreparedContentKey(
                  `${params.cacheKey}:word:${wordIndex}:continued-url-snapped`,
                  snappedPrefixText,
                ),
                text: snappedPrefixText,
                font: params.font,
              });
        const continuationFitRatio =
          freshPrefix.prefixWidth > 0 ? snappedPrefixWidth / freshPrefix.prefixWidth : 1;
        if (
          snappedPrefixWidth > 0 &&
          continuationFitRatio >= resolvePlainTextDelimitedStartRatioThreshold(word)
        ) {
          remainingWidth = Math.max(0, availableWidth - snappedPrefixWidth);
          lineHasContent = true;
          appendDebugText(snappedPrefixText, true);
          const nextRemainder = word.slice(snappedPrefixText.length);
          remainder = nextRemainder.length > 0 ? nextRemainder : null;
          remainderFragments =
            remainder != null ? splitPlainTextWrapFragments(remainder) : null;
          if (remainder == null) {
            wordIndex += 1;
          }
          if (wordIndex < words.length || remainder != null) {
            flushDebugLine();
            lineCount += 1;
            lineHasContent = false;
            remainingWidth = maxWidth;
          }
          continue;
        }
      }
      if (usesDelimitedWrapping) {
        if (availableWidth > 0.01) {
          const currentFit = measurePlainTextDelimitedTokenFit({
            cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued`,
            text: word,
            fragments: wordFragments,
            font: params.font,
            width: availableWidth,
            allowPartialFragment: false,
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
          if (currentFit.consumedText.length > 0 && currentFitRatio >= startRatioThreshold) {
            remainingWidth = Math.max(0, availableWidth - currentFit.consumedWidth);
            lineHasContent = true;
            appendDebugText(currentFit.consumedText, true);
            remainder = currentFit.remainder.length > 0 ? currentFit.remainder : null;
            remainderFragments = remainder != null ? splitPlainTextWrapFragments(remainder) : null;
            if (remainder == null) {
              wordIndex += 1;
            }
            if (wordIndex < words.length || remainder != null) {
              flushDebugLine();
              lineCount += 1;
              lineHasContent = false;
              remainingWidth = maxWidth;
            }
            continue;
          }
        }
      }
      if (!usesDelimitedWrapping && wordWidth > availableWidth + 0.01 && availableWidth > 0.01) {
        const fittingPrefix = findLargestCollapsedPlainTextPrefixThatFits({
          cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}:continued`,
          text: word,
          font: params.font,
          width: availableWidth,
        });
        if (fittingPrefix.prefixWidth > 0) {
          remainingWidth = Math.max(0, availableWidth - fittingPrefix.prefixWidth);
          lineHasContent = true;
          appendDebugText(word.slice(0, word.length - fittingPrefix.remainder.length), true);
          remainder = fittingPrefix.remainder.length > 0 ? fittingPrefix.remainder : null;
          remainderFragments = null;
          if (remainder == null) {
            wordIndex += 1;
          }
          if (wordIndex < words.length || remainder != null) {
            flushDebugLine();
            lineCount += 1;
            lineHasContent = false;
            remainingWidth = maxWidth;
          }
          continue;
        }
      }
      flushDebugLine();
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
      appendDebugText(fittingPrefix.consumedText, false);
      remainder = fittingPrefix.remainder.length > 0 ? fittingPrefix.remainder : null;
      remainderFragments = remainder != null ? splitPlainTextWrapFragments(remainder) : null;
      if (remainder == null) {
        wordIndex += 1;
      }

      if (wordIndex < words.length || remainder != null) {
        flushDebugLine();
        lineCount += 1;
        lineHasContent = false;
        remainingWidth = maxWidth;
      }
      continue;
    }

    if (wordWidth <= remainingWidth + 0.01) {
      remainingWidth = Math.max(0, remainingWidth - wordWidth);
      lineHasContent = true;
      appendDebugText(word, false);
      remainder = null;
      remainderFragments = null;
      wordIndex += 1;
      continue;
    }

    const fittingPrefix = findLargestCollapsedPlainTextPrefixThatFits({
      cacheKeyPrefix: `${params.cacheKey}:word:${wordIndex}`,
      text: word,
      font: params.font,
      width: remainingWidth,
    });

    remainingWidth = Math.max(0, remainingWidth - fittingPrefix.prefixWidth);
    lineHasContent = true;
    appendDebugText(word.slice(0, word.length - fittingPrefix.remainder.length), false);
    remainder = fittingPrefix.remainder.length > 0 ? fittingPrefix.remainder : null;
    remainderFragments = null;
    if (remainder == null) {
      wordIndex += 1;
    }

    if (wordIndex < words.length || remainder != null) {
      flushDebugLine();
      lineCount += 1;
      lineHasContent = false;
      remainingWidth = maxWidth;
    }
  }

  flushDebugLine();
  if (debugPlainText && plainTextDebugWindow) {
    plainTextDebugWindow.__ctxPlainTextDebug = {
      lineCount,
      lines: debugLines,
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
