import { useCallback, useRef, type CSSProperties, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";

import { WorkbenchPanePreview } from "./WorkbenchPanePreview";
import type {
  WorkbenchSplitBranchNode,
  WorkbenchSplitNode,
  WorkbenchSplitPaneNode,
  WorkbenchSplitResize,
} from "./WorkbenchTemplateTypes";

type WorkbenchSplitViewProps = {
  node: WorkbenchSplitNode;
  activePaneId?: string | null;
  onFocusPane?: (paneId: string, pane: WorkbenchSplitPaneNode) => void;
  onResizeSplit?: (resize: WorkbenchSplitResize) => void;
  renderPane?: (pane: WorkbenchSplitPaneNode, state: { active: boolean }) => ReactNode;
};

const clampPercent = (value: number, minPercent = 15, maxPercent = 85) =>
  Math.max(minPercent, Math.min(maxPercent, value));

const getSplitPercent = (node: WorkbenchSplitBranchNode) =>
  clampPercent(node.splitPercent ?? 50, node.minPercent, node.maxPercent);

type SplitStyle = CSSProperties & {
  "--wb-split-percent": string;
};

export function WorkbenchSplitView({
  node,
  activePaneId,
  onFocusPane,
  onResizeSplit,
  renderPane,
}: WorkbenchSplitViewProps) {
  if (node.kind === "pane") {
    const active = node.id === activePaneId || node.preview?.active === true;
    const content = renderPane ? (
      renderPane(node, { active })
    ) : node.content ? (
      node.content
    ) : node.preview ? (
      <WorkbenchPanePreview {...node.preview} active={active} />
    ) : (
      <WorkbenchPanePreview title={node.title ?? "Pane"} active={active} muted />
    );

    return (
      <div
        className={`wb-split-pane ${active ? "wb-split-pane-active" : ""} ${node.className ?? ""}`}
        tabIndex={0}
        onFocus={() => onFocusPane?.(node.id, node)}
        onPointerDown={() => onFocusPane?.(node.id, node)}
      >
        {content}
      </div>
    );
  }

  return (
    <WorkbenchSplitBranch
      node={node}
      activePaneId={activePaneId}
      onFocusPane={onFocusPane}
      onResizeSplit={onResizeSplit}
      renderPane={renderPane}
    />
  );
}

function WorkbenchSplitBranch({
  node,
  activePaneId,
  onFocusPane,
  onResizeSplit,
  renderPane,
}: WorkbenchSplitViewProps & { node: WorkbenchSplitBranchNode }) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const percent = getSplitPercent(node);
  const orientation = node.direction === "horizontal" ? "vertical" : "horizontal";
  const splitStyle: SplitStyle = { "--wb-split-percent": `${percent}%` };

  const emitResize = useCallback(
    (percentValue: number, source: WorkbenchSplitResize["source"]) => {
      onResizeSplit?.({
        splitId: node.id,
        direction: node.direction,
        percent: clampPercent(percentValue, node.minPercent, node.maxPercent),
        source,
      });
    },
    [node.direction, node.id, node.maxPercent, node.minPercent, onResizeSplit],
  );

  const percentFromPointer = useCallback(
    (event: globalThis.PointerEvent | PointerEvent<HTMLDivElement>) => {
      const container = containerRef.current;
      if (!container) return null;
      const rect = container.getBoundingClientRect();
      if (node.direction === "horizontal") {
        if (rect.width <= 0) return null;
        return ((event.clientX - rect.left) / rect.width) * 100;
      }
      if (rect.height <= 0) return null;
      return ((event.clientY - rect.top) / rect.height) * 100;
    },
    [node.direction],
  );

  const onHandlePointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (!onResizeSplit) return;
      event.preventDefault();
      const nextPercent = percentFromPointer(event);
      if (nextPercent != null) emitResize(nextPercent, "pointer");

      const onPointerMove = (moveEvent: globalThis.PointerEvent) => {
        const movePercent = percentFromPointer(moveEvent);
        if (movePercent != null) emitResize(movePercent, "pointer");
      };
      const onPointerUp = () => {
        window.removeEventListener("pointermove", onPointerMove);
        window.removeEventListener("pointerup", onPointerUp);
        window.removeEventListener("pointercancel", onPointerUp);
      };

      window.addEventListener("pointermove", onPointerMove);
      window.addEventListener("pointerup", onPointerUp);
      window.addEventListener("pointercancel", onPointerUp);
    },
    [emitResize, onResizeSplit, percentFromPointer],
  );

  const onHandleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!onResizeSplit) return;
    const step = event.shiftKey ? 10 : 5;
    const forwardKey = node.direction === "horizontal" ? "ArrowRight" : "ArrowDown";
    const backwardKey = node.direction === "horizontal" ? "ArrowLeft" : "ArrowUp";
    if (event.key === forwardKey) {
      event.preventDefault();
      emitResize(percent + step, "keyboard");
    } else if (event.key === backwardKey) {
      event.preventDefault();
      emitResize(percent - step, "keyboard");
    } else if (event.key === "Home") {
      event.preventDefault();
      emitResize(node.minPercent ?? 15, "keyboard");
    } else if (event.key === "End") {
      event.preventDefault();
      emitResize(node.maxPercent ?? 85, "keyboard");
    }
  };

  return (
    <div
      ref={containerRef}
      className={`wb-split wb-split-${node.direction} ${node.className ?? ""}`}
      style={splitStyle}
    >
      <div className="wb-split-section wb-split-section-first">
        <WorkbenchSplitView
          node={node.first}
          activePaneId={activePaneId}
          onFocusPane={onFocusPane}
          onResizeSplit={onResizeSplit}
          renderPane={renderPane}
        />
      </div>
      <div
        className="wb-split-handle"
        role="separator"
        tabIndex={0}
        aria-label={node.handleLabel ?? "Resize panes"}
        aria-orientation={orientation}
        aria-valuemin={node.minPercent ?? 15}
        aria-valuemax={node.maxPercent ?? 85}
        aria-valuenow={Math.round(percent)}
        onPointerDown={onHandlePointerDown}
        onKeyDown={onHandleKeyDown}
      />
      <div className="wb-split-section wb-split-section-second">
        <WorkbenchSplitView
          node={node.second}
          activePaneId={activePaneId}
          onFocusPane={onFocusPane}
          onResizeSplit={onResizeSplit}
          renderPane={renderPane}
        />
      </div>
    </div>
  );
}
