import { stripCitationMarkers } from "../../utils/citationMarkers";
import {
  createSessionMarkdownDocument,
  nodeChildren,
  type SessionMarkdownBlock,
} from "./sessionMarkdownContract";
import { parseSessionMarkdown, type SessionMarkdownNode } from "./sessionMarkdownShared";

const STREAMING_MARKDOWN_CACHE_LIMIT = 1000;

type NodePosition = {
  start?: { offset?: unknown };
  end?: { offset?: unknown };
};

export type SessionStreamingMarkdownLayout = {
  source: string;
  stableMarkdown: string;
  stableBlocks: readonly SessionMarkdownBlock[];
  trailingTail: string;
};

const streamingMarkdownCache = new Map<string, SessionStreamingMarkdownLayout>();

function pruneCache<T>(cache: Map<string, T>, limit: number) {
  while (cache.size > limit) {
    const oldestKey = cache.keys().next().value;
    if (typeof oldestKey !== "string") break;
    cache.delete(oldestKey);
  }
}

function readOffset(value: unknown, fallback: number): number {
  const offset = Number(value);
  return Number.isFinite(offset) && offset >= 0 ? Math.floor(offset) : fallback;
}

function readStartOffset(node: SessionMarkdownNode, fallback: number): number {
  return readOffset((node as SessionMarkdownNode & { position?: NodePosition }).position?.start?.offset, fallback);
}

function readEndOffset(node: SessionMarkdownNode, fallback: number): number {
  return readOffset((node as SessionMarkdownNode & { position?: NodePosition }).position?.end?.offset, fallback);
}

function endsWithBlankLine(source: string): boolean {
  return /(?:\r?\n)[\t ]*\r?\n[\t ]*$/.test(source);
}

function findUnmatchedFenceStartOffset(source: string): number | null {
  const lines = source.split("\n");
  let offset = 0;
  let openFence: { char: "`" | "~"; length: number; startOffset: number } | null = null;

  for (const line of lines) {
    const match = line.match(/^[ \t]{0,3}(`{3,}|~{3,})/);
    if (match) {
      const marker = match[1] ?? "";
      const char = marker[0] as "`" | "~";
      if (openFence && openFence.char === char && marker.length >= openFence.length) {
        openFence = null;
      } else if (!openFence) {
        openFence = { char, length: marker.length, startOffset: offset };
      }
    }
    offset += line.length + 1;
  }

  return openFence?.startOffset ?? null;
}

function findTrailingRiskyRootStartOffset(source: string, rootChildren: readonly SessionMarkdownNode[]): number | null {
  if (endsWithBlankLine(source)) return null;

  for (let index = rootChildren.length - 1; index >= 0; index -= 1) {
    const child = rootChildren[index]!;
    const endOffset = readEndOffset(child, -1);
    if (endOffset < source.length - 1) continue;
    switch (child.type) {
      case "list":
      case "blockquote":
      case "thematicBreak":
        return readStartOffset(child, 0);
      default:
        return null;
    }
  }

  return null;
}

function normalizeStreamingBoundary(source: string, startOffset: number): {
  stableMarkdown: string;
  trailingTail: string;
} {
  const rawStableMarkdown = source.slice(0, Math.max(0, startOffset));
  const rawTrailingTail = source.slice(Math.max(0, startOffset));
  const stableMarkdown = rawStableMarkdown.replace(/(?:\r?\n)+$/, "");
  const trailingTail = rawTrailingTail.replace(/^(?:\r?\n)+/, "");
  if (trailingTail.trim().length === 0) {
    return {
      stableMarkdown: source,
      trailingTail: "",
    };
  }
  return {
    stableMarkdown,
    trailingTail,
  };
}

export function resolveSessionStreamingMarkdownLayout(content: string): SessionStreamingMarkdownLayout {
  const cached = streamingMarkdownCache.get(content);
  if (cached) {
    return cached;
  }

  const source = stripCitationMarkers(content);
  if (source.trim().length === 0) {
    const emptyLayout: SessionStreamingMarkdownLayout = {
      source,
      stableMarkdown: "",
      stableBlocks: [],
      trailingTail: "",
    };
    streamingMarkdownCache.set(content, emptyLayout);
    pruneCache(streamingMarkdownCache, STREAMING_MARKDOWN_CACHE_LIMIT);
    return emptyLayout;
  }

  const root = parseSessionMarkdown(source);
  const rootChildren = nodeChildren(root);
  const unmatchedFenceStartOffset = findUnmatchedFenceStartOffset(source);
  const riskyRootStartOffset = findTrailingRiskyRootStartOffset(source, rootChildren);
  const tailStartOffset =
    unmatchedFenceStartOffset == null
      ? riskyRootStartOffset
      : riskyRootStartOffset == null
        ? unmatchedFenceStartOffset
        : Math.min(unmatchedFenceStartOffset, riskyRootStartOffset);

  if (tailStartOffset == null) {
    const stableDocument = createSessionMarkdownDocument(source);
    const layout: SessionStreamingMarkdownLayout = {
      source,
      stableMarkdown: source,
      stableBlocks: stableDocument.blocks,
      trailingTail: "",
    };
    streamingMarkdownCache.set(content, layout);
    pruneCache(streamingMarkdownCache, STREAMING_MARKDOWN_CACHE_LIMIT);
    return layout;
  }

  const normalized = normalizeStreamingBoundary(source, tailStartOffset);
  const stableDocument = normalized.stableMarkdown
    ? createSessionMarkdownDocument(normalized.stableMarkdown)
    : { source: "", blocks: [] as SessionMarkdownBlock[] };
  const layout: SessionStreamingMarkdownLayout = {
    source,
    stableMarkdown: normalized.stableMarkdown,
    stableBlocks: stableDocument.blocks,
    trailingTail: normalized.trailingTail,
  };
  streamingMarkdownCache.set(content, layout);
  pruneCache(streamingMarkdownCache, STREAMING_MARKDOWN_CACHE_LIMIT);
  return layout;
}

export function clearSessionStreamingMarkdownCaches(): void {
  streamingMarkdownCache.clear();
}
