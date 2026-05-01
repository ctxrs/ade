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
  filterGateManagedCrateNames,
  getTurboTaskNamesForCrates,
  isRustWorkspaceLevelInput,
} = require("./lib/rust_workspace_graph.cjs");
const { HOST_HEAVY_BUDGET_KEY, withHostJobBudget } = require("./lib/host_job_budget.cjs");
const { runTurbo } = require("./lib/turbo_runner.cjs");

function parseArgs(argv) {
  const args = {
    agentGate: false,
    all: false,
    changedFiles: [],
    crates: [],
    includeReverseDeps: false,
    mode: "workspace",
    resolvedCrates: false,
    runClippy: false,
    skipTests: false,
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
    } else if (arg === "--resolved-crates") {
      args.resolvedCrates = true;
    } else if (arg === "--skip-tests") {
      args.skipTests = true;
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
  if (
    args.resolvedCrates
    && (args.agentGate || args.all || args.changedFiles.length > 0 || args.includeReverseDeps)
  ) {
    throw new Error("--resolved-crates can only be combined with explicit --crate selections");
  }
  if (!args.agentGate && !args.all && args.changedFiles.length === 0 && args.crates.length === 0) {
    throw new Error("one of --all, --crate, or --changed-file is required");
  }
  if (args.skipTests && !args.runClippy) {
    throw new Error("--skip-tests requires --clippy so the Rust gate still performs work");
  }
  if (!new Set(["cargo", "mixed", "nextest"]).has(args.testStrategy)) {
    throw new Error(`unsupported test strategy: ${args.testStrategy}`);
  }
  return args;
}

function resolveCrates(graph, args) {
  if (args.resolvedCrates) {
    return filterGateManagedCrateNames(args.crates);
  }
  if (args.agentGate) {
    return [...AGENT_GATE_CRATES];
  }
  if (args.all) {
    return filterGateManagedCrateNames(graph.crates.map((crate) => crate.crateName));
  }

  const workspaceLevelChange = args.changedFiles.some(isRustWorkspaceLevelInput);
  if (workspaceLevelChange) {
    return filterGateManagedCrateNames(graph.crates.map((crate) => crate.crateName));
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
    return filterGateManagedCrateNames(resolved);
  }
  return filterGateManagedCrateNames(expandReverseDependencies(graph, resolved));
}

function rustRemoteCacheState(env) {
  const turboCacheMode = String(env.TURBO_CACHE_MODE || "local:rw").trim();
  const turboRemote = turboCacheMode.split(",").some((part) => part.trim().startsWith("remote:"));
  const sccacheState = String(env.CTX_RUST_CACHE_SCCACHE || "unconfigured").trim();
  const sccacheRemote = sccacheState === "enabled" && Boolean(String(env.SCCACHE_BUCKET || "").trim());
  return {
    sccache_remote: sccacheRemote,
    sccache_state: sccacheState,
    turbo_cache_mode: turboCacheMode,
    turbo_remote: turboRemote,
  };
}

function logRustRemoteCacheState(env, { runClippy }) {
  if (!runClippy) {
    return;
  }
  const state = rustRemoteCacheState(env);
  const status = state.turbo_remote || state.sccache_remote ? "active" : "inactive";
  console.error(
    "[ctx-cache] rust clippy remote cache %s: turbo=%s sccache=%s sccache_bucket=%s",
    status,
    state.turbo_cache_mode,
    state.sccache_state,
    state.sccache_remote ? "configured" : "missing",
  );
  if (status === "inactive") {
    console.error(
      "[ctx-cache] rust clippy remote cache inactive; configure TURBO_API/TURBO_TEAM/TURBO_TOKEN and CTX_SCCACHE_R2_* on Buildkite agents.",
    );
  }
}

function runTaskPhase({ coreRoot, env, crateNames, taskKind, turboConcurrency = null }) {
  if (crateNames.length === 0) {
    return;
  }
  runTurbo({
    coreRoot,
    env,
    taskNames: getTurboTaskNamesForCrates(crateNames, [taskKind]),
    concurrencyOverride: turboConcurrency,
  });
}

function runBazelPhase({ coreRoot, env, crateNames }) {
  if (crateNames.length === 0) {
    return;
  }
  const targetBatches = buildBazelTargetBatches(crateNames);
  if (targetBatches.length === 0) {
    return;
  }
  for (const targets of targetBatches) {
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
}

function buildBazelTargetBatches(crateNames) {
  const targets = getBazelTestTargetsForCrates(crateNames);
  if (targets.length === 0) {
    return [];
  }
  const ctxHttpTargets = targets.filter((target) => target.startsWith("//core/crates/ctx-http:"));
  const otherTargets = targets.filter((target) => !target.startsWith("//core/crates/ctx-http:"));
  const batches = [];
  if (otherTargets.length > 0) {
    batches.push(otherTargets);
  }
  for (const target of ctxHttpTargets) {
    batches.push([target]);
  }
  return batches;
}

function applyDefaultRustGateEnv(env, { cargoTestCrates, nextestCrates }) {
  if (!String(env.CARGO_INCREMENTAL ?? "").trim()) {
    env.CARGO_INCREMENTAL = "0";
  }
  if (cargoTestCrates.length > 0 && !String(env.RUST_TEST_THREADS ?? "").trim()) {
    env.RUST_TEST_THREADS = "1";
  }
  if (nextestCrates.length > 0 && !String(env.NEXTEST_TEST_THREADS ?? "").trim()) {
    env.NEXTEST_TEST_THREADS = "2";
  }
}

function applyBazelTestEnv(env, { bazelTestCrates }) {
  if (
    bazelTestCrates.includes("ctx-http") &&
    !String(env.CTX_BAZEL_LOCAL_TEST_JOBS ?? "").trim()
  ) {
    env.CTX_BAZEL_LOCAL_TEST_JOBS = "2";
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
  logRustRemoteCacheState(env, { runClippy: args.runClippy });
  const { bazelTestCrates, cargoTestCrates, nextestCrates } = partitionCratesForTestStrategy(
    crateNames,
    args.testStrategy,
  );
  applyDefaultRustGateEnv(env, {
    cargoTestCrates,
    nextestCrates,
  });
  applyBazelTestEnv(env, {
    bazelTestCrates,
  });

  withHostJobBudget({
    budgetKey: HOST_HEAVY_BUDGET_KEY,
    command: `run_rust_gate ${crateNames.join(" ")}`,
    cwd: coreRoot,
    env,
  }, () => {
    if (args.runClippy) {
      runTaskPhase({
        coreRoot,
        env,
        crateNames,
        taskKind: "clippy",
      });
    }
    if (!args.skipTests) {
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
          turboConcurrency: 1,
        });
      }
    }
  });
}

if (require.main === module) {
  main();
}

module.exports = {
  applyBazelTestEnv,
  applyDefaultRustGateEnv,
  buildBazelTargetBatches,
  parseArgs,
  resolveCrates,
  rustRemoteCacheState,
};
