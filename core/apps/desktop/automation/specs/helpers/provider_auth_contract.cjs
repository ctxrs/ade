const fs = require("node:fs");
const path = require("node:path");

const normalizeText = (value) =>
  String(value || "")
    .replace(/\s+/g, " ")
    .trim();

const nowIso = () => new Date().toISOString();

const deriveExecutionTopology = (daemonLocation, executionEnvironment) => {
  const normalizedDaemonLocation = normalizeText(daemonLocation);
  const normalizedExecutionEnvironment = normalizeText(executionEnvironment);
  if (!normalizedDaemonLocation || !normalizedExecutionEnvironment) return "";
  return `${normalizedDaemonLocation}_${normalizedExecutionEnvironment}`;
};

const createProviderAuthContractRecorder = ({
  outputPath,
  cell,
  providerId,
  authMode,
  daemonLocation,
  executionEnvironment,
}) => {
  const startedAt = nowIso();
  const assertions = [];
  const artifacts = {};
  const executionTopology = deriveExecutionTopology(daemonLocation, executionEnvironment);

  const recordAssertion = (name, status, detail = "", payload = null) => {
    assertions.push({
      name: normalizeText(name),
      status: normalizeText(status).toLowerCase() || "unknown",
      detail: normalizeText(detail),
      at: nowIso(),
      payload,
    });
  };

  const recordArtifact = (name, payload) => {
    artifacts[normalizeText(name)] = payload;
  };

  const finalize = ({ result, reason = "", error = "", extras = {} }) => {
    const report = {
      schema_version: 1,
      started_at: startedAt,
      completed_at: nowIso(),
      cell: normalizeText(cell),
      provider_id: normalizeText(providerId),
      auth_mode: normalizeText(authMode),
      daemon_location: normalizeText(daemonLocation),
      execution_environment: normalizeText(executionEnvironment),
      execution_topology: executionTopology,
      result: normalizeText(result).toLowerCase() || "unknown",
      reason: normalizeText(reason),
      error: normalizeText(error),
      assertions,
      artifacts,
      extras,
    };
    if (outputPath) {
      fs.mkdirSync(path.dirname(outputPath), { recursive: true });
      fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
    }
    return report;
  };

  return {
    recordAssertion,
    recordArtifact,
    finalize,
  };
};

module.exports = {
  createProviderAuthContractRecorder,
  deriveExecutionTopology,
  normalizeText,
};
