const { getBazelCoveredCrates } = require("./bazel_rust_targets.cjs");

const AGENT_GATE_CRATES = [
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
  "ctx-managed-installs",
  "ctx-provider-accounts",
  "ctx-provider-install",
  "ctx-provider-runtime",
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
];

const BAZEL_TEST_CRATES = new Set(getBazelCoveredCrates());
const FORCE_REVERSE_DEP_CRATES = new Set(["ctx-execution-runtime"]);
const SERIAL_CARGO_TEST_CRATES = new Set(["ctx-mcp", "ctx-providers", "ctx-store"]);
const ISOLATED_CARGO_TEST_CRATES = new Set([
  "ctx-mcp",
  "ctx-providers",
  "ctx-store",
]);

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
  FORCE_REVERSE_DEP_CRATES,
  ISOLATED_CARGO_TEST_CRATES,
  SERIAL_CARGO_TEST_CRATES,
  partitionCratesForTestStrategy,
};
