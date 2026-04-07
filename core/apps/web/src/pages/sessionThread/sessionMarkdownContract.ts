import {
  readMarkdownChecked,
  readMarkdownDepth,
  readMarkdownOrdered,
  type SessionMarkdownNode,
} from "./sessionMarkdownShared";

export type SessionMarkdownInlineNode =
  | { kind: "text"; text: string }
  | { kind: "inlineCode"; text: string }
  | { kind: "break" }
  | { kind: "strong"; children: SessionMarkdownInlineNode[] }
  | { kind: "emphasis"; children: SessionMarkdownInlineNode[] }
  | { kind: "delete"; children: SessionMarkdownInlineNode[] }
  | { kind: "link"; href: string; title: string | null; children: SessionMarkdownInlineNode[] }
  | { kind: "image"; src: string; alt: string; title: string | null };

export type SessionMarkdownBlockContext = "root" | "listItem";

export type SessionMarkdownBlockKind =
  | "paragraph"
  | "image"
  | "heading"
  | "list"
  | "blockquote"
  | "code"
  | "table"
  | "thematicBreak";

export type SessionMarkdownParagraphBlock = {
  kind: "paragraph";
  node: SessionMarkdownNode;
  inlines: SessionMarkdownInlineNode[];
};

export type SessionMarkdownImageBlock = {
  kind: "image";
  node: SessionMarkdownNode;
  src: string;
  alt: string;
  title: string | null;
};

export type SessionMarkdownHeadingBlock = {
  kind: "heading";
  node: SessionMarkdownNode;
  depth: number;
  inlines: SessionMarkdownInlineNode[];
};

export type SessionMarkdownListItem = {
  checked: boolean | null;
  blocks: SessionMarkdownBlock[];
};

export type SessionMarkdownListBlock = {
  kind: "list";
  node: SessionMarkdownNode;
  ordered: boolean;
  items: SessionMarkdownListItem[];
};

export type SessionMarkdownBlockquoteBlock = {
  kind: "blockquote";
  node: SessionMarkdownNode;
  blocks: SessionMarkdownBlock[];
};

export type SessionMarkdownCodeBlock = {
  kind: "code";
  node: SessionMarkdownNode;
  code: string;
  lang: string | null;
};

export type SessionMarkdownTableCell = {
  blocks: SessionMarkdownBlock[];
};

export type SessionMarkdownTableRow = {
  cells: SessionMarkdownTableCell[];
};

export type SessionMarkdownTableBlock = {
  kind: "table";
  node: SessionMarkdownNode;
  rows: SessionMarkdownTableRow[];
};

export type SessionMarkdownThematicBreakBlock = {
  kind: "thematicBreak";
  node: SessionMarkdownNode;
};

export type SessionMarkdownBlock =
  | SessionMarkdownParagraphBlock
  | SessionMarkdownImageBlock
  | SessionMarkdownHeadingBlock
  | SessionMarkdownListBlock
  | SessionMarkdownBlockquoteBlock
  | SessionMarkdownCodeBlock
  | SessionMarkdownTableBlock
  | SessionMarkdownThematicBreakBlock;

const ROOT_BLOCK_BEFORE_PX: Record<SessionMarkdownBlockKind, number> = {
  paragraph: 0,
  image: 0,
  heading: 16,
  list: 0,
  blockquote: 0,
  code: 10,
  table: 0,
  thematicBreak: 0,
};

const ROOT_BLOCK_AFTER_PX: Record<SessionMarkdownBlockKind, number> = {
  paragraph: 12,
  image: 12,
  heading: 8,
  list: 12,
  blockquote: 12,
  code: 10,
  table: 12,
  thematicBreak: 12,
};

const LIST_ITEM_BLOCK_BEFORE_PX: Record<SessionMarkdownBlockKind, number> = {
  ...ROOT_BLOCK_BEFORE_PX,
  paragraph: 0,
  image: 0,
};

