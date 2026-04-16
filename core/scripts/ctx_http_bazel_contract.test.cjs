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
const ctxHttpBuild = fs.readFileSync(path.join(coreRoot, "crates", "ctx-http", "BUILD.bazel"), "utf8");
const ctxHttpBazelTests = fs.readFileSync(
  path.join(coreRoot, "crates", "ctx-http", "ctx_http_bazel_tests.bzl"),
  "utf8",
);

test("ctx-http BUILD exposes Bazel-native base test targets", () => {
  assert.match(ctxHttpBuild, /rust_doc_test/);
  assert.match(ctxHttpBuild, /name = "lib_test_support"/);
  assert.match(ctxHttpBuild, /name = "unit_tests"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_daemon_golden_path_with_fake_provider"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_daemon_http_and_ws_streaming"/);
  assert.match(ctxHttpBuild, /name = "unit_tests_workspace_launch_reuses_startup_prewarm"/);
  assert.match(ctxHttpBuild, /--skip=lib_tests::daemon_golden_path_with_fake_provider/);
  assert.match(ctxHttpBuild, /--exact",\s*"lib_tests::daemon_golden_path_with_fake_provider/);
  assert.match(ctxHttpBuild, /--skip=lib_tests::daemon_http_and_ws_streaming/);
  assert.match(ctxHttpBuild, /--exact",\s*"lib_tests::daemon_http_and_ws_streaming/);
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
  assert.match(ctxHttpBuild, /declare_ctx_http_integration_tests/);
  assert.match(ctxHttpBuild, /CARGO_PKG_VERSION": "0\.58\.0"/);
});

test("ctx-http Bazel helper keeps quick-path and manual-only suites explicit", () => {
  assert.match(ctxHttpBazelTests, /CTX_HTTP_SUITE_ORDER = \[/);
  assert.match(ctxHttpBazelTests, /CTX_HTTP_MANUAL_ONLY_TESTS = \[/);
  assert.match(ctxHttpBazelTests, /CTX_HTTP_BAZEL_MANUAL_ONLY_TARGET = "manual-only"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_manifest_parse"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_checksum_mismatch"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_missing_artifact"/);
  assert.match(ctxHttpBazelTests, /"updates_failure_safety_interrupted_transfer"/);
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
