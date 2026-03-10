import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Message, SessionEvent, SessionTurn, SessionTurnTool } from "../api/client";
import type { AskUserQuestionAnswerState } from "./SessionPage.types";
import { deriveMessagesKey, deriveTurnsKey } from "./SessionPage.workbenchViewModel";
import { useWorkbenchThreadViewModelController } from "./useWorkbenchThreadViewModelController";

type ControllerProps = Parameters<typeof useWorkbenchThreadViewModelController>[0];
type ControllerResult = ReturnType<typeof useWorkbenchThreadViewModelController>;

let latestResult: ControllerResult | null = null;

const isToolItem = (
  item: ControllerResult["listItems"][number],
): item is Extract<ControllerResult["listItems"][number], { kind: "tool" }> => item.kind === "tool";

function Harness(props: ControllerProps) {
  latestResult = useWorkbenchThreadViewModelController(props);
  return null;
}

afterEach(() => {
  cleanup();
  latestResult = null;
});

const turns: SessionTurn[] = [
  {
    turn_id: "turn-1",
    session_id: "session-1",
    run_id: null,
    user_message_id: "message-1",
    status: "completed",
    start_seq: 1,
    end_seq: 2,
    started_at: "2025-12-15T00:00:00.000Z",
    updated_at: "2025-12-15T00:00:01.000Z",
    assistant_partial: "",
    thought_partial: "",
    metrics_json: null,
    tool_total: 1,
    tool_pending: 0,
    tool_running: 0,
    tool_completed: 1,
    tool_failed: 0,
  },
];

const messages = [
  {
    id: "message-1",
    session_id: "session-1",
    task_id: "task-1",
    turn_id: "turn-1",
    turn_sequence: 1,
    role: "user",
    content: "Run the check",
    attachments: [],
    delivery: "immediate",
    created_at: "2025-12-15T00:00:00.000Z",
    order_seq: 1,
  },
 ] as unknown as Message[];

const events: SessionEvent[] = [
  {
    seq: 1,
    id: "event-1",
    session_id: "session-1",
    run_id: "run-1",
    turn_id: "turn-1",
    event_type: "tool_call",
    payload_json: {
      tool_call_id: "tool-1",
      order_seq: 2,
    },
    created_at: "2025-12-15T00:00:00.500Z",
  },
];

const toolsByTurnId: Record<string, SessionTurnTool[]> = {
  "turn-1": [
    {
      session_id: "session-1",
      tool_call_id: "tool-1",
      turn_id: "turn-1",
      tool_kind: "shell",
      title: "pnpm test",
      status: "completed",
      input_json: { command: "pnpm test" },
      output_text: "ok",
      input_truncated: false,
      input_original_bytes: 9,
      output_truncated: false,
      output_original_bytes: 2,
      created_at: "2025-12-15T00:00:00.500Z",
      updated_at: "2025-12-15T00:00:01.000Z",
    },
  ],
};

function renderController(
  overrides: Partial<ControllerProps> = {},
): ReturnType<typeof render> {
  const baseProps: ControllerProps = {
    sessionId: "session-1",
    turnsKey: deriveTurnsKey(turns),
    messagesKey: deriveMessagesKey(messages),
    eventsKey: "1:1",
    verbosity: "default",
    turns,
    messages,
    events,
    toolsByTurnId,
    toolSummariesReady: true,
    askUserQuestionAnswers: new Map<string, AskUserQuestionAnswerState>(),
    enableDebugEvents: false,
  };
  return render(<Harness {...baseProps} {...overrides} />);
}

describe("useWorkbenchThreadViewModelController", () => {
  it("rebuilds when tool summaries become ready without transcript key changes", async () => {
    const { rerender } = renderController({ toolSummariesReady: false });

    await waitFor(() => {
      const tool = latestResult?.listItems.find(isToolItem);
      expect(tool?.title).not.toBe("pnpm test");
    });

    rerender(
      <Harness
        sessionId="session-1"
        turnsKey={deriveTurnsKey(turns)}
        messagesKey={deriveMessagesKey(messages)}
        eventsKey="1:1"
        verbosity="default"
        turns={turns}
        messages={messages}
        events={events}
        toolsByTurnId={toolsByTurnId}
        toolSummariesReady
        askUserQuestionAnswers={new Map<string, AskUserQuestionAnswerState>()}
        enableDebugEvents={false}
      />,
    );

    await waitFor(() => {
      const tool = latestResult?.listItems.find(isToolItem);
      expect(tool?.title).toBe("pnpm test");
    });
  });

  it("rebuilds filtered thread items when verbosity changes without transcript key changes", async () => {
    const { rerender } = renderController();

    await waitFor(() => {
      expect(latestResult?.listItems.some(isToolItem)).toBe(true);
    });

    rerender(
      <Harness
        sessionId="session-1"
        turnsKey={deriveTurnsKey(turns)}
        messagesKey={deriveMessagesKey(messages)}
        eventsKey="1:1"
        verbosity="terse"
        turns={turns}
        messages={messages}
        events={events}
        toolsByTurnId={toolsByTurnId}
        toolSummariesReady
        askUserQuestionAnswers={new Map<string, AskUserQuestionAnswerState>()}
        enableDebugEvents={false}
      />,
    );

    await waitFor(() => {
      expect(latestResult?.listItems.some(isToolItem)).toBe(false);
    });
  });
});
