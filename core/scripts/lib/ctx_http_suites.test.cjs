const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  CTX_HTTP_BAZEL_PACKAGE,
  CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS,
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITES,
  CTX_HTTP_SUITE_SCRIPT_INPUTS,
  MANUAL_ONLY_CTX_HTTP_TEST_FILES,
  buildCtxHttpSuiteCommands,
  buildCtxHttpSuiteTaskArgs,
  getCtxHttpSuiteNames,
  getCtxHttpSuiteTargets,
  getCtxHttpSuiteTaskName,
  validateCtxHttpSuites,
} = require("./ctx_http_suites.cjs");

const coreRoot = path.resolve(__dirname, "../..");
const canonicalSuiteNames = CTX_HTTP_SUITES.map((suite) => suite.name);

test("ctx-http suite assignments cover every integration test exactly once", () => {
  const validation = validateCtxHttpSuites(coreRoot);

  assert.deepEqual(validation, {
    duplicates: [],
    manualOnly: ["attachments_demo_react", "cloud_gateway_azure_e2e", "cloud_gateway_gcp_e2e"],
    missing: [],
    unknown: [],
  });
});

test("ctx-http suite names include the meta all task and stable suite task names", () => {
  const names = getCtxHttpSuiteNames({ includeAll: true });

  assert.deepEqual(names, [...canonicalSuiteNames, "all"]);
  assert.equal(names.includes("provider-runtime-live"), true);
  assert.equal(names.includes("provider-runtime-simulated"), true);
  assert.equal(names.includes("subagents-local-runtime"), true);
  assert.equal(names.includes("sandbox-runtime-container-e2e"), true);
  assert.equal(names.includes("provider-runtime"), false);
  assert.equal(names.includes("sandbox-cloud"), false);
  assert.throws(() => getCtxHttpSuiteTargets("provider-runtime"), /unknown ctx-http suite/u);
  assert.throws(() => buildCtxHttpSuiteCommands("sandbox-cloud"), /unknown ctx-http suite/u);
  assert.equal(getCtxHttpSuiteTaskName("workspace-stream"), "rust:ctx-http:test:workspace-stream");
});

test("ctx-http suite command builder expands base and meta suites predictably", () => {
  assert.deepEqual(buildCtxHttpSuiteCommands("base"), [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:base"],
      command: "node",
    },
  ]);

  const allCommands = buildCtxHttpSuiteCommands("all");
  assert.deepEqual(allCommands, [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", ...getCtxHttpSuiteTargets("all")],
      command: "node",
    },
  ]);
  assert.deepEqual(
    getCtxHttpSuiteTargets("all"),
    canonicalSuiteNames.map((suiteName) => `${CTX_HTTP_BAZEL_PACKAGE}:${suiteName}`),
  );
  assert.equal(new Set(getCtxHttpSuiteTargets("all")).size, canonicalSuiteNames.length);
  assert.deepEqual(CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS, [
    `${CTX_HTTP_BAZEL_PACKAGE}:manual-only`,
  ]);
  assert.equal(
    getCtxHttpSuiteTargets("all").some((target) => CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS.includes(target)),
    false,
  );
  assert.equal(
    CTX_HTTP_SUITE_SCRIPT_INPUTS.includes("crates/ctx-http/BUILD.bazel"),
    true,
  );
  assert.equal(
    CTX_HTTP_SUITE_SCRIPT_INPUTS.includes("crates/ctx-http/ctx_http_bazel_tests.bzl"),
    true,
  );
});

test("ctx-http suite command builder accepts explicit multi-suite selections", () => {
  assert.deepEqual(buildCtxHttpSuiteCommands(["base", "provider-auth"]), [
    {
      args: [
        "scripts/run_bazel_pilot.cjs",
        "test",
        "//core/crates/ctx-http:base",
        "//core/crates/ctx-http:provider-auth",
      ],
      command: "node",
    },
  ]);
  assert.deepEqual(buildCtxHttpSuiteTaskArgs(["base", "provider-auth"]), [
    "--suite",
    "base",
    "--suite",
    "provider-auth",
  ]);
  assert.throws(
    () => getCtxHttpSuiteTargets(["all", "base"]),
    /cannot mix 'all' with explicit suites/u,
  );
});

test("ctx-http integration suites declare source ownership and dependency crates", () => {
  assert.equal(CTX_HTTP_SHARED_SOURCE_GLOBS.includes("crates/ctx-http/src/daemon/**"), true);
  assert.deepEqual([...MANUAL_ONLY_CTX_HTTP_TEST_FILES].sort(), [
    "attachments_demo_react",
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
