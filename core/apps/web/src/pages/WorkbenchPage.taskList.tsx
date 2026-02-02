import React, { useCallback } from "react";
import type { TaskListContext } from "./WorkbenchPage.types";

type TaskListScrollerProps = React.HTMLAttributes<HTMLDivElement> & {
  context?: TaskListContext;
};

const ARCHIVED_SCROLL_LOAD_THRESHOLD_PX = 120;

const TaskListScroller = React.forwardRef<HTMLDivElement, TaskListScrollerProps>((props, ref) => {
  const { context, onScroll, ...rest } = props;
  const handleRef = useCallback(
    (node: HTMLDivElement | null) => {
      if (typeof ref === "function") {
        ref(node);
      } else if (ref) {
        ref.current = node;
      }
      context?.onScrollerChange?.(node);
    },
    [context, ref],
  );
  const handleScroll = useCallback(
    (event: React.UIEvent<HTMLDivElement>) => {
      onScroll?.(event);
      context?.onScroll?.(event);
      if (!context) return;
      if (context.archivedCollapsed) return;
      if (!context.hasMoreArchived) return;
      if (context.archivedFetchState === "loading") return;
      const target = event.currentTarget;
      if (!target) return;
      if (target.scrollTop + target.clientHeight < target.scrollHeight - ARCHIVED_SCROLL_LOAD_THRESHOLD_PX) return;
      context.onLoadMoreArchived();
    },
    [context, onScroll],
  );

  return <div {...rest} ref={handleRef} className="wb-task-scroll" onScroll={handleScroll} />;
});

const TaskListContainer = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
  <div {...props} ref={ref} className="wb-task-list" role="list" aria-label="Tasks" />
));

const TaskListHeader = () => (
  <div className="wb-section-header">
    <div className="wb-section-title">Active Tasks</div>
  </div>
);

export const TASK_LIST_COMPONENTS = {
  Scroller: TaskListScroller,
  List: TaskListContainer,
  Header: TaskListHeader,
};
