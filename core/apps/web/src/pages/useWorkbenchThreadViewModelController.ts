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
  buildWorkbenchThreadViewModelWarmKey,
  buildWorkbenchThreadViewModelWarmSnapshot,
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

function getTurnGroupKey(turnId: string): string {
  return `turn-${turnId}`;
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
  fullRebuild.current = (preferredKind = "reconcile") => {
    const rebuilt = buildWorkbenchThreadViewModelWarmSnapshot({
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

    perTurnCachesRef.current = rebuilt.caches;

    setState((previous) => {
      const lastOp = classifyWorkbenchThreadProjectionOp({
        current: previous.listItems,
        next: rebuilt.listItems,
        projectionRevision: projectionRev,
        fallbackKind: preferredKind,
      });
      return {
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
      };
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
    const toolSummariesChanged =
      lastToolSummariesReadyRef.current !== toolSummariesReady ||
      lastToolsByTurnIdRef.current !== toolsByTurnId;
    if (verbosityChanged || askUserQuestionAnswersChanged || toolSummariesChanged) {
      syncInvalidationRefs();
      fullRebuild.current(toolSummariesChanged ? "hydrate_tools" : "reconcile");
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
      setState((prev) => ({
        ...prev,
        projectionRevision: projectionRev,
        lastOp: createWorkbenchThreadProjectionOp("noop", projectionRev),
        changedItemIds: [],
        remeasureItemIds: [],
        eventsLen: events.length,
      }));
      return;
    }

    // Rebuild only the groups for the turns affected by the new events.
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
        syncInvalidationRefs();
        fullRebuild.current("reconcile");
        return;
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
        syncInvalidationRefs();
        fullRebuild.current("reconcile");
        return;
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
      const nextGroup = rebuilt.groups.find((x) => x.key === key);
      if (!nextGroup) {
        syncInvalidationRefs();
        fullRebuild.current("reconcile");
        return;
      }
      updatedGroups.push(nextGroup);

      const segment: WorkbenchListItem[] = [];
      if (nextGroup.header) {
        segment.push({ kind: "turn_header", id: `turn-header-${nextGroup.header.id}`, header: nextGroup.header });
      }
      segment.push(...filterThreadItemsForVerbosity(nextGroup.items, verbosity));
      updatedSegments.set(key, segment);
    }

    // Patch the flat list in O(groups) time by using stored group ranges.
    let nextList = state.listItems;
    let nextRanges = state.groupRanges;
    for (const [groupKey, segment] of updatedSegments.entries()) {
      const range = nextRanges.get(groupKey);
      if (!range) {
        syncInvalidationRefs();
        fullRebuild.current("reconcile");
        return;
      }
      const prevLen = range.end - range.start;
      const nextLen = segment.length;
      if (prevLen === nextLen) {
        nextList = [
          ...nextList.slice(0, range.start),
          ...segment,
          ...nextList.slice(range.end),
        ];
        continue;
      }
      // When the segment size changes, adjust subsequent ranges.
      const deltaLen = nextLen - prevLen;
      nextList = [
        ...nextList.slice(0, range.start),
        ...segment,
        ...nextList.slice(range.end),
      ];
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
    setState((prev) => {
      const lastOp = classifyWorkbenchThreadProjectionOp({
        current: prev.listItems,
        next: nextList,
        projectionRevision: projectionRev,
        fallbackKind: "append_stream",
      });
      return {
        ...prev,
        view: { groups: updatedGroups, debugEvents: prev.view.debugEvents },
        listItems: nextList,
        groupRanges: nextRanges,
        projectionRevision: projectionRev,
        lastOp,
        changedItemIds: lastOp.changedItemIds,
        remeasureItemIds: lastOp.remeasureItemIds,
        eventsLen: events.length,
      };
    });
  }, [
    askUserQuestionAnswers,
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

  useLayoutEffect(() => {
    persistWarmWorkbenchThreadViewModel(sessionId, {
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
      projectionRevision: state.projectionRevision,
      view: state.view,
      listItems: state.listItems,
      groupRanges: state.groupRanges,
      turnsLen: state.turnsLen,
      messagesLen: state.messagesLen,
      eventsLen: state.eventsLen,
      caches: perTurnCachesRef.current,
    });
  }, [
    askUserQuestionAnswers,
    assistantStreamingByTurnId,
    enableDebugEvents,
    events,
    eventsStamp,
    messages,
    messagesStamp,
    projectionRev,
    sessionId,
    state,
    toolSummariesReady,
    toolsByTurnId,
    turns,
    turnsStamp,
    verbosity,
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
