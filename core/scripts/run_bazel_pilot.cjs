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
const BAZELISK_SHIM = process.platform === "win32" ? "bazelisk.cmd" : "bazelisk";

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

function bazeliskBinaryPath() {
  const coreRoot = path.resolve(__dirname, "..");
  return path.join(coreRoot, "node_modules", ".bin", BAZELISK_SHIM);
}

function buildSpawnForPhase(invocation, phase) {
  return {
    command: bazeliskBinaryPath(),
    args: [...invocation.startupArgs, ...phase.commandArgs, ...phase.targets],
    options: {
      cwd: invocation.repoRoot,
      env: invocation.env,
      stdio: "inherit",
    },
  };
}

function buildBazelPilotSpawns({ argv, env = process.env } = {}) {
  const invocation = buildBazelPilotInvocation({ argv, env });
  return invocation.phases.map((phase) => buildSpawnForPhase(invocation, phase));
}

function buildBazelPilotSpawn({ argv, env = process.env } = {}) {
  return buildBazelPilotSpawns({ argv, env })[0];
}

function formatSpawnFailureMessage(command, error) {
  if (error && error.code === "ENOENT") {
    return [
      `error: Bazelisk is not installed at ${command}.`,
      "Install core JS dependencies first with `pnpm -C core install --frozen-lockfile`",
      "(or `pnpm -C core install` for a local dev refresh), then rerun the Bazel-backed command.",
    ].join(" ");
  }
  const details = error && error.message ? error.message : String(error || "unknown spawn failure");
  return `error: failed to start Bazelisk at ${command}: ${details}`;
}

function main() {
  const invocation = buildBazelPilotInvocation({ argv: process.argv.slice(2) });
  ensureDir(invocation.layout.tmpDir);
  ensureDir(invocation.layout.bazelOutputUserRoot);
  ensureDir(invocation.layout.bazelDiskCacheDir);
  ensureDir(invocation.layout.bazelRepositoryCacheDir);
  for (const phase of invocation.phases) {
    const spawn = buildSpawnForPhase(invocation, phase);
    const result = childProcess.spawnSync(spawn.command, spawn.args, spawn.options);
    if (result.error) {
      console.error(formatSpawnFailureMessage(spawn.command, result.error));
      process.exit(1);
    }
    if (typeof result.status === "number" && result.status !== 0) {
      process.exit(result.status);
    }
    if (result.signal) {
      console.error(`error: Bazelisk terminated with signal ${result.signal}`);
      process.exit(1);
    }
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  DEFAULT_BUILD_TARGETS,
  DEFAULT_TEST_TARGETS,
  bazeliskBinaryPath,
  buildBazelPilotSpawn,
  buildBazelPilotSpawns,
  buildBazelPilotInvocation,
  formatSpawnFailureMessage,
  parseArgs,
  parseRemoteExecutionMode,
};
