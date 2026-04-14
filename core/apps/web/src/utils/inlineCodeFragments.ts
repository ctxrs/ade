const INLINE_CODE_SLASH_COALESCE_MAX_GRAPHEMES = 8;
const INLINE_CODE_HYPHEN_PATH_COALESCE_MAX_GRAPHEMES = 21;

function countGraphemes(text: string): number {
  return Array.from(text).length;
}

export function splitInlineCodeFragments(text: string): string[] {
  if (text.length === 0) {
    return [];
  }

  const rawFragments: string[] = [];
  let current = "";
  for (const char of text) {
    current += char;
    if (char === "-" || char === "." || char === "/" || char === "\\") {
      rawFragments.push(current);
      current = "";
    }
  }
  if (current.length > 0) {
    rawFragments.push(current);
  }

  const fragments: string[] = [];
  for (const fragment of rawFragments) {
    const previous = fragments[fragments.length - 1] ?? null;
    const shouldCoalesceWithPrevious =
      previous != null &&
      /[\\/]$/.test(previous) &&
      countGraphemes(previous) <= INLINE_CODE_SLASH_COALESCE_MAX_GRAPHEMES;
    if (shouldCoalesceWithPrevious) {
      fragments[fragments.length - 1] = `${previous}${fragment}`;
      continue;
    }
    fragments.push(fragment);
  }

  const mergedFragments: string[] = [];
  for (const fragment of fragments) {
    const previous = mergedFragments[mergedFragments.length - 1] ?? null;
    const shouldMergeHyphenPathCluster =
      previous != null &&
      previous.endsWith("-") &&
      (/[\\/]/.test(previous) || /[\\/]/.test(fragment)) &&
      countGraphemes(previous) <= INLINE_CODE_HYPHEN_PATH_COALESCE_MAX_GRAPHEMES;
    if (shouldMergeHyphenPathCluster) {
      mergedFragments[mergedFragments.length - 1] = `${previous}${fragment}`;
      continue;
    }
    mergedFragments.push(fragment);
  }

  return mergedFragments;
}

export function isSealedInlineCodeFragment(fragment: string): boolean {
  return fragment.endsWith("-") || fragment.endsWith(".") || fragment.endsWith("/") || fragment.endsWith("\\");
}
