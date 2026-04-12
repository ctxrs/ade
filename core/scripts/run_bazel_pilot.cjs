#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { resolveCtxCacheLayout } = require("./lib/cache_roots.cjs");
const {
  getBazelBuildTargetsForCrates,
  getBazelCoveredCrates,
  getBazelTestTargetsForCrates,
  partitionBazelTargetsForLinuxRbe,
} = require("./lib/bazel_rust_targets.cjs");

const DEFAULT_TEST_TARGETS = getBazelTestTargetsForCrates(getBazelCoveredCrates());
const DEFAULT_BUILD_TARGETS = getBazelBuildTargetsForCrates(getBazelCoveredCrates());

function parseRemoteExecutionMode(value) {
  const normalized = String(value ?? "").trim().toLowerCase();
  if (["1", "true", "yes", "on", "all"].includes(normalized)) {
    return "all";
  }
  if (normalized === "linux") {
    return "linux";
  }
  return "off";
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

function buildPhaseCommandArgs({ command, layout, extraConfigArgs = [] }) {
  const commandArgs = [
    command,
    `--disk_cache=${layout.bazelDiskCacheDir}`,
    `--repository_cache=${layout.bazelRepositoryCacheDir}`,
    ...extraConfigArgs,
  ];
  if (command === "test") {
    commandArgs.push("--test_output=errors");
  }
  return commandArgs;
}

function buildInvocationPhases({ command, layout, remoteExecutionMode, targets }) {
  if (targets.length === 0) {
    return [];
  }
  if (command === "run" || remoteExecutionMode === "off") {
    return [
      {
        name: "local",
        commandArgs: buildPhaseCommandArgs({ command, layout }),
        targets,
      },
    ];
  }
  if (remoteExecutionMode === "all") {
    return [
      {
        name: "remote",
        commandArgs: buildPhaseCommandArgs({
          command,
          layout,
          extraConfigArgs: ["--config=buildbuddy-rbe"],
        }),
        targets,
      },
    ];
  }
  const { remoteTargets, localTargets } = partitionBazelTargetsForLinuxRbe(command, targets);
  const phases = [];
  if (remoteTargets.length > 0) {
    phases.push({
      name: "linux-rbe",
      commandArgs: buildPhaseCommandArgs({
        command,
        layout,
        extraConfigArgs: ["--config=buildbuddy-linux-rbe"],
      }),
      targets: remoteTargets,
    });
  }
  if (localTargets.length > 0) {
    phases.push({
      name: "local",
      commandArgs: buildPhaseCommandArgs({ command, layout }),
      targets: localTargets,
    });
  }
  return phases;
}

function buildBazelPilotInvocation({ argv, env = process.env } = {}) {
  const { command, targets } = parseArgs(argv || []);
  const coreRoot = path.resolve(__dirname, "..");
  const repoRoot = path.resolve(coreRoot, "..");
  const layout = resolveCtxCacheLayout({ cwd: coreRoot, env });
  const startupArgs = [`--output_user_root=${layout.bazelOutputUserRoot}`];
  const remoteExecutionMode = parseRemoteExecutionMode(env.CTX_BAZEL_REMOTE_EXECUTION);
  const phases = buildInvocationPhases({
    command,
    layout,
    remoteExecutionMode,
    targets,
  });
  const commandArgs = phases[0]?.commandArgs || [];
  const phaseTargets = phases[0]?.targets || [];

  return {
    command,
    commandArgs,
    layout,
    phases,
    remoteExecutionMode,
    repoRoot,
    startupArgs,
    targets: phaseTargets,
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
  for (const phase of invocation.phases) {
    const result = childProcess.spawnSync(
      "bazelisk",
      [...invocation.startupArgs, ...phase.commandArgs, ...phase.targets],
      {
        cwd: invocation.repoRoot,
        env: invocation.env,
        stdio: "inherit",
      },
    );
    if (typeof result.status === "number" && result.status !== 0) {
      process.exit(result.status);
    }
    if (result.error) {
      throw result.error;
    }
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  DEFAULT_BUILD_TARGETS,
  DEFAULT_TEST_TARGETS,
  buildBazelPilotInvocation,
  parseArgs,
  parseRemoteExecutionMode,
};
