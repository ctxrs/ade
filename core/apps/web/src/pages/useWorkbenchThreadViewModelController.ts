import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Message, SessionEvent, SessionTurn, SessionTurnTool } from "../api/client";
import { idToString } from "../api/client";
import type { AskUserQuestionAnswerState, WorkbenchListItem, WorkbenchThreadView } from "./SessionPage.types";
import type { SessionViewVerbosity } from "../state/uiStateStore";
import {
  buildWorkbenchThreadViewModelFromTurns,
  filterThreadItemsForVerbosity,
} from "./SessionPage.workbenchViewModel";

type Params = {
  sessionId: string;
  turnsKey: string;
  messagesKey: string;
  eventsKey: string;
  verbosity: SessionViewVerbosity;
  turns: SessionTurn[];
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
  // Snapshot markers for cheap "append-only" detection.
  turnsLen: number;
  messagesLen: number;
  eventsLen: number;
};

/**
 * Builds the workbench thread view model outside of render and incrementally updates it
 * on streaming event appends. This prevents full thread re-derivation on every WAL tick.
 *
 * For correctness, we still rebuild fully on structural changes (turns/messages).
 */
export function useWorkbenchThreadViewModelController(
  params: Params,
): { view: WorkbenchThreadView; listItems: WorkbenchListItem[] } {
  const {
    sessionId,
    turnsKey,
    messagesKey,
    eventsKey,
    verbosity,
    turns,
    messages,
    events,
    toolsByTurnId,
    toolSummariesReady,
    askUserQuestionAnswers,
    enableDebugEvents,
  } = params;

  const [state, setState] = useState<InternalState>(() => ({
    view: { groups: [], debugEvents: [] },
    listItems: [],
    groupRanges: new Map(),
    turnsLen: 0,
    messagesLen: 0,
    eventsLen: 0,
  }));

  const turnsById = useMemo(() => {
    const map = new Map<string, SessionTurn>();
    for (const t of turns) {
      const tid = idToString(t.turn_id);
      if (tid) map.set(tid, t);
    }
    return map;
  }, [turns]);

  const messagesByTurnIdRef = useRef<Map<string, Message[]>>(new Map());
  const eventsByTurnIdRef = useRef<Map<string, SessionEvent[]>>(new Map());
  const lastSessionIdRef = useRef(sessionId);
  const lastTurnsKeyRef = useRef(turnsKey);
  const lastMessagesKeyRef = useRef(messagesKey);
  const lastEventsKeyRef = useRef(eventsKey);
  const lastEnableDebugEventsRef = useRef(enableDebugEvents);

  const fullRebuild = useRef(() => {});
  fullRebuild.current = () => {
    const view = buildWorkbenchThreadViewModelFromTurns(
      turns,
      messages,
      toolSummariesReady ? toolsByTurnId : {},
      events,
      askUserQuestionAnswers,
    );

    const listItems: WorkbenchListItem[] = [];
    const groupRanges = new Map<string, { start: number; end: number }>();
    for (const g of view.groups) {
      const start = listItems.length;
      if (g.header) {
        listItems.push({ kind: "turn_header", id: `turn-header-${g.header.id}`, header: g.header });
      }
      const filtered = filterThreadItemsForVerbosity(g.items, verbosity);
      listItems.push(...filtered);
      const end = listItems.length;
      groupRanges.set(String(g.key), { start, end });
    }

    // Reset per-turn caches; incremental updates depend on these.
    const byTurnMsg = new Map<string, Message[]>();
    for (const m of messages) {
      const tid = idToString(m.turn_id);
      if (!tid) continue;
      const list = byTurnMsg.get(tid) ?? [];
      list.push(m);
      byTurnMsg.set(tid, list);
    }
    messagesByTurnIdRef.current = byTurnMsg;

    const byTurnEv = new Map<string, SessionEvent[]>();
    for (const ev of events) {
      const tid = idToString((ev as any).turn_id);
      if (!tid) continue;
      const list = byTurnEv.get(tid) ?? [];
      list.push(ev);
      byTurnEv.set(tid, list);
    }
    eventsByTurnIdRef.current = byTurnEv;

    setState({
      view,
      listItems,
      groupRanges,
      turnsLen: turns.length,
      messagesLen: messages.length,
      eventsLen: events.length,
    });
  };

  useLayoutEffect(() => {
    const sessionChanged = lastSessionIdRef.current !== sessionId;
    if (sessionChanged) {
      lastSessionIdRef.current = sessionId;
      lastTurnsKeyRef.current = turnsKey;
      lastMessagesKeyRef.current = messagesKey;
      lastEventsKeyRef.current = eventsKey;
      lastEnableDebugEventsRef.current = enableDebugEvents;
      messagesByTurnIdRef.current = new Map();
      eventsByTurnIdRef.current = new Map();
      fullRebuild.current();
      return;
    }

    const debugToggled = lastEnableDebugEventsRef.current !== enableDebugEvents;
    if (debugToggled) {
      lastEnableDebugEventsRef.current = enableDebugEvents;
      // Debug events are derived in the view-model builder; rebuild once when toggled.
      fullRebuild.current();
      return;
    }

    // If turns/messages changed in a non-append-only way, do a full rebuild.
    // These are structural changes and should be rare compared to streaming events.
    const turnsStructural = turnsKey !== lastTurnsKeyRef.current || turns.length !== state.turnsLen;
    const messagesStructural = messagesKey !== lastMessagesKeyRef.current || messages.length !== state.messagesLen;
    const eventsKeyChanged = eventsKey !== lastEventsKeyRef.current;
    if (turnsStructural || messagesStructural) {
      lastTurnsKeyRef.current = turnsKey;
      lastMessagesKeyRef.current = messagesKey;
      lastEventsKeyRef.current = eventsKey;
      fullRebuild.current();
      return;
    }

    // Incremental path: events appended only.
    if (events.length < state.eventsLen) {
      lastEventsKeyRef.current = eventsKey;
      fullRebuild.current();
      return;
    }
    if (events.length === state.eventsLen) {
      // If the key changed but length didn't, we can't assume append-only.
      if (eventsKeyChanged) {
        lastEventsKeyRef.current = eventsKey;
        fullRebuild.current();
      }
      return;
    }

    // Debug mode prioritizes correctness over streaming performance.
    // Rebuild once per event append (no infinite loop).
    if (enableDebugEvents) {
      lastEventsKeyRef.current = eventsKey;
      fullRebuild.current();
      return;
    }

    const delta = events.slice(state.eventsLen);
    const dirtyTurnIds = new Set<string>();
    for (const ev of delta) {
      const tid = idToString((ev as any).turn_id);
      if (tid) {
        dirtyTurnIds.add(tid);
        const list = eventsByTurnIdRef.current.get(tid) ?? [];
        list.push(ev);
        eventsByTurnIdRef.current.set(tid, list);
      }
    }
    if (dirtyTurnIds.size === 0) {
      setState((prev) => ({ ...prev, eventsLen: events.length }));
      return;
    }

    // Rebuild only the groups for the turns affected by the new events.
    const currentGroups = state.view.groups;
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
        updatedGroups.push(g);
        continue;
      }
      const msgs = messagesByTurnIdRef.current.get(turnId) ?? [];
      const evs = eventsByTurnIdRef.current.get(turnId) ?? [];
      const tools = toolSummariesReady ? { [turnId]: toolsByTurnId[turnId] ?? [] } : {};

      const rebuilt = buildWorkbenchThreadViewModelFromTurns([turn], msgs, tools, evs, askUserQuestionAnswers);
      const nextGroup = rebuilt.groups.find((x) => x.key === key) ?? rebuilt.groups[0];
      const finalGroup = nextGroup ?? g;
      updatedGroups.push(finalGroup);

      const segment: WorkbenchListItem[] = [];
      if (finalGroup.header) {
        segment.push({ kind: "turn_header", id: `turn-header-${finalGroup.header.id}`, header: finalGroup.header });
      }
      segment.push(...filterThreadItemsForVerbosity(finalGroup.items, verbosity));
      updatedSegments.set(key, segment);
    }

    // Patch the flat list in O(groups) time by using stored group ranges.
    let nextList = state.listItems;
    let nextRanges = state.groupRanges;
    for (const [groupKey, segment] of updatedSegments.entries()) {
      const range = nextRanges.get(groupKey);
      if (!range) continue;
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

    lastEventsKeyRef.current = eventsKey;
    setState((prev) => ({
      ...prev,
      view: { groups: updatedGroups, debugEvents: prev.view.debugEvents },
      listItems: nextList,
      groupRanges: nextRanges,
      eventsLen: events.length,
    }));
  }, [
    askUserQuestionAnswers,
    enableDebugEvents,
    events,
    eventsKey,
    fullRebuild,
    messages,
    messagesKey,
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
    turnsKey,
    turnsById,
  ]);

  return { view: state.view, listItems: state.listItems };
}
