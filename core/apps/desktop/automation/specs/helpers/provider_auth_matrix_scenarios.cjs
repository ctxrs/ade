const normalizeText = (value) => String(value || "").trim();

const providerAuthMatrixScenarioTags = ({
  cellId = "",
  providerId = "",
  authMode = "",
  envTarget = "",
} = {}) => {
  const tags = new Set([
    "local",
    "provider",
    "matrix",
    "provider-auth-matrix",
    normalizeText(cellId),
    normalizeText(providerId),
    normalizeText(authMode),
    normalizeText(envTarget),
  ]);

  if (normalizeText(authMode) === "auth_import") {
    tags.add("provider-auth-import");
  }

  if (
    normalizeText(providerId) === "codex"
    && normalizeText(envTarget) === "local_container"
    && (
      normalizeText(authMode) === "endpoint_api_key"
      || normalizeText(authMode) === "configure_later_then_connect"
    )
  ) {
    tags.add("local-codex-smoke");
  }

  return Array.from(tags).filter(Boolean);
};

module.exports = {
  providerAuthMatrixScenarioTags,
};
