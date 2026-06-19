import type { ReactNode } from "react";

import { WorkbenchSplitView } from "./WorkbenchSplitView";
import type { WorkbenchSplitNode, WorkbenchSplitPaneNode, WorkbenchSplitResize } from "./WorkbenchTemplateTypes";

type WorkbenchMultipaneTemplateProps = {
  splitTree: WorkbenchSplitNode | null;
  activePaneId?: string | null;
  emptyLabel?: string;
  header?: ReactNode;
  onFocusPane?: (paneId: string, pane: WorkbenchSplitPaneNode) => void;
  onResizeSplit?: (resize: WorkbenchSplitResize) => void;
  renderPane?: (pane: WorkbenchSplitPaneNode, state: { active: boolean }) => ReactNode;
};

export function WorkbenchMultipaneTemplate({
  splitTree,
  activePaneId,
  emptyLabel = "No panes configured.",
  header,
  onFocusPane,
  onResizeSplit,
  renderPane,
}: WorkbenchMultipaneTemplateProps) {
  return (
    <section className="wb-multipane-template">
      {header ? <div className="wb-multipane-header">{header}</div> : null}
      <div className="wb-multipane-body">
        {splitTree ? (
          <WorkbenchSplitView
            node={splitTree}
            activePaneId={activePaneId}
            onFocusPane={onFocusPane}
            onResizeSplit={onResizeSplit}
            renderPane={renderPane}
          />
        ) : (
          <div className="wb-template-empty">{emptyLabel}</div>
        )}
      </div>
    </section>
  );
}
