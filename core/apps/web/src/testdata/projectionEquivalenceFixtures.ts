import rawFixtures from "./projectionEquivalence.fixtures.json";
import type {
  Message,
  Session,
  SessionEvent,
  SessionHeadDelta,
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  SessionTurn,
  SessionTurnTool,
  SessionTurnToolSummary,
  Task,
  WorkspaceActiveHeadBatch,
  WorkspaceActiveSnapshot,
  WorkspaceActiveTaskSummary,
} from "@ctx/types";

type ReplayExpectation = {
  afterSeq: number;
  expectedSeqs: number[];
  expectGap: boolean;
};

type ActiveProjectionFixtureSpec = {
  userContent: string;
  assistantContent: string;
  toolCallId: string;
  toolTitle: string;
  toolKind: string;
  toolInput: Record<string, unknown>;
  toolOutput: string;
  streamAssistantChunk: string;
  orderSeqs: {
    user: number;
    tool: number;
    assistant: number;
  };
  replayExpectations: ReplayExpectation[];
  expected: {
    headLastEventSeq: number;
    summaryLastEventSeq: number;
    persistedEventTypes: string[];
    stableEventTypes: string[];
    renderItemKinds: string[];
  };
};

type SessionGapSeedFixtureSpec = {
  userContent: string;
  assistantContent: string;
  afterSeq: number;
  expected: {
    headLastEventSeq: number;
    reason: string;
  };
};

type ProjectionFixtureFile = {
  activeProjectionEquivalence: ActiveProjectionFixtureSpec;
  sessionGapSeedRehydrate: SessionGapSeedFixtureSpec;
};

const fixtures = rawFixtures as ProjectionFixtureFile;

const FIXTURE_TIMES = {
  created: "2026-03-09T00:00:00.000Z",
  tool: "2026-03-09T00:00:01.000Z",
  assistant: "2026-03-09T00:00:02.000Z",
  updated: "2026-03-09T00:00:03.000Z",
} as const;

type ProjectionEntityIds = {
  workspaceId: string;
  taskId: string;
  sessionId: string;
  turnId: string;
  userMessageId: string;
  assistantMessageId: string;
  userEventId: string;
  toolCallEventId: string;
  toolResultEventId: string;
  assistantEventId: string;
  partialEventId: string;
};

type ActiveProjectionFixture = {
  workspaceId: string;
  task: Task;
  session: Session;
  turn: SessionTurn;
  userMessage: Message;
  assistantMessage: Message;
  summary: SessionSnapshotSummary;
  head: SessionHeadSnapshot;
  activeSnapshot: WorkspaceActiveSnapshot;
  activeHeads: WorkspaceActiveHeadBatch;
  toolsByTurnId: Record<string, SessionTurnTool[]>;
  partialDelta: SessionHeadDelta;
  replayExpectations: ReplayExpectation[];
  expected: ActiveProjectionFixtureSpec["expected"] & {
    toolCallId: string;
    assistantContent: string;
  };
};

type SessionGapSeedFixture = {
  workspaceId: string;
  task: Task;
  session: Session;
  summary: SessionSnapshotSummary;
  head: SessionHeadSnapshot;
  activeSnapshot: WorkspaceActiveSnapshot;
  activeHeads: WorkspaceActiveHeadBatch;
  gapEvent: {
    type: "session_gap";
    workspace_id: string;
    snapshot_rev: number;
    session_id: string;
    after_seq: number;
    reason: string;
  };
  seedEvent: {
    type: "session_head_seed";
    workspace_id: string;
    snapshot_rev: number;
    head: SessionHeadSnapshot;
  };
  expected: SessionGapSeedFixtureSpec["expected"];
};

const buildTask = (taskId: string, workspaceId: string): Task => ({
  id: taskId,
  workspace_id: workspaceId,
  title: "Projection fixture task",
  status: "running",
  created_at: FIXTURE_TIMES.created,
  updated_at: FIXTURE_TIMES.updated,
});

const buildSession = (sessionId: string, taskId: string, workspaceId: string): Session => ({
  id: sessionId,
  task_id: taskId,
  workspace_id: workspaceId,
  worktree_id: "wt-projection",
  provider_id: "fake",
  model_id: "fake-model",
  title: "Projection fixture session",
  agent_role: "assistant",
  status: "active",
  created_at: FIXTURE_TIMES.created,
  updated_at: FIXTURE_TIMES.updated,
});

