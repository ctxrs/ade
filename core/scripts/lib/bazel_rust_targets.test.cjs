const assert = require("node:assert/strict");
const test = require("node:test");

const {
  getBazelBuildTargetsForCrates,
  getBazelCoveredCrates,
  getBazelTestTargetsForCrates,
} = require("./bazel_rust_targets.cjs");

test("Bazel-covered crates stay on the intended explicit Rust slice", () => {
  assert.deepEqual(getBazelCoveredCrates(), [
    "ctx-core",
    "ctx-harness-sources",
    "ctx-provider-accounts",
    "ctx-provider-auth-import",
    "ctx-providers",
  ]);
});

test("Bazel test target mapping expands per-crate tests deterministically", () => {
  assert.deepEqual(getBazelTestTargetsForCrates(["ctx-providers", "ctx-core", "ctx-core"]), [
    "//core/crates/ctx-core:unit_tests",
    "//core/crates/ctx-core:workspace_payload_corpus",
    "//core/crates/ctx-providers:unit_tests",
  ]);
});

test("Bazel build target mapping expands per-crate libraries deterministically", () => {
  assert.deepEqual(getBazelBuildTargetsForCrates(["ctx-provider-auth-import", "ctx-core"]), [
    "//core/crates/ctx-core:lib",
    "//core/crates/ctx-provider-auth-import:lib",
  ]);
});
