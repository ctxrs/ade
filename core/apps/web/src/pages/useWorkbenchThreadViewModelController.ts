import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Message, SessionEvent, SessionTurn, SessionTurnTool } from "../api/client";
import { idToString } from "../api/client";
import type { AskUserQuestionAnswerState, WorkbenchListItem, WorkbenchThreadView } from "./SessionPage.types";
import type { SessionViewVerbosity } from "../state/uiStateStore";
import type { AssistantStreamingState } from "../state/assistantStreaming";
import {
  buildWorkbenchThreadViewModelFromTurns,
  filterThreadItemsForVerbosity,
} from "./SessionPage.workbenchViewModel";
import {
  classifyWorkbenchThreadProjectionOp,
  createWorkbenchThreadProjectionOp,
  type WorkbenchThreadProjectionOp,
  type WorkbenchThreadProjectionOpKind,
} from "./sessionThreadProjection";
import {
  buildWorkbenchThreadViewModelLayoutKey,
  buildWorkbenchThreadViewModelSourceKey,
  buildWorkbenchThreadViewModelWarmKey,
  persistWarmWorkbenchThreadViewModel,
  primeWarmWorkbenchThreadViewModel,
  type WorkbenchThreadViewModelPerTurnCaches,
} from "./workbenchThreadViewModelWarmCache";

type Params = {
  sessionId: string;
  projectionRev?: number;
  turnsStamp: string;
  messagesStamp: string;
  eventsStamp: string;
  verbosity: SessionViewVerbosity;
  turns: SessionTurn[];
  assistantStreamingByTurnId?: Record<string, AssistantStreamingState>;
  messages: Message[];
  events: SessionEvent[];
  toolsByTurnId: Record<string, SessionTurnTool[]>;
  toolSummariesReady: boolean;
  askUserQuestionAnswers: Map<string, AskUserQuestionAnswerState>;
  // If enabled, we fall back to full rebuilds to keep debugEvents accurate.
  enableDebugEvents: boolean;
};

type InternalState = {
  view: WorkbenchThreadView;
  listItems: WorkbenchListItem[];
  groupRanges: Map<string, { start: number; end: number }>;
  projectionRevision: number;
  lastOp: WorkbenchThreadProjectionOp;
  changedItemIds: string[];
  remeasureItemIds: string[];
  // Snapshot markers for cheap "append-only" detection.
  turnsLen: number;
  messagesLen: number;
  eventsLen: number;
};

function haveSameIds(previous: readonly string[], next: readonly string[]): boolean {
  if (previous === next) return true;
  if (previous.length !== next.length) return false;
  for (let index = 0; index < previous.length; index += 1) {
    if (previous[index] !== next[index]) return false;
  }
  return true;
}

function areInternalStatesEquivalent(previous: InternalState, next: InternalState): boolean {
  return (
    previous === next ||
    (previous.view === next.view &&
      previous.listItems === next.listItems &&
      previous.groupRanges === next.groupRanges &&
      previous.projectionRevision === next.projectionRevision &&
      previous.lastOp.kind === next.lastOp.kind &&
      previous.lastOp.projectionRevision === next.lastOp.projectionRevision &&
      haveSameIds(previous.lastOp.changedItemIds, next.lastOp.changedItemIds) &&
      haveSameIds(previous.lastOp.remeasureItemIds, next.lastOp.remeasureItemIds) &&
      haveSameIds(previous.changedItemIds, next.changedItemIds) &&
      haveSameIds(previous.remeasureItemIds, next.remeasureItemIds) &&
      previous.turnsLen === next.turnsLen &&
      previous.messagesLen === next.messagesLen &&
      previous.eventsLen === next.eventsLen)
  );
}

function getTurnGroupKey(turnId: string): string {
  return `turn-${turnId}`;
}

function areToolMapsShallowEqual(
  previous: Record<string, SessionTurnTool[]>,
  next: Record<string, SessionTurnTool[]>,
): boolean {
  if (previous === next) return true;
  const previousKeys = Object.keys(previous);
  const nextKeys = Object.keys(next);
  if (previousKeys.length !== nextKeys.length) return false;
  for (const key of previousKeys) {
    if (!(key in next)) return false;
    if (previous[key] !== next[key]) return false;
  }
  return true;
}

function collectChangedToolTurnIds(
  previous: Record<string, SessionTurnTool[]>,
  next: Record<string, SessionTurnTool[]>,
): Set<string> {
  const changed = new Set<string>();
  const keys = new Set([...Object.keys(previous), ...Object.keys(next)]);
  for (const key of keys) {
    if (previous[key] !== next[key]) {
      changed.add(key);
    }
  }
  return changed;
}


