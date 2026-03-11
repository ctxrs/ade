const test = require("node:test");
const assert = require("node:assert/strict");

const {
  providerAuthMatrixScenarioTags,
} = require("./specs/helpers/provider_auth_matrix_scenarios.cjs");

test("codex local container endpoint cells include the local codex smoke alias", () => {
  const tags = providerAuthMatrixScenarioTags({
    cellId: "codex.endpoint_api_key.local.container_host_mounted",
    providerId: "codex",
    authMode: "endpoint_api_key",
    daemonLocation: "local",
    executionEnvironment: "container_host_mounted",
  });

  assert(tags.includes("provider-auth-matrix"));
  assert(tags.includes("local-codex-smoke"));
  assert(tags.includes("local_container_host_mounted"));
  assert.equal(tags.includes("local-codex-host-smoke"), false);
  assert(tags.includes("endpoint_api_key"));
});

test("codex local host required cells use the host-only codex smoke alias", () => {
  const tags = providerAuthMatrixScenarioTags({
    cellId: "codex.configure_later_then_connect.local.host",
    providerId: "codex",
    authMode: "configure_later_then_connect",
    daemonLocation: "local",
    executionEnvironment: "host",
  });

  assert(tags.includes("provider-auth-matrix"));
  assert(tags.includes("local-codex-host-smoke"));
  assert.equal(tags.includes("local-codex-smoke"), false);
  assert(tags.includes("configure_later_then_connect"));
});

test("auth import cells include the provider auth import alias", () => {
  const tags = providerAuthMatrixScenarioTags({
    cellId: "codex.auth_import.local.container_host_mounted",
    providerId: "codex",
    authMode: "auth_import",
    daemonLocation: "local",
    executionEnvironment: "container_host_mounted",
  });

  assert(tags.includes("provider-auth-matrix"));
  assert(tags.includes("provider-auth-import"));
  assert.equal(tags.includes("local-codex-smoke"), false);
  assert.equal(tags.includes("local-codex-host-smoke"), false);
});

test("non-codex cells do not inherit codex-only smoke aliases", () => {
  const tags = providerAuthMatrixScenarioTags({
    cellId: "gemini.endpoint_api_key.local.container_host_mounted",
    providerId: "gemini",
    authMode: "endpoint_api_key",
    daemonLocation: "local",
    executionEnvironment: "container_host_mounted",
  });

  assert(tags.includes("provider-auth-matrix"));
  assert.equal(tags.includes("local-codex-smoke"), false);
  assert.equal(tags.includes("local-codex-host-smoke"), false);
});
