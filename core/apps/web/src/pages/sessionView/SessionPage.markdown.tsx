import {
  Fragment,
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type MouseEvent,
  type ReactNode,
  type WheelEvent,
} from "react";
import { defaultUrlTransform } from "react-markdown";
import { Check, Copy } from "lucide-react";
import { ExternalLink } from "../../components/ExternalLink";
import {
  createSessionMarkdownDocument,
  resolveSessionMarkdownBlockEntryGapPx,
  resolveSessionMarkdownBlockGapPx,
  type SessionMarkdownBlock,
  type SessionMarkdownBlockContext,
  type SessionMarkdownInlineNode,
  type SessionMarkdownListBlock,
  type SessionMarkdownListItem,
  type SessionMarkdownTableCell,
} from "../sessionThread/sessionMarkdownContract";
import { copyTextToClipboard } from "../../utils/clipboard";
import {
  type FileRef,
  isAbsolutePath,
  parseFileRefToken,
  parseUrlToken,
  splitWhitespaceTokens,
} from "../../utils/codeTokenLinks";
import { desktopOpenFile, desktopOpenPath, isDesktopApp, openExternalLink } from "../../utils/desktop";

type ParsedContextOpen = {
  worktreeId?: string;
  file?: string;
  path?: string;
  line?: number;
  col?: number;
};

function parseContextOpenUrl(href: string): ParsedContextOpen | null {
  try {
    const url = new URL(href);
    if (url.protocol !== "ctx:") return null;
    if (url.hostname !== "open") return null;
    const worktreeId = url.searchParams.get("worktreeId") ?? "";
    const file = url.searchParams.get("file") ?? "";
    const path = url.searchParams.get("path") ?? "";
    if (!worktreeId && !path) return null;
    if (worktreeId && !file) return null;
    const line = url.searchParams.get("line");
    const col = url.searchParams.get("col");
    const parsedLine = line ? Number.parseInt(line, 10) : undefined;
    const parsedCol = col ? Number.parseInt(col, 10) : undefined;
    const normalizedLine = parsedLine && parsedLine > 0 ? parsedLine : undefined;
    const normalizedCol = parsedCol && parsedCol > 0 ? parsedCol : undefined;
    return {
      worktreeId: worktreeId || undefined,
      file: file || undefined,
      path: path || undefined,
      line: Number.isFinite(normalizedLine ?? NaN) ? normalizedLine : undefined,
      col: Number.isFinite(normalizedCol ?? NaN) ? normalizedCol : undefined,
    };
  } catch {
    return null;
  }
}

type CodeTokenOptions = {
  enableLinks: boolean;
  worktreeId: string | null;
  onFileOpenError?: (message: string | null) => void;
  wrapPlainTokens?: boolean;
};

function forwardVerticalWheelToTranscript(event: WheelEvent<HTMLElement>) {
  if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
  const transcriptScroller = event.currentTarget.closest("[data-pretext-virtualizer-list='1'], .wb-thread-scroller") as HTMLElement | null;
  if (!transcriptScroller) return;
  const maxScrollTop = Math.max(0, transcriptScroller.scrollHeight - transcriptScroller.clientHeight);
  if (maxScrollTop <= 0) return;
  const nextScrollTop = Math.max(0, Math.min(maxScrollTop, transcriptScroller.scrollTop + event.deltaY));
  if (Math.abs(nextScrollTop - transcriptScroller.scrollTop) <= 0.5) return;
  transcriptScroller.scrollTop = nextScrollTop;
  transcriptScroller.dispatchEvent(new Event("scroll", { bubbles: true }));
  event.preventDefault();
}

const hasModifier = (event: { metaKey?: boolean; ctrlKey?: boolean }): boolean =>
  Boolean(event.metaKey || event.ctrlKey);

const joinClassNames = (...values: Array<string | false | null | undefined>): string =>
  values.filter(Boolean).join(" ");

const modifierSubscribers = new Set<() => void>();
let modifierSnapshot = false;
let detachModifierListeners: (() => void) | null = null;

const emitModifierSnapshot = () => {
  for (const notify of modifierSubscribers) notify();
};

const setModifierSnapshot = (next: boolean) => {
  if (modifierSnapshot === next) return;
  modifierSnapshot = next;
  emitModifierSnapshot();
};

