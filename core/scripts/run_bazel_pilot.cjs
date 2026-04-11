#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { resolveCtxCacheLayout } = require("./lib/cache_roots.cjs");
const {
  getBazelBuildTargetsForCrates,
  getBazelCoveredCrates,
  getBazelTestTargetsForCrates,
} = require("./lib/bazel_rust_targets.cjs");

const DEFAULT_TEST_TARGETS = getBazelTestTargetsForCrates(getBazelCoveredCrates());
const DEFAULT_BUILD_TARGETS = getBazelBuildTargetsForCrates(getBazelCoveredCrates());

function parseEnabledFlag(value) {
  return ["1", "true", "yes", "on"].includes(String(value ?? "").trim().toLowerCase());
}

function parseArgs(argv) {
  const [command = "test", ...targets] = argv;
  if (!["build", "run", "test"].includes(command)) {
    throw new Error(`unsupported Bazel pilot command: ${command}`);
  }
  return {
    command,
    targets:
      targets.length > 0
        ? targets
        : command === "build"
          ? DEFAULT_BUILD_TARGETS
          : command === "run"
            ? []
          : DEFAULT_TEST_TARGETS,
  };
}

function ensureDir(dir) {
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function buildBazelPilotInvocation({ argv, env = process.env } = {}) {
  const { command, targets } = parseArgs(argv || []);
  const coreRoot = path.resolve(__dirname, "..");
  const repoRoot = path.resolve(coreRoot, "..");
  const layout = resolveCtxCacheLayout({ cwd: coreRoot, env });
  const startupArgs = [`--output_user_root=${layout.bazelOutputUserRoot}`];
  const extraConfigArgs = [];
  if (parseEnabledFlag(env.CTX_BAZEL_REMOTE_EXECUTION)) {
    extraConfigArgs.push("--config=buildbuddy-rbe");
  }
  const commandArgs = [
    command,
    `--disk_cache=${layout.bazelDiskCacheDir}`,
    `--repository_cache=${layout.bazelRepositoryCacheDir}`,
    ...extraConfigArgs,
  ];
  if (command === "test") {
    commandArgs.push("--test_output=errors");
  }

  return {
    command,
    layout,
    repoRoot,
    startupArgs,
    commandArgs,
    targets,
    env: {
      ...env,
      TMPDIR: layout.tmpDir,
    },
  };
}

function main() {
  const invocation = buildBazelPilotInvocation({ argv: process.argv.slice(2) });
  ensureDir(invocation.layout.tmpDir);
  ensureDir(invocation.layout.bazelOutputUserRoot);
  ensureDir(invocation.layout.bazelDiskCacheDir);
  ensureDir(invocation.layout.bazelRepositoryCacheDir);
  const result = childProcess.spawnSync(
    "bazelisk",
    [...invocation.startupArgs, ...invocation.commandArgs, ...invocation.targets],
    {
      cwd: invocation.repoRoot,
      env: invocation.env,
      stdio: "inherit",
    },
  );
  if (typeof result.status === "number") {
    process.exit(result.status);
  }
  process.exit(1);
}

if (require.main === module) {
  main();
}

module.exports = {
  DEFAULT_BUILD_TARGETS,
  DEFAULT_TEST_TARGETS,
  buildBazelPilotInvocation,
  parseArgs,
};
