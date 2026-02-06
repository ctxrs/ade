import {
  Fragment,
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type MutableRefObject,
  type PointerEvent,
  type ReactNode,
} from "react";
import { Check, Copy } from "lucide-react";
import {
  VirtuosoMessageList,
  VirtuosoMessageListLicense,
  type ContextAwareComponent,
  type DataWithScrollModifier,
  type FooterWrapperComponent,
  type ItemLocation,
  type ItemContent as MessageItemContent,
  type ScrollElementComponent,
} from "@virtuoso.dev/message-list";
import { blobUrl, type MessageAttachment } from "../api/client";
import { type SessionViewVerbosity } from "../state/uiStateStore";
import { copyTextToClipboard } from "../utils/clipboard";
import { MemoMarkdown } from "./SessionPage.markdown";
import {
  attachmentDisplayName,
  formatElapsedMs,
  formatToolInput,
  humanToolKind,
  humanToolStatus,
  humanTurnStatus,
  imageAttachmentSrc,
  looksLikeMarkdown,
  parseIsoMs,
  toolKindIcon,
  toolSummaryLine,
  truncateMiddle,
} from "./SessionPage.helpers";
import type { ThreadItem, WorkbenchListItem, WorkbenchTurnHeader } from "./SessionPage.types";

type WorkbenchMessageListStackProps = {
  virtuosoStyle: CSSProperties;
  data: DataWithScrollModifier<WorkbenchListItem> | null | undefined;
  itemContent: (index: number, item: WorkbenchListItem) => ReactNode;
  itemIdentity: (item: WorkbenchListItem) => unknown;
  initialLocation?: ItemLocation;
  scrollElement: ScrollElementComponent;
  footer: ContextAwareComponent;
  footerWrapper: FooterWrapperComponent;
  increaseViewportBy: number;
  showJumpToLatest: boolean;
  onJumpToLatest: () => void;
  licenseKey: string;
  scrollbarActive: boolean;
  scrollbarDragging: boolean;
  scrollbarNeeded: boolean;
  scrollbarTrackRef: MutableRefObject<HTMLDivElement | null>;
  scrollbarThumbRef: MutableRefObject<HTMLDivElement | null>;
  onScrollbarMouseLeave: () => void;
  onScrollbarTrackPointerDown: (event: PointerEvent<HTMLDivElement>) => void;
  onScrollbarThumbPointerDown: (event: PointerEvent<HTMLDivElement>) => void;
  onScrollbarThumbPointerMove: (event: PointerEvent<HTMLDivElement>) => void;
  onScrollbarThumbPointerUp: (event: PointerEvent<HTMLDivElement>) => void;
  scheduleScrollbarUpdate: () => void;
};