const updateModifierSnapshot = (event: { metaKey?: boolean; ctrlKey?: boolean }) => {
  setModifierSnapshot(hasModifier(event));
};

const clearModifierSnapshot = () => {
  setModifierSnapshot(false);
};

const subscribeModifierSnapshot = (notify: () => void) => {
  modifierSubscribers.add(notify);
  if (modifierSubscribers.size === 1 && typeof window !== "undefined") {
    const handleKeyboard = (event: KeyboardEvent) => updateModifierSnapshot(event);
    const handleMouse = (event: globalThis.MouseEvent) => updateModifierSnapshot(event);
    const handleBlur = () => clearModifierSnapshot();

    window.addEventListener("keydown", handleKeyboard);
    window.addEventListener("keyup", handleKeyboard);
    window.addEventListener("mousemove", handleMouse);
    window.addEventListener("blur", handleBlur);

    detachModifierListeners = () => {
      window.removeEventListener("keydown", handleKeyboard);
      window.removeEventListener("keyup", handleKeyboard);
      window.removeEventListener("mousemove", handleMouse);
      window.removeEventListener("blur", handleBlur);
    };
  }

  return () => {
    modifierSubscribers.delete(notify);
    if (modifierSubscribers.size > 0) return;
    detachModifierListeners?.();
    detachModifierListeners = null;
    clearModifierSnapshot();
  };
};

const getModifierSnapshot = () => modifierSnapshot;

function useModifierHoverState<T extends HTMLElement>() {
  const modifierDown = useSyncExternalStore(subscribeModifierSnapshot, getModifierSnapshot, () => false);
  const [hovered, setHovered] = useState(false);

  const syncFromPointer = useCallback((event: MouseEvent<T>) => {
    setHovered(true);
    updateModifierSnapshot(event);
  }, []);

  const clear = useCallback(() => {
    setHovered(false);
  }, []);

  return {
    modifierHoverActive: hovered && modifierDown,
    hoverProps: {
      onMouseEnter: syncFromPointer,
      onMouseMove: syncFromPointer,
      onMouseLeave: clear,
    },
  };
}

function ModifierAwareExternalLink({
  href,
  className,
  children,
  ...rest
}: React.ComponentProps<typeof ExternalLink>) {
  const { modifierHoverActive, hoverProps } = useModifierHoverState<HTMLAnchorElement>();
  return (
    <ExternalLink
      href={href}
      className={joinClassNames(className, modifierHoverActive && "ctx-modifier-hover")}
      {...hoverProps}
      {...rest}
    >
      {children}
    </ExternalLink>
  );
}

function ModifierAwareFileLink({
  href,
  className,
  children,
  ...rest
}: React.AnchorHTMLAttributes<HTMLAnchorElement>) {
  const { modifierHoverActive, hoverProps } = useModifierHoverState<HTMLAnchorElement>();
  return (
    <a
      data-allow-raw-anchor
      href={href}
      className={joinClassNames(className, modifierHoverActive && "ctx-modifier-hover")}
      {...hoverProps}
      {...rest}
    >
      {children}
    </a>
  );
}

function ModifierAwareCodePath({
  className,
  children,
  ...rest
}: React.HTMLAttributes<HTMLSpanElement>) {
  const { modifierHoverActive, hoverProps } = useModifierHoverState<HTMLSpanElement>();
  return (
    <span
      className={joinClassNames(className, modifierHoverActive && "ctx-modifier-hover")}
      {...hoverProps}
      {...rest}
    >
      {children}
    </span>
  );
}

const handleCodeTokenClick = async (
  event: MouseEvent<HTMLElement>,
  ref: FileRef,
  worktreeId: string | null,
  onFileOpenError?: (message: string | null) => void,
) => {
  if (!event.metaKey && !event.ctrlKey) return;
  if (!isDesktopApp()) return;
  if (!worktreeId && !isAbsolutePath(ref.path)) return;
  event.preventDefault();
  event.stopPropagation();

  try {
    if (isAbsolutePath(ref.path)) {
      await desktopOpenPath({
        path: ref.path,
        line: ref.line ?? null,
        col: ref.col ?? null,
      });
    } else {
      await desktopOpenFile({
        worktree_id: worktreeId ?? "",
        path: ref.path,
        line: ref.line ?? null,
        col: ref.col ?? null,
      });
    }
    onFileOpenError?.(null);
  } catch {
    // Ignore failures to keep interaction silent.
  }
};

