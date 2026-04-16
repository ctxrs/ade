const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const assert = require("node:assert/strict");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptText = fs.readFileSync(path.join(repoRoot, "core", "scripts", "test_updates_failure_safety.sh"), "utf8");
const buildText = fs.readFileSync(path.join(repoRoot, "core", "crates", "ctx-http", "BUILD.bazel"), "utf8");
const suiteText = fs.readFileSync(
  path.join(repoRoot, "core", "crates", "ctx-http", "ctx_http_bazel_tests.bzl"),
  "utf8",
);

test("updates failure safety helper uses Bazel targets", () => {
  assert.match(scriptText, /ROOT_DIR="\$\(cd "\$\(dirname "\$\{BASH_SOURCE\[0\]\}"\)\/\.\." && pwd\)"/);
  assert.match(scriptText, /cd "\$ROOT_DIR"/);
  assert.match(scriptText, /node scripts\/run_bazel_pilot\.cjs test/);
  for (const target of [
    "updates_failure_safety_manifest_parse",
    "updates_failure_safety_checksum_mismatch",
    "updates_failure_safety_missing_artifact",
    "updates_failure_safety_interrupted_transfer",
  ]) {
    assert.match(scriptText, new RegExp(`//core/crates/ctx-http:${target}`));
  }
});

test("ctx-http Bazel helper exports updater failure safety Bazel tests", () => {
  assert.match(buildText, /declare_ctx_http_integration_tests/);
  for (const target of [
    "updates_failure_safety_manifest_parse",
    "updates_failure_safety_checksum_mismatch",
    "updates_failure_safety_missing_artifact",
    "updates_failure_safety_interrupted_transfer",
  ]) {
    assert.match(suiteText, new RegExp(`"${target}"`));
  }
});
