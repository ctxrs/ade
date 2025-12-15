import { useLayoutEffect, useRef, useState } from "react";
import type React from "react";
import { createPortal } from "react-dom";

export type ComposerAutocompleteItem =
  | {
      key: string;
      kind: "slash";
      label: string;
      insertText: string;
      description?: string;
    }
  | {
      key: string;
      kind: "file";
      path: string;
      label: string;
      insertText: string;
      description?: string;
    };

function clamp(n: number, min: number, max: number) {
  return Math.max(min, Math.min(max, n));
}

function splitPath(path: string): { fileName: string; dirName: string } {
  const normalized = String(path).replace(/\\/g, "/");
  const parts = normalized.split("/").filter(Boolean);
  const fileName = parts.length > 0 ? parts[parts.length - 1] : normalized;
  const dirName = parts.length > 1 ? parts.slice(0, -1).join("/") : "";
  return { fileName, dirName };
}

function IconInsert({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M12 4v10"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <path
        d="M7 11l5 5 5-5"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function IconFolder({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M3 7a2 2 0 012-2h5l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2V7z"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function IconFile({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M6 3h7l5 5v13a1 1 0 01-1 1H6a1 1 0 01-1-1V4a1 1 0 011-1z"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
      <path
        d="M13 3v6h6"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function ComposerAutocompleteMenu({
  open,
  loading,
  items,
  activeIndex,
  onPick,
  onHoverIndex,
  anchorRect,
  anchorInputRect,
  inlineFallback,
}: {
  open: boolean;
  loading: boolean;
  items: ComposerAutocompleteItem[];
  activeIndex: number;
  onPick: (index: number) => void;
  onHoverIndex: (index: number) => void;
  anchorRect: DOMRect | null;
  anchorInputRect?: DOMRect | null;
  inlineFallback: boolean;
}) {
  const itemRefs = useRef<Array<HTMLDivElement | null>>([]);
  const [previewStyle, setPreviewStyle] = useState<React.CSSProperties | null>(null);

  const anchorForWidth = anchorInputRect ?? anchorRect;
  const anchorForPosition = anchorRect ?? anchorInputRect;
  const canPortal = Boolean(anchorForPosition) && Boolean(anchorForWidth) && !inlineFallback;

  const offset = 6;
  const margin = 10;
  const viewportH = window.innerHeight;
  const viewportW = window.innerWidth;

  const popoverWidth = canPortal
    ? clamp(anchorForWidth!.width || 0, 340, 520)
    : 320;

  const spaceBelow = canPortal ? viewportH - (anchorForPosition!.bottom + offset) - margin : 240;
  const spaceAbove = canPortal ? anchorForPosition!.top - offset - margin : 240;
  const rowH = 30;
  const estimatedRows = items.length > 0 ? items.length : loading ? 0 : 1;
  const estimatedHeight = estimatedRows * rowH + 2;
  const openAbove = canPortal ? spaceBelow < estimatedHeight && spaceAbove > spaceBelow : false;
  const maxHeight = clamp(Math.min(openAbove ? spaceAbove : spaceBelow, estimatedHeight || 360), 120, 360);

  const active = items[activeIndex] ?? null;
  const activeFilePath = active?.kind === "file" ? active.path : null;

  const popoverLeft = clamp(anchorForWidth?.left ?? 0, margin, viewportW - popoverWidth - margin);
  const popoverTop = openAbove ? null : (anchorForPosition?.bottom ?? 0) + offset;
  const popoverBottom = openAbove ? viewportH - (anchorForPosition?.top ?? 0) + offset : null;

  useLayoutEffect(() => {
    if (!open || !canPortal || !activeFilePath) {
      setPreviewStyle(null);
      return;
    }
    const el = itemRefs.current[activeIndex] ?? null;
    if (!el) {
      setPreviewStyle(null);
      return;
    }
    const r = el.getBoundingClientRect();
    const previewWidth = 280;
    const gap = 10;
    const maxPreviewHeight = 220;

    let left = popoverLeft + popoverWidth + gap;
    if (left + previewWidth + margin > viewportW) {
      left = popoverLeft - previewWidth - gap;
    }
    left = clamp(left, margin, viewportW - previewWidth - margin);
    const top = clamp(r.top - 6, margin, viewportH - maxPreviewHeight - margin);
    setPreviewStyle({
      position: "fixed",
      left,
      top,
      width: `${previewWidth}px`,
      maxHeight: `${maxPreviewHeight}px`,
    });
  }, [activeFilePath, activeIndex, canPortal, open, popoverLeft, popoverWidth, viewportH, viewportW]);

  itemRefs.current.length = items.length;

  if (!open) return null;

  const body = (
    <div className="composer-ac" style={{ maxHeight }} role="listbox" aria-label="Completions">
      {!loading && items.length === 0 && <div className="composer-ac-empty">No matches</div>}
      {items.map((it, idx) => {
        const active = idx === activeIndex;
        const file = it.kind === "file" ? splitPath(it.path) : null;
        const left = it.kind === "file" ? file?.fileName ?? it.label : it.label;
        const right =
          it.kind === "file"
            ? file?.dirName
              ? `…/${file.dirName}`
              : ""
            : it.description
              ? it.description
              : "";

        return (
          <div
            key={it.key}
            ref={(el) => {
              itemRefs.current[idx] = el;
            }}
            className={`composer-ac-item ${active ? "composer-ac-item-active" : ""}`}
            onMouseEnter={() => onHoverIndex(idx)}
            onMouseDown={(e) => {
              e.preventDefault();
              onPick(idx);
            }}
            role="option"
            aria-selected={active}
          >
            <span className={`composer-ac-icon ${it.kind === "file" ? "composer-ac-icon-file" : "composer-ac-icon-slash"}`} aria-hidden="true">
              {it.kind === "file" ? <IconInsert size={16} /> : "/"}
            </span>
            <span className="composer-ac-item-left" title={left}>
              {left}
            </span>
            {right && (
              <span className="composer-ac-item-right" title={right}>
                {right}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );

  if (!anchorForPosition || !anchorForWidth || inlineFallback) {
    return <div className="composer-ac-inline">{body}</div>;
  }

  const preview = activeFilePath ? (
    <div className="composer-ac-preview-popover" style={previewStyle ?? undefined} aria-hidden="true">
      {(() => {
        const normalized = activeFilePath.replace(/\\/g, "/");
        const parts = normalized.split("/").filter(Boolean);
        const dirs = parts.slice(0, -1);
        const file = parts[parts.length - 1] ?? normalized;
        const maxDirs = 6;
        const trimmed = dirs.length > maxDirs ? dirs.slice(dirs.length - maxDirs) : dirs;
        const hasMore = dirs.length > trimmed.length;
        return (
          <div className="composer-ac-preview-tree">
            {hasMore && <div className="composer-ac-preview-more">…</div>}
            {trimmed.map((seg, i) => (
              <div key={`${seg}:${i}`} className="composer-ac-preview-row">
                <span className="composer-ac-preview-icon" aria-hidden="true">
                  <IconFolder size={14} />
                </span>
                <span className="composer-ac-preview-name">{seg}</span>
              </div>
            ))}
            <div className="composer-ac-preview-row composer-ac-preview-row-file">
              <span className="composer-ac-preview-icon" aria-hidden="true">
                <IconFile size={14} />
              </span>
              <span className="composer-ac-preview-name">{file}</span>
            </div>
          </div>
        );
      })()}
    </div>
  ) : null;

  return (
    <>
      {createPortal(
        <div
          className="composer-ac-popover"
          style={{
            left: popoverLeft,
            width: popoverWidth,
            ...(popoverTop !== null ? { top: popoverTop } : {}),
            ...(popoverBottom !== null ? { bottom: popoverBottom } : {}),
          }}
        >
          {body}
        </div>,
        document.body,
      )}
      {previewStyle && preview ? createPortal(preview, document.body) : null}
    </>
  );
}
