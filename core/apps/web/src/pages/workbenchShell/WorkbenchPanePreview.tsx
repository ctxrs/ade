import type { WorkbenchPanePreviewProps } from "./WorkbenchTemplateTypes";

export function WorkbenchPanePreview({
  title,
  subtitle,
  badge,
  meta = [],
  statusLabel,
  active = false,
  muted = false,
  actions,
  children,
}: WorkbenchPanePreviewProps) {
  return (
    <section
      className={`wb-pane-preview ${active ? "wb-pane-preview-active" : ""} ${
        muted ? "wb-pane-preview-muted" : ""
      }`}
    >
      <header className="wb-pane-preview-header">
        <div className="wb-pane-preview-heading">
          <div className="wb-pane-preview-title-row">
            <div className="wb-pane-preview-title">{title}</div>
            {badge ? <span className="wb-pane-preview-badge">{badge}</span> : null}
          </div>
          {subtitle ? <div className="wb-pane-preview-subtitle">{subtitle}</div> : null}
        </div>
        {actions ? <div className="wb-pane-preview-actions">{actions}</div> : null}
      </header>
      {children ? <div className="wb-pane-preview-body">{children}</div> : null}
      {meta.length > 0 || statusLabel ? (
        <footer className="wb-pane-preview-footer">
          {meta.map((item) => (
            <span key={item} className="wb-pane-preview-meta">
              {item}
            </span>
          ))}
          {statusLabel ? <span className="wb-pane-preview-status">{statusLabel}</span> : null}
        </footer>
      ) : null}
    </section>
  );
}
