import { expect, type Page } from "playwright/test";

type E2EWorkspaceStream = {
  getConnectionState?: () => string | null;
  dispatchMessage?: (payload: unknown) => void;
};

type E2EWindow = Window & {
  __ctxE2E?: {
    workspaceStream?: E2EWorkspaceStream;
    getSessionHeadMessages?: (sessionId: string) => unknown[];
  };
};

export type VisualToolSummaryFixture = {
  toolCallId: string;
  title: string;
  kind: string;
  inputPreview: unknown;
  outputPreview: string;
  status?: "completed" | "running" | "failed";
};

export type VisualTurnFixture = {
  turnId: string;
  userContent: string;
  assistantContent?: string;
  status?: "completed" | "running";
  toolSummaries?: VisualToolSummaryFixture[];
};

type SeedVisualSessionHeadOptions = {
  workspaceId: string;
  taskId: string;
  sessionId: string;
  turns: VisualTurnFixture[];
};

const FIXTURE_START_MS = Date.UTC(2025, 0, 1, 12, 0, 0);

const isoAt = (offsetMs: number) => new Date(FIXTURE_START_MS + offsetMs).toISOString();

const expectedHeadTexts = (turns: VisualTurnFixture[]) =>
  turns.flatMap((turn) => [turn.userContent, turn.assistantContent].filter((value): value is string => Boolean(value)));

function buildHeadPayload(opts: SeedVisualSessionHeadOptions) {
  const messages: Array<Record<string, unknown>> = [];
  const events: Array<Record<string, unknown>> = [];
  const turns: Array<Record<string, unknown>> = [];
  const toolSummaries: Array<Record<string, unknown>> = [];

  let eventSeq = 1;
  let orderSeq = 1;
  let offsetMs = 0;

  for (const turn of opts.turns) {
    const userMessageId = `${turn.turnId}-user`;
    const userCreatedAt = isoAt(offsetMs);
    const toolFixtures = turn.toolSummaries ?? [];
    const toolStatusCounts = {
      running: toolFixtures.filter((tool) => tool.status === "running").length,
      completed: toolFixtures.filter((tool) => (tool.status ?? "completed") === "completed").length,
      failed: toolFixtures.filter((tool) => tool.status === "failed").length,
    };
    const turnStartSeq = eventSeq;

    messages.push({
      id: userMessageId,
      session_id: opts.sessionId,
      task_id: opts.taskId,
      turn_id: turn.turnId,
      turn_sequence: orderSeq,
      role: "user",
      content: turn.userContent,
      attachments: [],
      delivery: "immediate",
      created_at: userCreatedAt,
    });
    events.push({
      seq: eventSeq,
      id: `${turn.turnId}-event-user`,
      session_id: opts.sessionId,
      run_id: null,
      turn_id: turn.turnId,
      event_type: "user_message",
      payload_json: {
        message_id: userMessageId,
        content: turn.userContent,
        attachments: [],
        order_seq: orderSeq,
      },
      created_at: userCreatedAt,
    });
    eventSeq += 1;
    orderSeq += 1;
    offsetMs += 1_000;

    for (const tool of toolFixtures) {
      const toolCreatedAt = isoAt(offsetMs);
      const toolStatus = tool.status ?? "completed";
      toolSummaries.push({
        session_id: opts.sessionId,
        tool_call_id: tool.toolCallId,
        turn_id: turn.turnId,
        tool_kind: tool.kind,
        title: tool.title,
        status: toolStatus,
        input_preview: tool.inputPreview,
        output_preview: tool.outputPreview,
        input_truncated: null,
        input_original_bytes: null,
        output_truncated: null,
        output_original_bytes: null,
        created_at: toolCreatedAt,
        updated_at: toolCreatedAt,
      });
      events.push({
        seq: eventSeq,
        id: `${turn.turnId}-event-tool-${tool.toolCallId}`,
        session_id: opts.sessionId,
        run_id: null,
        turn_id: turn.turnId,
        event_type: "tool_call",
        payload_json: {
          tool_call_id: tool.toolCallId,
          title: tool.title,
          kind: tool.kind,
          input: tool.inputPreview,
          order_seq: orderSeq,
        },
        created_at: toolCreatedAt,
      });
      eventSeq += 1;
      offsetMs += 1_000;

      if (toolStatus === "completed" || toolStatus === "failed") {
        events.push({
          seq: eventSeq,
          id: `${turn.turnId}-event-tool-result-${tool.toolCallId}`,
          session_id: opts.sessionId,
          run_id: null,
          turn_id: turn.turnId,
          event_type: "tool_result",
          payload_json: {
            tool_call_id: tool.toolCallId,
            title: tool.title,
            kind: tool.kind,
            outputText: tool.outputPreview,
            order_seq: orderSeq,
          },
          created_at: isoAt(offsetMs),
        });
        eventSeq += 1;
        offsetMs += 1_000;
      }

      orderSeq += 1;
    }

    if (turn.assistantContent) {
      const assistantMessageId = `${turn.turnId}-assistant`;
      const assistantCreatedAt = isoAt(offsetMs);
      messages.push({
        id: assistantMessageId,
        session_id: opts.sessionId,
        task_id: opts.taskId,
        turn_id: turn.turnId,
        turn_sequence: orderSeq,
        role: "assistant",
        content: turn.assistantContent,
        attachments: [],
        delivery: "immediate",
        created_at: assistantCreatedAt,
      });
      events.push({
        seq: eventSeq,
        id: `${turn.turnId}-event-assistant`,
        session_id: opts.sessionId,
        run_id: null,
        turn_id: turn.turnId,
        event_type: "assistant_complete",
        payload_json: {
          message_id: assistantMessageId,
          content: turn.assistantContent,
          full_content: turn.assistantContent,
          order_seq: orderSeq,
        },
        created_at: assistantCreatedAt,
      });
      eventSeq += 1;
      orderSeq += 1;
      offsetMs += 1_000;
    }

    const lastStatus = turn.status ?? "completed";
    turns.push({
      turn_id: turn.turnId,
      session_id: opts.sessionId,
      user_message_id: userMessageId,
      status: lastStatus,
      start_seq: turnStartSeq,
      end_seq: eventSeq - 1,
      started_at: userCreatedAt,
      updated_at: isoAt(Math.max(0, offsetMs - 1_000)),
      assistant_partial: null,
      thought_partial: null,
      tool_total: toolFixtures.length,
      tool_pending: 0,
      tool_running: toolStatusCounts.running,
      tool_completed: toolStatusCounts.completed,
      tool_failed: toolStatusCounts.failed,
    });

    offsetMs += 4_000;
  }

  const lastTurn = opts.turns[opts.turns.length - 1];
  const lastTurnStatus = lastTurn?.status ?? "completed";
  const isWorking = lastTurnStatus === "running";
  const lastEventSeq = Math.max(0, eventSeq - 1);

  return {
    session: {
      id: opts.sessionId,
      task_id: opts.taskId,
    },
    turns,
    messages,
    events: events.filter((event) => {
      const eventType = String(event.event_type ?? "");
      return eventType.startsWith("tool_");
    }),
    tool_summaries: toolSummaries,
    last_event_seq: lastEventSeq,
    state_rev: lastEventSeq,
    activity: {
      is_working: isWorking,
      last_turn_status: lastTurnStatus,
    },
    has_more_turns: false,
    history_cursor: null,
    has_more_history: false,
    summary_checkpoint: null,
  };
}

