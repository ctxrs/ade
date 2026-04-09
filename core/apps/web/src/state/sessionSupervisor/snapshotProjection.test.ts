import { describe, expect, it } from "vitest";
import { publish } from "./snapshotProjection";
import { createInternalEntry, type SessionSupervisorSnapshot } from "./entryState";

describe("sessionSupervisor snapshotProjection", () => {
  it("projects support-owned tool state into threadProjection", () => {
    const entry = createInternalEntry("session-1", { transientSeqStart: 1, warmTtlMs: 60_000 });
    entry.turns = [{
      turn_id: "turn-1",
      session_id: "session-1",
      run_id: null,
      user_message_id: "message-1",
      status: "running",
      start_seq: 1,
      end_seq: null,
      started_at: "2026-04-08T12:00:00.000Z",
      updated_at: "2026-04-08T12:00:00.000Z",
      assistant_partial: "",
      thought_partial: "",
      metrics_json: null,
      tool_total: 1,
      tool_pending: 0,
      tool_running: 1,
      tool_completed: 0,
      tool_failed: 0,
    }];
    entry.messages = [];
    entry.events = [];
    entry.turnsRev = 2;
    entry.messagesRev = 3;
    entry.eventsRev = 4;
    entry.assistantStreamingRev = 5;
    entry.projectionRev = 6;
    entry.support.stateLoaded = true;
    entry.support.toolSummariesReady = true;
    entry.support.turnToolsByTurnId = {
      "turn-1": [{
        session_id: "session-1",
        tool_call_id: "tool-1",
        turn_id: "turn-1",
        tool_kind: "execute",
        provider_tool_name: "Bash",
        title: "Run pwd",
        subtitle: null,
        status: "running",
        input_json: { command: "pwd" },
        output_text: null,
        order_seq: 1,
        input_truncated: null,
        input_original_bytes: null,
        output_truncated: null,
        output_original_bytes: null,
        first_event_seq: 1,
        created_at: "2026-04-08T12:00:00.000Z",
        updated_at: "2026-04-08T12:00:00.000Z",
      }],
    };

    const host = {
      maxCachedSessions: 10,
      listeners: new Set<() => void>(),
      snapshot: { connection: "idle", sessions: {} } as SessionSupervisorSnapshot,
      entries: new Map([[entry.sessionId, entry]]),
    };

    publish.call(host);

    const projected = host.snapshot.sessions["session-1"]?.threadProjection;
    expect(projected?.loaded).toBe(true);
    expect(projected?.toolSummariesReady).toBe(true);
    expect(projected?.toolsByTurnId).toBe(entry.support.turnToolsByTurnId);
    expect(projected?.toolsByTurnId["turn-1"]?.[0]?.tool_call_id).toBe("tool-1");
    expect(projected?.projectionRev).toBe(6);
  });

  it("reuses the published session entry when transcript-facing inputs are unchanged", () => {
    const entry = createInternalEntry("session-1", { transientSeqStart: 1, warmTtlMs: 60_000 });
    entry.support.stateLoaded = true;
    entry.turns = [];
    entry.messages = [];
    entry.events = [];

    const host = {
      maxCachedSessions: 10,
      listeners: new Set<() => void>(),
      snapshot: { connection: "idle", sessions: {} } as SessionSupervisorSnapshot,
      entries: new Map([[entry.sessionId, entry]]),
    };

    publish.call(host);
    const firstPublished = host.snapshot.sessions["session-1"];

    publish.call(host);
    const secondPublished = host.snapshot.sessions["session-1"];

    expect(secondPublished).toBe(firstPublished);
    expect(secondPublished?.threadProjection).toBe(firstPublished?.threadProjection);
  });
});