export const WorkbenchMessageListStack = memo(function WorkbenchMessageListStack({
  virtuosoStyle,
  data,
  itemContent,
  itemIdentity,
  initialLocation,
  scrollElement,
  footer,
  footerWrapper,
  increaseViewportBy,
  showJumpToLatest,
  onJumpToLatest,
  licenseKey,
  scrollbarActive,
  scrollbarDragging,
  scrollbarNeeded,
  scrollbarTrackRef,
  scrollbarThumbRef,
  onScrollbarMouseLeave,
  onScrollbarTrackPointerDown,
  onScrollbarThumbPointerDown,
  onScrollbarThumbPointerMove,
  onScrollbarThumbPointerUp,
  scheduleScrollbarUpdate,
}: WorkbenchMessageListStackProps) {
  const ItemContent = useCallback<MessageItemContent<WorkbenchListItem, unknown>>(
    ({ index, data }) => {
      if (!data) return <div style={{ height: 1 }} />;
      return (
        <div role="listitem" data-thread-item-id={data.id}>
          {itemContent(index, data)}
        </div>
      );
    },
    [itemContent],
  );

  return (
    <div className="thread-stack wb-thread-stack" onMouseLeave={onScrollbarMouseLeave}>
      <VirtuosoMessageListLicense licenseKey={licenseKey}>
        <VirtuosoMessageList<WorkbenchListItem, unknown>
          style={virtuosoStyle}
          data={data}
          itemIdentity={itemIdentity}
          computeItemKey={({ data }) => data.id}
          ItemContent={ItemContent}
          initialLocation={initialLocation}
          ScrollElement={scrollElement}
          Footer={footer}
          FooterWrapper={footerWrapper}
          shortSizeAlign="bottom"
          increaseViewportBy={increaseViewportBy}
        />
      </VirtuosoMessageListLicense>

      <div
        className={`wb-scrollbar${scrollbarActive ? " is-active" : ""}${scrollbarDragging ? " is-dragging" : ""}${scrollbarNeeded ? "" : " is-hidden"}`}
        aria-hidden="true"
      >
        <div
          className="wb-scrollbar-track"
          ref={(node) => {
            scrollbarTrackRef.current = node;
            if (node) scheduleScrollbarUpdate();
          }}
          onPointerDown={onScrollbarTrackPointerDown}
        >
          <div
            className="wb-scrollbar-thumb"
            ref={(node) => {
              scrollbarThumbRef.current = node;
              if (node) scheduleScrollbarUpdate();
            }}
            onPointerDown={onScrollbarThumbPointerDown}
            onPointerMove={onScrollbarThumbPointerMove}
            onPointerUp={onScrollbarThumbPointerUp}
            onPointerCancel={onScrollbarThumbPointerUp}
          />
        </div>
      </div>

      {showJumpToLatest && (
        <button
          type="button"
          className="new-activity-overlay"
          aria-label="Jump to latest"
          title="Jump to latest"
          onClick={onJumpToLatest}
        >
          ↓
        </button>
      )}
    </div>
  );
});

export function ThreadItemView({
  item,
  worktreeId,
  onFileOpenError,
  modifierDown,
  onToggleMessageExpanded,
}: {
  item: ThreadItem;
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  modifierDown: boolean;
  onToggleMessageExpanded?: (expanded: boolean) => void;
}) {
  switch (item.kind) {
    case "message":
      return (
        <CollapsibleMessage
          id={item.id}
          role={item.role}
          content={item.content}
          attachments={item.attachments}
          worktreeId={worktreeId}
          onFileOpenError={onFileOpenError}
          modifierDown={modifierDown}
          onToggleExpanded={onToggleMessageExpanded}
        />
      );
    case "assistant":
    case "tool":
      return null;
    case "tool_group":
    case "turn_status":
      return null;
    default:
      return null;
  }
}

