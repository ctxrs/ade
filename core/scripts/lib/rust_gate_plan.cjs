const AGENT_GATE_CRATES = [
  "ctx-core",
  "ctx-harness-sources",
  "ctx-http",
  "ctx-lsp",
  "ctx-mcp",
  "ctx-provider-accounts",
  "ctx-providers",
  "ctx-store",
];

const SERIAL_CARGO_TEST_CRATES = new Set(["ctx-http", "ctx-store"]);

function sortUnique(values) {
  return [...new Set(values)].filter(Boolean).sort();
}

function partitionCratesForTestStrategy(crateNames, strategy) {
  const selectedCrates = sortUnique(crateNames);
  if (strategy === "cargo") {
    return {
      cargoTestCrates: selectedCrates,
      nextestCrates: [],
    };
  }
  if (strategy === "nextest") {
    return {
      cargoTestCrates: [],
      nextestCrates: selectedCrates,
    };
  }
  if (strategy === "mixed") {
    const cargoTestCrates = [];
    const nextestCrates = [];
    for (const crateName of selectedCrates) {
      if (SERIAL_CARGO_TEST_CRATES.has(crateName)) {
        cargoTestCrates.push(crateName);
      } else {
        nextestCrates.push(crateName);
      }
    }
    return {
      cargoTestCrates,
      nextestCrates,
    };
  }
  throw new Error(`unknown test strategy: ${strategy}`);
}

module.exports = {
  AGENT_GATE_CRATES,
  SERIAL_CARGO_TEST_CRATES,
  partitionCratesForTestStrategy,
};
