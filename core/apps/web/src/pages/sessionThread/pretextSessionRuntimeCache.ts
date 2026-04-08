import {
  createPretextVirtualizerCore,
  type PretextVirtualizerDiagnosticEvent,
  type PretextVirtualizerSnapshot,
} from "@pretext-virtualizer/core";
import type { WorkbenchListItem } from "../SessionPage.types";
import {
  getWorkbenchMessageListLayoutRevision,
  getWorkbenchListItemHeightRevision,
  type WorkbenchMessageListUiState,
} from "../sessionMessageListItemIdentity";
import { getPretextVirtualizerRowLayout } from "./pretextVirtualizerRowLayout";

export const SESSION_PRETEXT_OVERSCAN_PX = 480;
export const SESSION_PRETEXT_BOTTOM_THRESHOLD_PX = 16;
export const SESSION_PRETEXT_MAX_RESTORABLE_RUNTIMES = 12;

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
  uiStateRevision: string;
  preparedSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem>;
  preparedItems: readonly WorkbenchListItem[];
  restoreSnapshot: PretextVirtualizerSnapshot<WorkbenchListItem> | null;
  hasVisibleMount: boolean;
  isVisible: boolean;
  lastTouchedAtMs: number;
};

type RuntimeBindings = {
  uiState: WorkbenchMessageListUiState;
  uiStateRevision?: string;
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

function getSessionTranscriptUiStateRevision(uiState: WorkbenchMessageListUiState): string {
  return getWorkbenchMessageListLayoutRevision(uiState, {
    verbosity: uiState.verbosity,
  });
}

function touchRuntime(record: SessionPretextRuntimeRecord): void {
  record.lastTouchedAtMs = Date.now();
}

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
  record.uiStateRevision = bindings.uiStateRevision ?? getSessionTranscriptUiStateRevision(bindings.uiState);
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
  const uiState = createDefaultSessionTranscriptUiState();
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
    uiState,
    uiStateRevision: getSessionTranscriptUiStateRevision(uiState),
    preparedSnapshot: core.getSnapshot(),
    preparedItems: [],
    restoreSnapshot: null,
    hasVisibleMount: false,
    isVisible: false,
    lastTouchedAtMs: Date.now(),
  };
  bindRuntime(record, {
    uiState: record.uiState,
    uiStateRevision: record.uiStateRevision,
  });
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
  touchRuntime(record);
  return record;
}

export function noteSessionPretextRuntimeSnapshot(
  record: SessionPretextRuntimeRecord,
  snapshot: PretextVirtualizerSnapshot<WorkbenchListItem>,
  listItems: readonly WorkbenchListItem[],
): void {
  record.preparedSnapshot = snapshot;
  record.preparedItems = listItems;
  if (record.isVisible) {
    record.restoreSnapshot = snapshot;
    record.hasVisibleMount = true;
  }
  touchRuntime(record);
}

export function primeSessionPretextRuntime(
  params: PrimeSessionPretextRuntimeParams,
): SessionPretextRuntimeRecord {
  const record = getOrCreateSessionPretextRuntime(params.sessionId);
  const nextUiStateRevision = getSessionTranscriptUiStateRevision(params.uiState);
  const uiStateChanged = record.uiStateRevision !== nextUiStateRevision;
  const itemsChanged = record.preparedItems !== params.listItems;
  if (uiStateChanged) {
    bindRuntime(record, {
      uiState: params.uiState,
      uiStateRevision: nextUiStateRevision,
    });
  }
  const nextWidth = Number.isFinite(params.viewportWidth) ? params.viewportWidth : 0;
  const nextHeight = Number.isFinite(params.viewportHeight) ? params.viewportHeight ?? 0 : 0;
  const viewportChanged =
    (nextWidth > 0 && record.preparedSnapshot.viewportWidth !== nextWidth) ||
    (nextHeight > 0 && record.preparedSnapshot.viewportHeight !== nextHeight);
  if (!uiStateChanged && !itemsChanged && !viewportChanged) {
    touchRuntime(record);
    return record;
  }
  if (viewportChanged) {
    record.preparedSnapshot = record.core.syncViewport({
      width: nextWidth,
      height: nextHeight,
      scrollTop: record.preparedSnapshot.scrollTop,
    });
  }
  if (itemsChanged || uiStateChanged) {
    const anchor = record.restoreSnapshot?.anchor ?? { kind: "bottom" as const };
    record.preparedSnapshot = record.core.replaceItems(params.listItems, anchor);
    record.preparedItems = params.listItems;
  }
  touchRuntime(record);
  return record;
}

export function markSessionPretextRuntimeVisible(
  record: SessionPretextRuntimeRecord,
  visible: boolean,
): void {
  record.isVisible = visible;
  if (visible) {
    record.hasVisibleMount = true;
  }
  touchRuntime(record);
}

export function readSessionPretextRuntimePreparedState(record: SessionPretextRuntimeRecord): {
  snapshot: PretextVirtualizerSnapshot<WorkbenchListItem>;
  listItems: readonly WorkbenchListItem[];
} {
  return {
    snapshot: record.preparedSnapshot,
    listItems: record.preparedItems,
  };
}

export function readSessionPretextRuntimeRestoreSnapshot(
  record: SessionPretextRuntimeRecord,
): PretextVirtualizerSnapshot<WorkbenchListItem> | null {
  return record.restoreSnapshot;
}

export function pruneSessionPretextRuntimeCache(retainedPreparedSessionIds: readonly string[]): void {
  const retained = new Set(retainedPreparedSessionIds);
  const restorableEntries: Array<[string, SessionPretextRuntimeRecord]> = [];
  for (const [sessionId, record] of runtimeCache.entries()) {
    if (record.isVisible) continue;
    if (retained.has(sessionId)) continue;
    if (!record.restoreSnapshot) {
      runtimeCache.delete(sessionId);
      continue;
    }
    restorableEntries.push([sessionId, record]);
  }
  if (restorableEntries.length <= SESSION_PRETEXT_MAX_RESTORABLE_RUNTIMES) {
    return;
  }
  restorableEntries
    .sort(([, left], [, right]) => right.lastTouchedAtMs - left.lastTouchedAtMs)
    .slice(SESSION_PRETEXT_MAX_RESTORABLE_RUNTIMES)
    .forEach(([sessionId]) => {
      runtimeCache.delete(sessionId);
    });
}

export function getSessionPretextRuntimeCacheSize(): number {
  return runtimeCache.size;
}

export function resetSessionPretextRuntimeCache(): void {
  runtimeCache.clear();
}
