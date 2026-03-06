const readText = (value) => {
  if (typeof value === "string") return value.trim();
  if (value instanceof Error) return String(value.message || value).trim();
  return String(value || "").trim();
};

const classifyFirstTurnFailure = (error) => {
  const detail = readText(error).toLowerCase();
  return detail.includes("timed out") ? "timed_out" : "failed";
};

const defaultRunFirstTurn = async (workspaceId, options, timeoutMs) => {
  const { runProviderFirstTurnApiSmoke } = require("./workspace_wizard_flow.cjs");
  return runProviderFirstTurnApiSmoke(workspaceId, options, timeoutMs);
};

const defaultCollectDiagnostics = async (workspaceId) => {
  const { collectCodexSmokeDiagnostics } = require("./workspace_wizard_flow.cjs");
  return collectCodexSmokeDiagnostics(workspaceId);
};

const runDeterministicFirstTurnOutcome = async (
  workspaceId,
  {
    providerId = "codex",
    modelId = "default",
    prompt = "hello",
    timeoutMs = 180_000,
    runFirstTurn = defaultRunFirstTurn,
    collectDiagnostics = defaultCollectDiagnostics,
  } = {},
) => {
  try {
    const result = await runFirstTurn(
      workspaceId,
      {
        providerId,
        modelId,
        prompt,
      },
      timeoutMs,
    );
    const assistantMessage = readText(result?.assistantMessage);
    return {
      status: "success",
      task_id: readText(result?.taskId) || null,
      session_id: readText(result?.sessionId) || null,
      assistant_preview: assistantMessage.slice(0, 200),
    };
  } catch (error) {
    const detail = readText(error);
    const outcome = {
      status: classifyFirstTurnFailure(detail),
      stage: "turn",
      detail,
      task_id: null,
      session_id: null,
    };
    try {
      const diagnostics = await collectDiagnostics(workspaceId);
      outcome.diagnostics = diagnostics;
      outcome.task_id = readText(diagnostics?.taskId) || null;
      outcome.session_id = readText(diagnostics?.sessionId) || null;
    } catch (diagnosticsError) {
      outcome.diagnostics_error = readText(diagnosticsError);
    }
    return outcome;
  }
};

module.exports = {
  classifyFirstTurnFailure,
  runDeterministicFirstTurnOutcome,
};
