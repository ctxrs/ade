const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITES,
  MANUAL_ONLY_CTX_HTTP_TEST_FILES,
  buildCtxHttpSuiteCommands,
  getCtxHttpSuiteNames,
  getCtxHttpSuiteTaskName,
  validateCtxHttpSuites,
} = require("./ctx_http_suites.cjs");

const coreRoot = path.resolve(__dirname, "../..");

test("ctx-http suite assignments cover every integration test exactly once", () => {
  const validation = validateCtxHttpSuites(coreRoot);

  assert.deepEqual(validation, {
    duplicates: [],
    manualOnly: ["cloud_gateway_azure_e2e", "cloud_gateway_gcp_e2e"],
    missing: [],
    unknown: [],
  });
});

test("ctx-http suite names include the meta all task and stable suite task names", () => {
  const names = getCtxHttpSuiteNames({ includeAll: true });

  assert.deepEqual(names, [
    "base",
    "workspace-stream",
    "provider-auth",
    "provider-runtime",
    "repo-vcs",
    "lsp",
    "turns-terminal",
    "artifacts-updates",
    "sandbox-cloud",
    "all",
  ]);
  assert.equal(getCtxHttpSuiteTaskName("workspace-stream"), "rust:ctx-http:test:workspace-stream");
});

test("ctx-http suite command builder expands base and meta suites predictably", () => {
  assert.deepEqual(buildCtxHttpSuiteCommands("base"), [
    {
      args: ["test", "-q", "-p", "ctx-http", "--lib", "--bins"],
      command: "cargo",
    },
    {
      args: ["test", "-q", "-p", "ctx-http", "--doc"],
      command: "cargo",
    },
  ]);

  const allCommands = buildCtxHttpSuiteCommands("all");
  assert.equal(allCommands.length > 10, true);
  assert.deepEqual(allCommands[0], {
    args: ["test", "-q", "-p", "ctx-http", "--lib", "--bins"],
    command: "cargo",
  });
  assert.equal(
    allCommands.some((command) => command.args.includes("cloud_gateway_azure_e2e")),
    false,
  );
  assert.equal(
    allCommands.some((command) => command.args.includes("cloud_gateway_gcp_e2e")),
    false,
  );
});

test("ctx-http integration suites declare source ownership and dependency crates", () => {
  assert.equal(CTX_HTTP_SHARED_SOURCE_GLOBS.includes("crates/ctx-http/src/daemon/**"), true);
  assert.deepEqual([...MANUAL_ONLY_CTX_HTTP_TEST_FILES].sort(), [
    "cloud_gateway_azure_e2e",
    "cloud_gateway_gcp_e2e",
  ]);

  for (const suite of CTX_HTTP_SUITES) {
    assert.equal(Array.isArray(suite.dependencyCrates), true);
    assert.equal(Array.isArray(suite.sourceGlobs), true);
    if (suite.type === "integration") {
      assert.equal(suite.sourceGlobs.length > 0, true, `${suite.name} should own source globs`);
    }
  }
});