/**
 * Narrow sanctioned projector fast path:
 * - only append-only event deltas
 * - only when transcript stamps and local projection inputs are unchanged
 * - rebuild immediately on any ambiguity
 */
export function useWorkbenchThreadViewModelController(
  params: Params,
): {
  view: WorkbenchThreadView;
  listItems: WorkbenchListItem[];
  groupRanges: Map<string, { start: number; end: number }>;
  projectionRevision: number;
  lastOp: WorkbenchThreadProjectionOp;
  changedItemIds: string[];
  remeasureItemIds: string[];
} {
  const {
    sessionId,
    projectionRev: projectionRevInput,
    turnsStamp,
    messagesStamp,
    eventsStamp,
    verbosity,
    turns,
    assistantStreamingByTurnId = {},
    messages,
    events,
    toolsByTurnId,
    toolSummariesReady,
    askUserQuestionAnswers,
    enableDebugEvents,
  } = params;
  const projectionRev = projectionRevInput ?? 0;

  const buildWarmSnapshot = () =>
    primeWarmWorkbenchThreadViewModel({
      sessionId,
      projectionRev,
      turnsStamp,
      messagesStamp,
      eventsStamp,
      verbosity,
      turns,
      assistantStreamingByTurnId,
      messages,
      events,
      toolsByTurnId,
      toolSummariesReady,
      askUserQuestionAnswers,
      enableDebugEvents,
    });

  const persistCurrentWarmSnapshot = (nextState: InternalState) => {
    const sourceKey = buildWorkbenchThreadViewModelSourceKey({
      sessionId,
      projectionRev,
      turnsStamp,
      messagesStamp,
      eventsStamp,
      verbosity,
      turns,
      assistantStreamingByTurnId,
      messages,
      events,
      toolsByTurnId,
      toolSummariesReady,
      askUserQuestionAnswers,
      enableDebugEvents,
    });
    const layoutKey = buildWorkbenchThreadViewModelLayoutKey({ verbosity });
    persistWarmWorkbenchThreadViewModel(sessionId, {
      sourceKey,
      layoutKey,
      warmKey: buildWorkbenchThreadViewModelWarmKey({
        sessionId,
        projectionRev,
        turnsStamp,
        messagesStamp,
        eventsStamp,
        verbosity,
        turns,
        assistantStreamingByTurnId,
        messages,
        events,
        toolsByTurnId,
        toolSummariesReady,
        askUserQuestionAnswers,
        enableDebugEvents,
      }),
      projectionRevision: nextState.projectionRevision,
      view: nextState.view,
      listItems: nextState.listItems,
      groupRanges: nextState.groupRanges,
      turnsLen: nextState.turnsLen,
      messagesLen: nextState.messagesLen,
      eventsLen: nextState.eventsLen,
      caches: perTurnCachesRef.current,
    });
  };

  const initialBuildRef = useRef<{ state: InternalState; caches: WorkbenchThreadViewModelPerTurnCaches } | null>(null);
  if (initialBuildRef.current === null) {
    const initialState = buildWarmSnapshot();
    const initialOp = createWorkbenchThreadProjectionOp("noop", projectionRev);
    initialBuildRef.current = {
      state: {
        view: initialState.view,
        listItems: initialState.listItems,
        groupRanges: initialState.groupRanges,
        projectionRevision: projectionRev,
        lastOp: initialOp,
        changedItemIds: initialOp.changedItemIds,
        remeasureItemIds: initialOp.remeasureItemIds,
        turnsLen: initialState.turnsLen,
        messagesLen: initialState.messagesLen,
        eventsLen: initialState.eventsLen,
      },
      caches: initialState.caches,
    };
  }
  const initialBuild = initialBuildRef.current!;

  const [state, setState] = useState<InternalState>(() => ({
    ...initialBuild.state,
  }));

  const turnsById = useMemo(() => {
    const map = new Map<string, SessionTurn>();
    for (const t of turns) {
      const tid = idToString(t.turn_id);
      if (tid) map.set(tid, t);
    }
    return map;
  }, [turns, turnsStamp]);

  const perTurnCachesRef = useRef<WorkbenchThreadViewModelPerTurnCaches>(initialBuild.caches);
  const lastSessionIdRef = useRef(sessionId);
  const lastTurnsStampRef = useRef(turnsStamp);
  const lastMessagesStampRef = useRef(messagesStamp);
  const lastEventsStampRef = useRef(eventsStamp);
  const lastEventsRef = useRef(events);
  const lastEnableDebugEventsRef = useRef(enableDebugEvents);
  const lastVerbosityRef = useRef(verbosity);
  const lastAskUserQuestionAnswersRef = useRef(askUserQuestionAnswers);
  const lastToolSummariesReadyRef = useRef(toolSummariesReady);
  const lastToolsByTurnIdRef = useRef(toolsByTurnId);

  const fullRebuild = useRef((_preferredKind?: WorkbenchThreadProjectionOpKind) => {});
  const commitNextState = (previous: InternalState, next: InternalState): boolean => {
    if (areInternalStatesEquivalent(previous, next)) {
      return false;
    }
    persistCurrentWarmSnapshot(next);
    setState(next);
    return true;
  };
  fullRebuild.current = (preferredKind = "reconcile") => {
    const rebuilt = buildWarmSnapshot();

    perTurnCachesRef.current = rebuilt.caches;

    const lastOp = classifyWorkbenchThreadProjectionOp({
      current: state.listItems,
      next: rebuilt.listItems,
      projectionRevision: projectionRev,
      fallbackKind: preferredKind,
    });
    if (
      lastOp.kind === "noop" &&
      state.turnsLen === rebuilt.turnsLen &&
      state.messagesLen === rebuilt.messagesLen &&
      state.eventsLen === rebuilt.eventsLen
    ) {
      const nextView =
        rebuilt.view.debugEvents === state.view.debugEvents
          ? state.view
          : { groups: state.view.groups, debugEvents: rebuilt.view.debugEvents };
      if (state.lastOp.kind === "noop" && nextView === state.view) {
        return;
      }
      commitNextState(state, {
        ...state,
        view: nextView,
        lastOp,
        changedItemIds: [],
        remeasureItemIds: [],
      });
      return;
    }
    commitNextState(state, {
      view: rebuilt.view,
      listItems: rebuilt.listItems,
      groupRanges: rebuilt.groupRanges,
      projectionRevision: projectionRev,
      lastOp,
      changedItemIds: lastOp.changedItemIds,
      remeasureItemIds: lastOp.remeasureItemIds,
      turnsLen: rebuilt.turnsLen,
      messagesLen: rebuilt.messagesLen,
      eventsLen: rebuilt.eventsLen,
    });
  };

  useLayoutEffect(() => {
    const syncInvalidationRefs = () => {
      lastTurnsStampRef.current = turnsStamp;
      lastMessagesStampRef.current = messagesStamp;
      lastEventsStampRef.current = eventsStamp;
      lastEventsRef.current = events;
      lastEnableDebugEventsRef.current = enableDebugEvents;
      lastVerbosityRef.current = verbosity;
      lastAskUserQuestionAnswersRef.current = askUserQuestionAnswers;
      lastToolSummariesReadyRef.current = toolSummariesReady;
      lastToolsByTurnIdRef.current = toolsByTurnId;
    };

    const rebuildDirtyTurnGroups = (
      dirtyTurnIds: ReadonlySet<string>,
      fallbackKind: WorkbenchThreadProjectionOpKind,
      nextEventsLen: number,
    ): boolean => {
      if (dirtyTurnIds.size === 0) {
        syncInvalidationRefs();
        const nextState: InternalState = {
          ...state,
          projectionRevision: projectionRev,
          lastOp: createWorkbenchThreadProjectionOp("noop", projectionRev),
          changedItemIds: [],
          remeasureItemIds: [],
          eventsLen: nextEventsLen,
        };
        commitNextState(state, nextState);
        return true;
      }

      const currentGroups = state.view.groups;
      const currentTurnGroupKeys = new Set(
        currentGroups
          .map((group) => String(group.key ?? ""))
          .filter((groupKey) => groupKey.startsWith("turn-")),
      );
      for (const turnId of dirtyTurnIds) {
        const groupKey = getTurnGroupKey(turnId);
        if (
          !turnsById.has(turnId)
          || !state.groupRanges.has(groupKey)
          || !currentTurnGroupKeys.has(groupKey)
        ) {
          return false;
        }
      }

      const updatedGroups: WorkbenchThreadView["groups"] = [];
      const updatedSegments = new Map<string, WorkbenchListItem[]>();

      for (const g of currentGroups) {
        const key = String(g.key ?? "");
        if (!key.startsWith("turn-")) {
          updatedGroups.push(g);
          continue;
        }
        const turnId = key.slice("turn-".length);
        if (!dirtyTurnIds.has(turnId)) {
          updatedGroups.push(g);
          continue;
        }

        const turn = turnsById.get(turnId);
        if (!turn) {
          return false;
        }
        const msgs = perTurnCachesRef.current.messagesByTurnId.get(turnId) ?? [];
        const evs = perTurnCachesRef.current.eventsByTurnId.get(turnId) ?? [];
        const tools = toolSummariesReady ? { [turnId]: toolsByTurnId[turnId] ?? [] } : {};
        const assistantStreaming =
          assistantStreamingByTurnId[turnId] == null ? {} : { [turnId]: assistantStreamingByTurnId[turnId]! };

        const rebuilt = buildWorkbenchThreadViewModelFromTurns(
          [turn],
          msgs,
          tools,
          evs,
          assistantStreaming,
          askUserQuestionAnswers,
        );
        const nextGroup = rebuilt.groups.find((group) => group.key === key);
        if (!nextGroup) {
          return false;
        }
        updatedGroups.push(nextGroup);

        const segment: WorkbenchListItem[] = [];
        if (nextGroup.header) {
          segment.push({ kind: "turn_header", id: `turn-header-${nextGroup.header.id}`, header: nextGroup.header });
        }
        segment.push(...filterThreadItemsForVerbosity(nextGroup.items, verbosity));
        updatedSegments.set(key, segment);
      }

      let nextList = state.listItems;
      let nextRanges = state.groupRanges;
      for (const [groupKey, segment] of updatedSegments.entries()) {
        const range = nextRanges.get(groupKey);
        if (!range) {
          return false;
        }
        const prevLen = range.end - range.start;
        const nextLen = segment.length;
        nextList = [
          ...nextList.slice(0, range.start),
          ...segment,
          ...nextList.slice(range.end),
        ];
        if (prevLen === nextLen) {
          continue;
        }
        const deltaLen = nextLen - prevLen;
        const adjusted = new Map<string, { start: number; end: number }>();
        for (const [k, r] of nextRanges.entries()) {
          if (k === groupKey) {
            adjusted.set(k, { start: r.start, end: r.start + nextLen });
            continue;
          }
          if (r.start >= range.end) {
            adjusted.set(k, { start: r.start + deltaLen, end: r.end + deltaLen });
          } else {
            adjusted.set(k, r);
          }
        }
        nextRanges = adjusted;
      }

      syncInvalidationRefs();
      const lastOp = classifyWorkbenchThreadProjectionOp({
        current: state.listItems,
        next: nextList,
        projectionRevision: projectionRev,
        fallbackKind,
      });
      const nextState: InternalState =
        lastOp.kind === "noop" && state.eventsLen === nextEventsLen
          ? {
              ...state,
              projectionRevision: projectionRev,
              lastOp,
              changedItemIds: [],
              remeasureItemIds: [],
            }
          : {
              ...state,
              view: { groups: updatedGroups, debugEvents: state.view.debugEvents },
              listItems: nextList,
              groupRanges: nextRanges,
              projectionRevision: projectionRev,
              lastOp,
              changedItemIds: lastOp.changedItemIds,
              remeasureItemIds: lastOp.remeasureItemIds,
              eventsLen: nextEventsLen,
            };
      commitNextState(state, nextState);
      return true;
    };

    const sessionChanged = lastSessionIdRef.current !== sessionId;
    if (sessionChanged) {
      lastSessionIdRef.current = sessionId;
      syncInvalidationRefs();
      const rebuilt = buildWarmSnapshot();
      perTurnCachesRef.current = rebuilt.caches;
      const lastOp = createWorkbenchThreadProjectionOp("noop", projectionRev);
      setState({
        view: rebuilt.view,
        listItems: rebuilt.listItems,
        groupRanges: rebuilt.groupRanges,
        projectionRevision: projectionRev,
        lastOp,
        changedItemIds: [],
        remeasureItemIds: [],
        turnsLen: rebuilt.turnsLen,
        messagesLen: rebuilt.messagesLen,
        eventsLen: rebuilt.eventsLen,
      });
      return;
    }

    const debugToggled = lastEnableDebugEventsRef.current !== enableDebugEvents;
    if (debugToggled) {
      syncInvalidationRefs();
      // Debug events are derived in the view-model builder; rebuild once when toggled.
      fullRebuild.current("reconcile");
      return;
    }

    const verbosityChanged = lastVerbosityRef.current !== verbosity;
    const askUserQuestionAnswersChanged = lastAskUserQuestionAnswersRef.current !== askUserQuestionAnswers;
    const toolSummariesReadyChanged = lastToolSummariesReadyRef.current !== toolSummariesReady;
    const dirtyToolTurnIds = collectChangedToolTurnIds(lastToolsByTurnIdRef.current, toolsByTurnId);
    const toolSummariesChanged = toolSummariesReadyChanged || dirtyToolTurnIds.size > 0;
    if (verbosityChanged || askUserQuestionAnswersChanged || toolSummariesChanged) {
      if (verbosityChanged || askUserQuestionAnswersChanged) {
        syncInvalidationRefs();
        fullRebuild.current("reconcile");
        return;
      }
      const localizedToolTurnIds = toolSummariesReadyChanged
        ? new Set([
            ...Object.keys(lastToolsByTurnIdRef.current),
            ...Object.keys(toolsByTurnId),
          ])
        : dirtyToolTurnIds;
      if (!areToolMapsShallowEqual(lastToolsByTurnIdRef.current, toolsByTurnId) || toolSummariesReadyChanged) {
        if (rebuildDirtyTurnGroups(localizedToolTurnIds, "hydrate_tools", state.eventsLen)) {
          return;
        }
      }
      syncInvalidationRefs();
      fullRebuild.current("hydrate_tools");
      return;
    }

    // If turns/messages changed in a non-append-only way, do a full rebuild.
    // These are structural changes and should be rare compared to streaming events.
    const turnsStructural = turnsStamp !== lastTurnsStampRef.current || turns.length !== state.turnsLen;
    const messagesStructural =
      messagesStamp !== lastMessagesStampRef.current || messages.length !== state.messagesLen;
    const eventsStampChanged = eventsStamp !== lastEventsStampRef.current;
    if (turnsStructural || messagesStructural) {
      syncInvalidationRefs();
      fullRebuild.current("reconcile");
      return;
    }

    // Incremental path: events appended only.
    if (events.length < state.eventsLen) {
      syncInvalidationRefs();
      fullRebuild.current("reconcile");
      return;
    }
    if (events.length === state.eventsLen) {
      // If the stamp changed but length didn't, we can't assume append-only.
      if (eventsStampChanged) {
        syncInvalidationRefs();
        fullRebuild.current("reconcile");
      }
      return;
    }

    // Debug mode prioritizes correctness over streaming performance.
    // Rebuild once per event append (no infinite loop).
    if (enableDebugEvents) {
      syncInvalidationRefs();
      fullRebuild.current("reconcile");
      return;
    }

    const previousEvents = lastEventsRef.current;
    let appendOnlyPrefixStable = previousEvents.length === state.eventsLen;
    if (appendOnlyPrefixStable) {
      for (let i = 0; i < state.eventsLen; i += 1) {
        if (previousEvents[i] !== events[i]) {
          appendOnlyPrefixStable = false;
          break;
        }
      }
    }
    if (!appendOnlyPrefixStable) {
      syncInvalidationRefs();
      fullRebuild.current("reconcile");
      return;
    }

    const delta = events.slice(state.eventsLen);
    const dirtyTurnIds = new Set<string>();
    for (const ev of delta) {
      const tid = idToString(ev.turn_id ?? "");
      if (tid) {
        dirtyTurnIds.add(tid);
        const list = perTurnCachesRef.current.eventsByTurnId.get(tid) ?? [];
        list.push(ev);
        perTurnCachesRef.current.eventsByTurnId.set(tid, list);
      }
    }
    if (dirtyTurnIds.size === 0) {
      syncInvalidationRefs();
      const nextState: InternalState = {
        ...state,
        projectionRevision: projectionRev,
        lastOp: createWorkbenchThreadProjectionOp("noop", projectionRev),
        changedItemIds: [],
        remeasureItemIds: [],
        eventsLen: events.length,
      };
      commitNextState(state, nextState);
      return;
    }

    if (rebuildDirtyTurnGroups(dirtyTurnIds, "append_stream", events.length)) {
      return;
    }
    syncInvalidationRefs();
    fullRebuild.current("reconcile");
  }, [
    askUserQuestionAnswers,
    assistantStreamingByTurnId,
    enableDebugEvents,
    events,
    eventsStamp,
    fullRebuild,
    messages,
    messagesStamp,
    projectionRev,
    sessionId,
    verbosity,
    state.eventsLen,
    state.messagesLen,
    state.turnsLen,
    state.view.groups,
    state.groupRanges,
    state.listItems,
    toolSummariesReady,
    toolsByTurnId,
    turns,
    turnsStamp,
    turnsById,
  ]);

  return {
    view: state.view,
    listItems: state.listItems,
    groupRanges: state.groupRanges,
    projectionRevision: state.projectionRevision,
    lastOp: state.lastOp,
    changedItemIds: state.changedItemIds,
    remeasureItemIds: state.remeasureItemIds,
  };
}
