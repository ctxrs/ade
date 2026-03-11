const normalizeText = (value) => String(value || "").trim();

const deriveExecutionTopology = (daemonLocation, executionEnvironment) => {
  const normalizedDaemonLocation = normalizeText(daemonLocation);
  const normalizedExecutionEnvironment = normalizeText(executionEnvironment);
  if (!normalizedDaemonLocation || !normalizedExecutionEnvironment) return "";
  return `${normalizedDaemonLocation}_${normalizedExecutionEnvironment}`;
};

const providerAuthMatrixScenarioTags = ({
  cellId = "",
  providerId = "",
  authMode = "",
  daemonLocation = "",
  executionEnvironment = "",
} = {}) => {
  const normalizedProviderId = normalizeText(providerId);
  const normalizedAuthMode = normalizeText(authMode);
  const normalizedDaemonLocation = normalizeText(daemonLocation);
  const normalizedExecutionEnvironment = normalizeText(executionEnvironment);
  const executionTopology = deriveExecutionTopology(normalizedDaemonLocation, normalizedExecutionEnvironment);
  const tags = new Set([
    normalizedDaemonLocation || "local",
    "provider",
    "matrix",
    "provider-auth-matrix",
    normalizeText(cellId),
    normalizedProviderId,
    normalizedAuthMode,
    normalizedDaemonLocation,
    normalizedExecutionEnvironment,
    executionTopology,
  ]);

  if (normalizedAuthMode === "auth_import") {
    tags.add("provider-auth-import");
  }

  if (
    normalizedProviderId === "codex"
    && (
      normalizedAuthMode === "endpoint_api_key"
      || normalizedAuthMode === "configure_later_then_connect"
    )
  ) {
    if (
      normalizedDaemonLocation === "local"
      && normalizedExecutionEnvironment === "container_host_mounted"
    ) {
      tags.add("local-codex-smoke");
    }
    if (
      normalizedDaemonLocation === "local"
      && normalizedExecutionEnvironment === "host"
    ) {
      tags.add("local-codex-host-smoke");
    }
  }

  return Array.from(tags).filter(Boolean);
};

module.exports = {
  deriveExecutionTopology,
  providerAuthMatrixScenarioTags,
};
