const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { HOST_HEAVY_BUDGET_KEY, withHostJobBudget } = require("./host_job_budget.cjs");

function resolveTurboBinary(coreRoot) {
  const binDir = childProcess
    .execFileSync("pnpm", ["bin"], {
      cwd: coreRoot,
      encoding: "utf8",
    })
    .trim();
  const binaryName = process.platform === "win32" ? "turbo.cmd" : "turbo";
  const turboBinary = path.join(binDir, binaryName);
  if (!fs.existsSync(turboBinary)) {
    throw new Error(
      `Turbo binary not found at ${turboBinary}. Run 'pnpm -C ${coreRoot} install --frozen-lockfile' to install workspace tools.`,
    );
  }
  return turboBinary;
}

function buildTurboRunArgs({ env, taskNames, extraArgs = [], concurrencyOverride = null }) {
  const concurrency =
    concurrencyOverride ?? env.CTX_VERIFY_TURBO_CONCURRENCY ?? "4";
  return [
    "run",
    ...taskNames,
    ...extraArgs,
    `--cache-dir=${env.TURBO_CACHE_DIR}`,
    "--output-logs=errors-only",
    "--log-order=grouped",
    `--concurrency=${concurrency}`,
    "--ui=stream",
    `--cache=${env.TURBO_CACHE_MODE || "local:rw"}`,
  ];
}

function runTurbo({
  coreRoot,
  env,
  taskNames,
  extraArgs = [],
  concurrencyOverride = null,
  resolveTurboBinaryImpl = resolveTurboBinary,
  spawnSyncImpl = childProcess.spawnSync,
  withHostJobBudgetImpl = withHostJobBudget,
}) {
  const turboBinary = resolveTurboBinaryImpl(coreRoot);
  return withHostJobBudgetImpl({
    budgetKey: HOST_HEAVY_BUDGET_KEY,
    command: `turbo ${taskNames.join(" ")}`,
    cwd: coreRoot,
    env,
  }, () => {
    const result = spawnSyncImpl(
      turboBinary,
      buildTurboRunArgs({
        env,
        taskNames,
        extraArgs,
        concurrencyOverride,
      }),
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
  });
}

module.exports = {
  buildTurboRunArgs,
  resolveTurboBinary,
  runTurbo,
};
