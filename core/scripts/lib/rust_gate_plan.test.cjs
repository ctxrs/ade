const assert = require("node:assert/strict");
const test = require("node:test");

const {
  AGENT_GATE_CRATES,
  partitionCratesForTestStrategy,
} = require("./rust_gate_plan.cjs");

test("agent gate crate list keeps the expected ctx-http-centered fast gate", () => {
  assert.deepEqual(AGENT_GATE_CRATES, [
    "ctx-core",
    "ctx-harness-sources",
    "ctx-http",
    "ctx-lsp",
    "ctx-mcp",
    "ctx-provider-accounts",
    "ctx-providers",
    "ctx-store",
  ]);
});

test("mixed test strategy keeps ctx-http and ctx-store on cargo test and routes the rest to nextest", () => {
  const plan = partitionCratesForTestStrategy(
    ["ctx-http", "ctx-store", "ctx-provider-accounts", "ctx-http"],
    "mixed",
  );

  assert.deepEqual(plan, {
    cargoTestCrates: ["ctx-http", "ctx-store"],
    nextestCrates: ["ctx-provider-accounts"],
  });
});

test("nextest strategy keeps every crate on nextest", () => {
  const plan = partitionCratesForTestStrategy(["ctx-store", "ctx-http"], "nextest");

  assert.deepEqual(plan, {
    cargoTestCrates: [],
    nextestCrates: ["ctx-http", "ctx-store"],
  });
});
