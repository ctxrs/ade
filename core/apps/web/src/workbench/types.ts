import type { WorkbenchModeId } from "../components/WorkbenchComposer";

export type SplitDirection = "horizontal" | "vertical";

export type LayoutNode =
  | {
      kind: "split";
      id: string;
      direction: SplitDirection;
      ratio: number;
      first: LayoutNode;
      second: LayoutNode;
    }
  | {
      kind: "leaf";
      id: string;
      tabs: WorkbenchTab[];
      activeTabId: string;
    };

export type WorkbenchTab =
  | {
      id: string;
      kind: "new_task";
      titleOverride?: string;
      viewMode?: "compact" | "normal" | "verbose";
    }
  | {
      id: string;
      kind: "track";
      ref: {
        taskId: string;
        trackId: string | null;
        sessionId?: string | null;
      };
      titleOverride?: string;
      viewMode?: "compact" | "normal" | "verbose";
    };

export type WorkbenchScrollState = {
  stickToBottom: boolean;
  anchorItemId: string | null;
  updatedAtMs: number;
};

export type WorkbenchDraft = {
  text: string;
  modeId: WorkbenchModeId;
  updatedAtMs: number;
};

export type PersistedWorkbenchWindowV1 = {
  v: 1;
  layout: LayoutNode;
  focusedLeafId: string;
  scrollByKey: Record<string, WorkbenchScrollState | undefined>;
};

export type PersistedWorkbenchDraftV1 = {
  v: 1;
  key: string;
  draft: WorkbenchDraft;
};
