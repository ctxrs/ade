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
  const outputUserRoot = path.join(layout.targetsDir, "bazel", layout.scopeKey);
  const diskCacheDir = path.join(layout.cacheDir, "bazel-disk", layout.repoCacheSlug);
  const repositoryCacheDir = path.join(layout.cacheDir, "bazel-repository", layout.repoCacheSlug);
  const startupArgs = [`--output_user_root=${outputUserRoot}`];
  const commandArgs = [
    command,
    `--disk_cache=${diskCacheDir}`,
    `--repository_cache=${repositoryCacheDir}`,
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
  ensureDir(path.join(invocation.layout.targetsDir, "bazel", invocation.layout.scopeKey));
  ensureDir(path.join(invocation.layout.cacheDir, "bazel-disk", invocation.layout.repoCacheSlug));
  ensureDir(path.join(invocation.layout.cacheDir, "bazel-repository", invocation.layout.repoCacheSlug));
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
