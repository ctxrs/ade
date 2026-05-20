#!/usr/bin/env node

const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const {
  DEFAULT_INFISICAL_ENV,
  resolveBuildBuddyApiKey,
} = require("./lib/buildbuddy_operator_env.cjs");
const {
  DEFAULT_BUILDBUDDY_API_BASE_URL,
  buildInvocationUrl,
} = require("./lib/buildbuddy_client.cjs");
const { removeLegacyTurboEnv, resolveCtxCacheLayout } = require("./lib/cache_roots.cjs");
const {
  getBazelBuildTargetsForCrates,
  getBazelCoveredCrates,
  getBazelTestTargetsForCrates,
  partitionBazelTargetsForLinuxRbe,
} = require("./lib/bazel_rust_targets.cjs");
const { HOST_HEAVY_BUDGET_KEY, withHostJobBudget } = require("./lib/host_job_budget.cjs");
const { appendHostSample } = require("./lib/verification_host_sampler.cjs");
const {
  createRunArtifacts,
  deriveBazelMetrics,
  finalizeRunArtifacts,
} = require("./lib/verification_run_store.cjs");

const DEFAULT_TEST_TARGETS = getBazelTestTargetsForCrates(getBazelCoveredCrates());
const DEFAULT_BUILD_TARGETS = getBazelBuildTargetsForCrates(getBazelCoveredCrates());
const BAZELISK_SHIM = process.platform === "win32" ? "bazelisk.cmd" : "bazelisk";
const BAZELISK_PACKAGE_DIR_PREFIX = "@bazel+bazelisk@";
const DARWIN_SERIAL_TEST_TARGETS = new Set([
  "//core/apps/web:unit_tests_non_pretext",
]);
const DARWIN_LOCAL_ONLY_TEST_TARGETS = new Set([
  "//core/apps/web/e2e:premerge_required",
  "//core/apps/web:unit_tests_non_pretext",
]);
const RUST_CLIPPY_BAZEL_ARGS = Object.freeze([
  "--aspects=@rules_rust//rust:defs.bzl%rust_clippy_aspect",
  "--output_groups=+clippy_checks",
]);

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