export function WorkbenchTurnHeaderView({
  header,
  plainText,
  expanded,
  onToggle,
}: {
  header: WorkbenchTurnHeader;
  plainText: string;
  expanded: boolean;
  onToggle: () => void;
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

  const handleCopy = useCallback(
    async (e: MouseEvent) => {
      e.stopPropagation();
      const content = header.content ?? "";
      if (!content.trim()) return;
      const ok = await copyTextToClipboard(content);
      if (!ok) return;
      setCopied(true);
    },
    [header.content]
  );

  const handleClick = () => {
    const selection = window.getSelection()?.toString() ?? "";
    if (selection.trim()) return;
    onToggle();
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onToggle();
  };

  const hasContent = (header.content ?? "").trim().length > 0;

  return (
    <div
      className={`wb-turn-header ${expanded ? "wb-turn-header-expanded" : "wb-turn-header-collapsed"}`}
      role="button"
      tabIndex={0}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      aria-expanded={expanded}
    >
      <div className="wb-turn-header-bubble">
        {hasContent && (
          <button
            type="button"
            className="wb-turn-header-copy"
            aria-label={copied ? "Copied" : "Copy message"}
            title={copied ? "Copied" : "Copy message"}
            onClick={handleCopy}
          >
            {copied ? <Check size={12} aria-hidden="true" /> : <Copy size={12} aria-hidden="true" />}
          </button>
        )}
        <div className="wb-turn-header-content">
          {plainText.split("\n").map((line, idx, list) => (
            <span key={`${header.id}-${idx}`}>
              {line}
              {idx < list.length - 1 ? <br /> : null}
            </span>
          ))}
        </div>
        {header.attachments.length > 0 && (
          <div className="wb-turn-header-attachments" aria-label="Attachments">
            {header.attachments.map((a, idx) => {
              if (a.kind !== "image" && a.kind !== "image_ref") return null;
              const src = imageAttachmentSrc(a);
              const name = attachmentDisplayName(a.name);
              return <img key={idx} className="wb-turn-header-attachment-img" src={src} alt={name} title={name} />;
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function CollapsibleMessage({
  id,
  role,
  content,
  attachments,
  worktreeId,
  onFileOpenError,
  modifierDown,
  onToggleExpanded,
}: {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  attachments: MessageAttachment[];
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  modifierDown: boolean;
  onToggleExpanded?: (expanded: boolean) => void;
}) {
  const lines = (content || "").split("\n");
  const isLong = lines.length > 20 || content.length > 1500;
  const [expanded, setExpanded] = useState(!isLong);
  const shown = expanded ? content : lines.slice(0, 20).join("\n");

  return (
    <div className={`msg ${role}`}>
      <div className="role">{role}</div>
      <div id={`msg-${id}`}>
        <div className={modifierDown ? "markdown-modifier" : undefined}>
          <MemoMarkdown
            content={shown}
            linkifyFiles={role === "assistant"}
            worktreeId={worktreeId}
            onFileOpenError={onFileOpenError}
          />
        </div>
      </div>
      {attachments?.length > 0 && (
        <div className="attachments">
          {attachments.map((a, idx) => {
            if (a.kind !== "image" && a.kind !== "image_ref") return null;
            const src =
              a.kind === "image_ref"
                ? blobUrl(a.blob_id)
                : `data:${a.mime_type};base64,${a.data_base64}`;
            return (
              <img
                key={idx}
                className="attachment-img"
                src={src}
                alt={a.name ?? `image-${idx}`}
              />
            );
          })}
        </div>
      )}
      {isLong && (
        <button
          type="button"
          className="link"
          aria-expanded={expanded}
          aria-controls={`msg-${id}`}
          onClick={() =>
            setExpanded((e) => {
              const next = !e;
              onToggleExpanded?.(next);
              return next;
            })
          }
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}

export function AssistantEntry({
  content,
  worktreeId,
  onFileOpenError,
  modifierDown,
}: {
  content: string;
  worktreeId: string | null;
  onFileOpenError: (message: string | null) => void;
  modifierDown: boolean;
}) {
  return (
    <div className="wb-assistant-entry">
      <div className="wb-assistant-body">
        <div className={modifierDown ? "markdown-modifier" : undefined}>
          <MemoMarkdown
            content={content}
            linkifyFiles
            worktreeId={worktreeId}
            onFileOpenError={onFileOpenError}
          />
        </div>
      </div>
    </div>
  );
}

export function WorkbenchToolRow({
  item,
  verbosity,
  expanded,
  onToggle,
}: {
  item: Extract<ThreadItem, { kind: "tool" }>;
  verbosity: SessionViewVerbosity;
  expanded: boolean;
  onToggle: () => void;
}) {
  const kind = String(item.tool_kind ?? "").toLowerCase();
  const pathFromLoc = item.locations?.[0]?.path;
  const title = String(item.title ?? "").trim();
  const summary = toolSummaryLine(kind, item.input);

  const normalizeWorktreePath = (p?: string) => {
    const s = String(p ?? "").trim();
    if (!s) return "";
    // Strip the ctx worktree prefix.
    return s.replace(
      /\/home\/[^/]+\/\.ctx\/worktrees\/[0-9a-f-]+\/[0-9a-f-]+\//g,
      "",
    );
  };

  const shortPath = (p?: string) => {
    const s0 = normalizeWorktreePath(p);
    const s = String(s0 ?? "").trim();
    if (!s) return "";
    const parts = s.split(/[\\/]/).filter(Boolean);
    if (parts.length <= 2) return s;
    return `${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
  };

  const shortCommand = (raw: string) => {
    let cmd = String(raw ?? "").trim();
    if (!cmd) return "";
    cmd = cmd.replace(/^\/bin\/bash\s+-lc\s+/, "");
    cmd = cmd.replace(/^bash\s+-lc\s+/, "");
    cmd = cmd.replace(/^set\s+-euo\s+pipefail\s*;?\s*/i, "");
    cmd = cmd.replace(/^\s*&&\s*/, "");
    cmd = cmd.replace(/^"(.+)"$/, "$1");
    cmd = cmd.replace(
      /\/home\/[^/]+\/\.ctx\/worktrees\/[0-9a-f-]+\/[0-9a-f-]+\//g,
      "",
    );
    const mSupercat = cmd.match(/(?:^|\s)(\.?\/?scripts\/supercat\.sh)\s+([^\s&;]+)/);
    if (mSupercat) return `./scripts/supercat.sh ${shortPath(mSupercat[2])}`;
    const mReadMany =
      cmd.match(/rg\s+--files\s+([^\s|&;]+)\s*\|\s*sort\b[\s\S]*xargs[\s\S]*\bcat\b/) ??
      cmd.match(/find\s+([^\s|&;]+)\s+.*xargs[\s\S]*\bcat\b/);
    if (mReadMany) return `Read ${shortPath(mReadMany[1])}`;
    const mCat = cmd.match(/(?:^|[;&|]\s*)cat\s+([^\s|&;]+)(?:\s|$)/);
    if (mCat) return `Read ${shortPath(mCat[1])}`;
    const mLs = cmd.match(/(?:^|[;&|]\s*)(?:ls|ls\s+-la|ls\s+-l)\s+([^\s|&;]+)(?:\s|$)/);
    if (mLs) return `Explored ${shortPath(mLs[1])}`;
    const mRg = cmd.match(/(?:^|[;&|]\s*)rg\s+([^\s]+)\s+([^\s|&;]+)(?:\s|$)/);
    if (mRg) return `Searched ${truncateMiddle(mRg[1], 60)}`;
    const first = cmd.split(/\s+/).slice(0, 4).join(" ");
    return truncateMiddle(first, 80);
  };

  const makeParts = (verb: string, rest?: string) => {
    const trimmedRest = String(rest ?? "").trim();
    return {
      verb,
      rest: trimmedRest,
      label: trimmedRest ? `${verb} ${trimmedRest}` : verb,
    };
  };

  const parsePrefixed = (value: string, verbs: string[]) => {
    const trimmed = String(value ?? "").trim();
    if (!trimmed) return null;
    for (const verb of verbs) {
      if (trimmed === verb) return makeParts(verb);
      if (trimmed.startsWith(`${verb} `)) return makeParts(verb, trimmed.slice(verb.length + 1));
    }
    return null;
  };

  const labelParts = (() => {
    const normalizedTitle = normalizeWorktreePath(title);
    const titlePrefixed = parsePrefixed(normalizedTitle, [
      "Read",
      "Explored",
      "Searched",
      "Wrote",
      "Edited",
      "Run",
      "Fetch",
      "Search",
      "List",
      "Write",
      "Edit",
    ]);
    const titleRest = titlePrefixed?.rest ?? "";
    const trimmedSummary = String(summary ?? "").trim();
    const hasSummary = trimmedSummary.length > 0;

    if (normalizedTitle && normalizedTitle !== "Tool") {
      if (titlePrefixed) {
        if (!titlePrefixed.rest && hasSummary) return makeParts(titlePrefixed.verb, trimmedSummary);
        return titlePrefixed;
      }
      if (hasSummary && !/\s/.test(normalizedTitle)) {
        return makeParts(normalizedTitle, trimmedSummary);
      }
      return makeParts(normalizedTitle);
    }

    const parsed = Array.isArray((item.input as any)?.parsed_cmd) ? ((item.input as any).parsed_cmd as any[]) : [];
    if (parsed.length > 0) {
      const c0 = parsed[0] ?? {};
      if (c0.type === "list_files" && c0.path) return makeParts("Explored", shortPath(c0.path));
      if (c0.type === "read_file" && c0.path) return makeParts("Read", shortPath(c0.path));
      if (c0.type === "search") {
        const q = String(c0.query ?? c0.pattern ?? c0.regex ?? c0.text ?? "").trim();
        if (q) return makeParts("Searched", truncateMiddle(q, 90));
        if (c0.path) return makeParts("Searched", shortPath(c0.path));
        return makeParts("Searched");
      }
    }
    if (kind === "search") {
      const q = String(
        item.input?.query ??
          item.input?.pattern ??
          item.input?.regex ??
          item.input?.text ??
          summary ??
          titleRest ??
          "",
      ).trim();
      return q ? makeParts("Searched", truncateMiddle(q, 90)) : makeParts("Searched");
    }
    if (kind === "execute") {
      const cmd = Array.isArray(item.input?.command) ? item.input.command.join(" ") : item.input?.command;
      const short = shortCommand(cmd ?? "");
      const parsedShort = parsePrefixed(short, ["Read", "Explored", "Searched", "Wrote", "Edited"]);
      if (parsedShort) return parsedShort;
      return short ? makeParts("Run", short) : makeParts("Run");
    }
    if (kind === "read_file" || kind === "read") {
      const p = pathFromLoc ?? summary ?? titleRest;
      return p ? makeParts("Read", shortPath(p)) : makeParts("Read");
    }
    if (kind === "list" || kind === "list_files") {
      const p = summary || pathFromLoc || titleRest;
      return p ? makeParts("Explored", shortPath(p)) : makeParts("Explored");
    }
    if (kind === "write" || kind === "edit" || kind === "apply_patch") {
      const p = summary || pathFromLoc || titleRest;
      const verb = kind === "write" ? "Wrote" : "Edited";
      return p ? makeParts(verb, shortPath(p)) : makeParts(verb);
    }
    if (kind === "fetch" || kind === "http") {
      const p = summary || titleRest;
      return p ? makeParts("Fetch", p) : makeParts("Fetch");
    }
    if (kind === "error") return makeParts("Error");

    const fallbackLabel = normalizeWorktreePath(title) || humanToolKind(item.tool_kind);
    const parsedLabel = parsePrefixed(fallbackLabel, [
      "Read",
      "Explored",
      "Searched",
      "Wrote",
      "Edited",
      "Run",
    ]);
    if (parsedLabel) return parsedLabel;
    const fallbackVerb = humanToolKind(item.tool_kind);
    if (fallbackLabel && fallbackLabel !== fallbackVerb) {
      return makeParts(fallbackVerb, fallbackLabel);
    }
    return makeParts(fallbackVerb);
  })();

  const { verb, rest, label } = labelParts;
  const showOutputPreview = verbosity === "verbose";
  const hasOutputText = !!item.output_text?.trim();
  const hasDetails = !!item.input || (showOutputPreview && hasOutputText);

  return (
    <div className="wb-tool-row">
      <button
        type="button"
        className={`wb-event-row ${expanded ? "wb-event-row-expanded" : ""}`}
        onClick={hasDetails ? onToggle : undefined}
        aria-expanded={hasDetails ? expanded : undefined}
        title={label}
      >
        <span className="wb-event-text">
          <span className="wb-tool-verb">{verb}</span>
          {rest ? (
            <>
              <span className="wb-tool-sep" aria-hidden="true">
                ·
              </span>
              <span className="wb-tool-rest">{rest}</span>
            </>
          ) : null}
        </span>
      </button>
      {hasDetails && expanded && (
        <div className="wb-tool-details">
          {item.input && (
            <div className="wb-tool-section">
              <div className="wb-tool-section-title">Input</div>
              <pre className="wb-tool-pre">{formatToolInput(item.tool_kind, item.input)}</pre>
            </div>
          )}
          {showOutputPreview && !!item.output_text?.trim() && (
            <div className="wb-tool-section">
              <div className="wb-tool-section-title">Output</div>
              {looksLikeMarkdown(item.output_text) ? (
                <div className="wb-tool-markdown">
                  <MemoMarkdown content={item.output_text} />
                </div>
              ) : (
                <pre className="wb-tool-pre">{item.output_text}</pre>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function WorkbenchThoughtRow({ item }: { item: Extract<ThreadItem, { kind: "thought" }> }) {
  return <div className="wb-thought-row">{item.content}</div>;
}

export function WorkbenchTurnStatusRow({
  item,
  nowMs,
}: {
  item: Extract<ThreadItem, { kind: "turn_status" }>;
  nowMs: number;
}) {
  const isRunning = item.status === "running" || item.status === "queued";
  const isCompleted = item.status === "completed";
  const customStatus = item.custom_status?.trim();
  const statusLabel = isRunning && customStatus ? customStatus : humanTurnStatus(item.status);
  const startMs = parseIsoMs(item.started_at);
  const endMs = isRunning ? nowMs : parseIsoMs(item.updated_at) ?? nowMs;
  const elapsedMs = startMs != null && endMs != null ? Math.max(0, endMs - startMs) : 0;
  const elapsedLabel = formatElapsedMs(elapsedMs);

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
    const content = item.assistant_messages_content ?? "";
    if (!content.trim()) return;
    const ok = await copyTextToClipboard(content);
    if (!ok) return;
    setCopied(true);
  }, [item.assistant_messages_content]);

  const hasContent = (item.assistant_messages_content ?? "").trim().length > 0;
  const showCopyButton = isCompleted && hasContent;

  return (
    <div className="wb-turn-status">
      <span className="wb-turn-status-label">{statusLabel}</span>
      <span className="wb-turn-status-dot" aria-hidden="true">
        ·
      </span>
      <span className="wb-turn-status-time">{elapsedLabel}</span>
      {showCopyButton && (
        <>
          <span className="wb-turn-status-dot" aria-hidden="true">
            ·
          </span>
          <button
            type="button"
            className="wb-turn-status-copy"
            aria-label={copied ? "Copied" : "Copy response"}
            title={copied ? "Copied" : "Copy response"}
            onClick={() => void handleCopy()}
          >
            {copied ? <Check size={12} aria-hidden="true" /> : <Copy size={12} aria-hidden="true" />}
          </button>
        </>
      )}
    </div>
  );
}

export function WorkbenchToolGroupRow({
  item,
  verbosity,
  expanded,
  onToggle,
  toolsLoading,
  onRequestTools,
  onToggleTool,
  expandedToolById,
}: {
  item: Extract<ThreadItem, { kind: "tool_group" }>;
  verbosity: SessionViewVerbosity;
  expanded: boolean;
  onToggle: () => void;
  toolsLoading: boolean;
  onRequestTools: () => void;
  onToggleTool: (id: string) => void;
  expandedToolById: Record<string, boolean>;
}) {
  const total = Math.max(item.tool_total ?? 0, item.tools.length);
  const parts: string[] = [];
  if (total > 0) {
    parts.push(`${total} tool${total === 1 ? "" : "s"}`);
  }
  if ((item.tool_running ?? 0) > 0) {
    parts.push(`${item.tool_running} running`);
  }
  if ((item.tool_failed ?? 0) > 0) {
    parts.push(`${item.tool_failed} failed`);
  }
  if (parts.length === 0 && item.thought.trim()) {
    parts.push("Thought");
  }
  const label = parts.join(" · ") || "Activity";
  const hasDetails = total > 0 || item.thought.trim().length > 0;

  useEffect(() => {
    if (!expanded) return;
    if (total > 0 && item.tools.length === 0 && !toolsLoading) {
      onRequestTools();
    }
  }, [expanded, total, item.tools.length, toolsLoading, onRequestTools]);

  return (
    <div className="wb-tool-group">
      <button
        type="button"
        className={`wb-event-row ${expanded ? "wb-event-row-expanded" : ""}`}
        onClick={hasDetails ? onToggle : undefined}
        aria-expanded={hasDetails ? expanded : undefined}
        title={label}
      >
        <span className="wb-event-text">{label}</span>
        {hasDetails && <span className="wb-event-chev">{expanded ? "▴" : "▾"}</span>}
      </button>

      {hasDetails && expanded && (
        <div className="wb-tool-group-body">
          {total > 0 && item.tools.length === 0 && toolsLoading && (
            <div className="wb-tool-loading">Loading tools...</div>
          )}
          {item.tools.map((tool) => (
            <WorkbenchToolRow
              key={tool.id}
              item={tool}
              verbosity={verbosity}
              expanded={expandedToolById[tool.id] ?? false}
              onToggle={() => onToggleTool(tool.id)}
            />
          ))}
          {item.thought.trim() && (
            <div className="wb-tool-thought">
              <div className="wb-tool-section-title">Thought</div>
              <pre className="wb-tool-pre">{item.thought}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ToolCard({ item }: { item: Extract<ThreadItem, { kind: "tool" }> }) {
  const [expanded, setExpanded] = useState(false);
  const isRunning = item.status === "in_progress" || item.status === "pending";
  const isFailed = item.status === "failed";
  const hasOutput = item.output_text.trim().length > 0;
  const shouldDefaultOpen = item.tool_kind === "execute" && isRunning;
  const isOpen = expanded || shouldDefaultOpen;
  const summary = useMemo(() => toolSummaryLine(item.tool_kind, item.input), [item.tool_kind, item.input]);
  const path = item.locations?.length === 1 ? item.locations[0]?.path : null;
  const subtitleItems: ReactNode[] = [
    <span key="status" className={`pill ${isFailed ? "err" : isRunning ? "run" : "ok"}`}>
      {humanToolStatus(item.status)}
    </span>,
  ];
  if (path) subtitleItems.push(<span key="path" className="muted tool-path">{path}</span>);
  if (summary) subtitleItems.push(<span key="summary" className="muted tool-summary">{summary}</span>);

  return (
    <div className={`tool-card ${isOpen ? "expanded" : ""}`}>
      <button
        type="button"
        className="tool-header"
        onClick={() => setExpanded((e) => !e)}
        aria-expanded={isOpen}
        aria-controls={`tool-${item.id}`}
      >
        <div className="tool-header-left">
          <span className={`tool-icon kind-${item.tool_kind}`}>{toolKindIcon(item.tool_kind)}</span>
          <div className="tool-title-wrap">
            <div className="tool-title">{item.title}</div>
            <div className="tool-subtitle">
              {subtitleItems.map((node, idx) => (
                <Fragment key={idx}>
                  {idx > 0 && (
                    <span className="tool-status-dot" aria-hidden="true">
                      ·
                    </span>
                  )}
                  {node}
                </Fragment>
              ))}
            </div>
          </div>
        </div>
        <div className="tool-header-right">
          <span className="muted">{new Date(item.updated_at).toLocaleTimeString()}</span>
          <span className="thinking-chev">{isOpen ? "▴" : "▾"}</span>
        </div>
      </button>

      {isOpen && (
        <div id={`tool-${item.id}`} className="tool-body">
          {item.input && (
            <div className="tool-section">
              <div className="tool-section-title">Input</div>
              <pre className="tool-pre">{formatToolInput(item.tool_kind, item.input)}</pre>
            </div>
          )}
          {hasOutput && (
            <div className="tool-section">
              <div className="tool-section-title">Output</div>
              {looksLikeMarkdown(item.output_text) ? (
                <div className="tool-markdown">
                  <MemoMarkdown content={item.output_text} />
                </div>
              ) : (
                <pre className="tool-pre tool-output">{item.output_text}</pre>
              )}
            </div>
          )}
          <details className="tool-raw">
            <summary className="link">
              Raw event {item.updates_seen > 1 ? `(updated ${item.updates_seen}×)` : ""}
            </summary>
            <pre className="json">{JSON.stringify(item.raw, null, 2)}</pre>
          </details>
        </div>
      )}
    </div>
  );
}
