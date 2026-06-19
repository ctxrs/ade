import type { ReactNode } from "react";

import type { WorkbenchKanbanCard, WorkbenchKanbanLane } from "./WorkbenchTemplateTypes";

type WorkbenchKanbanTemplateProps = {
  lanes: WorkbenchKanbanLane[];
  selectedTaskId?: string | null;
  emptyLabel?: string;
  renderCardAccessory?: (card: WorkbenchKanbanCard, lane: WorkbenchKanbanLane) => ReactNode;
  onSelectTask: (taskId: string, card: WorkbenchKanbanCard, lane: WorkbenchKanbanLane) => void;
};

const cardToneClass = (tone: WorkbenchKanbanCard["tone"]) =>
  tone && tone !== "default" ? `wb-kanban-card-${tone}` : "";

export function WorkbenchKanbanTemplate({
  lanes,
  selectedTaskId,
  emptyLabel = "No tasks to show.",
  renderCardAccessory,
  onSelectTask,
}: WorkbenchKanbanTemplateProps) {
  if (lanes.length === 0) {
    return <div className="wb-template-empty">{emptyLabel}</div>;
  }

  return (
    <div className="wb-kanban-template" aria-label="Workbench task lanes">
      {lanes.map((lane) => (
        <section key={lane.id} className="wb-kanban-lane" aria-labelledby={`wb-kanban-lane-${lane.id}`}>
          <header className="wb-kanban-lane-header">
            <h2 id={`wb-kanban-lane-${lane.id}`} className="wb-kanban-lane-title">
              {lane.title}
            </h2>
            <span className="wb-kanban-lane-count">{lane.countLabel ?? String(lane.cards.length)}</span>
          </header>
          <div className="wb-kanban-cards">
            {lane.cards.length > 0 ? (
              lane.cards.map((card) => {
                const selected = card.id === selectedTaskId;
                return (
                  <button
                    key={card.id}
                    type="button"
                    className={`wb-kanban-card ${cardToneClass(card.tone)} ${
                      selected ? "wb-kanban-card-selected" : ""
                    }`}
                    disabled={card.disabled}
                    aria-current={selected ? "true" : undefined}
                    onClick={() => onSelectTask(card.id, card, lane)}
                  >
                    <span className="wb-kanban-card-main">
                      <span className="wb-kanban-card-title">{card.title}</span>
                      {card.subtitle ? <span className="wb-kanban-card-subtitle">{card.subtitle}</span> : null}
                      {card.description ? (
                        <span className="wb-kanban-card-description">{card.description}</span>
                      ) : null}
                    </span>
                    {card.meta?.length ? (
                      <span className="wb-kanban-card-meta">
                        {card.meta.map((item) => (
                          <span key={item} className="wb-kanban-card-meta-item">
                            {item}
                          </span>
                        ))}
                      </span>
                    ) : null}
                    {renderCardAccessory ? (
                      <span className="wb-kanban-card-accessory">{renderCardAccessory(card, lane)}</span>
                    ) : null}
                  </button>
                );
              })
            ) : (
              <div className="wb-kanban-empty">{lane.emptyLabel ?? "No tasks"}</div>
            )}
          </div>
        </section>
      ))}
    </div>
  );
}