export async function waitForWorkspaceStreamReady(page: Page): Promise<void> {
  await expect
    .poll(async () => page.evaluate(() => (window as E2EWindow).__ctxE2E?.workspaceStream?.getConnectionState?.()), {
      timeout: 20_000,
    })
    .toBe("connected");
  await expect
    .poll(async () =>
      page.evaluate(() => typeof (window as E2EWindow).__ctxE2E?.workspaceStream?.dispatchMessage === "function"),
      { timeout: 20_000 },
    )
    .toBe(true);
}

export async function seedVisualSessionHead(page: Page, opts: SeedVisualSessionHeadOptions): Promise<void> {
  const head = buildHeadPayload(opts);
  const payload = {
    type: "event",
    event: {
      type: "session_head_seed",
      workspace_id: opts.workspaceId,
      head,
    },
  };
  await waitForWorkspaceStreamReady(page);
  await page.evaluate((message) => {
    (window as E2EWindow).__ctxE2E?.workspaceStream?.dispatchMessage?.(message);
  }, payload);

  const expectedTexts = expectedHeadTexts(opts.turns);
  if (expectedTexts.length === 0) return;

  await expect
    .poll(async () =>
      page.evaluate(
        ({ currentSessionId, expected }) => {
          const messages = (window as E2EWindow).__ctxE2E?.getSessionHeadMessages?.(currentSessionId) ?? [];
          return expected.every((text) => messages.some((message) => String(message).includes(text)));
        },
        { currentSessionId: opts.sessionId, expected: expectedTexts },
      ),
    )
    .toBe(true);
}

export async function seedVisualSessionHeads(
  page: Page,
  seeds: SeedVisualSessionHeadOptions[],
): Promise<void> {
  for (const seed of seeds) {
    await seedVisualSessionHead(page, seed);
  }
}
