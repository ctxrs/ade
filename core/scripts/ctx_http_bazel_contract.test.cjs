const test = require("node:test");
const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const {
  CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS,
  getCtxHttpSuiteTargets,
} = require("./lib/ctx_http_suites.cjs");
const { getBazelTestTargetsForCrates } = require("./lib/bazel_rust_targets.cjs");

const coreRoot = path.resolve(__dirname, "..");
const desktopVersion = JSON.parse(
  fs.readFileSync(path.join(coreRoot, "apps", "desktop", "package.json"), "utf8"),
).version;
const ctxHttpBuild = fs.readFileSync(path.join(coreRoot, "crates", "ctx-http", "BUILD.bazel"), "utf8");
const ctxHttpBazelTests = fs.readFileSync(
  path.join(coreRoot, "crates", "ctx-http", "ctx_http_bazel_tests.bzl"),
  "utf8",
);
const escapeRegExp = (value) => String(value || "").replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

function extractBuildStringList(name, seen = new Set()) {
  assert.equal(seen.has(name), false, `cyclic BUILD list alias while resolving ${name}`);
  seen.add(name);
  const match = ctxHttpBuild.match(new RegExp(`${name}\\s*=\\s*(?:[^\\[]*\\+\\s*)?\\[([\\s\\S]*?)\\]`));
  if (match) {
    return [...match[1].matchAll(/"([^"]+)"/g)].map((entry) => entry[1]);
  }
  const alias = ctxHttpBuild.match(new RegExp(`${name}\\s*=\\s*([A-Z0-9_]+)\\s*$`, "m"));
  if (alias) {
    return extractBuildStringList(alias[1], seen);
  }
  assert.fail(`expected ${name} list in ctx-http BUILD`);
}

function extractCtxHttpSuiteTestList(suiteName) {
  const match = ctxHttpBazelTests.match(new RegExp(`"${escapeRegExp(suiteName)}"\\s*:\\s*\\[([\\s\\S]*?)\\],`));
  assert.ok(match, `expected ${suiteName} suite in ctx_http_bazel_tests.bzl`);
  return [...match[1].matchAll(/"([^"]+)"/g)].map((entry) => entry[1]);
}

function extractTestSuiteLabels(suiteName) {
  const match = ctxHttpBuild.match(
    new RegExp(`test_suite\\(\\s*name = "${escapeRegExp(suiteName)}",[\\s\\S]*?tests = \\[([\\s\\S]*?)\\],\\s*\\)`),
  );
  assert.ok(match, `expected ${suiteName} test_suite in ctx-http BUILD`);
  return [...match[1].matchAll(/"([^"]+)"/g)].map((entry) => entry[1]);
}

