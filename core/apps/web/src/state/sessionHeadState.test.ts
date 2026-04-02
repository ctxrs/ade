import { describe, expect, it } from "vitest";
import type { SessionEvent, SessionHeadSnapshot, SessionTurn, SessionTurnToolSummary } from "../api/client";
import {
  mergeSessionMessages,
  mergeSessionToolSummaries,
  mergeSessionTurns,
  sanitizeSessionHeadSnapshot,
} from "./sessionHeadState";

describe("sessionHeadState", () => {
  it("sanitizes partial head content while preserving final thought events", () => {
    const head: SessionHeadSnapshot = {
      session: {
        id: "session-1",
        task_id: "task-1",
        workspace_id: "ws-1",
        worktree_id: "wt-1",
        provider_id: "codex",
        model_id: "gpt-5",
        title: "Session",
        agent_role: "implementer",
        status: "active",
        created_at: "2026-03-09T00:00:00.000Z",
        updated_at: "2026-03-09T00:00:00.000Z",
      },
      turns: [
        {
          turn_id: "turn-1",
          session_id: "session-1",
          run_id: null,
          user_message_id: "user-1",
          status: "running",
          start_seq: 1,
          end_seq: null,
          started_at: "2026-03-09T00:00:00.000Z",
          updated_at: "2026-03-09T00:00:00.000Z",
          assistant_partial: "partial assistant",
          thought_partial: "partial thought",
          metrics_json: null,
          tool_total: 0,
          tool_pending: 0,
          tool_running: 0,
          tool_completed: 0,
          tool_failed: 0,
        },
      ],
      tool_summaries: [],
      events: [
        {
          seq: 2,
          id: "assistant-chunk",
          session_id: "session-1",
          turn_id: "turn-1",
          event_type: "assistant_chunk",
          payload_json: { content_fragment: "partial" },
          created_at: "2026-03-09T00:00:01.000Z",
        },
        {
          seq: 3,
          id: "thought-partial",
          session_id: "session-1",
          turn_id: "turn-1",
          event_type: "thought_chunk",
          payload_json: { content_fragment: "partial thought" },
          created_at: "2026-03-09T00:00:02.000Z",
        },
        {
          seq: 4,
          id: "assistant-complete",
          session_id: "session-1",
          turn_id: "turn-1",
          event_type: "assistant_complete",
          payload_json: { full_content: "final answer", message_id: "provider-msg-1", order_seq: 2 },
          created_at: "2026-03-09T00:00:02.500Z",
        },
        {
          seq: 5,
          id: "thought-final",
          session_id: "session-1",
          turn_id: "turn-1",
          event_type: "thought_chunk",
          payload_json: { is_final: true, full_content: "final thought" },
          created_at: "2026-03-09T00:00:03.000Z",
        },
      ] as SessionEvent[],
      messages: [],
      last_event_seq: 5,
      state_rev: 0,
      activity: { is_working: true },
      has_more_turns: false,
      history_cursor: null,
      has_more_history: false,
    };

    const sanitized = sanitizeSessionHeadSnapshot(head);
    expect(sanitized.turns[0]?.assistant_partial).toBeNull();
    expect(sanitized.turns[0]?.thought_partial).toBeNull();
    expect((sanitized.events ?? []).map((event) => event.id)).toEqual(["thought-final"]);
  });

  it("keeps tool summaries aligned to the visible head turns", () => {
    const turns: SessionTurn[] = [
      {
        turn_id: "turn-2",
        session_id: "session-1",
        run_id: null,
        user_message_id: "user-2",
        status: "completed",
        start_seq: 10,
        end_seq: 12,
        started_at: "2026-03-09T00:00:10.000Z",
        updated_at: "2026-03-09T00:00:12.000Z",
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
    const previous: SessionTurnToolSummary[] = [
      {
        session_id: "session-1",
        tool_call_id: "tool-old",
        turn_id: "turn-1",
        status: "completed",
        order_seq: 10,
        created_at: "2026-03-09T00:00:01.000Z",
        updated_at: "2026-03-09T00:00:02.000Z",
      },
    ];
    const incoming: SessionTurnToolSummary[] = [
      {
        session_id: "session-1",
        tool_call_id: "tool-new",
        turn_id: "turn-2",
        status: "completed",
        order_seq: 11,
        created_at: "2026-03-09T00:00:11.000Z",
        updated_at: "2026-03-09T00:00:12.000Z",
      },
    ];

    expect(mergeSessionToolSummaries(previous, incoming, turns).map((summary) => summary.tool_call_id)).toEqual([
      "tool-new",
    ]);
  });

  it("shares message and turn merge ordering across snapshot consumers", () => {
    const mergedTurns = mergeSessionTurns(
      [
        {
          turn_id: "turn-1",
          session_id: "session-1",
          run_id: null,
          user_message_id: "user-1",
          status: "running",
          start_seq: 1,
          end_seq: null,
          started_at: "2026-03-09T00:00:00.000Z",
          updated_at: "2026-03-09T00:00:00.000Z",
          assistant_partial: "hel",
          thought_partial: "",
          metrics_json: null,
          tool_total: 0,
          tool_pending: 0,
          tool_running: 0,
          tool_completed: 0,
          tool_failed: 0,
        },
      ],
      [
        {
          turn_id: "turn-1",
          session_id: "session-1",
          run_id: null,
          user_message_id: "user-1",
          status: "running",
          start_seq: 1,
          end_seq: null,
          started_at: "2026-03-09T00:00:00.000Z",
          updated_at: "2026-03-09T00:00:01.000Z",
          assistant_partial: "hello",
          thought_partial: "",
          metrics_json: null,
          tool_total: 0,
          tool_pending: 0,
          tool_running: 0,
          tool_completed: 0,
          tool_failed: 0,
        },
      ],
    );
    const mergedMessages = mergeSessionMessages(
      [
        {
          id: "message-2",
          session_id: "session-1",
          task_id: "task-1",
          role: "assistant",
          content: "second",
          delivery: "immediate",
          created_at: "2026-03-09T00:00:02.000Z",
          turn_sequence: 2,
        },
      ],
      [
        {
          id: "message-1",
          session_id: "session-1",
          task_id: "task-1",
          role: "user",
          content: "first",
          delivery: "immediate",
          created_at: "2026-03-09T00:00:01.000Z",
          turn_sequence: 1,
        },
      ],
    );

    expect(mergedTurns[0]?.assistant_partial).toBe("hello");
    expect(mergedMessages.map((message) => message.id)).toEqual(["message-1", "message-2"]);
  });
});