const handleUrlTokenClick = (event: MouseEvent<HTMLElement>, href: string) => {
  if (!isDesktopApp()) return;
  if (!event.metaKey && !event.ctrlKey) {
    event.preventDefault();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  void openExternalLink(href);
};

const buildCodeTokenNodes = (text: string, opts: CodeTokenOptions, parts = splitWhitespaceTokens(text)): ReactNode[] => {
  return parts.map((part, idx) => {
    if (!part) return null;
    if (part.trim() === "") return part;
    if (opts.enableLinks) {
      const urlRef = parseUrlToken(part);
      if (urlRef) {
        return (
          <ModifierAwareExternalLink
            key={`token-${idx}`}
            className="code-token code-token-url"
            data-allow-raw-anchor
            href={urlRef.url}
            rel="noreferrer noopener"
            target="_blank"
            onClick={(event) => handleUrlTokenClick(event, urlRef.url)}
          >
            {part}
          </ModifierAwareExternalLink>
        );
      }

      const ref = parseFileRefToken(part);
      if (ref && (opts.worktreeId || isAbsolutePath(ref.path))) {
        return (
          <ModifierAwareCodePath
            key={`token-${idx}`}
            className="code-token code-token-path"
            onClick={(event) => handleCodeTokenClick(event, ref, opts.worktreeId, opts.onFileOpenError)}
          >
            {part}
          </ModifierAwareCodePath>
        );
      }
    }

    if (!opts.wrapPlainTokens) return part;
    return (
      <span key={`token-${idx}`} className="code-token">
        {part}
      </span>
    );
  });
};

function TokenizedInlineCode({
  codeString,
  codeParts,
  className,
  enableLinks,
  worktreeId,
  onFileOpenError,
}: {
  codeString: string;
  codeParts: readonly string[];
  className?: string;
  enableLinks: boolean;
  worktreeId: string | null;
  onFileOpenError?: (message: string | null) => void;
}) {
  const handleDoubleClick = useCallback((event: MouseEvent<HTMLElement>) => {
    if (event.metaKey || event.ctrlKey) return;
    const selection = window.getSelection();
    if (!selection) return;
    const range = document.createRange();
    range.selectNodeContents(event.currentTarget);
    selection.removeAllRanges();
    selection.addRange(range);
  }, []);

  const content = buildCodeTokenNodes(
    codeString,
    {
      enableLinks,
      worktreeId,
      onFileOpenError,
      wrapPlainTokens: true,
    },
    [...codeParts],
  );
  return (
    <code className={className} onDoubleClick={handleDoubleClick}>
      {content}
    </code>
  );
}

function FencedCodeBlock({
  codeString,
  enableLinks,
  worktreeId,
  onFileOpenError,
}: {
  codeString: string;
  enableLinks: boolean;
  worktreeId: string | null;
  onFileOpenError?: (message: string | null) => void;
}) {
  const [copied, setCopied] = useState(false);
  const resetTimerRef = useRef<number | null>(null);

  useEffect(() => {
    if (!copied) return;
    if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    resetTimerRef.current = window.setTimeout(() => {
      setCopied(false);
      resetTimerRef.current = null;
    }, 1000);
    return () => {
      if (resetTimerRef.current) window.clearTimeout(resetTimerRef.current);
    };
  }, [copied]);

  const handleCopy = useCallback(async () => {
    const ok = await copyTextToClipboard(codeString);
    if (!ok) return;

    setCopied(true);
  }, [codeString]);

  const content = enableLinks
    ? buildCodeTokenNodes(codeString, { enableLinks, worktreeId, onFileOpenError })
    : codeString;

  return (
    <div className="codeblock">
      <div className="codeblock-toolbar">
        <button
          type="button"
          className="wb-icon codeblock-copy"
          aria-label={copied ? "Copied" : "Copy code"}
          title={copied ? "Copied" : "Copy"}
          onClick={() => void handleCopy()}
        >
          {copied ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        </button>
      </div>
      <div className="codeblock-body">
        <pre className="codeblock-pre" onWheelCapture={forwardVerticalWheelToTranscript}>
          <code className="codeblock-code">{content}</code>
        </pre>
      </div>
    </div>
  );
}

type MarkdownRenderOptions = {
  enableLinks: boolean;
  worktreeId: string | null;
  onFileOpenError?: (message: string | null) => void;
};

function normalizeMarkdownHref(href: string): string {
  if (href.startsWith("ctx://")) return href;
  return defaultUrlTransform(href);
}

function renderMarkdownLink(
  href: string,
  children: ReactNode,
  opts: MarkdownRenderOptions,
  key: string,
  className?: string,
): ReactNode {
  const normalizedHref = normalizeMarkdownHref(href);
  const isContextOpen = normalizedHref.startsWith("ctx://open?");
  const markdownLinkClassName = [className, "ctx-markdown-link"].filter(Boolean).join(" ");
  if (!isContextOpen) {
    const modifierOpenTitle = isDesktopApp() ? "Cmd/Ctrl+Click to open link" : undefined;
    return (
      <ModifierAwareExternalLink
        key={key}
        href={normalizedHref}
        className={markdownLinkClassName}
        title={modifierOpenTitle}
        onClick={(event) => handleUrlTokenClick(event, normalizedHref)}
      >
        {children}
      </ModifierAwareExternalLink>
    );
  }

  const handleClick = async (event: MouseEvent<HTMLAnchorElement>) => {
    if (!isDesktopApp()) return;
    if (!event.metaKey && !event.ctrlKey) return;
    event.preventDefault();
    const parsed = parseContextOpenUrl(normalizedHref);
    if (!parsed) return;
    try {
      if (parsed.worktreeId && parsed.file) {
        await desktopOpenFile({
          worktree_id: parsed.worktreeId,
          path: parsed.file,
          line: parsed.line ?? null,
          col: parsed.col ?? null,
        });
      } else if (parsed.path) {
        await desktopOpenPath({
          path: parsed.path,
          line: parsed.line ?? null,
          col: parsed.col ?? null,
        });
      } else {
        return;
      }
      opts.onFileOpenError?.(null);
    } catch {
      // Ignore failures to keep interaction silent.
    }
  };

  return (
    <ModifierAwareFileLink
      key={key}
      href={normalizedHref}
      className={[className, "ctx-markdown-link", "ctx-file-link"].filter(Boolean).join(" ")}
      title="Cmd/Ctrl+Click to open in editor"
      onClick={handleClick}
    >
      {children}
    </ModifierAwareFileLink>
  );
}

function renderInlineNodes(
  nodes: readonly SessionMarkdownInlineNode[],
  opts: MarkdownRenderOptions,
  keyPrefix: string,
): ReactNode[] {
  return nodes.map((node, index) => {
    const key = `${keyPrefix}-${index}`;
    switch (node.kind) {
      case "text":
        return node.text;
      case "break":
        return <br key={key} />;
      case "inlineCode":
        return (
          <TokenizedInlineCode
            key={key}
            codeString={node.text}
            codeParts={node.parts}
            enableLinks={opts.enableLinks}
            worktreeId={opts.worktreeId}
            onFileOpenError={opts.onFileOpenError}
          />
        );
      case "strong":
        return <strong key={key}>{renderInlineNodes(node.children, opts, `${key}-strong`)}</strong>;
      case "emphasis":
        return <em key={key}>{renderInlineNodes(node.children, opts, `${key}-emphasis`)}</em>;
      case "delete":
        return <del key={key}>{renderInlineNodes(node.children, opts, `${key}-delete`)}</del>;
      case "link":
        return renderMarkdownLink(
          node.href,
          renderInlineNodes(node.children, opts, `${key}-link`),
          opts,
          key,
        );
      case "image":
        return <img key={key} src={node.src} alt={node.alt} title={node.title ?? undefined} />;
      default:
        return null;
    }
  });
}

function renderTableCellContent(
  cell: SessionMarkdownTableCell,
  isHeader: boolean,
  opts: MarkdownRenderOptions,
  keyPrefix: string,
): ReactNode {
  if (cell.blocks.length === 0) return null;
  return cell.blocks.map((block, index) => {
    const key = `${keyPrefix}-block-${index}`;
    switch (block.kind) {
      case "paragraph":
        return <Fragment key={key}>{renderInlineNodes(block.inlines, opts, `${key}-paragraph`)}</Fragment>;
      case "heading":
        switch (Math.max(1, Math.min(4, block.depth))) {
          case 1:
            return <h1 key={key}>{renderInlineNodes(block.inlines, opts, `${key}-heading`)}</h1>;
          case 2:
            return <h2 key={key}>{renderInlineNodes(block.inlines, opts, `${key}-heading`)}</h2>;
          case 3:
            return <h3 key={key}>{renderInlineNodes(block.inlines, opts, `${key}-heading`)}</h3>;
          default:
            return <h4 key={key}>{renderInlineNodes(block.inlines, opts, `${key}-heading`)}</h4>;
        }
      case "image":
        return <img key={key} src={block.src} alt={block.alt} title={block.title ?? undefined} />;
      case "thematicBreak":
        return <Fragment key={key}>{isHeader ? "—" : "—"}</Fragment>;
      case "code":
        return <code key={key} className="codeblock-code">{block.code}</code>;
      default:
        return null;
    }
  });
}

function renderListItem(item: SessionMarkdownListItem, opts: MarkdownRenderOptions, keyPrefix: string): ReactNode {
  const checkbox =
    item.checked == null ? null : (
      <input
        type="checkbox"
        checked={item.checked}
        readOnly
        disabled
        aria-hidden="true"
        className="wb-md-task-checkbox"
      />
    );
  return (
    <li key={keyPrefix} className={item.checked != null ? "wb-md-task-item" : "wb-md-list-item"}>
      {checkbox ? (
        <div className="wb-md-task-row">
          {checkbox}
          <div className="wb-md-list-item-body wb-md-stack">
            {renderBlockStack(item.blocks, opts, "listItem", `${keyPrefix}-body`)}
          </div>
        </div>
      ) : (
        <div className="wb-md-list-row">
          <span className="wb-md-list-item-marker" aria-hidden="true">
            {item.markerText}
          </span>
          <div className="wb-md-list-item-body wb-md-stack">
            {renderBlockStack(item.blocks, opts, "listItem", `${keyPrefix}-body`)}
          </div>
        </div>
      )}
    </li>
  );
}

function renderListBlock(
  block: SessionMarkdownListBlock,
  opts: MarkdownRenderOptions,
  keyPrefix: string,
): ReactNode {
  const ListTag = (block.ordered ? "ol" : "ul") as "ol" | "ul";
  const style = {
    "--wb-md-marker-column-width": `${block.markerColumnWidthPx}px`,
  } as CSSProperties;
  return (
    <ListTag
      className={block.ordered ? "wb-md-ordered-list" : "wb-md-unordered-list"}
      style={style}
    >
      {block.items.map((item, index) => renderListItem(item, opts, `${keyPrefix}-item-${index}`))}
    </ListTag>
  );
}

function renderBlock(
  block: SessionMarkdownBlock,
  opts: MarkdownRenderOptions,
  context: SessionMarkdownBlockContext,
  marginTopPx: number,
  keyPrefix: string,
): ReactNode {
  const shellStyle = marginTopPx > 0 ? { marginTop: `${marginTopPx}px` } : undefined;
  switch (block.kind) {
    case "paragraph":
      return (
        <div key={keyPrefix} className="wb-md-block wb-md-block--paragraph" style={shellStyle}>
          <p>{renderInlineNodes(block.inlines, opts, `${keyPrefix}-paragraph`)}</p>
        </div>
      );
    case "image":
      return (
        <div key={keyPrefix} className="wb-md-block wb-md-block--image" style={shellStyle}>
          <img src={block.src} alt={block.alt} title={block.title ?? undefined} />
        </div>
      );
    case "heading":
      return (
        <div key={keyPrefix} className={`wb-md-block wb-md-block--heading wb-md-block--heading-${block.depth}`} style={shellStyle}>
          {Math.max(1, Math.min(4, block.depth)) === 1 ? (
            <h1>{renderInlineNodes(block.inlines, opts, `${keyPrefix}-heading`)}</h1>
          ) : Math.max(1, Math.min(4, block.depth)) === 2 ? (
            <h2>{renderInlineNodes(block.inlines, opts, `${keyPrefix}-heading`)}</h2>
          ) : Math.max(1, Math.min(4, block.depth)) === 3 ? (
            <h3>{renderInlineNodes(block.inlines, opts, `${keyPrefix}-heading`)}</h3>
          ) : (
            <h4>{renderInlineNodes(block.inlines, opts, `${keyPrefix}-heading`)}</h4>
          )}
        </div>
      );
    case "list":
      return (
        <div key={keyPrefix} className="wb-md-block wb-md-block--list" style={shellStyle}>
          {renderListBlock(block, opts, `${keyPrefix}-list`)}
        </div>
      );
    case "blockquote":
      return (
        <div key={keyPrefix} className="wb-md-block wb-md-block--blockquote" style={shellStyle}>
          <blockquote className="wb-md-blockquote">
            <div className="wb-md-stack">{renderBlockStack(block.blocks, opts, "root", `${keyPrefix}-blockquote`)}</div>
          </blockquote>
        </div>
      );
    case "code":
      return (
        <div key={keyPrefix} className="wb-md-block wb-md-block--code" style={shellStyle}>
          <FencedCodeBlock
            codeString={block.code}
            enableLinks={opts.enableLinks}
            worktreeId={opts.worktreeId}
            onFileOpenError={opts.onFileOpenError}
          />
        </div>
      );
    case "table":
      return (
        <div key={keyPrefix} className="wb-md-block wb-md-block--table" style={shellStyle}>
          <div className="wb-md-table-scroll" onWheelCapture={forwardVerticalWheelToTranscript}>
            <table className="wb-md-table">
              <tbody className="wb-md-table-body">
                {block.rows.map((row, rowIndex) => (
                  <tr key={`${keyPrefix}-row-${rowIndex}`} className="wb-md-table-row">
                    {row.cells.map((cell, cellIndex) => {
                      const isHeader = rowIndex === 0;
                      const CellTag = isHeader ? "th" : "td";
                      return (
                        <CellTag
                          key={`${keyPrefix}-row-${rowIndex}-cell-${cellIndex}`}
                          className={joinClassNames(
                            "wb-md-table-cell",
                            isHeader && "wb-md-table-cell-head",
                          )}
                        >
                          {renderTableCellContent(
                            cell,
                            isHeader,
                            opts,
                            `${keyPrefix}-row-${rowIndex}-cell-${cellIndex}`,
                          )}
                        </CellTag>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      );
    case "thematicBreak":
      return (
        <div key={keyPrefix} className="wb-md-block wb-md-block--thematic-break" style={shellStyle}>
          <hr />
        </div>
      );
    default:
      return null;
  }
}

function renderBlockStack(
  blocks: readonly SessionMarkdownBlock[],
  opts: MarkdownRenderOptions,
  context: SessionMarkdownBlockContext,
  keyPrefix: string,
): ReactNode[] {
  return blocks.map((block, index) => {
    const marginTopPx =
      index === 0
        ? resolveSessionMarkdownBlockEntryGapPx(block.kind, context)
        : resolveSessionMarkdownBlockGapPx(blocks[index - 1]!.kind, block.kind, context);
    return renderBlock(block, opts, context, marginTopPx, `${keyPrefix}-${index}`);
  });
}

export function Markdown({
  content,
  linkifyFiles = false,
  worktreeId = null,
  onFileOpenError,
}: {
  content: string;
  linkifyFiles?: boolean;
  worktreeId?: string | null;
  onFileOpenError?: (message: string | null) => void;
}) {
  const blocks = useMemo(() => createSessionMarkdownDocument(content).blocks, [content]);
  const renderOptions = useMemo<MarkdownRenderOptions>(
    () => ({
      enableLinks: Boolean(linkifyFiles),
      worktreeId,
      onFileOpenError,
    }),
    [linkifyFiles, onFileOpenError, worktreeId],
  );

  return (
    <div className="wb-markdown-root wb-md-stack">
      {renderBlockStack(blocks, renderOptions, "root", "markdown")}
    </div>
  );
}

// Memoize markdown so selection isn't disrupted by unrelated re-renders.
export const MemoMarkdown = memo(
  Markdown,
  (prev, next) =>
    prev.content === next.content &&
    prev.linkifyFiles === next.linkifyFiles &&
    prev.worktreeId === next.worktreeId,
);
