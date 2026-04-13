const assert = require("node:assert/strict");
const test = require("node:test");

const {
  getBazelBuildTargetsForCrates,
  getBazelCoveredCrates,
  getBazelTestTargetsForCrates,
} = require("./bazel_rust_targets.cjs");

test("Bazel-covered crates stay on the intended explicit Rust slice", () => {
  assert.deepEqual(getBazelCoveredCrates(), [
    "ctx-avf-linux-runtime",
    "ctx-bundled-assets",
    "ctx-core",
    "ctx-execution-runtime",
    "ctx-fs",
    "ctx-harness-runtime",
    "ctx-harness-setup",
    "ctx-harness-sources",
    "ctx-linux-sandbox-runtime",
    "ctx-lsp",
    "ctx-provider-accounts",
    "ctx-provider-auth-import",
    "ctx-provider-install",
    "ctx-provider-matrix",
    "ctx-providers",
    "ctx-runtime-assets",
    "ctx-sandbox-container-runtime",
    "ctx-sandbox-contract",
    "ctx-sandbox-materialization",
    "ctx-session-tools",
    "ctx-storage-admission",
    "ctx-store",
    "ctx-workspace-active-snapshot",
    "ctx-workspace-config",
    "ctx-workspace-container",
    "ctx-workspace-runtime",
    "ctx-worktree-data-plane",
  ]);
});

test("Bazel test target mapping expands per-crate tests deterministically", () => {
  assert.deepEqual(
    getBazelTestTargetsForCrates([
      "ctx-execution-runtime",
      "ctx-core",
      "ctx-lsp",
      "ctx-store",
      "ctx-workspace-config",
      "ctx-core",
    ]),
    [
      "//core/crates/ctx-core:unit_tests",
      "//core/crates/ctx-core:workspace_payload_corpus",
      "//core/crates/ctx-execution-runtime:unit_tests",
      "//core/crates/ctx-lsp:lsp_manager_smoke",
      "//core/crates/ctx-store:sqlite_hardening",
      "//core/crates/ctx-store:unit_tests",
      "//core/crates/ctx-workspace-config:unit_tests",
    ],
  );
});

test("Bazel build target mapping expands per-crate libraries deterministically", () => {
  assert.deepEqual(
    getBazelBuildTargetsForCrates([
      "ctx-avf-linux-runtime",
      "ctx-provider-auth-import",
      "ctx-bundled-assets",
      "ctx-execution-runtime",
      "ctx-harness-setup",
      "ctx-provider-install",
      "ctx-runtime-assets",
      "ctx-sandbox-materialization",
      "ctx-session-tools",
      "ctx-storage-admission",
      "ctx-workspace-active-snapshot",
      "ctx-workspace-runtime",
      "ctx-worktree-data-plane",
      "ctx-provider-matrix",
      "ctx-providers",
      "ctx-core",
      "ctx-fs",
      "ctx-store",
      "ctx-workspace-config",
    ]),
    [
      "//core/crates/ctx-avf-linux-runtime:lib",
      "//core/crates/ctx-bundled-assets:lib",
      "//core/crates/ctx-core:lib",
      "//core/crates/ctx-execution-runtime:lib",
      "//core/crates/ctx-fs:lib",
      "//core/crates/ctx-harness-setup:lib",
      "//core/crates/ctx-provider-auth-import:lib",
      "//core/crates/ctx-provider-install:lib",
      "//core/crates/ctx-provider-matrix:lib",
      "//core/crates/ctx-providers:lib",
      "//core/crates/ctx-runtime-assets:lib",
      "//core/crates/ctx-sandbox-materialization:lib",
      "//core/crates/ctx-session-tools:lib",
      "//core/crates/ctx-storage-admission:lib",
      "//core/crates/ctx-store:lib",
      "//core/crates/ctx-workspace-active-snapshot:lib",
      "//core/crates/ctx-workspace-config:lib",
      "//core/crates/ctx-workspace-runtime:lib",
      "//core/crates/ctx-worktree-data-plane:lib",
    ],
  );
});
