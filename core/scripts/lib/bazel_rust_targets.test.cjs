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
    "ctx-lsp",
    "ctx-provider-accounts",
    "ctx-provider-auth-import",
    "ctx-providers",
    "ctx-workspace-active-snapshot",
  ]);
});

test("Bazel test target mapping expands per-crate tests deterministically", () => {
  assert.deepEqual(
    getBazelTestTargetsForCrates(["ctx-providers", "ctx-core", "ctx-lsp", "ctx-core"]),
    [
      "//core/crates/ctx-core:unit_tests",
      "//core/crates/ctx-core:workspace_payload_corpus",
      "//core/crates/ctx-lsp:lsp_manager_smoke",
      "//core/crates/ctx-providers:unit_tests",
    ],
  );
});

test("Bazel build target mapping expands per-crate libraries deterministically", () => {
  assert.deepEqual(
    getBazelBuildTargetsForCrates([
      "ctx-provider-auth-import",
      "ctx-workspace-active-snapshot",
      "ctx-core",
    ]),
    [
      "//core/crates/ctx-core:lib",
      "//core/crates/ctx-provider-auth-import:lib",
      "//core/crates/ctx-workspace-active-snapshot:lib",
    ],
  );
});
