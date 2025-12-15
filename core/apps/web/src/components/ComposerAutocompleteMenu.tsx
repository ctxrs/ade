import { createPortal } from "react-dom";

export type ComposerAutocompleteItem = {
  key: string;
  label: string;
  insertText: string;
  description?: string;
  kind: "slash" | "file";
};

export function ComposerAutocompleteMenu({
  open,
  loading,
  items,
  activeIndex,
  onPick,
  onHoverIndex,
  anchorRect,
  inlineFallback,
}: {
  open: boolean;
  loading: boolean;
  items: ComposerAutocompleteItem[];
  activeIndex: number;
  onPick: (index: number) => void;
  onHoverIndex: (index: number) => void;
  anchorRect: DOMRect | null;
  inlineFallback: boolean;
}) {
  if (!open) return null;

  const body = (
    <div className="composer-ac">
      {loading && <div className="composer-ac-loading">Searching…</div>}
      {!loading && items.length === 0 && (
        <div className="composer-ac-empty">No matches</div>
      )}
      {items.map((it, idx) => (
        <div
          key={it.key}
          className={`composer-ac-item ${idx === activeIndex ? "composer-ac-item-active" : ""}`}
          onMouseEnter={() => onHoverIndex(idx)}
          onMouseDown={(e) => {
            e.preventDefault();
            onPick(idx);
          }}
          role="option"
          aria-selected={idx === activeIndex}
        >
          <div className="composer-ac-item-main">
            <span className="composer-ac-item-label">{it.label}</span>
            {it.description && <span className="composer-ac-item-desc">{it.description}</span>}
          </div>
        </div>
      ))}
    </div>
  );

  if (!anchorRect || inlineFallback) {
    return <div className="composer-ac-inline">{body}</div>;
  }

  return createPortal(
    <div
      className="composer-ac-popover"
      style={{
        left: Math.max(8, Math.min(window.innerWidth - 320, anchorRect.left)),
        top: Math.min(window.innerHeight - 240, anchorRect.bottom + 6),
      }}
    >
      {body}
    </div>,
    document.body,
  );
}

