const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  CTX_HTTP_BAZEL_PACKAGE,
  CTX_HTTP_BASE_CHILD_SUITE_NAMES,
  CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS,
  CTX_HTTP_SAFE_CONCURRENT_SUITES,
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITES,
  CTX_HTTP_SUITE_CONCURRENCY_CLASSES,
  CTX_HTTP_SUITE_SCRIPT_INPUTS,
  MANUAL_ONLY_CTX_HTTP_TEST_FILES,
  buildCtxHttpSuiteCommands,
  buildCtxHttpSuiteTaskArgs,
  getCtxHttpSuiteConcurrencyClass,
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
  assert.equal(names.includes("unit-tests-api"), true);
  assert.equal(names.includes("unit-tests-lib"), true);
  assert.equal(names.includes("unit-tests-lib-session-head-large"), true);
  assert.equal(names.includes("bin-tests"), true);
  assert.equal(names.includes("doc-tests"), true);
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
  assert.deepEqual(
    allCommands,
    [
      {
        args: ["scripts/run_bazel_pilot.cjs", "test", ...getCtxHttpSuiteTargets("all")],
        command: "node",
      },
    ],
  );
  assert.equal(new Set(getCtxHttpSuiteTargets("all")).size, getCtxHttpSuiteTargets("all").length);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:unit-tests-api`), true);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:bin_tests`), true);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:doc_tests`), true);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:base`), false);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:attachments-routing`), false);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:workspace-stream`), false);
  assert.equal(getCtxHttpSuiteTargets("all").includes(`${CTX_HTTP_BAZEL_PACKAGE}:repo-vcs`), false);
  assert.deepEqual(CTX_HTTP_BASE_CHILD_SUITE_NAMES, [
    "unit-tests-api",
    "unit-tests-execution-setup",
    "unit-tests-lib",
    "unit-tests-lib-session-head-large",
    "unit-tests-workspace-runtime",
    "unit-tests-daemon-and-scheduler",
    "unit-tests-provider-and-settings",
    "unit-tests-merge-queue",
    "bin-tests",
    "doc-tests",
  ]);
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

test("ctx-http long suites expand to direct Bazel test targets", () => {
  assert.deepEqual(buildCtxHttpSuiteCommands("attachments-routing"), [
    {
      args: [
        "scripts/run_bazel_pilot.cjs",
        "test",
        "//core/crates/ctx-http:global_id_routing_http",
        "//core/crates/ctx-http:image_attachments_http_e2e",
        "//core/crates/ctx-http:workspace_attachments_local_canonical",
      ],
      command: "node",
    },
  ]);
  assert.deepEqual(getCtxHttpSuiteTargets("workspace-stream"), [
    `${CTX_HTTP_BAZEL_PACKAGE}:cache_rehydration`,
    `${CTX_HTTP_BAZEL_PACKAGE}:fault_matrix`,
    `${CTX_HTTP_BAZEL_PACKAGE}:hot_endpoints_no_db`,
    `${CTX_HTTP_BAZEL_PACKAGE}:replay_properties`,
    `${CTX_HTTP_BAZEL_PACKAGE}:task_default_session_http`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http_workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http_workspace_stream_subscribe_does_not_reemit_when_worktree_vcs_is_already_computing`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_active_snapshot_http_worktree_vcs_summary_refresh_reloads_live_inventory_before_ready_publish`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_stream_context_window_metrics`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_stream_no_gaps_under_activity`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_stream_stress_active_heads_lag`,
  ]);
  assert.deepEqual(getCtxHttpSuiteTargets("repo-vcs"), [
    `${CTX_HTTP_BAZEL_PACKAGE}:jj_merge_queue_basics`,
    `${CTX_HTTP_BAZEL_PACKAGE}:merge_queue_isolation`,
    `${CTX_HTTP_BAZEL_PACKAGE}:repo_clone_branch_and_safety`,
    `${CTX_HTTP_BAZEL_PACKAGE}:repo_init_initial_commit`,
    `${CTX_HTTP_BAZEL_PACKAGE}:repo_validate_destination`,
    `${CTX_HTTP_BAZEL_PACKAGE}:session_diff_unavailable`,
    `${CTX_HTTP_BAZEL_PACKAGE}:workspace_merge_queue_config_http`,
    `${CTX_HTTP_BAZEL_PACKAGE}:worktree_archive_http`,
    `${CTX_HTTP_BAZEL_PACKAGE}:worktree_vcs_snapshot`,
  ]);
});

test("ctx-http suite command builder accepts explicit multi-suite selections", () => {
  assert.deepEqual(buildCtxHttpSuiteCommands(["base", "provider-auth"]), [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:base", "//core/crates/ctx-http:provider-auth"],
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

test("ctx-http suite concurrency metadata classifies every concrete suite", () => {
  const validClasses = new Set(Object.values(CTX_HTTP_SUITE_CONCURRENCY_CLASSES));
  for (const suite of CTX_HTTP_SUITES) {
    assert.equal(validClasses.has(getCtxHttpSuiteConcurrencyClass(suite.name)), true, suite.name);
  }
  for (const suiteName of CTX_HTTP_SAFE_CONCURRENT_SUITES) {
    assert.equal(getCtxHttpSuiteConcurrencyClass(suiteName), CTX_HTTP_SUITE_CONCURRENCY_CLASSES.SAFE);
  }
  assert.equal(getCtxHttpSuiteConcurrencyClass("unit-tests-api"), CTX_HTTP_SUITE_CONCURRENCY_CLASSES.SERIALIZED);
  assert.equal(getCtxHttpSuiteConcurrencyClass("base"), CTX_HTTP_SUITE_CONCURRENCY_CLASSES.SERIALIZED);
  assert.throws(() => getCtxHttpSuiteConcurrencyClass("all"), /unknown ctx-http suite/u);
});
