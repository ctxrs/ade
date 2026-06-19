import type { CSSProperties, ReactNode } from "react";
import type { WorkbenchBuiltinTemplateId, WorkbenchTemplateId } from "../../workbench/types";

export type { WorkbenchBuiltinTemplateId, WorkbenchTemplateId };

export type WorkbenchTemplateDefinition = {
  id: WorkbenchBuiltinTemplateId;
  label: string;
  description: string;
};

export const WORKBENCH_TEMPLATE_REGISTRY: Record<WorkbenchBuiltinTemplateId, WorkbenchTemplateDefinition> = {
  classic: {
    id: "classic",
    label: "Classic",
    description: "Current conversation-first workbench layout.",
  },
  kanban: {
    id: "kanban",
    label: "Kanban",
    description: "Task lanes for status-driven work.",
  },
  multipane: {
    id: "multipane",
    label: "Multipane",
    description: "Resizable pane tree for parallel context.",
  },
  review: {
    id: "review",
    label: "Review",
    description: "High-level task detail with review surfaces.",
  },
};

export const WORKBENCH_TEMPLATE_LIST: WorkbenchTemplateDefinition[] = [
  WORKBENCH_TEMPLATE_REGISTRY.classic,
  WORKBENCH_TEMPLATE_REGISTRY.kanban,
  WORKBENCH_TEMPLATE_REGISTRY.multipane,
  WORKBENCH_TEMPLATE_REGISTRY.review,
];

export const isWorkbenchBuiltinTemplateId = (value: string): value is WorkbenchBuiltinTemplateId =>
  Object.prototype.hasOwnProperty.call(WORKBENCH_TEMPLATE_REGISTRY, value);

export const isWorkbenchTemplateId = (value: string): value is WorkbenchTemplateId =>
  isWorkbenchBuiltinTemplateId(value) || /^plugin:[^/]+\/[^/]+$/.test(value);

export type WorkbenchKanbanTaskTone = "default" | "active" | "success" | "warning" | "danger";

export type WorkbenchKanbanCard = {
  id: string;
  title: string;
  subtitle?: string | null;
  description?: string | null;
  meta?: string[];
  tone?: WorkbenchKanbanTaskTone;
  disabled?: boolean;
};

export type WorkbenchKanbanLane = {
  id: string;
  title: string;
  countLabel?: string;
  emptyLabel?: string;
  cards: WorkbenchKanbanCard[];
};

export type WorkbenchPanePreviewProps = {
  title: string;
  subtitle?: string | null;
  badge?: string | null;
  meta?: string[];
  statusLabel?: string | null;
  active?: boolean;
  muted?: boolean;
  actions?: ReactNode;
  children?: ReactNode;
};

export type WorkbenchSplitPaneNode = {
  id: string;
  kind: "pane";
  title?: string;
  preview?: WorkbenchPanePreviewProps;
  content?: ReactNode;
  className?: string;
};

export type WorkbenchSplitBranchNode = {
  id: string;
  kind: "split";
  direction: "horizontal" | "vertical";
  first: WorkbenchSplitNode;
  second: WorkbenchSplitNode;
  splitPercent?: number;
  minPercent?: number;
  maxPercent?: number;
  handleLabel?: string;
  className?: string;
};

export type WorkbenchSplitNode = WorkbenchSplitPaneNode | WorkbenchSplitBranchNode;

export type WorkbenchSplitResize = {
  splitId: string;
  direction: WorkbenchSplitBranchNode["direction"];
  percent: number;
  source: "pointer" | "keyboard";
};

export type WorkbenchReviewMetric = {
  label: string;
  value: ReactNode;
};

export type WorkbenchReviewDetail = {
  id?: string;
  label: string;
  value: ReactNode;
};

export type WorkbenchTemplateSlotProps = {
  className?: string;
  style?: CSSProperties;
};
