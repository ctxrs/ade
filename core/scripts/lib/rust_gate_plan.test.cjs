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
    "ctx-crp-protocol",
    "ctx-execution-runtime",
    "ctx-fs",
    "ctx-harness-setup",
    "ctx-harness-runtime",
    "ctx-harness-sources",
    "ctx-http",
    "ctx-http-auth",
    "ctx-http-test-support",
    "ctx-linux-sandbox-runtime",
    "ctx-llm-relay-authority",
    "ctx-llm-relay-contract",
    "ctx-mcp",
    "ctx-mcp-auth",
    "ctx-mcp-command",
    "ctx-managed-installs",
    "ctx-mobile-access-service",
    "ctx-observability",
    "ctx-org-policy",
    "ctx-provider-accounts",
    "ctx-provider-install",
    "ctx-provider-runtime",
    "ctx-provider-matrix",
    "ctx-workspace-services",
    "ctx-provider-auth-import",
    "ctx-providers",
    "ctx-resource-utilization",
    "ctx-route-contracts",
    "ctx-run-archive-service",
    "ctx-run-scheduler",
    "ctx-runtime-assets",
    "ctx-sandbox-contract",
    "ctx-sandbox-container-runtime",
    "ctx-sandbox-materialization",
    "ctx-session-artifacts",
    "ctx-session-message-service",
    "ctx-session-runtime",
    "ctx-session-service",
    "ctx-subagent-service",
    "ctx-session-tools",
    "ctx-settings-model",
    "ctx-settings-service",
    "ctx-store",
    "ctx-storage-admission",
    "ctx-task-service",
    "ctx-update-service",
    "ctx-worktree-data-plane",
    "ctx-workspace-attachments",
    "ctx-workspace-container",
    "ctx-workspace-active-snapshot",
    "ctx-workspace-config",
    "ctx-workspace-runtime",
    "ctx-workspace-stream-service",
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
      "ctx-session-artifacts",
      "ctx-session-tools",
      "ctx-settings-model",
      "ctx-settings-service",
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
      "ctx-session-artifacts",
      "ctx-session-tools",
      "ctx-settings-model",
      "ctx-settings-service",
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