const buildTurn = (
  sessionId: string,
  turnId: string,
  userMessageId: string,
  lastEventSeq: number,
  toolTotal: number,
): SessionTurn => ({
  turn_id: turnId,
  session_id: sessionId,
  run_id: null,
  user_message_id: userMessageId,
  status: "completed",
  start_seq: 1,
  end_seq: lastEventSeq,
  started_at: FIXTURE_TIMES.created,
  updated_at: FIXTURE_TIMES.updated,
  assistant_partial: null,
  thought_partial: null,
  metrics_json: null,
  tool_total: toolTotal,
  tool_pending: 0,
  tool_running: 0,
  tool_completed: toolTotal,
  tool_failed: 0,
});

const buildUserMessage = (
  session: Session,
  turnId: string,
  messageId: string,
  content: string,
  orderSeq: number,
): Message => ({
  id: messageId,
  session_id: session.id,
  task_id: session.task_id,
  turn_id: turnId,
  turn_sequence: orderSeq,
  role: "user",
  content,
  attachments: [],
  delivery: "immediate",
  created_at: FIXTURE_TIMES.created,
});

const buildAssistantMessage = (
  session: Session,
  turnId: string,
  messageId: string,
  content: string,
  orderSeq: number,
): Message => ({
  id: messageId,
  session_id: session.id,
  task_id: session.task_id,
  turn_id: turnId,
  turn_sequence: orderSeq,
  role: "assistant",
  content,
  attachments: [],
  delivery: "immediate",
  created_at: FIXTURE_TIMES.assistant,
});

const buildToolSummary = (
  sessionId: string,
  turnId: string,
  spec: Pick<ActiveProjectionFixtureSpec, "toolCallId" | "toolKind" | "toolInput" | "toolOutput" | "toolTitle">,
): SessionTurnToolSummary => ({
  session_id: sessionId,
  tool_call_id: spec.toolCallId,
  turn_id: turnId,
  tool_kind: spec.toolKind,
  title: spec.toolTitle,
  status: "completed",
  input_preview: spec.toolInput,
  output_preview: spec.toolOutput,
  input_truncated: null,
  input_original_bytes: null,
  output_truncated: null,
  output_original_bytes: null,
  created_at: FIXTURE_TIMES.tool,
  updated_at: FIXTURE_TIMES.updated,
});

const buildToolByTurnId = (
  sessionId: string,
  turnId: string,
  spec: Pick<ActiveProjectionFixtureSpec, "toolCallId" | "toolKind" | "toolInput" | "toolOutput" | "toolTitle">,
): Record<string, SessionTurnTool[]> => ({
  [turnId]: [
    {
      session_id: sessionId,
      tool_call_id: spec.toolCallId,
      turn_id: turnId,
      tool_kind: spec.toolKind,
      title: spec.toolTitle,
      status: "completed",
      input_json: spec.toolInput,
      output_text: spec.toolOutput,
      input_truncated: null,
      input_original_bytes: null,
      output_truncated: null,
      output_original_bytes: null,
      created_at: FIXTURE_TIMES.tool,
      updated_at: FIXTURE_TIMES.updated,
    },
  ],
});

const buildStableEvents = (
  sessionId: string,
  turnId: string,
  ids: ProjectionEntityIds,
  spec: ActiveProjectionFixtureSpec,
): SessionEvent[] => [
  {
    seq: 1,
    id: ids.userEventId,
    session_id: sessionId,
    run_id: null,
    turn_id: turnId,
    event_type: "user_message",
    payload_json: {
      message_id: ids.userMessageId,
      content: spec.userContent,
      attachments: [],
      order_seq: spec.orderSeqs.user,
    },
    created_at: FIXTURE_TIMES.created,
  },
  {
    seq: 2,
    id: ids.toolCallEventId,
    session_id: sessionId,
    run_id: null,
    turn_id: turnId,
    event_type: "tool_call",
    payload_json: {
      tool_call_id: spec.toolCallId,
      title: spec.toolTitle,
      kind: spec.toolKind,
      input: spec.toolInput,
      order_seq: spec.orderSeqs.tool,
    },
    created_at: FIXTURE_TIMES.tool,
  },
  {
    seq: 3,
    id: ids.toolResultEventId,
    session_id: sessionId,
    run_id: null,
    turn_id: turnId,
    event_type: "tool_result",
    payload_json: {
      tool_call_id: spec.toolCallId,
      title: spec.toolTitle,
      kind: spec.toolKind,
      outputText: spec.toolOutput,
      order_seq: spec.orderSeqs.tool,
    },
    created_at: FIXTURE_TIMES.assistant,
  },
  {
    seq: 4,
    id: ids.assistantEventId,
    session_id: sessionId,
    run_id: null,
    turn_id: turnId,
    event_type: "assistant_complete",
    payload_json: {
      message_id: ids.assistantMessageId,
      content: spec.assistantContent,
      full_content: spec.assistantContent,
      order_seq: spec.orderSeqs.assistant,
    },
    created_at: FIXTURE_TIMES.updated,
  },
];

