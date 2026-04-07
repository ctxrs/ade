import {
  createPretextVirtualizerCore,
  type PretextVirtualizerDiagnosticEvent,
  type PretextVirtualizerSnapshot,
} from "@pretext-virtualizer/core";
import type { WorkbenchListItem } from "../SessionPage.types";
import {
  getWorkbenchListItemHeightRevision,
  type WorkbenchMessageListUiState,
} from "../sessionMessageListItemIdentity";
import { getPretextVirtualizerRowLayout } from "./pretextVirtualizerRowLayout";

export const SESSION_PRETEXT_OVERSCAN_PX = 480;
export const SESSION_PRETEXT_BOTTOM_THRESHOLD_PX = 16;

type PlannedLayoutGetter = (
  item: WorkbenchListItem,
  viewport: {
    width: number;
    widthBucket: `w${number}`;
  },
) => {
  height: number;
};

type SessionPretextRuntimeRecord = {
  sessionId: string;
  core: ReturnType<typeof createPretextVirtualizerCore<WorkbenchListItem>>;
  callbacks: {
    getLayoutRevision: (item: WorkbenchListItem) => string | number;
    getPlannedLayout: PlannedLayoutGetter;
    onDiagnosticEvent?: ((event: PretextVirtualizerDiagnosticEvent<WorkbenchListItem>) => void) | null;
  };
  uiState: WorkbenchMessageListUiState;
  lastSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>;
  lastItems: readonly WorkbenchListItem[];
  hasVisibleMount: boolean;
};

type RuntimeBindings = {
  uiState: WorkbenchMessageListUiState;
  onDiagnosticEvent?: ((event: PretextVirtualizerDiagnosticEvent<WorkbenchListItem>) => void) | null;
};

type PrimeSessionPretextRuntimeParams = {
  sessionId: string;
  listItems: readonly WorkbenchListItem[];
  uiState: WorkbenchMessageListUiState;
  viewportWidth: number;
  viewportHeight?: number;
};

const runtimeCache = new Map<string, SessionPretextRuntimeRecord>();

export function createDefaultSessionTranscriptUiState(
  verbosity?: string,
  turnToolsLoading: readonly string[] = [],
): WorkbenchMessageListUiState {
  return {
    expandedTurnHeaders: {},
    expandedTurnDetailsById: {},
    expandedToolById: {},
    expandedMessageById: {},
    turnToolsLoading,
    verbosity,
  };
}

function bindRuntime(record: SessionPretextRuntimeRecord, bindings: RuntimeBindings): void {
  record.uiState = bindings.uiState;
  record.callbacks.getLayoutRevision = (item) =>
    getWorkbenchListItemHeightRevision(item, bindings.uiState, {
      verbosity: bindings.uiState.verbosity,
    });
  record.callbacks.getPlannedLayout = (item, viewport) =>
    getPretextVirtualizerRowLayout(item, viewport.width, {
      expandedTurnHeaders: bindings.uiState.expandedTurnHeaders,
      expandedTurnDetailsById: bindings.uiState.expandedTurnDetailsById,
      expandedMessageById: bindings.uiState.expandedMessageById,
      turnToolsLoading: bindings.uiState.turnToolsLoading,
    });
  record.callbacks.onDiagnosticEvent = bindings.onDiagnosticEvent ?? null;
}

function createSessionPretextRuntime(sessionId: string): SessionPretextRuntimeRecord {
  const callbacks: SessionPretextRuntimeRecord["callbacks"] = {
    getLayoutRevision: () => 0,
    getPlannedLayout: () => ({ height: 1 }),
    onDiagnosticEvent: null,
  };
  const core = createPretextVirtualizerCore<WorkbenchListItem>({
    initialItems: [],
    getId: (item) => item.id,
    getLayoutRevision: (item) => callbacks.getLayoutRevision(item),
    getPlannedLayout: (item, viewport) => callbacks.getPlannedLayout(item, viewport),
    overscanPx: SESSION_PRETEXT_OVERSCAN_PX,
    bottomThresholdPx: SESSION_PRETEXT_BOTTOM_THRESHOLD_PX,
    onDiagnosticEvent: (event) => {
      callbacks.onDiagnosticEvent?.(event);
    },
  });

  const record: SessionPretextRuntimeRecord = {
    sessionId,
    core,
    callbacks,
    uiState: createDefaultSessionTranscriptUiState(),
    lastSnapshot: core.getSnapshot(),
    lastItems: [],
    hasVisibleMount: false,
  };
  bindRuntime(record, { uiState: record.uiState });
  return record;
}

export function getOrCreateSessionPretextRuntime(
  sessionId: string,
  bindings?: RuntimeBindings,
): SessionPretextRuntimeRecord {
  let record = runtimeCache.get(sessionId);
  if (!record) {
    record = createSessionPretextRuntime(sessionId);
    runtimeCache.set(sessionId, record);
  }
  if (bindings) {
    bindRuntime(record, bindings);
  }
  return record;
}

export function noteSessionPretextRuntimeSnapshot(
  record: SessionPretextRuntimeRecord,
  snapshot: PretextVirtualizerSnapshot<WorkbenchListItem>,
  listItems: readonly WorkbenchListItem[],
): void {
  record.lastSnapshot = snapshot;
  record.lastItems = listItems;
}

export function primeSessionPretextRuntime(
  params: PrimeSessionPretextRuntimeParams,
): SessionPretextRuntimeRecord {
  const record = getOrCreateSessionPretextRuntime(params.sessionId, {
    uiState: params.uiState,
  });
  const nextWidth = Number.isFinite(params.viewportWidth) ? params.viewportWidth : 0;
  const nextHeight = Number.isFinite(params.viewportHeight) ? params.viewportHeight ?? 0 : 0;
  if (nextWidth > 0 || nextHeight > 0) {
    record.lastSnapshot = record.core.syncViewport({
      width: nextWidth,
      height: nextHeight,
      scrollTop: record.lastSnapshot.scrollTop,
    });
  }
  if (record.lastItems !== params.listItems) {
    const anchor = record.hasVisibleMount ? record.lastSnapshot.anchor : { kind: "bottom" as const };
    record.lastSnapshot = record.core.replaceItems(params.listItems, anchor);
    record.lastItems = params.listItems;
  }
  return record;
}

export function markSessionPretextRuntimeVisible(
  record: SessionPretextRuntimeRecord,
  visible: boolean,
): void {
  record.hasVisibleMount = visible;
}

export function getSessionPretextRuntimeCacheSize(): number {
  return runtimeCache.size;
}

export function resetSessionPretextRuntimeCache(): void {
  runtimeCache.clear();
}
