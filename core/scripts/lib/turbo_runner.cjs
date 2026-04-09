const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

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

function buildTurboRunArgs({ env, taskNames, extraArgs = [] }) {
  return [
    "run",
    ...taskNames,
    ...extraArgs,
    `--cache-dir=${env.TURBO_CACHE_DIR}`,
    "--output-logs=errors-only",
    "--log-order=grouped",
    `--concurrency=${env.CTX_VERIFY_TURBO_CONCURRENCY || "4"}`,
    "--ui=stream",
    `--cache=${env.TURBO_CACHE_MODE || "local:rw"}`,
  ];
}

function runTurbo({ coreRoot, env, taskNames, extraArgs = [] }) {
  const turboBinary = resolveTurboBinary(coreRoot);
  const result = childProcess.spawnSync(
    turboBinary,
    buildTurboRunArgs({
      env,
      taskNames,
      extraArgs,
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
}

module.exports = {
  buildTurboRunArgs,
  resolveTurboBinary,
  runTurbo,
};
