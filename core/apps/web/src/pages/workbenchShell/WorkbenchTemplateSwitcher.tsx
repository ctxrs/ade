import { GitBranch, LayersPlus, SplitSquareVertical, Square } from "lucide-react";

import {
  WORKBENCH_TEMPLATE_LIST,
  type WorkbenchTemplateDefinition,
  type WorkbenchBuiltinTemplateId,
  type WorkbenchTemplateId,
} from "./WorkbenchTemplateTypes";

type WorkbenchTemplateSwitcherProps = {
  activeTemplateId: WorkbenchTemplateId;
  templates?: WorkbenchTemplateDefinition[];
  disabledTemplateIds?: WorkbenchBuiltinTemplateId[];
  onSelectTemplate: (templateId: WorkbenchBuiltinTemplateId) => void;
  ariaLabel?: string;
};

const TEMPLATE_ICONS = {
  classic: Square,
  kanban: LayersPlus,
  multipane: SplitSquareVertical,
  review: GitBranch,
} satisfies Record<WorkbenchBuiltinTemplateId, typeof Square>;

export function WorkbenchTemplateSwitcher({
  activeTemplateId,
  templates = WORKBENCH_TEMPLATE_LIST,
  disabledTemplateIds = [],
  onSelectTemplate,
  ariaLabel = "Workbench template",
}: WorkbenchTemplateSwitcherProps) {
  const disabledIds = new Set(disabledTemplateIds);

  return (
    <div className="wb-template-switcher" role="radiogroup" aria-label={ariaLabel}>
      {templates.map((template) => {
        const Icon = TEMPLATE_ICONS[template.id];
        const active = template.id === activeTemplateId;
        const disabled = disabledIds.has(template.id);
        return (
          <button
            key={template.id}
            type="button"
            className={`wb-template-switcher-item ${active ? "wb-template-switcher-item-active" : ""}`}
            role="radio"
            aria-checked={active}
            aria-label={template.label}
            title={`${template.label}: ${template.description}`}
            disabled={disabled}
            onClick={() => onSelectTemplate(template.id)}
          >
            <Icon size={14} strokeWidth={2.2} aria-hidden="true" />
            <span className="wb-template-switcher-label">{template.label}</span>
          </button>
        );
      })}
    </div>
  );
}