function parseRemoteExecutionMode(value, { platform = process.platform } = {}) {
  const normalized = String(value ?? "").trim().toLowerCase();
  if (!normalized) {
    if (platform === "darwin") {
      return "cache";
    }
    if (platform === "linux") {
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

function parseBazelTestTimeoutSeconds(env = process.env) {
  const rawValue = String(env?.CTX_BAZEL_TEST_TIMEOUT_SECONDS ?? "").trim();
  if (rawValue) {
    const normalized = rawValue.toLowerCase();
    if (["0", "false", "no", "off"].includes(normalized)) {
      return null;
    }
    if (!/^[1-9]\d*$/.test(rawValue)) {
      throw new Error(
        "error: CTX_BAZEL_TEST_TIMEOUT_SECONDS must be a positive integer number of seconds, or 0/off to disable.",
      );
    }
    return Number.parseInt(rawValue, 10);
  }
  return String(env?.CTX_VERIFY_PARENT_ENTRYPOINT || "").trim() ? 1200 : null;
}

function resolveLocalTestJobs({
  command,
  env = process.env,
  platform = process.platform,
  targets = [],
} = {}) {
  const explicitJobs = parsePositiveIntegerEnv(env.CTX_BAZEL_LOCAL_TEST_JOBS);
  if (explicitJobs !== null) {
    return explicitJobs;
  }
  if (
    command === "test"
    && platform === "darwin"
    && targets.some((target) => DARWIN_SERIAL_TEST_TARGETS.has(target))
  ) {
    return 1;
  }
  return null;
}

function resolveRemoteExecutionMode({
  command,
  env = process.env,
  platform = process.platform,
  targets = [],
} = {}) {
  const explicitMode = String(env.CTX_BAZEL_REMOTE_EXECUTION ?? "").trim();
  if (explicitMode) {
    return parseRemoteExecutionMode(explicitMode, { platform });
  }
  if (
    command === "test"
    && platform === "darwin"
    && targets.some((target) => DARWIN_LOCAL_ONLY_TEST_TARGETS.has(target))
  ) {
    return "off";
  }
  return parseRemoteExecutionMode(explicitMode, { platform });
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
  let rustClippy = false;
  const targets = [];
  for (const arg of rawArgs) {
    if (arg === "--rust-clippy") {
      if (command !== "build") {
        throw new Error("--rust-clippy is only supported for Bazel build invocations");
      }
      rustClippy = true;
      continue;
    }
    targets.push(arg);
  }
  return {
    command,
    rustClippy,
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

function buildPhaseCommandArgs({
  command,
  layout,
  extraConfigArgs = [],
  bazelJobs = null,
  bazelTestTimeoutSeconds = null,
  rustClippy = false,
}) {
  const commandArgs = [
    command,
    `--disk_cache=${layout.bazelDiskCacheDir}`,
    `--repository_cache=${layout.bazelRepositoryCacheDir}`,
    ...extraConfigArgs,
    ...(rustClippy ? RUST_CLIPPY_BAZEL_ARGS : []),
  ];
  if (bazelJobs != null) {
    commandArgs.push(`--jobs=${bazelJobs}`);
  }
  if (command === "test") {
    commandArgs.push("--test_output=errors");
    if (bazelTestTimeoutSeconds != null) {
      commandArgs.push(`--test_timeout=${bazelTestTimeoutSeconds}`);
    }
  }
  return commandArgs;
}

function redactCommandArgs(args) {
  return args.map((arg) => (
    String(arg).startsWith("--remote_header=x-buildbuddy-api-key=")
      ? "--remote_header=x-buildbuddy-api-key=<redacted>"
      : arg
  ));
}

function telemetryDisabled(env = process.env) {
  return ["1", "true", "yes", "on"].includes(String(env.CTX_DISABLE_VERIFICATION_TELEMETRY || "").trim().toLowerCase());
}

function resolveBuildBuddyApiBaseUrl(env = process.env) {
  return String(
    env.BUILDBUDDY_API_BASE_URL
    || env.BUILD_BUDDY_API_BASE_URL
    || DEFAULT_BUILDBUDDY_API_BASE_URL,
  ).trim() || DEFAULT_BUILDBUDDY_API_BASE_URL;
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
  rustClippy = false,
  targets,
  env,
}) {
  const bazelJobs = parsePositiveInteger(env?.CTX_BAZEL_JOBS);
  const bazelTestTimeoutSeconds = parseBazelTestTimeoutSeconds(env);
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
        bazelTestTimeoutSeconds,
        rustClippy,
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
        bazelTestTimeoutSeconds,
        rustClippy,
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
        bazelTestTimeoutSeconds,
        rustClippy,
      }),
      targets,
    },
    ];
  }
  const { remoteTargets, localTargets } = partitionBazelTargetsForLinuxRbe(command, targets, {
    rustClippy,
  });
  const phases = [];
  if (remoteTargets.length > 0) {
    phases.push({
      name: "linux-rbe",
      commandArgs: buildPhaseCommandArgs({
        command,
        layout,
        extraConfigArgs: [...buildBuddyConfigArgs, "--config=buildbuddy-linux-rbe"],
        bazelJobs,
        bazelTestTimeoutSeconds,
        rustClippy,
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
        bazelTestTimeoutSeconds,
        rustClippy,
      }),
      targets: localTargets,
    });
  }
  return phases;
}

function resolvePhaseBudgetKey(phase) {
  return phase?.name === "local" ? HOST_HEAVY_BUDGET_KEY : null;
}

function createBuildBuddyPhaseMetadata({ enabled, apiBaseUrl }) {
  if (!enabled) {
    return null;
  }
  const invocationId = crypto.randomUUID();
  return {
    invocationId,
    invocationUrl: buildInvocationUrl(invocationId, apiBaseUrl),
  };
}

