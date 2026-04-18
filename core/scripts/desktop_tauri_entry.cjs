#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { HOST_HEAVY_BUDGET_KEY, withHostJobBudget } = require("./lib/host_job_budget.cjs");

const coreRoot = path.resolve(__dirname, "..");
const desktopAppRoot = path.join(coreRoot, "apps", "desktop");
const localTauriBin = path.join(desktopAppRoot, "node_modules", ".bin", "tauri");

function fail(message) {
  console.error(`desktop_tauri_entry failed: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const command = String(argv[2] || "").trim();
  const rawTauriArgs = argv.slice(3);
  const tauriArgs = rawTauriArgs[0] === "--" ? rawTauriArgs.slice(1) : rawTauriArgs;
  if (!command) {
    fail("missing command (expected build or dev)");
  }
  if (!["build", "dev"].includes(command)) {
    fail(`unsupported command '${command}' (expected build or dev)`);
  }
  return { command, tauriArgs };
}

function resolvePrepMode({ command, tauriArgs }) {
  if (command === "dev") {
    return "dev";
  }
  return tauriArgs.includes("--debug") ? "debug-build" : "release-build";
}

function shouldSkipPrep(env = process.env) {
  return String(env.CTX_DESKTOP_SKIP_PREP || "").trim() === "1";
}

function resolveTauriBudgetKey(command) {
  return String(command || "").trim() === "build" ? HOST_HEAVY_BUDGET_KEY : "";
}

function createInvocation(argv = process.argv, env = process.env) {
  const parsed = parseArgs(argv);
  const skipPrep = shouldSkipPrep(env);
  return {
    ...parsed,
    prepMode: resolvePrepMode(parsed),
    prepCommand: skipPrep ? "" : "node",
    prepArgs: skipPrep ? [] : ["scripts/desktop_prepare.cjs", "--mode", resolvePrepMode(parsed)],
    skipPrep,
    tauriBudgetKey: resolveTauriBudgetKey(parsed.command),
    tauriCommand: resolveTauriCommand(),
    tauriExecArgs: [parsed.command, ...parsed.tauriArgs],
  };
}

function resolveTauriCommand() {
  if (!path.isAbsolute(localTauriBin)) {
    throw new Error(`expected absolute tauri path, got ${localTauriBin}`);
  }
  return localTauriBin;
}

function normalizeTauriCliEnv(env = process.env) {
  const normalizedEnv = { ...env };
  if (String(normalizedEnv.CI || "").trim() === "1") {
    normalizedEnv.CI = "true";
  }
  return normalizedEnv;
}

function run(
  command,
  args,
  {
    budgetKey = "",
    env = process.env,
    spawnSyncImpl = childProcess.spawnSync,
    withHostJobBudgetImpl = withHostJobBudget,
  } = {},
) {
  const invoke = () => {
    const result = spawnSyncImpl(command, args, {
      cwd: command === "node" ? coreRoot : desktopAppRoot,
      stdio: "inherit",
      env,
    });
    if (result.status !== 0) {
      process.exit(result.status ?? 1);
    }
  };
  if (!budgetKey) {
    invoke();
    return;
  }
  withHostJobBudgetImpl({
    budgetKey,
    command: `${command} ${args.join(" ")}`.trim(),
    cwd: command === "node" ? coreRoot : desktopAppRoot,
    env,
  }, invoke);
}

function main(
  argv = process.argv,
  {
    spawnSyncImpl = childProcess.spawnSync,
    withHostJobBudgetImpl = withHostJobBudget,
  } = {},
) {
  const invocation = createInvocation(argv);
  if (!invocation.skipPrep) {
    run(invocation.prepCommand, invocation.prepArgs, {
      spawnSyncImpl,
      withHostJobBudgetImpl,
    });
  }
  run(invocation.tauriCommand, invocation.tauriExecArgs, {
    env: normalizeTauriCliEnv(process.env),
    budgetKey: invocation.tauriBudgetKey,
    spawnSyncImpl,
    withHostJobBudgetImpl,
  });
}

if (require.main === module) {
  main();
}

module.exports = {
  createInvocation,
  main,
  normalizeTauriCliEnv,
  parseArgs,
  resolvePrepMode,
  resolveTauriBudgetKey,
  shouldSkipPrep,
};
