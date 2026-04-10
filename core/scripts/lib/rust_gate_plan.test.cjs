const assert = require("node:assert/strict");
const test = require("node:test");

const {
  AGENT_GATE_CRATES,
  BAZEL_TEST_CRATES,
  ISOLATED_CARGO_TEST_CRATES,
  partitionCratesForTestStrategy,
} = require("./rust_gate_plan.cjs");

test("agent gate crate list keeps the expected ctx-http-centered fast gate", () => {
  assert.deepEqual(AGENT_GATE_CRATES, [
    "ctx-avf-linux-runtime",
    "ctx-bundled-assets",
    "ctx-core",
    "ctx-harness-setup",
    "ctx-harness-sources",
    "ctx-http",
    "ctx-lsp",
    "ctx-linux-sandbox-runtime",
    "ctx-mcp",
    "ctx-provider-accounts",
    "ctx-provider-auth-import",
    "ctx-providers",
    "ctx-runtime-assets",
    "ctx-sandbox-contract",
    "ctx-sandbox-container-runtime",
    "ctx-store",
    "ctx-worktree-data-plane",
    "ctx-workspace-active-snapshot",
  ]);
});

test("mixed test strategy keeps ctx-http and ctx-store on cargo test and routes the rest to nextest", () => {
  const plan = partitionCratesForTestStrategy(
    [
      "ctx-http",
      "ctx-store",
      "ctx-avf-linux-runtime",
      "ctx-bundled-assets",
      "ctx-provider-accounts",
      "ctx-harness-setup",
      "ctx-runtime-assets",
      "ctx-sandbox-contract",
      "ctx-worktree-data-plane",
      "ctx-workspace-active-snapshot",
      "ctx-http",
    ],
    "mixed",
  );

  assert.deepEqual(plan, {
    bazelTestCrates: [
      "ctx-avf-linux-runtime",
      "ctx-bundled-assets",
      "ctx-harness-setup",
      "ctx-provider-accounts",
      "ctx-runtime-assets",
      "ctx-sandbox-contract",
      "ctx-workspace-active-snapshot",
      "ctx-worktree-data-plane",
    ],
    cargoTestCrates: ["ctx-http", "ctx-store"],
    nextestCrates: [],
  });
});

test("ctx-store remains isolated from the parallel cargo tail", () => {
  assert.deepEqual([...ISOLATED_CARGO_TEST_CRATES].sort(), ["ctx-store"]);
});

test("mixed test strategy keeps the Bazel-covered slice explicit", () => {
  assert.deepEqual([...BAZEL_TEST_CRATES].sort(), [
    "ctx-avf-linux-runtime",
    "ctx-bundled-assets",
    "ctx-core",
    "ctx-harness-setup",
    "ctx-harness-sources",
    "ctx-linux-sandbox-runtime",
    "ctx-lsp",
    "ctx-provider-accounts",
    "ctx-provider-auth-import",
    "ctx-providers",
    "ctx-runtime-assets",
    "ctx-sandbox-container-runtime",
    "ctx-sandbox-contract",
    "ctx-workspace-active-snapshot",
    "ctx-worktree-data-plane",
  ]);
});

test("nextest strategy keeps every crate on nextest", () => {
  const plan = partitionCratesForTestStrategy(["ctx-store", "ctx-http"], "nextest");

  assert.deepEqual(plan, {
    bazelTestCrates: [],
    cargoTestCrates: [],
    nextestCrates: ["ctx-http", "ctx-store"],
  });
});