function buildBazelPilotInvocation({
  argv,
  buildBuddyApiKeyResolver = resolveBuildBuddyApiKey,
  env = process.env,
  platform = process.platform,
} = {}) {
  const sanitizedEnv = removeLegacyTurboEnv({ ...env });
  const parsed = parseArgs(argv || []);
  const { command, rustClippy = false, targets } = parsed;
  const coreRoot = path.resolve(__dirname, "..");
  const repoRoot = path.resolve(coreRoot, "..");
  const layout = resolveCtxCacheLayout({ cwd: coreRoot, env: sanitizedEnv });
  const startupArgs = [];
  if (parseBatchMode(sanitizedEnv.CTX_BAZEL_BATCH, { platform })) {
    startupArgs.push("--batch");
  }
  startupArgs.push(`--output_user_root=${layout.bazelOutputUserRoot}`);
  const remoteExecutionMode = resolveRemoteExecutionMode({
    command,
    env: sanitizedEnv,
    platform,
    targets,
  });
  const resolvedEnv = resolveBuildBuddyEnv({
    buildBuddyApiKeyResolver,
    env: sanitizedEnv,
    remoteExecutionMode,
  });
  let telemetry = null;
  if (!telemetryDisabled(sanitizedEnv)) {
    try {
      telemetry = createRunArtifacts({
        cwd: coreRoot,
        env: resolvedEnv,
        entrypoint: "run_bazel_pilot",
        kind: "bazel",
        layout,
      });
      ensureDir(telemetry.runDir);
    } catch (error) {
      console.error(`warning: failed to initialize Bazel telemetry: ${String(error?.message || error)}`);
      telemetry = null;
    }
  }
  const localTestJobs = resolveLocalTestJobs({
    command,
    env: sanitizedEnv,
    platform,
    targets,
  });
  const bazelTestTimeoutSeconds = parseBazelTestTimeoutSeconds(sanitizedEnv);
  const buildBuddyAuthArgs = buildBuildBuddyAuthArgs(resolvedEnv);
  const buildBuddyEnabled = remoteExecutionMode !== "off";
  const buildBuddyApiBaseUrl = resolveBuildBuddyApiBaseUrl(resolvedEnv);
  const phases = buildInvocationPhases({
    buildBuddyConfigArgs: remoteExecutionMode === "off" ? [] : ["--config=buildbuddy-cache"],
    command,
    layout,
    remoteExecutionMode,
    rustClippy,
    targets,
    env: sanitizedEnv,
  }).map((phase, phaseIndex) => {
    const buildBuddy = createBuildBuddyPhaseMetadata({
      enabled: buildBuddyEnabled,
      apiBaseUrl: buildBuddyApiBaseUrl,
    });
    const bepJsonPath = path.join(
      telemetry?.runDir || layout.tmpDir,
      `bazel-bep-${String(phaseIndex + 1).padStart(2, "0")}-${phase.name}.jsonl`,
    );
    return {
      ...phase,
      bepJsonPath,
      budgetKey: resolvePhaseBudgetKey(phase),
      buildBuddy,
      commandArgs: [
        ...phase.commandArgs,
        `--build_event_json_file=${bepJsonPath}`,
        ...(command === "test" && localTestJobs !== null
          ? [`--local_test_jobs=${localTestJobs}`]
          : []),
        ...(buildBuddy == null ? [] : [`--invocation_id=${buildBuddy.invocationId}`]),
        ...buildBuddyAuthArgs,
      ],
    };
  });
  const commandArgs = phases[0]?.commandArgs || [];
  const phaseTargets = phases[0]?.targets || [];

  return {
    command,
    commandArgs,
    layout,
    phases,
    buildBuddyApiBaseUrl,
    buildBuddyEnabled,
    bazelTestTimeoutSeconds,
    remoteExecutionMode,
    repoRoot,
    runArgs: parsed.runArgs || [],
    rustClippy,
    startupArgs,
    telemetry,
    targets: phaseTargets,
    env: {
      ...resolvedEnv,
      BUILD_WORKSPACE_DIRECTORY: String(sanitizedEnv.BUILD_WORKSPACE_DIRECTORY || repoRoot),
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
  const repoLocalShim = path.join(repoRoot, "core", "node_modules", ".bin", BAZELISK_SHIM);
  if (fileExists(repoLocalShim) && pathRunnable(repoLocalShim)) {
    return repoLocalShim;
  }
  const pnpmBazeliskScript = resolvePnpmBazeliskScript({
    repoRoot,
    fileExists,
    readDir,
    pathRunnable,
  });
  if (pnpmBazeliskScript) {
    return pnpmBazeliskScript;
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

const BEP_CACHE_STAT_KEYS = Object.freeze({
  actionCacheHits: "actionCacheHits",
  actionCacheMisses: "actionCacheMisses",
  action_cache_hits: "actionCacheHits",
  action_cache_misses: "actionCacheMisses",
  actionsCacheHits: "actionCacheHits",
  actionsCacheMisses: "actionCacheMisses",
  actionsCreated: "actionsCreated",
  actionsExecuted: "actionsExecuted",
  actions_created: "actionsCreated",
  actions_executed: "actionsExecuted",
  casCacheHits: "casCacheHits",
  casCacheMisses: "casCacheMisses",
  cas_cache_hits: "casCacheHits",
  cas_cache_misses: "casCacheMisses",
  remoteCacheHits: "actionCacheHits",
});

function toBepNumber(value) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  const normalized = String(value ?? "").trim();
  if (!/^\d+$/u.test(normalized)) {
    return null;
  }
  return Number.parseInt(normalized, 10);
}

function collectBepCacheStats(value, stats, depth = 0) {
  if (value == null || depth > 20) {
    return;
  }
  if (Array.isArray(value)) {
    for (const entry of value) {
      collectBepCacheStats(entry, stats, depth + 1);
    }
    return;
  }
  if (typeof value !== "object") {
    return;
  }
  for (const [key, entry] of Object.entries(value)) {
    const mappedKey = BEP_CACHE_STAT_KEYS[key];
    const numericValue = toBepNumber(entry);
    if (mappedKey && numericValue != null) {
      stats[mappedKey] = (stats[mappedKey] || 0) + numericValue;
    }
    if (entry && typeof entry === "object") {
      collectBepCacheStats(entry, stats, depth + 1);
    }
  }
}

function normalizeBepCacheStats(stats) {
  const normalized = {};
  for (const key of [
    "actionCacheHits",
    "actionCacheMisses",
    "actionsCreated",
    "actionsExecuted",
    "casCacheHits",
    "casCacheMisses",
  ]) {
    if (Object.hasOwn(stats, key)) {
      normalized[key] = Number(stats[key] || 0);
    }
  }
  if (!Object.hasOwn(normalized, "actionCacheMisses") && Number(normalized.actionsExecuted || 0) > 0) {
    normalized.actionCacheMisses = Number(normalized.actionsExecuted);
  }
  if (
    !Object.hasOwn(normalized, "actionCacheHits")
    && Object.hasOwn(normalized, "actionsCreated")
    && Object.hasOwn(normalized, "actionsExecuted")
  ) {
    normalized.actionCacheHits = Math.max(
      0,
      Number(normalized.actionsCreated || 0) - Number(normalized.actionsExecuted || 0),
    );
  }
  return Object.keys(normalized).length > 0 ? normalized : null;
}

function readBazelBuildEventMetrics(bepJsonPath, {
  existsSyncImpl = fs.existsSync,
  readFileSyncImpl = fs.readFileSync,
} = {}) {
  const normalizedPath = String(bepJsonPath || "").trim();
  if (!normalizedPath || !existsSyncImpl(normalizedPath)) {
    return null;
  }
  const stats = {};
  for (const line of String(readFileSyncImpl(normalizedPath, "utf8") || "").split(/\r?\n/u)) {
    const trimmed = line.trim();
    if (!trimmed) {
      continue;
    }
    try {
      collectBepCacheStats(JSON.parse(trimmed), stats);
    } catch {
      // Bazel BEP JSON is newline-delimited. Ignore malformed partial lines from interrupted runs.
    }
  }
  return normalizeBepCacheStats(stats);
}

function mergeCacheStats(entries) {
  const stats = {};
  for (const entry of entries) {
    if (!entry || typeof entry !== "object") {
      continue;
    }
    for (const [key, value] of Object.entries(entry)) {
      const numericValue = toBepNumber(value);
      if (numericValue != null) {
        stats[key] = (stats[key] || 0) + numericValue;
      }
    }
  }
  return Object.keys(stats).length > 0 ? stats : null;
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

function createPhaseFailureResult({ command = "", error, invocation, phase, startedAt, startedMs }) {
  const durationMs = Math.max(0, Date.now() - startedMs);
  return {
    command,
    commandArgs: [],
    durationMs,
    error: String(error?.message || error || "unknown phase failure"),
    name: phase.name,
    queueTimeMs: durationMs,
    runArgs: invocation.command === "run" ? invocation.runArgs : [],
    startedAt,
    status: 1,
    signal: "",
    targets: phase.targets,
  };
}

function buildBazelPilotSummary(invocation, phaseResults, exitCode) {
  const summarizeTargets = (phaseName) => phaseResults
    .filter((phase) => phase.name === phaseName)
    .reduce((total, phase) => total + (Array.isArray(phase.targets) ? phase.targets.length : 0), 0);
  const localPhases = phaseResults.filter((phase) => phase.name === "local");
  const remotePhases = phaseResults.filter((phase) => phase.name !== "local");
  const cacheStats = mergeCacheStats(phaseResults.map((phase) => phase.cacheStats));
  const summary = {
    buildBuddyInvocations: phaseResults
      .filter((phase) => String(phase.buildBuddyInvocationId || "").trim())
      .map((phase) => ({
        phaseName: phase.name,
        invocationId: phase.buildBuddyInvocationId,
        invocationUrl: phase.buildBuddyInvocationUrl,
      })),
    command: invocation.command,
    cacheStats,
    durationMs: phaseResults.reduce((total, phase) => total + Number(phase.durationMs || 0), 0),
    entrypoint: "run_bazel_pilot",
    exitCode,
    kind: "bazel",
    bazelTestTimeoutSeconds: invocation.bazelTestTimeoutSeconds ?? null,
    rustClippy: Boolean(invocation.rustClippy),
    localPhaseCount: localPhases.length,
    localTargetCount: summarizeTargets("local"),
    localSpill: ["all", "linux", "darwin"].includes(String(invocation.remoteExecutionMode || "")) && localPhases.length > 0,
    parentEntrypoint: String(invocation.env.CTX_VERIFY_PARENT_ENTRYPOINT || "").trim(),
    parentRunId: String(invocation.env.CTX_VERIFY_PARENT_RUN_ID || "").trim(),
    phaseCount: phaseResults.length,
    phases: phaseResults.map((phase) => ({
      buildBuddyInvocationId: String(phase.buildBuddyInvocationId || ""),
      buildBuddyInvocationUrl: String(phase.buildBuddyInvocationUrl || ""),
      cacheStats: phase.cacheStats || null,
      durationMs: Number(phase.durationMs || 0),
      name: phase.name,
      queueTimeMs: Number(phase.queueTimeMs || 0),
      status: Number(phase.status || 0),
      targets: Array.isArray(phase.targets) ? [...phase.targets] : [],
      targetCount: Array.isArray(phase.targets) ? phase.targets.length : 0,
    })),
    remoteExecutionMode: invocation.remoteExecutionMode,
    remotePhaseCount: remotePhases.length,
    remoteTargetCount: remotePhases.reduce(
      (total, phase) => total + (Array.isArray(phase.targets) ? phase.targets.length : 0),
      0,
    ),
    runId: String(invocation.telemetry?.runId || ""),
    success: exitCode === 0,
  };
  return {
    ...summary,
    ...deriveBazelMetrics(summary),
  };
}

function formatBazelPilotSummaryLine(summary) {
  return `CTX_BAZEL_PILOT_SUMMARY ${JSON.stringify(summary)}`;
}

function runBazelPilotInvocationPhases(invocation, {
  exitImpl = process.exit,
  emitSummaryImpl = (line) => process.stderr.write(`${line}\n`),
  logErrorImpl = console.error,
  readBazelBuildEventMetricsImpl = readBazelBuildEventMetrics,
  spawnSyncImpl = childProcess.spawnSync,
  withHostJobBudgetImpl = withHostJobBudget,
} = {}) {
  const phaseResults = [];
  const startedAt = new Date().toISOString();
  let exitCode = 0;
  const finalizeTelemetry = () => {
    if (invocation.telemetry == null) {
      return;
    }
    try {
      finalizeRunArtifacts(invocation.telemetry, {
        startedAt,
        completedAt: new Date().toISOString(),
        durationMs: phaseResults.reduce((total, phase) => total + Number(phase.durationMs || 0), 0),
        success: exitCode === 0,
        bazelCommand: invocation.command,
        bazelTestTimeoutSeconds: invocation.bazelTestTimeoutSeconds ?? null,
        buildBuddyApiBaseUrl: invocation.buildBuddyApiBaseUrl,
        buildBuddyEnabled: invocation.buildBuddyEnabled,
        cacheStats: mergeCacheStats(phaseResults.map((phase) => phase.cacheStats)),
        buildBuddyInvocations: phaseResults
          .filter((phase) => String(phase.buildBuddyInvocationId || "").trim())
          .map((phase) => ({
            phaseName: phase.name,
            invocationId: phase.buildBuddyInvocationId,
            invocationUrl: phase.buildBuddyInvocationUrl,
          })),
        parentEntrypoint: String(invocation.env.CTX_VERIFY_PARENT_ENTRYPOINT || "").trim(),
        parentRunId: String(invocation.env.CTX_VERIFY_PARENT_RUN_ID || "").trim(),
        remoteExecutionMode: invocation.remoteExecutionMode,
        phaseCount: invocation.phases.length,
        targets: invocation.phases.flatMap((phase) => phase.targets),
        phases: phaseResults,
      }, { env: invocation.env });
    } catch (error) {
      logErrorImpl(`warning: failed to record Bazel telemetry: ${String(error?.message || error)}`);
    }
  };

  try {
    for (const phase of invocation.phases) {
      if (exitCode !== 0) {
        break;
      }
      const phaseStartedAt = new Date().toISOString();
      const phaseStartedMs = Date.now();
      const runPhase = () => {
        if (invocation.telemetry != null) {
          appendHostSample(invocation.telemetry.hostSamplesPath, `phase:start:${phase.name}`);
        }
        const queueTimeMs = Math.max(0, Date.now() - phaseStartedMs);
        const spawn = buildSpawnForPhase(invocation, phase);
        const result = spawnSyncImpl(spawn.command, spawn.args, spawn.options);
        const phaseResult = {
          command: spawn.command,
          commandArgs: redactCommandArgs(spawn.args),
          durationMs: Date.now() - phaseStartedMs,
          name: phase.name,
          queueTimeMs,
          runArgs: invocation.command === "run" ? invocation.runArgs : [],
          startedAt: phaseStartedAt,
          targets: phase.targets,
          signal: result.signal || "",
          status: typeof result.status === "number" ? result.status : result.error ? 1 : 0,
        };
        const cacheStats = readBazelBuildEventMetricsImpl(phase.bepJsonPath);
        if (cacheStats) {
          phaseResult.cacheStats = cacheStats;
        }
        if (result.error) {
          phaseResult.error = String(result.error.message || result.error);
          phaseResults.push(phaseResult);
          if (invocation.telemetry != null) {
            appendHostSample(invocation.telemetry.hostSamplesPath, `phase:end:${phase.name}`);
          }
          logErrorImpl(formatSpawnFailureMessage(spawn.command, result.error));
          exitCode = 1;
          return;
        }
        if (phase.buildBuddy != null) {
          phaseResult.buildBuddyInvocationId = phase.buildBuddy.invocationId;
          phaseResult.buildBuddyInvocationUrl = phase.buildBuddy.invocationUrl;
        }
        if (typeof result.status === "number" && result.status !== 0) {
          phaseResults.push(phaseResult);
          if (invocation.telemetry != null) {
            appendHostSample(invocation.telemetry.hostSamplesPath, `phase:end:${phase.name}`);
          }
          exitCode = result.status;
          return;
        }
        if (result.signal) {
          phaseResults.push(phaseResult);
          if (invocation.telemetry != null) {
            appendHostSample(invocation.telemetry.hostSamplesPath, `phase:end:${phase.name}`);
          }
          logErrorImpl(`error: Bazelisk terminated with signal ${result.signal}`);
          exitCode = 1;
          return;
        }
        if (invocation.command === "run") {
          if (invocation.telemetry != null) {
            appendHostSample(invocation.telemetry.hostSamplesPath, `phase:run-script:start:${phase.name}`);
          }
          const runStartedMs = Date.now();
          const runResult = spawnSyncImpl(
            spawn.runScriptPath,
            invocation.runArgs,
            spawn.options,
          );
          phaseResult.runScriptDurationMs = Date.now() - runStartedMs;
          phaseResult.durationMs += phaseResult.runScriptDurationMs;
          if (runResult.error) {
            phaseResult.runScriptError = String(runResult.error.message || runResult.error);
            phaseResults.push(phaseResult);
            if (invocation.telemetry != null) {
              appendHostSample(invocation.telemetry.hostSamplesPath, `phase:end:${phase.name}`);
            }
            logErrorImpl(formatSpawnFailureMessage(spawn.runScriptPath, runResult.error));
            exitCode = 1;
            return;
          }
          if (typeof runResult.status === "number" && runResult.status !== 0) {
            phaseResult.runScriptStatus = runResult.status;
            phaseResults.push(phaseResult);
            if (invocation.telemetry != null) {
              appendHostSample(invocation.telemetry.hostSamplesPath, `phase:end:${phase.name}`);
            }
            exitCode = runResult.status;
            return;
          }
          if (runResult.signal) {
            phaseResult.runScriptSignal = runResult.signal;
            phaseResults.push(phaseResult);
            if (invocation.telemetry != null) {
              appendHostSample(invocation.telemetry.hostSamplesPath, `phase:end:${phase.name}`);
            }
            logErrorImpl(`error: Bazel run script terminated with signal ${runResult.signal}`);
            exitCode = 1;
            return;
          }
        }
        phaseResults.push(phaseResult);
        if (invocation.telemetry != null) {
          appendHostSample(invocation.telemetry.hostSamplesPath, `phase:end:${phase.name}`);
        }
      };
      try {
        if (phase.budgetKey) {
          withHostJobBudgetImpl({
            budgetKey: phase.budgetKey,
            command: `bazel ${invocation.command} ${phase.targets.join(" ")}`,
            cwd: invocation.repoRoot,
            env: invocation.env,
          }, runPhase);
        } else {
          runPhase();
        }
      } catch (error) {
        if (invocation.telemetry != null) {
          appendHostSample(invocation.telemetry.hostSamplesPath, `phase:end:${phase.name}`);
        }
        const phaseResult = createPhaseFailureResult({
          command: phase.budgetKey ? `budget:${phase.budgetKey}` : "",
          error,
          invocation,
          phase,
          startedAt: phaseStartedAt,
          startedMs: phaseStartedMs,
        });
        phaseResults.push(phaseResult);
        logErrorImpl(`error: failed to run Bazel phase ${phase.name}: ${String(error?.message || error)}`);
        exitCode = 1;
      }
    }
  } finally {
    finalizeTelemetry();
  }
  emitSummaryImpl(
    formatBazelPilotSummaryLine(buildBazelPilotSummary(invocation, phaseResults, exitCode)),
  );
  if (exitCode !== 0) {
    exitImpl(exitCode);
  }
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
  runBazelPilotInvocationPhases(invocation);
}

if (require.main === module) {
  main();
}

module.exports = {
  DEFAULT_BUILD_TARGETS,
  DEFAULT_TEST_TARGETS,
  RUST_CLIPPY_BAZEL_ARGS,
  bazeliskBinaryPath,
  commandExists,
  buildBazelPilotSpawn,
  buildBazelPilotSpawns,
  buildBazelPilotInvocation,
  buildInvocationPhases,
  buildBuildBuddyAuthArgs,
  buildPhaseCommandArgs,
  formatSpawnFailureMessage,
  parseArgs,
  parseBazelTestTimeoutSeconds,
  parsePositiveIntegerEnv,
  parseBatchMode,
  parseRemoteExecutionMode,
  readBazelBuildEventMetrics,
  resolveLocalTestJobs,
  resolveRemoteExecutionMode,
  resolvePhaseBudgetKey,
  resolveBuildBuddyApiBaseUrl,
  resolveBazeliskCommand,
  buildBazelPilotSummary,
  formatBazelPilotSummaryLine,
  runBazelPilotInvocationPhases,
};
