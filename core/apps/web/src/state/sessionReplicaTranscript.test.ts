import { describe, expect, it } from "vitest";
import type { SessionEvent, SessionTurn } from "../api/client";
import {
  applyReplicaTranscriptEvent,
  type SessionReplicaTranscriptEntry,
} from "./sessionReplicaTranscript";

const mkTurn = (status: SessionTurn["status"]): SessionTurn => ({
  turn_id: "turn-1",
  session_id: "session-1",
  run_id: "run-1",
  user_message_id: "message-1",
  status,
  start_seq: 1,
  end_seq: null,
  started_at: new Date(1).toISOString(),
  updated_at: new Date(1).toISOString(),
  assistant_partial: "",
  thought_partial: "",
  metrics_json: null,
  tool_total: 0,
  tool_pending: 0,
  tool_running: 0,
  tool_completed: 0,
  tool_failed: 0,
});

const mkEvent = (
  seq: number,
  event_type: SessionEvent["event_type"],
  payload_json: Record<string, unknown>,
): SessionEvent => ({
  seq,
  id: `event-${seq}`,
  session_id: "session-1",
  run_id: "run-1",
  turn_id: "turn-1",
  event_type,
  payload_json,
  created_at: new Date(seq).toISOString(),
});

const mkEntry = (): SessionReplicaTranscriptEntry => ({
  sessionId: "session-1",
  turns: [mkTurn("running")],
  turnsRev: 0,
  assistantStreamingByTurnId: {},
  assistantStreamingRev: 0,
  messages: [],
  messagesRev: 0,
  events: [],
  eventsRev: 0,
  toolSummaries: [],
  nextTransientSeq: -1,
  startedTurnIds: new Set(["turn-1"]),
  toolStatusByKey: new Map(),
  toolIdsByTurn: new Map(),
});

describe("sessionReplicaTranscript", () => {
  it("promotes failed turns to interrupted when turn_interrupted follows cancel fallout", () => {
    const entry = mkEntry();

    applyReplicaTranscriptEvent(entry, mkEvent(2, "error", { message: "cancelled" }));
    expect(entry.turns[0]?.status).toBe("failed");

    applyReplicaTranscriptEvent(entry, mkEvent(3, "turn_interrupted", { reason: "user_interrupt" }));
    expect(entry.turns[0]?.status).toBe("interrupted");
  });
});
