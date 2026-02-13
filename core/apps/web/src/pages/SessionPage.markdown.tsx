import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, Copy } from "lucide-react";
import { copyTextToClipboard } from "../utils/clipboard";
import { stripCitationMarkers } from "../utils/citationMarkers";
import {
  type FileRef,
  isAbsolutePath,
  parseFileRefToken,
  parseUrlToken,
  splitWhitespaceTokens,
} from "../utils/codeTokenLinks";
import { desktopOpenFile, desktopOpenPath, isDesktopApp, openExternalLink } from "../utils/desktop";

type MdastNode = {
  type?: string;
  children?: MdastNode[];
  [key: string]: unknown;
};

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
  if (!event.metaKey && !event.ctrlKey) {
    event.preventDefault();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  void openExternalLink(href);
};

const buildCodeTokenNodes = (text: string, opts: CodeTokenOptions): ReactNode[] => {
  const parts = splitWhitespaceTokens(text);
  return parts.map((part, idx) => {
    if (!part) return null;
    if (part.trim() === "") return part;
    if (opts.enableLinks) {
      const urlRef = parseUrlToken(part);
      if (urlRef) {
        return (
          <a
            key={`token-${idx}`}
            className="code-token code-token-url"
            href={urlRef.url}
            rel="noreferrer noopener"
            target="_blank"
            onClick={(event) => handleUrlTokenClick(event, urlRef.url)}
          >
            {part}
          </a>
        );
      }

      const ref = parseFileRefToken(part);
      if (ref && (opts.worktreeId || isAbsolutePath(ref.path))) {
        return (
          <span
            key={`token-${idx}`}
            className="code-token code-token-path"
            onClick={(event) => handleCodeTokenClick(event, ref, opts.worktreeId, opts.onFileOpenError)}
          >
            {part}
          </span>
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
  className,
  enableLinks,
  worktreeId,
  onFileOpenError,
}: {
  codeString: string;
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

  const content = buildCodeTokenNodes(codeString, {
    enableLinks,
    worktreeId,
    onFileOpenError,
    wrapPlainTokens: true,
  });
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
        <pre className="codeblock-pre">
          <code className="codeblock-code">{content}</code>
        </pre>
      </div>
    </div>
  );
}

function remarkNormalizeCursorMarkdown() {
  return (tree: MdastNode) => {
    const walk = (node: MdastNode) => {
      if (node?.type === "listItem" && Array.isArray(node.children)) {
        const maybeInlineFromCode = (codeNode: MdastNode): string | null => {
          if (codeNode?.type !== "code") return null;
          const lang = typeof codeNode.lang === "string" ? codeNode.lang : undefined;
          const raw = typeof codeNode.value === "string" ? codeNode.value : "";
          const trimmed = raw.replace(/[\r\n]+$/, "");
          if ((!lang || lang === "code") && trimmed.length > 0 && !trimmed.includes("\n")) return trimmed;
          return null;
        };

        // Cursor sometimes emits list items that are a single one-line fenced code block.
        if (node.children.length === 1) {
          const trimmed = maybeInlineFromCode(node.children[0] as MdastNode);
          if (trimmed) {
            node.children = [
              {
                type: "paragraph",
                children: [{ type: "inlineCode", value: trimmed }],
              },
            ];
          }
        }

        // Some models emit list items as:
        //   - code
        //     ```code
        //     .github/
        //     ```
        // Normalize that to a single inlineCode row.
        if (node.children.length === 2) {
          const [p, codeNode] = node.children as MdastNode[];
          const pChildren = (p as MdastNode & { children?: MdastNode[] })?.children;
          if (p?.type === "paragraph" && Array.isArray(pChildren)) {
            const kids = pChildren;
            if (
              kids.length === 1 &&
              kids[0]?.type === "text" &&
              String((kids[0] as MdastNode & { value?: unknown }).value ?? "").trim().toLowerCase() === "code"
            ) {
              const trimmed = maybeInlineFromCode(codeNode);
              if (trimmed) {
                node.children = [
                  {
                    type: "paragraph",
                    children: [{ type: "inlineCode", value: trimmed }],
                  },
                ];
              }
            }
          }
        }
      }

      if (!Array.isArray(node.children)) return;
      for (let i = 0; i < node.children.length; i++) {
        const child = node.children[i] as MdastNode;
        if (
          child?.type === "paragraph" &&
          Array.isArray(child.children) &&
          child.children.length === 1 &&
          (child.children[0] as MdastNode | undefined)?.type === "text"
        ) {
          const raw = String((child.children[0] as MdastNode & { value?: unknown } | undefined)?.value ?? "");
          const trimmed = raw.trim();
          if (/^(⸻|—{3,}|-{3,}|_{3,}|\*{3,})$/.test(trimmed)) {
            node.children[i] = { type: "thematicBreak" };
            continue;
          }
        }
        walk(child);
      }
    };

    walk(tree);
  };
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
  const remarkPlugins = [remarkGfm, remarkNormalizeCursorMarkdown];
  // Models/providers sometimes emit citations using private-use unicode wrappers. Those IDs are not
  // resolvable links in ctx today (we don't store the underlying sources table), so hide them.
  const normalized = stripCitationMarkers(content);

  return (
    <div>
      <ReactMarkdown
        remarkPlugins={remarkPlugins}
        urlTransform={(url) =>
          url.startsWith("ctx://")
            ? url
            : defaultUrlTransform(url)
        }
        components={{
          a({ href, children, className, ...rest }) {
            const isContextOpen =
              typeof href === "string" && href.startsWith("ctx://open?");
            if (!isContextOpen) {
              const handleExternalClick = (event: MouseEvent<HTMLAnchorElement>) => {
                if (!isDesktopApp()) return;
                if (!href) return;
                event.preventDefault();
                void openExternalLink(href);
              };
              return (
                <a href={href} className={className} onClick={handleExternalClick} {...rest}>
                  {children}
                </a>
              );
            }

            const handleClick = async (event: MouseEvent<HTMLAnchorElement>) => {
              if (!isDesktopApp()) {
                return;
              }
              if (!event.metaKey && !event.ctrlKey) return;
              event.preventDefault();
              if (!href) return;
              const parsed = parseContextOpenUrl(href);
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
                onFileOpenError?.(null);
              } catch {
                // Ignore failures to keep interaction silent.
              }
            };

            const combinedClassName = [className, "ctx-file-link"].filter(Boolean).join(" ");
            return (
              <a
                href={href}
                className={combinedClassName}
                title="Cmd/Ctrl+Click to open in editor"
                onClick={handleClick}
                {...rest}
              >
                {children}
              </a>
            );
          },
          pre({ children }) {
            return <>{children}</>;
          },
          code({ inline, className, children }: { inline?: boolean; className?: string; children?: ReactNode }) {
            const match = /language-([A-Za-z0-9_-]+)/.exec(className || "");
            const rawLang = match?.[1];
            const lang = rawLang && rawLang !== "code" ? rawLang : undefined;
            const codeString = String(children ?? "").replace(/[\r\n]+$/, "");
            const enableLinks = Boolean(linkifyFiles);
            if (inline || (!lang && !codeString.includes("\n"))) {
              return (
                <TokenizedInlineCode
                  codeString={codeString}
                  className={className}
                  enableLinks={enableLinks}
                  worktreeId={worktreeId}
                  onFileOpenError={onFileOpenError}
                />
              );
            }

            return (
              <FencedCodeBlock
                codeString={codeString}
                enableLinks={enableLinks}
                worktreeId={worktreeId}
                onFileOpenError={onFileOpenError}
              />
            );
          },
        }}
      >
        {normalized}
      </ReactMarkdown>
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
