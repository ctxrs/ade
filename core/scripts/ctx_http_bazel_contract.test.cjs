const test = require("node:test");
const assert = require("node:assert/strict");
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

test("ctx-http BUILD exposes Bazel-native base test targets", () => {
  assert.match(ctxHttpBuild, /rust_doc_test/);
  assert.match(ctxHttpBuild, /name = "lib_test_support"/);
  assert.match(ctxHttpBuild, /name = "unit_tests"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_api"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_daemon"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_startup_prewarm_runtime_warmup"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_refresh_clears_stale_prewarm_metadata"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_reuses_active_runtime_prewarm"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_installer"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_lib"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_lib_execution_launch_startup_prewarm_kind_supported"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_merge_queue"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_provider_launch"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_provider_matrix"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_scheduler"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_settings"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_workspace_runtime"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_daemon_golden_path_with_fake_provider"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_daemon_http_and_ws_streaming"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_workspace_launch_reuses_startup_prewarm"/);
  assert.match(ctxHttpBuild, /"lib_tests::"/);
  assert.match(ctxHttpBuild, /"execution_setup::"/);
  assert.match(ctxHttpBuild, /"workspace_runtime::"/);
  assert.match(ctxHttpBuild, /"--test-threads=1"/);
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::startup_prewarm_runs_runtime_warmup_for_cold_container_settings/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::startup_prewarm_runs_runtime_warmup_for_cold_container_settings/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::successful_workspace_launch_refresh_clears_stale_prewarm_metadata/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::successful_workspace_launch_refresh_clears_stale_prewarm_metadata/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::workspace_launch_reuses_active_runtime_prewarm_without_second_image_load/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::workspace_launch_reuses_active_runtime_prewarm_without_second_image_load/,
  );
  assert.match(ctxHttpBuild, /--skip=lib_tests::daemon_golden_path_with_fake_provider/);
  assert.match(ctxHttpBuild, /--exact",\s*"lib_tests::daemon_golden_path_with_fake_provider/);
  assert.match(ctxHttpBuild, /--skip=lib_tests::daemon_http_and_ws_streaming/);
  assert.match(ctxHttpBuild, /--exact",\s*"lib_tests::daemon_http_and_ws_streaming/);
  assert.match(ctxHttpBuild, /--skip=lib_tests::execution_launch_startup_prewarm_kind_supported/);
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"lib_tests::execution_launch_startup_prewarm_kind_supported/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::workspace_launch_reuses_startup_prewarm_without_second_image_load_when_machine_is_ready/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::workspace_launch_reuses_startup_prewarm_without_second_image_load_when_machine_is_ready/,
  );
  assert.match(ctxHttpBuild, /name = "bin_tests"/);
  assert.match(ctxHttpBuild, /name = "doc_tests"/);
  assert.match(ctxHttpBuild, /name = "base"/);
  assert.match(ctxHttpBuild, /name = "ctx-http-lsp-test-server"/);
  assert.match(ctxHttpBuild, /name = "llama_server_mock"/);
  assert.match(ctxHttpBuild, /CTX_HTTP_TEST_CORPUS_DATA = glob\(\["tests\/corpus\/\*\*"\]\)/);
  assert.match(ctxHttpBuild, /CTX_HTTP_TEST_FIXTURE_DATA = glob\(\["tests\/fixtures\/\*\*"\]\)/);
  assert.match(
    ctxHttpBuild,
    /CTX_HTTP_INTEGRATION_TEST_DATA = CTX_HTTP_COMPILE_DATA \+ CTX_HTTP_TEST_CORPUS_DATA \+ CTX_HTTP_TEST_FIXTURE_DATA/,
  );
  assert.match(ctxHttpBuild, /declare_ctx_http_integration_tests/);
  assert.match(ctxHttpBuild, new RegExp(`CARGO_PKG_VERSION": "${escapeRegExp(desktopVersion)}"`));
});

test("ctx-http Bazel helper keeps quick-path and manual-only suites explicit", () => {
  assert.match(ctxHttpBazelTests, /CTX_HTTP_SUITE_ORDER = \[/);
  assert.match(ctxHttpBazelTests, /CTX_HTTP_MANUAL_ONLY_TESTS = \[/);
  assert.match(ctxHttpBazelTests, /CTX_HTTP_BAZEL_MANUAL_ONLY_TARGET = "manual-only"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_manifest_parse"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_checksum_mismatch"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_missing_artifact"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_interrupted_transfer"/);
  assert.match(
    ctxHttpBazelTests,
    /"workspace_active_snapshot_http_workspace_stream_under_load_no_gap_or_reset"/,
  );
  assert.match(
    ctxHttpBazelTests,
    /"message_idempotency_post_message_idempotent_conflict_on_change"/,
  );
  assert.match(
    ctxHttpBazelTests,
    /"global_id_routing_http_message_delete_route_is_session_scoped"/,
  );
  assert.match(ctxHttpBazelTests, /CARGO_BIN_EXE_ctx-http-lsp-test-server/);
  assert.match(ctxHttpBazelTests, /CARGO_BIN_EXE_llama_server_mock/);
  assert.match(ctxHttpBazelTests, /CARGO_BIN_EXE_ctx/);
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