const buildSummary = (
  session: Session,
  assistantContent: string,
  lastEventSeq: number,
): SessionSnapshotSummary => ({
  session,
  last_message_at: FIXTURE_TIMES.assistant,
  last_message_preview: assistantContent,
  last_event_seq: lastEventSeq,
  state_rev: lastEventSeq,
  activity: { is_working: false, last_turn_status: "completed" },
  unread: false,
});

const buildActiveSnapshot = (
  workspaceId: string,
  activeTask: WorkspaceActiveTaskSummary,
): WorkspaceActiveSnapshot => ({
  workspace_id: workspaceId,
  snapshot_rev: 4,
  archived_rev: 0,
  active: {
    total_count: 1,
    tasks: [activeTask],
  },
});

const buildHead = (
  session: Session,
  turn: SessionTurn,
  messages: Message[],
  events: SessionEvent[],
  toolSummaries: SessionTurnToolSummary[],
  lastEventSeq: number,
): SessionHeadSnapshot => ({
  session,
  turns: [turn],
  tool_summaries: toolSummaries,
  events,
  messages,
  last_event_seq: lastEventSeq,
  state_rev: lastEventSeq,
  activity: { is_working: false, last_turn_status: "completed" },
  has_more_turns: false,
  history_cursor: null,
  has_more_history: false,
  summary_checkpoint: null,
  head_window: {
    turn_limit: 5,
    message_limit: 200,
    event_limit: 200,
    byte_limit: 1500000,
    turn_count: 1,
    message_count: messages.length,
    event_count: events.length,
    bytes: 1024,
    truncated: false,
  },
});

const idsFor = (prefix: string): ProjectionEntityIds => ({
  workspaceId: `ws-${prefix}`,
  taskId: `task-${prefix}`,
  sessionId: `session-${prefix}`,
  turnId: `turn-${prefix}`,
  userMessageId: `msg-${prefix}-user`,
  assistantMessageId: `msg-${prefix}-assistant`,
  userEventId: `event-${prefix}-user`,
  toolCallEventId: `event-${prefix}-tool-call`,
  toolResultEventId: `event-${prefix}-tool-result`,
  assistantEventId: `event-${prefix}-assistant`,
  partialEventId: `event-${prefix}-partial`,
});

