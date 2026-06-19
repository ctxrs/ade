import type { ReactNode } from "react";

import type { WorkbenchReviewDetail, WorkbenchReviewMetric } from "./WorkbenchTemplateTypes";

type WorkbenchReviewTemplateProps = {
  title: string;
  subtitle?: string | null;
  statusLabel?: string | null;
  metrics?: WorkbenchReviewMetric[];
  details?: WorkbenchReviewDetail[];
  actions?: ReactNode;
  activeTaskSlot?: ReactNode;
  diffSlot?: ReactNode;
  sidebarSlot?: ReactNode;
  emptyTaskLabel?: string;
  emptyDiffLabel?: string;
};

export function WorkbenchReviewTemplate({
  title,
  subtitle,
  statusLabel,
  metrics = [],
  details = [],
  actions,
  activeTaskSlot,
  diffSlot,
  sidebarSlot,
  emptyTaskLabel = "Select a task to review.",
  emptyDiffLabel = "No diff selected.",
}: WorkbenchReviewTemplateProps) {
  return (
    <section className="wb-review-template">
      <header className="wb-review-header">
        <div className="wb-review-heading">
          <div className="wb-review-title-row">
            <h1 className="wb-review-title">{title}</h1>
            {statusLabel ? <span className="wb-review-status">{statusLabel}</span> : null}
          </div>
          {subtitle ? <div className="wb-review-subtitle">{subtitle}</div> : null}
        </div>
        {actions ? <div className="wb-review-actions">{actions}</div> : null}
      </header>

      {metrics.length > 0 || details.length > 0 ? (
        <div className="wb-review-summary">
          {metrics.map((metric) => (
            <div key={metric.label} className="wb-review-metric">
              <div className="wb-review-metric-value">{metric.value}</div>
              <div className="wb-review-metric-label">{metric.label}</div>
            </div>
          ))}
          {details.map((detail, index) => (
            <div key={detail.id ?? `${detail.label}:${index}`} className="wb-review-detail">
              <span className="wb-review-detail-label">{detail.label}</span>
              <span className="wb-review-detail-value">{detail.value}</span>
            </div>
          ))}
        </div>
      ) : null}

      <div className={`wb-review-content ${sidebarSlot ? "wb-review-content-with-sidebar" : ""}`}>
        <main className="wb-review-main">
          <section className="wb-review-panel wb-review-task-panel" aria-label="Active task">
            {activeTaskSlot ?? <div className="wb-template-empty">{emptyTaskLabel}</div>}
          </section>
          <section className="wb-review-panel wb-review-diff-panel" aria-label="Diff review">
            {diffSlot ?? <div className="wb-template-empty">{emptyDiffLabel}</div>}
          </section>
        </main>
        {sidebarSlot ? <aside className="wb-review-sidebar">{sidebarSlot}</aside> : null}
      </div>
    </section>
  );
}