const LIST_ITEM_BLOCK_AFTER_PX: Record<SessionMarkdownBlockKind, number> = {
  ...ROOT_BLOCK_AFTER_PX,
  paragraph: 0,
  image: 0,
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === "object" && !Array.isArray(value);

const readString = (value: unknown): string => (typeof value === "string" ? value : "");

export function nodeChildren(node: SessionMarkdownNode | null | undefined): SessionMarkdownNode[] {
  if (!node || !Array.isArray(node.children)) return [];
  return node.children.filter((child): child is SessionMarkdownNode => isRecord(child));
}

function normalizeInlineNodes(nodes: readonly SessionMarkdownNode[]): SessionMarkdownInlineNode[] {
  const normalized: SessionMarkdownInlineNode[] = [];
  for (const node of nodes) {
    switch (node.type) {
      case "text":
        normalized.push({ kind: "text", text: readString(node.value).replace(/\u00a0/g, " ") });
        break;
      case "inlineCode":
        normalized.push({ kind: "inlineCode", text: readString(node.value) });
        break;
      case "break":
        normalized.push({ kind: "break" });
        break;
      case "strong":
        normalized.push({ kind: "strong", children: normalizeInlineNodes(nodeChildren(node)) });
        break;
      case "emphasis":
        normalized.push({ kind: "emphasis", children: normalizeInlineNodes(nodeChildren(node)) });
        break;
      case "delete":
        normalized.push({ kind: "delete", children: normalizeInlineNodes(nodeChildren(node)) });
        break;
      case "link":
        normalized.push({
          kind: "link",
          href: readString(node.url),
          title: readString(node.title) || null,
          children: normalizeInlineNodes(nodeChildren(node)),
        });
        break;
      case "image":
        normalized.push({
          kind: "image",
          src: readString(node.url),
          alt: readString(node.alt),
          title: readString(node.title) || null,
        });
        break;
      default:
        normalized.push(...normalizeInlineNodes(nodeChildren(node)));
        break;
    }
  }
  return normalized;
}

function isStandaloneImageParagraph(node: SessionMarkdownNode): boolean {
  if (node.type !== "paragraph") return false;
  const children = nodeChildren(node).filter((child) => child.type !== "text" || readString(child.value).trim().length > 0);
  return children.length === 1 && children[0]?.type === "image";
}

function normalizeTableRows(node: SessionMarkdownNode): SessionMarkdownTableRow[] {
  return nodeChildren(node)
    .filter((child) => child.type === "tableRow")
    .map((row) => ({
      cells: nodeChildren(row)
        .filter((cell) => cell.type === "tableCell")
        .map((cell) => ({ blocks: normalizeSessionMarkdownBlocks(nodeChildren(cell)) })),
    }));
}

function normalizeListItems(node: SessionMarkdownNode): SessionMarkdownListItem[] {
  return nodeChildren(node)
    .filter((child) => child.type === "listItem")
    .map((item) => ({
      checked: readMarkdownChecked(item),
      blocks: normalizeSessionMarkdownBlocks(nodeChildren(item)),
    }));
}

export function normalizeSessionMarkdownBlocks(nodes: readonly SessionMarkdownNode[]): SessionMarkdownBlock[] {
  const normalized: SessionMarkdownBlock[] = [];
  for (const node of nodes) {
    switch (node.type) {
      case "definition":
      case "yaml":
      case "html":
        break;
      case "heading":
        normalized.push({
          kind: "heading",
          node,
          depth: readMarkdownDepth(node, 1),
          inlines: normalizeInlineNodes(nodeChildren(node)),
        });
        break;
      case "list":
        normalized.push({
          kind: "list",
          node,
          ordered: readMarkdownOrdered(node),
          items: normalizeListItems(node),
        });
        break;
      case "blockquote":
        normalized.push({
          kind: "blockquote",
          node,
          blocks: normalizeSessionMarkdownBlocks(nodeChildren(node)),
        });
        break;
      case "code":
        normalized.push({
          kind: "code",
          node,
          code: readString(node.value).replace(/[\r\n]+$/, ""),
          lang: readString(node.lang) || null,
        });
        break;
      case "table":
        normalized.push({
          kind: "table",
          node,
          rows: normalizeTableRows(node),
        });
        break;
      case "thematicBreak":
        normalized.push({ kind: "thematicBreak", node });
        break;
      case "paragraph":
        if (isStandaloneImageParagraph(node)) {
          const imageNode = nodeChildren(node)[0]!;
          normalized.push({
            kind: "image",
            node,
            src: readString(imageNode.url),
            alt: readString(imageNode.alt),
            title: readString(imageNode.title) || null,
          });
          break;
        }
        normalized.push({
          kind: "paragraph",
          node,
          inlines: normalizeInlineNodes(nodeChildren(node)),
        });
        break;
      default:
        if (nodeChildren(node).length > 0) {
          normalized.push(...normalizeSessionMarkdownBlocks(nodeChildren(node)));
        } else {
          normalized.push({
            kind: "paragraph",
            node,
            inlines: normalizeInlineNodes([node]),
          });
        }
        break;
    }
  }
  return normalized;
}

function resolveBlockBeforePx(kind: SessionMarkdownBlockKind, context: SessionMarkdownBlockContext): number {
  return context === "listItem" ? LIST_ITEM_BLOCK_BEFORE_PX[kind] : ROOT_BLOCK_BEFORE_PX[kind];
}

function resolveBlockAfterPx(kind: SessionMarkdownBlockKind, context: SessionMarkdownBlockContext): number {
  return context === "listItem" ? LIST_ITEM_BLOCK_AFTER_PX[kind] : ROOT_BLOCK_AFTER_PX[kind];
}

export function resolveSessionMarkdownBlockEntryGapPx(
  kind: SessionMarkdownBlockKind,
  context: SessionMarkdownBlockContext,
): number {
  return resolveBlockBeforePx(kind, context);
}

export function resolveSessionMarkdownBlockGapPx(
  previousKind: SessionMarkdownBlockKind,
  nextKind: SessionMarkdownBlockKind,
  context: SessionMarkdownBlockContext,
): number {
  return Math.max(resolveBlockAfterPx(previousKind, context), resolveBlockBeforePx(nextKind, context));
}
