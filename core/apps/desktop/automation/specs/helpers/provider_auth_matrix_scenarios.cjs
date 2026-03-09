const normalizeText = (value) => String(value || "").trim();

const providerAuthMatrixScenarioTags = ({
  cellId = "",
  providerId = "",
  authMode = "",
  envTarget = "",
} = {}) => {
  const normalizedProviderId = normalizeText(providerId);
  const normalizedAuthMode = normalizeText(authMode);
  const normalizedEnvTarget = normalizeText(envTarget);
  const tags = new Set([
    "local",
    "provider",
    "matrix",
    "provider-auth-matrix",
    normalizeText(cellId),
    normalizedProviderId,
    normalizedAuthMode,
    normalizedEnvTarget,
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
    if (normalizedEnvTarget === "local_container") {
      tags.add("local-codex-smoke");
    }
    if (normalizedEnvTarget === "local_host") {
      tags.add("local-codex-host-smoke");
    }
  }

  return Array.from(tags).filter(Boolean);
};

module.exports = {
  providerAuthMatrixScenarioTags,
};
