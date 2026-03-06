const assert = require("node:assert/strict");
const test = require("node:test");

const {
  classifyFirstTurnFailure,
  runDeterministicFirstTurnOutcome,
} = require("./specs/helpers/first_turn_contract.cjs");

test("classifyFirstTurnFailure marks timeout messages explicitly", () => {
  assert.equal(classifyFirstTurnFailure("codex first turn timed out (session=abc)"), "timed_out");
  assert.equal(classifyFirstTurnFailure("codex first turn failed (status=failed)"), "failed");
});

test("runDeterministicFirstTurnOutcome returns structured success payload", async () => {
  const outcome = await runDeterministicFirstTurnOutcome("workspace-1", {
    providerId: "codex",
    modelId: "openrouter/openai/gpt-5.2-codex",
    prompt: "hello",
    runFirstTurn: async () => ({
      taskId: "task-123",
      sessionId: "session-456",
      assistantMessage: "pong".repeat(80),
    }),
    collectDiagnostics: async () => {
      throw new Error("should not collect diagnostics on success");
    },
  });

  assert.deepEqual(outcome, {
    status: "success",
    task_id: "task-123",
    session_id: "session-456",
    assistant_preview: "pong".repeat(50),
  });
});

test("runDeterministicFirstTurnOutcome captures diagnostics on failure", async () => {
  const diagnostics = {
    taskId: "task-123",
    sessionId: "session-456",
    sessionEvents: [{ event_type: "error" }],
  };
  const outcome = await runDeterministicFirstTurnOutcome("workspace-1", {
    runFirstTurn: async () => {
      throw new Error("codex first turn timed out (session=session-456)");
    },
    collectDiagnostics: async () => diagnostics,
  });

  assert.equal(outcome.status, "timed_out");
  assert.equal(outcome.stage, "turn");
  assert.equal(outcome.task_id, "task-123");
  assert.equal(outcome.session_id, "session-456");
  assert.equal(outcome.detail, "codex first turn timed out (session=session-456)");
  assert.deepEqual(outcome.diagnostics, diagnostics);
});

test("runDeterministicFirstTurnOutcome preserves the original failure when diagnostics also fail", async () => {
  const outcome = await runDeterministicFirstTurnOutcome("workspace-1", {
    runFirstTurn: async () => {
      throw new Error("codex first turn failed (status=failed)");
    },
    collectDiagnostics: async () => {
      throw new Error("diagnostics unavailable");
    },
  });

  assert.equal(outcome.status, "failed");
  assert.equal(outcome.detail, "codex first turn failed (status=failed)");
  assert.equal(outcome.diagnostics_error, "diagnostics unavailable");
});