function runUnitWrapperScript(args) {
  return childProcess.spawnSync("bash", [
    path.join(coreRoot, "crates", "ctx-http", "tests", "run_unit_test_harness.sh"),
    ...args,
  ], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function assertNoDuplicates(name, values) {
  const duplicates = values.filter((value, index) => values.indexOf(value) !== index);
  assert.deepEqual([...new Set(duplicates)].sort(), [], `${name} must not contain duplicate labels`);
}

test("ctx-http BUILD exposes Bazel-native base test targets", () => {
  assert.match(ctxHttpBuild, /rust_doc_test/);
  assert.match(ctxHttpBuild, /name = "lib_test_support"/);
  assert.doesNotMatch(ctxHttpBuild, /load\("@rules_rust\/\/rust:defs\.bzl"[^)]*"rust_test"/);
  assert.match(ctxHttpBuild, /rust_test = declare_ctx_http_filtered_unit_test/);
  assert.match(ctxHttpBuild, /declare_ctx_http_unit_test_harness\(/);
  assert.match(ctxHttpBazelTests, /CTX_HTTP_UNIT_TEST_HARNESS_NAME = "ctx_http_unit_test_harness"/);
  assert.match(ctxHttpBazelTests, /tags = \["manual"\]/);
  assert.match(ctxHttpBazelTests, /args = \["--list"\]/);
  assert.match(ctxHttpBuild, /name = "unit_tests"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_api"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_daemon"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_lib"/);
  assert.match(
    ctxHttpBazelTests,
    /sh_test\([\s\S]*name = name[\s\S]*"\$\(rootpath :\{\}\)"[\s\S]*CTX_HTTP_UNIT_TEST_HARNESS_NAME[\s\S]*data = \[":\{\}"\.format\(CTX_HTTP_UNIT_TEST_HARNESS_NAME\)\]/,
  );
  assert.match(ctxHttpBuild, /name = "unit_tests_lib_execution_launch_startup_prewarm_kind_supported"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_merge_queue"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_merge_queue_enabled_workspace_resume_after_open"/);
  assert.match(ctxHttpBuild, /"\/\/core\/crates\/ctx-managed-installs:unit_tests"/);
  assert.match(ctxHttpBuild, /"\/\/core\/crates\/ctx-provider-matrix:unit_tests"/);
  assert.match(ctxHttpBuild, /"\/\/core\/crates\/ctx-provider-runtime:unit_tests"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_scheduler"/);
  assert.match(ctxHttpBuild, /"\/\/core\/crates\/ctx-settings-model:unit_tests"/);
  assert.match(ctxHttpBuild, /"\/\/core\/crates\/ctx-settings-service:unit_tests"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_workspace_runtime"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_workspace_runtime_reuses_running_container"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_workspace_runtime_reclaim_idle_machine"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_workspace_runtime_reclaim_idle_runtime_with_containers"/);
  assert.match(
    ctxHttpBuild,
    /name = "unit_tests_workspace_runtime_running_unreachable_machine_reconfiguration"/,
  );
  assert.match(
    ctxHttpBuild,
    /name = "unit_tests_workspace_runtime_recreates_machine_for_memory_profile_change"/,
  );
  assert.match(ctxHttpBuild, /name = "unit_tests_daemon_golden_path_with_fake_provider"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_daemon_http_and_ws_streaming"/);
  assert.match(ctxHttpBuild, /"lib_tests::"/);
  assert.match(ctxHttpBuild, /"daemon::workspace_runtime::"/);
  assert.match(ctxHttpBuild, /"--test-threads=1"/);
  assert.match(ctxHttpBuild, /--skip=lib_tests::daemon_smoke::golden_path::daemon_golden_path_with_fake_provider/);
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"lib_tests::daemon_smoke::golden_path::daemon_golden_path_with_fake_provider/,
  );
  assert.match(ctxHttpBuild, /--skip=lib_tests::daemon_smoke::streaming::daemon_http_and_ws_streaming/);
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"lib_tests::daemon_smoke::streaming::daemon_http_and_ws_streaming/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=lib_tests::execution_launch::startup_prewarm::execution_launch_startup_prewarm_kind_supported/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"lib_tests::execution_launch::startup_prewarm::execution_launch_startup_prewarm_kind_supported/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=daemon::workspace_runtime::tests::runtime_prepare::prepare_reuses_running_workspace_container_without_front_loading_image_readiness/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"daemon::workspace_runtime::tests::runtime_prepare::prepare_reuses_running_workspace_container_without_front_loading_image_readiness/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=daemon::workspace_runtime::tests::reclaim::maybe_reclaim_sandbox_machine_stops_idle_machine/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"daemon::workspace_runtime::tests::reclaim::maybe_reclaim_sandbox_machine_stops_idle_machine/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=daemon::workspace_runtime::tests::reclaim::maybe_reclaim_sandbox_machine_stops_idle_runtime_with_running_workspace_containers/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"daemon::workspace_runtime::tests::reclaim::maybe_reclaim_sandbox_machine_stops_idle_runtime_with_running_workspace_containers/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=daemon::workspace_runtime::tests::machine_recovery::materialization::ensure_sandbox_machine_materialized_defers_reconfiguration_when_machine_is_running_but_engine_unreachable/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"daemon::workspace_runtime::tests::machine_recovery::materialization::ensure_sandbox_machine_materialized_defers_reconfiguration_when_machine_is_running_but_engine_unreachable/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=daemon::workspace_runtime::tests::machine_recovery::materialization::ensure_sandbox_machine_materialized_recreates_machine_for_memory_profile_change_when_engine_is_down/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"daemon::workspace_runtime::tests::machine_recovery::materialization::ensure_sandbox_machine_materialized_recreates_machine_for_memory_profile_change_when_engine_is_down/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=daemon::merge_queue::tests::resume::enabled_workspace_queued_rows_resume_only_after_open/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"daemon::merge_queue::tests::resume::enabled_workspace_queued_rows_resume_only_after_open/,
  );
  assert.match(ctxHttpBuild, /name = "bin_tests"/);
  assert.match(ctxHttpBuild, /name = "bin_tests_root_help"/);
  assert.match(ctxHttpBuild, /args = \["\$\(location :ctx\)", "root-help"\]/);
  assert.match(ctxHttpBuild, /name = "bin_tests_serve_help"/);
  assert.match(ctxHttpBuild, /args = \["\$\(location :ctx\)", "serve-help"\]/);
  assert.match(ctxHttpBuild, /name = "bin_tests_init_help"/);
  assert.match(ctxHttpBuild, /args = \["\$\(location :ctx\)", "init-help"\]/);
  assert.match(ctxHttpBuild, /name = "bin_tests_self_update_help"/);
  assert.match(ctxHttpBuild, /args = \["\$\(location :ctx\)", "self-update-help"\]/);
  assert.match(ctxHttpBuild, /name = "doc_tests"/);
  assert.match(ctxHttpBuild, /name = "base"/);
  assert.match(ctxHttpBuild, /name = "unit-tests-api"/);
  assert.match(ctxHttpBuild, /name = "unit-tests-daemon-and-scheduler"/);
  assert.match(ctxHttpBuild, /name = "unit-tests-lib"/);
  assert.match(ctxHttpBuild, /name = "unit-tests-lib-session-head-large"/);
  assert.match(ctxHttpBuild, /name = "unit-tests-merge-queue"/);
  assert.match(ctxHttpBuild, /name = "unit-tests-provider-and-settings"/);
  assert.match(ctxHttpBuild, /name = "unit-tests-workspace-runtime"/);
  assert.match(
    ctxHttpBuild,
    /":unit_tests_merge_queue_enabled_workspace_resume_after_open"/,
  );
  assert.match(
    ctxHttpBuild,
    /":unit_tests_workspace_runtime_recreates_machine_for_memory_profile_change"/,
  );
  assert.match(
    ctxHttpBuild,
    /test_suite\([\s\S]*name = "base"[\s\S]*":unit_tests"[\s\S]*":bin_tests"[\s\S]*":doc_tests"/,
  );
  assert.match(ctxHttpBuild, /name = "llama_server_mock"/);
  assert.match(ctxHttpBuild, /CTX_HTTP_TEST_CORPUS_DATA = glob\(\["tests\/corpus\/\*\*"\]\)/);
  assert.match(ctxHttpBuild, /CTX_HTTP_TEST_FIXTURE_DATA = glob\(\["tests\/fixtures\/\*\*"\]\)/);
  assert.match(
    ctxHttpBuild,
    /CTX_HTTP_INTEGRATION_TEST_DATA = CTX_HTTP_COMPILE_DATA \+ CTX_HTTP_TEST_CORPUS_DATA \+ CTX_HTTP_TEST_FIXTURE_DATA/,
  );
  assert.match(ctxHttpBuild, /CTX_HTTP_INTEGRATION_BINARY_DATA = \{/);
  assert.match(ctxHttpBuild, /"provider_scenarios_offline": \["\/\/core\/crates\/ctx-mcp:ctx-mcp"\]/);
  assert.match(ctxHttpBuild, /"noisy_output_backpressure": \["\/\/core\/crates\/ctx-mcp:ctx-mcp"\]/);
  assert.match(ctxHttpBuild, /"session_model_api": \["\/\/core\/crates\/ctx-mcp:ctx-mcp"\]/);
  assert.match(ctxHttpBuild, /"title_generation_local": \[":llama_server_mock"\]/);
  assert.match(ctxHttpBuild, /"CARGO_BIN_EXE_ctx-mcp": "\$\(rootpath \/\/core\/crates\/ctx-mcp:ctx-mcp\)"/);
  assert.match(ctxHttpBuild, /"CARGO_BIN_EXE_llama_server_mock": "\$\(rootpath :llama_server_mock\)"/);
  assert.match(ctxHttpBuild, /declare_ctx_http_integration_tests/);
  assert.match(ctxHttpBuild, new RegExp(`CARGO_PKG_VERSION": "${escapeRegExp(desktopVersion)}"`));
});

test("ctx-http unit-family suites cover every unit test target exactly once", () => {
  const unitFamilySuites = [
    "unit-tests-api",
    "unit-tests-daemon-and-scheduler",
    "unit-tests-lib",
    "unit-tests-lib-session-head-large",
    "unit-tests-merge-queue",
    "unit-tests-org-policy",
    "unit-tests-provider-and-settings",
    "unit-tests-storage-admission",
    "unit-tests-workspace-runtime",
  ];
  const declaredUnitTargets = [...ctxHttpBuild.matchAll(/name = "(unit_tests_[^"]+)"/g)]
    .map((match) => `:${match[1]}`)
    .sort();
  const familyTargets = unitFamilySuites
    .flatMap((suiteName) => extractTestSuiteLabels(suiteName))
    .sort();
  const localFamilyTargets = familyTargets.filter((target) => target.startsWith(":"));

  assertNoDuplicates("ctx-http unit-family suite targets", familyTargets);
  assert.equal(familyTargets.includes(":ctx_http_unit_test_harness"), false);
  assert.equal(familyTargets.includes("//core/crates/ctx-settings-model:unit_tests"), true);
  assert.equal(familyTargets.includes("//core/crates/ctx-settings-service:unit_tests"), true);
  assert.deepEqual(localFamilyTargets, declaredUnitTargets);
  assert.deepEqual(
    extractTestSuiteLabels("unit_tests").sort(),
    unitFamilySuites.map((suiteName) => `:${suiteName}`).sort(),
  );
});

test("ctx-http shared unit harness is excluded from public suite routing", () => {
  assert.equal(extractTestSuiteLabels("unit_tests").includes(":ctx_http_unit_test_harness"), false);
  assert.equal(extractTestSuiteLabels("base").includes(":ctx_http_unit_test_harness"), false);
  assert.equal(getCtxHttpSuiteTargets("all").includes("//core/crates/ctx-http:ctx_http_unit_test_harness"), false);
});

test("ctx-http unit harness wrapper fails clearly for invalid invocation", () => {
  const noArgs = runUnitWrapperScript([]);
  assert.equal(noArgs.status, 2);
  assert.match(noArgs.stderr, /usage: run_unit_test_harness\.sh <harness-runfile>/);

  const missingHarness = runUnitWrapperScript(["missing/harness"]);
  assert.equal(missingHarness.status, 1);
  assert.match(missingHarness.stderr, /failed to locate executable ctx-http unit test harness: missing\/harness/);

  const unsupportedFilter = childProcess.spawnSync("bash", [
    path.join(coreRoot, "crates", "ctx-http", "tests", "run_unit_test_harness.sh"),
    "missing/harness",
  ], {
    encoding: "utf8",
    env: {
      ...process.env,
      TESTBRIDGE_TEST_ONLY: "lib_tests::provider_routes::",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  assert.equal(unsupportedFilter.status, 2);
  assert.match(unsupportedFilter.stderr, /Bazel --test_filter is not supported/);
});

test("ctx-http Bazel dependency buckets do not duplicate labels", () => {
  const sharedExternal = extractBuildStringList("CTX_HTTP_SHARED_EXTERNAL_DEPS");
  const directTests = extractBuildStringList("CTX_HTTP_DIRECT_TEST_DEPS");
  const integrationDirectTests = extractBuildStringList("CTX_HTTP_INTEGRATION_DIRECT_TEST_DEPS");
  const integrationDirectTestsAliasDirect = /^CTX_HTTP_INTEGRATION_DIRECT_TEST_DEPS = CTX_HTTP_DIRECT_TEST_DEPS$/m
    .test(ctxHttpBuild);

  assertNoDuplicates("CTX_HTTP_SHARED_EXTERNAL_DEPS", sharedExternal);
  assertNoDuplicates("CTX_HTTP_DIRECT_TEST_DEPS", directTests);
  assertNoDuplicates("CTX_HTTP_INTEGRATION_DIRECT_TEST_DEPS", integrationDirectTests);

  if (integrationDirectTestsAliasDirect) {
    assert.deepEqual(integrationDirectTests, directTests);
  } else {
    const directTestLabels = new Set(directTests);
    const repeatedIntegrationLabels = integrationDirectTests.filter((label) => directTestLabels.has(label));
    assert.deepEqual(
      repeatedIntegrationLabels,
      [],
      "CTX_HTTP_INTEGRATION_DIRECT_TEST_DEPS is appended to CTX_HTTP_DIRECT_TEST_DEPS and must only list additional labels",
    );
  }
});

test("ctx-http Bazel helper keeps quick-path and manual-only suites explicit", () => {
  assert.match(ctxHttpBazelTests, /CTX_HTTP_SUITE_ORDER = \[/);
  assert.match(ctxHttpBazelTests, /CTX_HTTP_MANUAL_ONLY_TESTS = \[/);
  assert.match(ctxHttpBazelTests, /CTX_HTTP_BAZEL_MANUAL_ONLY_TARGET = "manual-only"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_manifest_parse"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_checksum_mismatch"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_missing_artifact"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_interrupted_transfer"/);
  assert.match(ctxHttpBazelTests, /"provider_current_ctx_version_regressions"/);
  const workspaceStreamTargets = extractCtxHttpSuiteTestList("workspace-stream");
  assert.equal(workspaceStreamTargets.includes("workspace_active_snapshot_http"), true);
  assert.deepEqual(
    workspaceStreamTargets.filter((target) => target.startsWith("workspace_active_snapshot_http_")),
    [
      "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state",
      "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs",
      "workspace_active_snapshot_http_workspace_stream_subscribe_does_not_reemit_when_worktree_vcs_is_already_computing",
      "workspace_active_snapshot_http_worktree_vcs_summary_refresh_reloads_live_inventory_before_ready_publish",
    ],
  );
  assert.match(
    ctxHttpBazelTests,
    /"workspace_active_snapshot_http": \{[\s\S]*"--skip",[\s\S]*"workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs"[\s\S]*"timeout": "long"/,
  );
  assert.match(
    ctxHttpBazelTests,
    /"message_idempotency_post_message_idempotent_conflict_on_change"/,
  );
  assert.match(
    ctxHttpBazelTests,
    /"global_id_routing_http_message_delete_route_is_session_scoped"/,
  );
  assert.match(ctxHttpBazelTests, /binary_data\.get\(source_name, \[\]\)/);
  assert.match(ctxHttpBazelTests, /binary_rustc_env\.get\(source_name, \{\}\)/);
  assert.doesNotMatch(ctxHttpBazelTests, /merged\["CARGO_BIN_EXE_ctx-mcp"\]/);
});

test("ctx-http Bazel rust mapping covers the non-manual suite labels only", () => {
  assert.deepEqual(
    getBazelTestTargetsForCrates(["ctx-http"]),
    [...getCtxHttpSuiteTargets("all")].sort(),
  );
  assert.equal(
    getBazelTestTargetsForCrates(["ctx-http"]).some((target) => CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS.includes(target)),
    false,
  );
});
