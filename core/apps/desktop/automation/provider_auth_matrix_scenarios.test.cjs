const test = require("node:test");
const assert = require("node:assert/strict");

const {
  providerAuthMatrixScenarioTags,
} = require("./specs/helpers/provider_auth_matrix_scenarios.cjs");

test("codex local container endpoint cells include the local codex smoke alias", () => {
  const tags = providerAuthMatrixScenarioTags({
    cellId: "codex.endpoint_api_key.local_container",
    providerId: "codex",
    authMode: "endpoint_api_key",
    envTarget: "local_container",
  });

  assert(tags.includes("provider-auth-matrix"));
  assert(tags.includes("local-codex-smoke"));
  assert(tags.includes("endpoint_api_key"));
});

test("auth import cells include the provider auth import alias", () => {
  const tags = providerAuthMatrixScenarioTags({
    cellId: "codex.auth_import.local_container",
    providerId: "codex",
    authMode: "auth_import",
    envTarget: "local_container",
  });

  assert(tags.includes("provider-auth-matrix"));
  assert(tags.includes("provider-auth-import"));
  assert.equal(tags.includes("local-codex-smoke"), false);
});

test("non-codex cells do not inherit codex-only smoke aliases", () => {
  const tags = providerAuthMatrixScenarioTags({
    cellId: "gemini.endpoint_api_key.local_container",
    providerId: "gemini",
    authMode: "endpoint_api_key",
    envTarget: "local_container",
  });

  assert(tags.includes("provider-auth-matrix"));
  assert.equal(tags.includes("local-codex-smoke"), false);
});
