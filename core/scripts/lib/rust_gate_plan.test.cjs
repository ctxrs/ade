const assert = require("node:assert/strict");
const test = require("node:test");

const { getBazelCoveredCrates } = require("./bazel_rust_targets.cjs");
const {
  AGENT_GATE_CRATES,
  BAZEL_TEST_CRATES,
  FORCE_REVERSE_DEP_CRATES,
  ISOLATED_CARGO_TEST_CRATES,
  partitionCratesForTestStrategy,
} = require("./rust_gate_plan.cjs");

test("agent gate crate list keeps the expected ctx-http-centered fast gate", () => {
  assert.deepEqual(AGENT_GATE_CRATES, [
    "ctx-avf-linux-runtime",
    "ctx-bundled-assets",
    "ctx-core",
    "ctx-execution-runtime",
    "ctx-fs",
    "ctx-harness-setup",
    "ctx-harness-runtime",
    "ctx-harness-sources",
    "ctx-http",
    "ctx-lsp",
    "ctx-linux-sandbox-runtime",
    "ctx-mcp",
    "ctx-provider-accounts",
    "ctx-provider-install",
    "ctx-provider-matrix",
    "ctx-provider-auth-import",
    "ctx-providers",
    "ctx-runtime-assets",
    "ctx-sandbox-contract",
    "ctx-sandbox-container-runtime",
    "ctx-sandbox-materialization",
    "ctx-session-tools",
    "ctx-store",
    "ctx-storage-admission",
    "ctx-worktree-data-plane",
    "ctx-workspace-container",
    "ctx-workspace-active-snapshot",
    "ctx-workspace-config",
    "ctx-workspace-runtime",
  ]);
});

test("mixed test strategy routes the selected Bazel-covered slice to Bazel with no cargo or nextest tail", () => {
  const plan = partitionCratesForTestStrategy(
    [
      "ctx-http",
      "ctx-mcp",
      "ctx-providers",
      "ctx-store",
      "ctx-avf-linux-runtime",
      "ctx-bundled-assets",
      "ctx-execution-runtime",
      "ctx-fs",
      "ctx-provider-accounts",
      "ctx-provider-install",
      "ctx-provider-matrix",
      "ctx-harness-setup",
      "ctx-runtime-assets",
      "ctx-sandbox-contract",
      "ctx-sandbox-materialization",
      "ctx-session-tools",
      "ctx-storage-admission",
      "ctx-worktree-data-plane",
      "ctx-workspace-active-snapshot",
      "ctx-workspace-config",
      "ctx-workspace-runtime",
      "ctx-http",
    ],
    "mixed",
  );

  assert.deepEqual(plan, {
    bazelTestCrates: [
      "ctx-avf-linux-runtime",
      "ctx-bundled-assets",
      "ctx-execution-runtime",
      "ctx-fs",
      "ctx-harness-setup",
      "ctx-http",
      "ctx-mcp",
      "ctx-provider-accounts",
      "ctx-provider-install",
      "ctx-provider-matrix",
      "ctx-providers",
      "ctx-runtime-assets",
      "ctx-sandbox-contract",
      "ctx-sandbox-materialization",
      "ctx-session-tools",
      "ctx-storage-admission",
      "ctx-store",
      "ctx-workspace-active-snapshot",
      "ctx-workspace-config",
      "ctx-workspace-runtime",
      "ctx-worktree-data-plane",
    ],
    cargoTestCrates: [],
    nextestCrates: [],
  });
});

test("ctx-mcp and the heavier integration crates remain isolated from the parallel cargo tail", () => {
  assert.deepEqual([...ISOLATED_CARGO_TEST_CRATES].sort(), [
    "ctx-mcp",
    "ctx-providers",
    "ctx-store",
  ]);
});

test("ctx-execution-runtime changes force reverse-dependency coverage", () => {
  assert.deepEqual([...FORCE_REVERSE_DEP_CRATES].sort(), ["ctx-execution-runtime"]);
});

test("mixed test strategy keeps the Bazel-covered slice explicit", () => {
  assert.deepEqual([...BAZEL_TEST_CRATES].sort(), getBazelCoveredCrates());
  assert.equal(BAZEL_TEST_CRATES.has("ctx-http"), true);
});

test("nextest strategy keeps every crate on nextest", () => {
  const plan = partitionCratesForTestStrategy(["ctx-store", "ctx-http"], "nextest");

  assert.deepEqual(plan, {
    bazelTestCrates: [],
    cargoTestCrates: [],
    nextestCrates: ["ctx-http", "ctx-store"],
  });
});
