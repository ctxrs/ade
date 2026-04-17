#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const {
  DEFAULT_INFISICAL_ENV,
  resolveBuildBuddyApiKey,
} = require("./lib/buildbuddy_operator_env.cjs");
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
const BAZELISK_PACKAGE_DIR_PREFIX = "@bazel+bazelisk@";

function parseBatchMode(value, { platform = process.platform } = {}) {
  if (value == null || String(value).trim() === "") {
    return platform === "darwin";
  }
  const normalized = String(value).trim().toLowerCase();
  if (["1", "true", "yes", "on"].includes(normalized)) {
    return true;
  }
  if (["0", "false", "no", "off"].includes(normalized)) {
    return false;
  }
  return platform === "darwin";
}

function commandExists(commandName, { env = process.env, cwd } = {}) {
  const result = childProcess.spawnSync(commandName, ["--version"], {
    cwd,
    env,
    encoding: "utf8",
    stdio: "ignore",
  });
  if (result.error && result.error.code === "ENOENT") {
    return false;
  }
  return true;
}

function pathIsRunnable(candidatePath, { access = fs.accessSync } = {}) {
  try {
    access(candidatePath, fs.constants.R_OK | fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function resolvePnpmBazeliskScript({
  repoRoot,
  fileExists = fs.existsSync,
  readDir = fs.readdirSync,
  pathRunnable = pathIsRunnable,
} = {}) {
  const pnpmDir = path.join(repoRoot, "core", "node_modules", ".pnpm");
  if (!fileExists(pnpmDir)) {
    return "";
  }
  let entries = [];
  try {
    entries = readDir(pnpmDir);
  } catch {
    return "";
  }
  const packageEntry = entries
    .filter((entry) => entry.startsWith(BAZELISK_PACKAGE_DIR_PREFIX))
    .sort()
    .at(-1);
  if (!packageEntry) {
    return "";
  }
  const scriptPath = path.join(
    pnpmDir,
    packageEntry,
    "node_modules",
    "@bazel",
    "bazelisk",
    "bazelisk.js",
  );
  if (!fileExists(scriptPath) || !pathRunnable(scriptPath)) {
    return "";
  }
  return scriptPath;
}

function parseRemoteExecutionMode(value) {
  const normalized = String(value ?? "").trim().toLowerCase();
  if (!normalized) {
    if (process.platform === "darwin") {
      return "cache";
    }
    if (process.platform === "linux") {
      return "linux";
    }
    return "off";
  }
  if (["0", "false", "no", "off"].includes(normalized)) {
    return "off";
  }
  if (normalized === "cache") {
    return "cache";
  }
  if (["1", "true", "yes", "on", "all"].includes(normalized)) {
    return "all";
  }
  if (normalized === "linux") {
    return "linux";
  }
  if (normalized === "darwin" || normalized === "macos") {
    return "darwin";
  }
  return "off";
}

function parsePositiveInteger(value) {
  const normalized = String(value ?? "").trim();
  if (!normalized) {
    return null;
  }
  if (!/^[0-9]+$/.test(normalized)) {
    throw new Error(`expected a positive integer but received '${value}'`);
  }
  const parsed = Number.parseInt(normalized, 10);
  return parsed > 0 ? parsed : null;
}

function parsePositiveIntegerEnv(value) {
  const normalized = String(value ?? "").trim();
  if (!normalized) {
    return null;
  }
  if (!/^\d+$/.test(normalized)) {
    return null;
  }
  const parsed = Number.parseInt(normalized, 10);
  return parsed > 0 ? parsed : null;
}

function parseArgs(argv) {
  const [command = "test", ...rawArgs] = argv;
  if (!["build", "run", "test"].includes(command)) {
    throw new Error(`unsupported Bazel pilot command: ${command}`);
  }
  if (command === "run") {
    const separatorIndex = rawArgs.indexOf("--");
    const targets = separatorIndex === -1 ? rawArgs : rawArgs.slice(0, separatorIndex);
    const runArgs = separatorIndex === -1 ? [] : rawArgs.slice(separatorIndex + 1);
    return runArgs.length > 0
      ? { command, targets, runArgs }
      : { command, targets };
  }
  return {
    command,
    targets:
      rawArgs.length > 0
        ? rawArgs
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

function buildPhaseCommandArgs({ command, layout, extraConfigArgs = [], bazelJobs = null }) {
  const commandArgs = [
    command,
    `--disk_cache=${layout.bazelDiskCacheDir}`,
    `--repository_cache=${layout.bazelRepositoryCacheDir}`,
    ...extraConfigArgs,
  ];
  if (bazelJobs != null) {
    commandArgs.push(`--jobs=${bazelJobs}`);
  }
  if (command === "test") {
    commandArgs.push("--test_output=errors");
  }
  return commandArgs;
}

function buildBuildBuddyAuthArgs(env) {
  const apiKey = String(env?.BUILD_BUDDY_API_KEY || env?.BUILDBUDDY_API_KEY || "").trim();
  if (!apiKey) {
    return [];
  }
  return [`--remote_header=x-buildbuddy-api-key=${apiKey}`];
}

function resolveBuildBuddyInfisicalEnv(env) {
  return (
    String(env?.CTX_BAZEL_INFISICAL_ENV || env?.INFISICAL_ENV || "").trim()
    || DEFAULT_INFISICAL_ENV
  );
}

function summarizeBuildBuddyResolutionError(error) {
  const lines = String(error?.message || error || "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length === 0) {
    return "";
  }
  const messageLine = lines.find((line) => line.startsWith("Message:"));
  if (messageLine) {
    return messageLine;
  }
  const responseLine = lines.find((line) => line.startsWith("Response Code:"));
  if (responseLine) {
    return responseLine;
  }
  return lines.find((line) => !line.startsWith("A new release of infisical is available:"))
    || lines[0];
}

function formatMissingBuildBuddyApiKeyMessage({
  infisicalEnv,
  remoteExecutionMode,
  error,
}) {
  const reason = summarizeBuildBuddyResolutionError(error);
  const reasonSuffix = reason ? ` Underlying error: ${reason}` : "";
  return [
    `error: BuildBuddy remote cache/execution is enabled by default (mode: ${remoteExecutionMode}).`,
    `Failed to resolve BUILD_BUDDY_API_KEY from the environment or Infisical (env: ${infisicalEnv}).`,
    "Export BUILD_BUDDY_API_KEY or BUILDBUDDY_API_KEY, or authenticate Infisical in core/ so",
    `\`infisical secrets get BUILD_BUDDY_API_KEY --plain --env ${infisicalEnv}\` succeeds,`,
    "or opt out with CTX_BAZEL_REMOTE_EXECUTION=off.",
    reasonSuffix,
  ]
    .join(" ")
    .trim();
}

function resolveBuildBuddyEnv({
  buildBuddyApiKeyResolver = resolveBuildBuddyApiKey,
  env = process.env,
  remoteExecutionMode,
} = {}) {
  if (remoteExecutionMode === "off") {
    return env;
  }
  const infisicalEnv = resolveBuildBuddyInfisicalEnv(env);
  let apiKey = "";
  try {
    apiKey = String(buildBuddyApiKeyResolver({ env, infisicalEnv }) || "").trim();
  } catch (error) {
    throw new Error(
      formatMissingBuildBuddyApiKeyMessage({
        infisicalEnv,
        remoteExecutionMode,
        error,
      }),
    );
  }
  if (!apiKey) {
    throw new Error(
      formatMissingBuildBuddyApiKeyMessage({
        infisicalEnv,
        remoteExecutionMode,
      }),
    );
  }
  return {
    ...env,
    BUILD_BUDDY_API_KEY: apiKey,
    BUILDBUDDY_API_KEY: apiKey,
  };
}

function buildInvocationPhases({
  buildBuddyConfigArgs = [],
  command,
  layout,
  remoteExecutionMode,
  targets,
  env,
}) {
  const bazelJobs = parsePositiveInteger(env?.CTX_BAZEL_JOBS);
  if (targets.length === 0) {
    return [];
  }
  if (command === "run" || remoteExecutionMode === "off" || remoteExecutionMode === "cache") {
    return [
      {
        name: "local",
        commandArgs: buildPhaseCommandArgs({
          command,
          layout,
          extraConfigArgs: buildBuddyConfigArgs,
          bazelJobs,
        }),
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
          extraConfigArgs: [...buildBuddyConfigArgs, "--config=buildbuddy-rbe"],
          bazelJobs,
        }),
        targets,
      },
    ];
  }
  if (remoteExecutionMode === "darwin") {
    return [
      {
        name: "darwin-rbe",
        commandArgs: buildPhaseCommandArgs({
          command,
          layout,
          extraConfigArgs: [...buildBuddyConfigArgs, "--config=buildbuddy-darwin-rbe"],
          bazelJobs,
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
        extraConfigArgs: [...buildBuddyConfigArgs, "--config=buildbuddy-linux-rbe"],
        bazelJobs,
      }),
      targets: remoteTargets,
    });
  }
  if (localTargets.length > 0) {
    phases.push({
      name: "local",
      commandArgs: buildPhaseCommandArgs({
        command,
        layout,
        extraConfigArgs: buildBuddyConfigArgs,
        bazelJobs,
      }),
      targets: localTargets,
    });
  }
  return phases;
}

function buildBazelPilotInvocation({
  argv,
  buildBuddyApiKeyResolver = resolveBuildBuddyApiKey,
  env = process.env,
} = {}) {
  const parsed = parseArgs(argv || []);
  const { command, targets } = parsed;
  const coreRoot = path.resolve(__dirname, "..");
  const repoRoot = path.resolve(coreRoot, "..");
  const layout = resolveCtxCacheLayout({ cwd: coreRoot, env });
  const startupArgs = [];
  if (parseBatchMode(env.CTX_BAZEL_BATCH)) {
    startupArgs.push("--batch");
  }
  startupArgs.push(`--output_user_root=${layout.bazelOutputUserRoot}`);
  const remoteExecutionMode = parseRemoteExecutionMode(env.CTX_BAZEL_REMOTE_EXECUTION);
  const resolvedEnv = resolveBuildBuddyEnv({
    buildBuddyApiKeyResolver,
    env,
    remoteExecutionMode,
  });
  const localTestJobs = parsePositiveIntegerEnv(env.CTX_BAZEL_LOCAL_TEST_JOBS);
  const buildBuddyAuthArgs = buildBuildBuddyAuthArgs(resolvedEnv);
  const phases = buildInvocationPhases({
    buildBuddyConfigArgs: remoteExecutionMode === "off" ? [] : ["--config=buildbuddy-cache"],
    command,
    layout,
    remoteExecutionMode,
    targets,
    env,
  }).map((phase) => ({
    ...phase,
    commandArgs: [
      ...phase.commandArgs,
      ...(command === "test" && localTestJobs !== null
        ? [`--local_test_jobs=${localTestJobs}`]
        : []),
      ...buildBuddyAuthArgs,
    ],
  }));
  const commandArgs = phases[0]?.commandArgs || [];
  const phaseTargets = phases[0]?.targets || [];

  return {
    command,
    commandArgs,
    layout,
    phases,
    remoteExecutionMode,
    repoRoot,
    runArgs: parsed.runArgs || [],
    startupArgs,
    targets: phaseTargets,
    env: {
      ...resolvedEnv,
      BUILD_WORKSPACE_DIRECTORY: String(env.BUILD_WORKSPACE_DIRECTORY || repoRoot),
      TMPDIR: layout.tmpDir,
    },
  };
}

function resolveBazeliskCommand({
  repoRoot = path.resolve(__dirname, "..", ".."),
  env = process.env,
  fileExists = fs.existsSync,
  pathRunnable = pathIsRunnable,
  readDir = fs.readdirSync,
  commandAvailable = commandExists,
} = {}) {
  const pnpmBazeliskScript = resolvePnpmBazeliskScript({
    repoRoot,
    fileExists,
    readDir,
    pathRunnable,
  });
  if (pnpmBazeliskScript) {
    return pnpmBazeliskScript;
  }
  const repoLocalShim = path.join(repoRoot, "core", "node_modules", ".bin", BAZELISK_SHIM);
  if (fileExists(repoLocalShim) && pathRunnable(repoLocalShim)) {
    return repoLocalShim;
  }
  for (const candidate of [BAZELISK_SHIM, "bazel"]) {
    if (commandAvailable(candidate, { env, cwd: repoRoot })) {
      return candidate;
    }
  }
  return repoLocalShim;
}

function bazeliskBinaryPath({ repoRoot = path.resolve(__dirname, "..", ".."), env = process.env } = {}) {
  return resolveBazeliskCommand({ repoRoot, env });
}

function buildSpawnForPhase(invocation, phase) {
  const args = [...invocation.startupArgs, ...phase.commandArgs];
  let runScriptPath = "";
  if (invocation.command === "run") {
    runScriptPath = path.join(
      invocation.layout.tmpDir,
      `bazel-run-${process.pid}-${phase.name}.sh`,
    );
    args.push(`--script_path=${runScriptPath}`);
  }
  args.push(...phase.targets);
  return {
    command: bazeliskBinaryPath({ repoRoot: invocation.repoRoot, env: invocation.env }),
    args,
    options: {
      cwd: invocation.repoRoot,
      env: invocation.env,
      stdio: "inherit",
    },
    runScriptPath,
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
  let invocation;
  try {
    invocation = buildBazelPilotInvocation({ argv: process.argv.slice(2) });
  } catch (error) {
    console.error(String(error?.message || error));
    process.exit(1);
  }
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
    if (invocation.command === "run") {
      const runResult = childProcess.spawnSync(
        spawn.runScriptPath,
        invocation.runArgs,
        spawn.options,
      );
      if (runResult.error) {
        console.error(formatSpawnFailureMessage(spawn.runScriptPath, runResult.error));
        process.exit(1);
      }
      if (typeof runResult.status === "number" && runResult.status !== 0) {
        process.exit(runResult.status);
      }
      if (runResult.signal) {
        console.error(`error: Bazel run script terminated with signal ${runResult.signal}`);
        process.exit(1);
      }
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
  commandExists,
  buildBazelPilotSpawn,
  buildBazelPilotSpawns,
  buildBazelPilotInvocation,
  buildBuildBuddyAuthArgs,
  formatSpawnFailureMessage,
  parseArgs,
  parsePositiveIntegerEnv,
  parseBatchMode,
  parseRemoteExecutionMode,
  resolveBazeliskCommand,
};