export function getActiveProjectionFixture(): ActiveProjectionFixture {
  const spec = fixtures.activeProjectionEquivalence;
  const ids = idsFor("projection");
  const task = buildTask(ids.taskId, ids.workspaceId);
  const session = buildSession(ids.sessionId, ids.taskId, ids.workspaceId);
  const turn = buildTurn(
    ids.sessionId,
    ids.turnId,
    ids.userMessageId,
    spec.expected.headLastEventSeq,
    1,
  );
  const userMessage = buildUserMessage(
    session,
    ids.turnId,
    ids.userMessageId,
    spec.userContent,
    spec.orderSeqs.user,
  );
  const assistantMessage = buildAssistantMessage(
    session,
    ids.turnId,
    ids.assistantMessageId,
    spec.assistantContent,
    spec.orderSeqs.assistant,
  );
  const stableEvents = buildStableEvents(ids.sessionId, ids.turnId, ids, spec);
  const toolSummary = buildToolSummary(ids.sessionId, ids.turnId, spec);
  const head = buildHead(
    session,
    turn,
    [userMessage, assistantMessage],
    stableEvents,
    [toolSummary],
    spec.expected.headLastEventSeq,
  );
  const summary = buildSummary(
    session,
    spec.assistantContent,
    spec.expected.summaryLastEventSeq,
  );
  const activeTask: WorkspaceActiveTaskSummary = {
    task,
    primary_session: summary,
    primary_session_head: head,
    sessions: [summary],
    sort_at: FIXTURE_TIMES.updated,
  };

  return {
    workspaceId: ids.workspaceId,
    task,
    session,
    turn,
    userMessage,
    assistantMessage,
    summary,
    head,
    activeSnapshot: buildActiveSnapshot(ids.workspaceId, activeTask),
    activeHeads: {
      workspace_id: ids.workspaceId,
      snapshot_rev: 4,
      heads: [head],
    },
    toolsByTurnId: buildToolByTurnId(ids.sessionId, ids.turnId, spec),
    partialDelta: {
      session_id: ids.sessionId,
      last_event_seq: 3,
      state_rev: 3,
      event: {
        seq: -1,
        id: ids.partialEventId,
        session_id: ids.sessionId,
        run_id: null,
        turn_id: ids.turnId,
        event_type: "assistant_chunk",
        payload_json: {
          content_fragment: spec.streamAssistantChunk,
          order_seq: spec.orderSeqs.assistant,
        },
        transient: true,
        created_at: FIXTURE_TIMES.assistant,
      },
      turn: null,
      message: null,
      tool_summaries: [],
    },
    replayExpectations: spec.replayExpectations,
    expected: {
      ...spec.expected,
      toolCallId: spec.toolCallId,
      assistantContent: spec.assistantContent,
    },
  };
}

export function getSessionGapSeedFixture(): SessionGapSeedFixture {
  const spec = fixtures.sessionGapSeedRehydrate;
  const ids = idsFor("gap-seed");
  const task = buildTask(ids.taskId, ids.workspaceId);
  const session = buildSession(ids.sessionId, ids.taskId, ids.workspaceId);
  const turn = buildTurn(
    ids.sessionId,
    ids.turnId,
    ids.userMessageId,
    spec.expected.headLastEventSeq,
    0,
  );
  const userMessage = buildUserMessage(session, ids.turnId, ids.userMessageId, spec.userContent, 1);
  const assistantMessage = buildAssistantMessage(session, ids.turnId, ids.assistantMessageId, spec.assistantContent, 3);
  const stableEvents: SessionEvent[] = [
    {
      seq: 1,
      id: ids.userEventId,
      session_id: ids.sessionId,
      run_id: null,
      turn_id: ids.turnId,
      event_type: "user_message",
      payload_json: {
        message_id: ids.userMessageId,
        content: spec.userContent,
        attachments: [],
        order_seq: 1,
      },
      created_at: FIXTURE_TIMES.created,
    },
    {
      seq: 4,
      id: ids.assistantEventId,
      session_id: ids.sessionId,
      run_id: null,
      turn_id: ids.turnId,
      event_type: "assistant_complete",
      payload_json: {
        message_id: ids.assistantMessageId,
        content: spec.assistantContent,
        full_content: spec.assistantContent,
        order_seq: 3,
      },
      created_at: FIXTURE_TIMES.updated,
    },
  ];
  const head = buildHead(
    session,
    turn,
    [userMessage, assistantMessage],
    stableEvents,
    [],
    spec.expected.headLastEventSeq,
  );
  const summary = buildSummary(
    session,
    spec.assistantContent,
    spec.expected.headLastEventSeq,
  );
  const activeTask: WorkspaceActiveTaskSummary = {
    task,
    primary_session: summary,
    primary_session_head: head,
    sessions: [summary],
    sort_at: FIXTURE_TIMES.updated,
  };

  return {
    workspaceId: ids.workspaceId,
    task,
    session,
    summary,
    head,
    activeSnapshot: buildActiveSnapshot(ids.workspaceId, activeTask),
    activeHeads: {
      workspace_id: ids.workspaceId,
      snapshot_rev: 4,
      heads: [head],
    },
    gapEvent: {
      type: "session_gap",
      workspace_id: ids.workspaceId,
      snapshot_rev: 5,
      session_id: ids.sessionId,
      after_seq: spec.afterSeq,
      reason: spec.expected.reason,
    },
    seedEvent: {
      type: "session_head_seed",
      workspace_id: ids.workspaceId,
      snapshot_rev: 5,
      head,
    },
    expected: spec.expected,
  };
}
