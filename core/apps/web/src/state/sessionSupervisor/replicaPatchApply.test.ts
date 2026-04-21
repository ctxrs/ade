import { describe, expect, it, vi } from "vitest";
import { applyReplicaPatches } from "./replicaPatchApply";
import { applyToolSummaries } from "./headProjection";
import { createInternalEntry } from "./entryState";
import type { SessionSupervisorReplicaPatchHost } from "./replicaPatchApply";
import type { SessionSupervisorHeadProjectionHost } from "./headProjection";

function createReplicaHost(entry: ReturnType<typeof createInternalEntry>): SessionSupervisorReplicaPatchHost {
  return {
    workspaceSnapshotState: null,
    ensureEntry: () => entry,
    resolveSessionMode: () => entry.mode ?? null,
    resetEntryProjectionForReplace: () => undefined,
    setSessionLoadState: () => undefined,
    setFatalError: () => undefined,
    applyAcpMetaFromEvents: () => false,
    applyGitStatusSnapshotFromEvents: () => false,
    syncStateCache: () => undefined,
    clearSupportLoadError: () => undefined,
    adoptLoadedSubagentInvocationsRevision: () => undefined,
    ensureProviderOptions: async () => undefined,
    ensureSubagentInvocations: async () => undefined,
    syncSupportLoadsForOpenSession: () => undefined,
    bumpTurnsRev: () => undefined,
  };
}

function createHeadHost(): SessionSupervisorHeadProjectionHost {
  return {
    workspaceSnapshotState: null,
    workspaceSessionHeadsById: new Map(),
    stateCacheBySessionId: new Map(),
    publish: vi.fn(),
    mergeTurns: () => undefined,
    mergeEvents: () => undefined,
    mergeMessages: () => undefined,
    applyAcpMeta: () => false,
    applyAcpMetaFromEvents: () => false,
    applyGitStatusSnapshotFromEvents: () => false,
    ensureProviderOptions: async () => undefined,
    resolveSessionMode: () => "active",
    setSessionLoadState: () => undefined,
    syncSupportLoadsForOpenSession: () => undefined,
    ensureThoughtCache: async () => undefined,
    adoptLoadedSubagentInvocationsRevision: () => undefined,
    clearSupportLoadError: () => undefined,
    bumpTurnsRev: () => undefined,
    bumpMessagesRev: () => undefined,
    bumpEventsRev: () => undefined,
  };
}

describe("replicaPatchApply", () => {
  it("does not mark the entry changed for no-op canonical transcript patches", () => {
    const entry = createInternalEntry("session-1", { transientSeqStart: 1, warmTtlMs: 60_000 });
    entry.turnsHydrated = true;
    entry.turns = [];
    entry.messages = [];
    entry.events = [];
    entry.assistantStreamingByTurnId = {};
    entry.turnsRev = 2;
    entry.messagesRev = 3;
    entry.eventsRev = 4;
    entry.assistantStreamingRev = 5;

    const host = createReplicaHost(entry);
    const result = applyReplicaPatches(host, [{
      sessionId: "session-1",
      op: "append",
      data: {
        appendMode: "metadata_update",
        turns: entry.turns,
        messages: entry.messages,
        events: entry.events,
        assistantStreamingByTurnId: entry.assistantStreamingByTurnId,
      },
    }]);

    expect(result.changed).toBe(false);
    expect(entry.turnsRev).toBe(2);
    expect(entry.messagesRev).toBe(3);
    expect(entry.eventsRev).toBe(4);
    expect(entry.assistantStreamingRev).toBe(5);
  });

  it("does not republish duplicate tool summaries", () => {
    const entry = createInternalEntry("session-1", { transientSeqStart: 1, warmTtlMs: 60_000 });
    const summaries = [{
      session_id: "session-1",
      tool_call_id: "tool-1",
      turn_id: "turn-1",
      tool_kind: "execute",
      provider_tool_name: "Bash",
      title: "Run pwd",
      subtitle: null,
      status: "running",
      input_preview: { command: "pwd" },
      order_seq: 1,
      input_truncated: null,
      input_original_bytes: null,
      output_truncated: null,
      output_original_bytes: null,
      first_event_seq: 1,
      created_at: "2026-04-08T12:00:00.000Z",
      updated_at: "2026-04-08T12:00:00.000Z",
    }];
    entry.toolSummaries = summaries;
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
        summary_only: true,
      } as typeof entry.support.turnToolsByTurnId[string][number] & { summary_only: boolean }],
    };
    entry.support.turnToolsHydratedByTurnId = { "turn-1": false };

    const host = createHeadHost();
    applyToolSummaries.call(host, entry, summaries);

    expect(host.publish).not.toHaveBeenCalled();
  });

  it("preserves a prior local user-message anchor when a replace patch points the turn at a missing user message", () => {
    const entry = createInternalEntry("session-1", { transientSeqStart: 1, warmTtlMs: 60_000 });
    entry.turnsHydrated = true;
    entry.turns = [{
      turn_id: "turn-1",
      session_id: "session-1",
      run_id: null,
      user_message_id: "message-local-user",
      status: "running",
      start_seq: 1,
      end_seq: null,
      started_at: "2026-04-14T00:00:00.000Z",
      updated_at: "2026-04-14T00:00:00.000Z",
      assistant_partial: null,
      thought_partial: null,
      metrics_json: null,
      tool_total: 0,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 0,
      tool_failed: 0,
    }];
    entry.messages = [{
      id: "message-local-user",
      session_id: "session-1",
      task_id: "task-1",
      turn_id: "turn-1",
      role: "user",
      content: "optimistic first message",
      delivery: "immediate",
      created_at: "2026-04-14T00:00:00.000Z",
    }];
    entry.messagesRev = 1;
    entry.turnsRev = 1;

    const host = createReplicaHost(entry);
    const result = applyReplicaPatches(host, [{
      sessionId: "session-1",
      op: "replace",
      data: {
        freshness: "authoritative",
        turns: [{
          ...entry.turns[0]!,
          user_message_id: "message-server-missing",
          status: "completed",
          end_seq: 2,
          updated_at: "2026-04-14T00:00:05.000Z",
        }],
        messages: [{
          id: "message-assistant",
          session_id: "session-1",
          task_id: "task-1",
          turn_id: "turn-1",
          role: "assistant",
          content: "done: optimistic first message",
          delivery: "immediate",
          created_at: "2026-04-14T00:00:05.000Z",
        }],
        events: [],
        turnsHydrated: true,
        loading: false,
      },
    }]);

    expect(result.changed).toBe(true);
    expect(entry.turns[0]?.user_message_id).toBe("message-local-user");
    expect(entry.messages.map((message) => message.id)).toEqual([
      "message-local-user",
      "message-assistant",
    ]);
  });
});
