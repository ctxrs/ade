export const WORKBENCH_TASK_IDLE_EVENT = "ctx:workbench-task-idle" as const;

export type WorkbenchTaskIdleDetail = {
  allTasksIdle: boolean;
};
