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
  assert.match(
    ctxHttpBuild,
    /name = "unit_tests_execution_setup_concurrent_launch_start_is_deduplicated"/,
  );
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_startup_prewarm_runtime_warmup"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_refresh_clears_stale_prewarm_metadata"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_reuses_active_runtime_prewarm"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_startup_prewarm_runtime_probe_reuse"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_runtime_launch_ready_promotion"/);
  assert.match(
    ctxHttpBuild,
    /name = "unit_tests_execution_setup_runtime_launch_ready_scope_starts_shared_vm"/,
  );
  assert.match(ctxHttpBuild, /name = "unit_tests_execution_setup_builder_prewarm_shared_all_job"/);
  assert.match(
    ctxHttpBuild,
    /name = "unit_tests_execution_setup_workspace_launch_not_blocked_by_background_runtime_prewarm"/,
  );
  assert.match(
    ctxHttpBuild,
    /name = "unit_tests_execution_setup_successful_workspace_launch_writes_missing_prewarm_metadata"/,
  );
  assert.match(ctxHttpBuild, /name = "unit_tests_installer"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_lib"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_lib_execution_launch_startup_prewarm_kind_supported"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_merge_queue"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_merge_queue_enabled_workspace_resume_after_open"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_provider_launch"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_provider_matrix"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_scheduler"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_settings"/);
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
  assert.match(ctxHttpBuild, /name = "unit_tests_workspace_launch_reuses_startup_prewarm"/);
  assert.match(ctxHttpBuild, /"lib_tests::"/);
  assert.match(ctxHttpBuild, /"execution_setup::"/);
  assert.match(ctxHttpBuild, /"workspace_runtime::"/);
  assert.match(ctxHttpBuild, /"--test-threads=1"/);
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::concurrent_launch_start_is_deduplicated/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::concurrent_launch_start_is_deduplicated/,
  );
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
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::startup_prewarm_reuses_initial_runtime_probe_for_gate_checks/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::startup_prewarm_reuses_initial_runtime_probe_for_gate_checks/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::runtime_prewarm_launch_ready_request_reuses_running_runtime_job_and_promotes_scope/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::runtime_prewarm_launch_ready_request_reuses_running_runtime_job_and_promotes_scope/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::runtime_prewarm_launch_ready_scope_starts_shared_vm_and_reports_launch_ready/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::runtime_prewarm_launch_ready_scope_starts_shared_vm_and_reports_launch_ready/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::builder_prewarm_reuses_background_all_job/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::builder_prewarm_reuses_background_all_job/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::workspace_launch_is_not_blocked_by_background_runtime_prewarm_job/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::workspace_launch_is_not_blocked_by_background_runtime_prewarm_job/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::successful_workspace_launch_writes_missing_prewarm_metadata/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::successful_workspace_launch_writes_missing_prewarm_metadata/,
  );
  assert.match(ctxHttpBuild, /--skip=lib_tests::daemon_smoke::daemon_golden_path_with_fake_provider/);
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"lib_tests::daemon_smoke::daemon_golden_path_with_fake_provider/,
  );
  assert.match(ctxHttpBuild, /--skip=lib_tests::daemon_smoke::daemon_http_and_ws_streaming/);
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"lib_tests::daemon_smoke::daemon_http_and_ws_streaming/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=lib_tests::execution_launch::execution_launch_startup_prewarm_kind_supported/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"lib_tests::execution_launch::execution_launch_startup_prewarm_kind_supported/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=execution_setup::tests::workspace_launch_reuses_startup_prewarm_without_second_image_load_when_machine_is_ready/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"execution_setup::tests::workspace_launch_reuses_startup_prewarm_without_second_image_load_when_machine_is_ready/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=workspace_runtime::tests::prepare_reuses_running_workspace_container_without_front_loading_image_readiness/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"workspace_runtime::tests::prepare_reuses_running_workspace_container_without_front_loading_image_readiness/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=workspace_runtime::tests::maybe_reclaim_sandbox_machine_stops_idle_machine/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"workspace_runtime::tests::maybe_reclaim_sandbox_machine_stops_idle_machine/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=workspace_runtime::tests::maybe_reclaim_sandbox_machine_stops_idle_runtime_with_running_workspace_containers/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"workspace_runtime::tests::maybe_reclaim_sandbox_machine_stops_idle_runtime_with_running_workspace_containers/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=workspace_runtime::tests::ensure_sandbox_machine_materialized_defers_reconfiguration_when_machine_is_running_but_engine_unreachable/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"workspace_runtime::tests::ensure_sandbox_machine_materialized_defers_reconfiguration_when_machine_is_running_but_engine_unreachable/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=workspace_runtime::tests::ensure_sandbox_machine_materialized_recreates_machine_for_memory_profile_change_when_engine_is_down/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"workspace_runtime::tests::ensure_sandbox_machine_materialized_recreates_machine_for_memory_profile_change_when_engine_is_down/,
  );
  assert.match(
    ctxHttpBuild,
    /--skip=merge_queue::tests::enabled_workspace_queued_rows_resume_only_after_open/,
  );
  assert.match(
    ctxHttpBuild,
    /--exact",\s*"merge_queue::tests::enabled_workspace_queued_rows_resume_only_after_open/,
  );
  assert.match(ctxHttpBuild, /name = "bin_tests"/);
  assert.match(ctxHttpBuild, /name = "doc_tests"/);
  assert.match(ctxHttpBuild, /name = "base"/);
  assert.match(
    ctxHttpBuild,
    /":unit_tests_execution_setup_concurrent_launch_start_is_deduplicated"/,
  );
  assert.match(
    ctxHttpBuild,
    /":unit_tests_execution_setup_startup_prewarm_runtime_probe_reuse"/,
  );
  assert.match(
    ctxHttpBuild,
    /":unit_tests_execution_setup_runtime_launch_ready_promotion"/,
  );
  assert.match(
    ctxHttpBuild,
    /":unit_tests_execution_setup_runtime_launch_ready_scope_starts_shared_vm"/,
  );
  assert.match(
    ctxHttpBuild,
    /":unit_tests_execution_setup_builder_prewarm_shared_all_job"/,
  );
  assert.match(
    ctxHttpBuild,
    /":unit_tests_execution_setup_workspace_launch_not_blocked_by_background_runtime_prewarm"/,
  );
  assert.match(
    ctxHttpBuild,
    /":unit_tests_execution_setup_successful_workspace_launch_writes_missing_prewarm_metadata"/,
  );
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
  assert.match(ctxHttpBazelTests, /"provider_current_ctx_version_regressions"/);
  assert.match(
    ctxHttpBazelTests,
    /"workspace_active_snapshot_http_workspace_stream_under_load_no_gap_or_reset"/,
  );
  assert.match(
    ctxHttpBazelTests,
    /"message_idempotency_post_message_idempotent_conflict_on_change"/,
  );
  assert.match(ctxHttpBazelTests, /"buffers_http_e2e"/);
  assert.match(
    ctxHttpBazelTests,
    /"global_id_routing_http_message_delete_route_is_session_scoped"/,
  );
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
