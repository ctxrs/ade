const { getBazelCoveredCrates } = require("./bazel_rust_targets.cjs");

const AGENT_GATE_CRATES = [
  "ctx-core",
  "ctx-harness-sources",
  "ctx-http",
  "ctx-lsp",
  "ctx-mcp",
  "ctx-provider-accounts",
  "ctx-provider-auth-import",
  "ctx-providers",
  "ctx-sandbox-contract",
  "ctx-store",
  "ctx-worktree-data-plane",
  "ctx-workspace-active-snapshot",
];

const BAZEL_TEST_CRATES = new Set(getBazelCoveredCrates());
const SERIAL_CARGO_TEST_CRATES = new Set(["ctx-http", "ctx-store"]);
const ISOLATED_CARGO_TEST_CRATES = new Set(["ctx-store"]);

function sortUnique(values) {
  return [...new Set(values)].filter(Boolean).sort();
}

function partitionCratesForTestStrategy(crateNames, strategy) {
  const selectedCrates = sortUnique(crateNames);
  if (strategy === "cargo") {
    return {
      bazelTestCrates: [],
      cargoTestCrates: selectedCrates,
      nextestCrates: [],
    };
  }
  if (strategy === "nextest") {
    return {
      bazelTestCrates: [],
      cargoTestCrates: [],
      nextestCrates: selectedCrates,
    };
  }
  if (strategy === "mixed") {
    const bazelTestCrates = [];
    const cargoTestCrates = [];
    const nextestCrates = [];
    for (const crateName of selectedCrates) {
      if (BAZEL_TEST_CRATES.has(crateName)) {
        bazelTestCrates.push(crateName);
      } else if (SERIAL_CARGO_TEST_CRATES.has(crateName)) {
        cargoTestCrates.push(crateName);
      } else {
        nextestCrates.push(crateName);
      }
    }
    return {
      bazelTestCrates,
      cargoTestCrates,
      nextestCrates,
    };
  }
  throw new Error(`unknown test strategy: ${strategy}`);
}

module.exports = {
  AGENT_GATE_CRATES,
  BAZEL_TEST_CRATES,
  ISOLATED_CARGO_TEST_CRATES,
  SERIAL_CARGO_TEST_CRATES,
  partitionCratesForTestStrategy,
};
