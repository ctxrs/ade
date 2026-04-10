#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const { getBazelTestTargetsForCrates } = require("./lib/bazel_rust_targets.cjs");
const {
  AGENT_GATE_CRATES,
  FORCE_REVERSE_DEP_CRATES,
  ISOLATED_CARGO_TEST_CRATES,
  partitionCratesForTestStrategy,
} = require("./lib/rust_gate_plan.cjs");
const {
  buildWorkspaceGraph,
  collectChangedCrates,
  expandReverseDependencies,
  getTurboTaskNamesForCrates,
} = require("./lib/rust_workspace_graph.cjs");
const { runTurbo } = require("./lib/turbo_runner.cjs");

function parseArgs(argv) {
  const args = {
    agentGate: false,
    all: false,
    changedFiles: [],
    crates: [],
    includeReverseDeps: false,
    mode: "workspace",
    runClippy: false,
    testStrategy: "mixed",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--agent-gate") {
      args.agentGate = true;
    } else if (arg === "--all") {
      args.all = true;
    } else if (arg === "--changed-file") {
      args.changedFiles.push(argv[index + 1] || "");
      index += 1;
    } else if (arg === "--crate") {
      args.crates.push(argv[index + 1] || "");
      index += 1;
    } else if (arg === "--include-reverse-deps") {
      args.includeReverseDeps = true;
    } else if (arg === "--mode") {
      args.mode = argv[index + 1] || args.mode;
      index += 1;
    } else if (arg === "--clippy") {
      args.runClippy = true;
    } else if (arg === "--test-strategy") {
      args.testStrategy = argv[index + 1] || args.testStrategy;
      index += 1;
    } else {
      throw new Error(`unknown arg: ${arg}`);
    }
  }

  if (args.agentGate && (args.all || args.changedFiles.length > 0 || args.crates.length > 0)) {
    throw new Error("--agent-gate cannot be combined with --all, --crate, or --changed-file");
  }
  if (!args.agentGate && !args.all && args.changedFiles.length === 0 && args.crates.length === 0) {
    throw new Error("one of --all, --crate, or --changed-file is required");
  }
  if (!new Set(["cargo", "mixed", "nextest"]).has(args.testStrategy)) {
    throw new Error(`unsupported test strategy: ${args.testStrategy}`);
  }
  return args;
}

function resolveCrates(graph, args) {
  if (args.agentGate) {
    return [...AGENT_GATE_CRATES];
  }
  if (args.all) {
    return graph.crates.map((crate) => crate.crateName);
  }

  const directCrates = new Set(args.crates);
  for (const changedCrate of collectChangedCrates(graph, args.changedFiles)) {
    directCrates.add(changedCrate);
  }

  const resolved = [...directCrates].filter(Boolean).sort();
  const forcedReverseDeps = resolved.some((crateName) =>
    FORCE_REVERSE_DEP_CRATES.has(crateName),
  );
  if (!args.includeReverseDeps && !forcedReverseDeps) {
    return resolved;
  }
  return expandReverseDependencies(graph, resolved);
}

function runTaskPhase({ coreRoot, env, crateNames, taskKind }) {
  if (crateNames.length === 0) {
    return;
  }
  runTurbo({
    coreRoot,
    env,
    taskNames: getTurboTaskNamesForCrates(crateNames, [taskKind]),
  });
}

function runBazelPhase({ coreRoot, env, crateNames }) {
  if (crateNames.length === 0) {
    return;
  }
  const targets = getBazelTestTargetsForCrates(crateNames);
  if (targets.length === 0) {
    return;
  }
  const result = childProcess.spawnSync(
    "node",
    ["scripts/run_bazel_pilot.cjs", "test", ...targets],
    {
      cwd: coreRoot,
      env,
      stdio: "inherit",
    },
  );
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const coreRoot = path.resolve(__dirname, "..");
  const graph = buildWorkspaceGraph(coreRoot);
  const crateNames = resolveCrates(graph, args);
  if (crateNames.length === 0) {
    return;
  }

  const { env } = buildCtxCacheEnv({
    cwd: coreRoot,
    env: process.env,
    mode: args.mode,
    mkdir: true,
  });
  const { bazelTestCrates, cargoTestCrates, nextestCrates } = partitionCratesForTestStrategy(
    crateNames,
    args.testStrategy,
  );
  if (cargoTestCrates.length > 0 && !String(env.RUST_TEST_THREADS ?? "").trim()) {
    env.RUST_TEST_THREADS = "1";
  }
  if (nextestCrates.length > 0 && !String(env.NEXTEST_TEST_THREADS ?? "").trim()) {
    env.NEXTEST_TEST_THREADS = "2";
  }

  if (args.runClippy) {
    runTaskPhase({
      coreRoot,
      env,
      crateNames,
      taskKind: "clippy",
    });
  }
  runBazelPhase({
    coreRoot,
    env,
    crateNames: bazelTestCrates,
  });
  runTaskPhase({
    coreRoot,
    env,
    crateNames: nextestCrates,
    taskKind: "nextest",
  });
  const parallelCargoTestCrates = cargoTestCrates.filter(
    (crateName) => !ISOLATED_CARGO_TEST_CRATES.has(crateName),
  );
  const isolatedCargoTestCrates = cargoTestCrates.filter((crateName) =>
    ISOLATED_CARGO_TEST_CRATES.has(crateName),
  );
  runTaskPhase({
    coreRoot,
    env,
    crateNames: parallelCargoTestCrates,
    taskKind: "test",
  });
  for (const crateName of isolatedCargoTestCrates) {
    runTaskPhase({
      coreRoot,
      env,
      crateNames: [crateName],
      taskKind: "test",
    });
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  parseArgs,
  resolveCrates,
};
