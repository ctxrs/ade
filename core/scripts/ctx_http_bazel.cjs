#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { readDesktopVersion } = require("./desktop_version.cjs");
const {
  ensureWebDistArtifact,
  resolveDirectRunBazelVersion,
} = require("./lib/web_dist_cache.cjs");
const { resolveCtxCacheLayout } = require("./lib/cache_roots.cjs");
const { HOST_HEAVY_BUDGET_KEY, withHostJobBudget } = require("./lib/host_job_budget.cjs");
const { bazeliskBinaryPath, buildBuildBuddyAuthArgs } = require("./run_bazel_pilot.cjs");

const DEFAULT_PROFILE = "release";
const PREPARE_DESKTOP_SIDECARS_COMMAND = "prepare-desktop-sidecars";
const PRINT_DESKTOP_SIDECAR_ENV_COMMAND = "print-sidecar-env";
const BAZEL_PILOT_SCRIPT_PATH = path.join("core", "scripts", "run_bazel_pilot.cjs");
const DESKTOP_SIDECAR_TARGETS = Object.freeze({
  ctxBin: "//core/crates/ctx-http:ctx",
  ctxMcpBin: "//core/crates/ctx-mcp:ctx-mcp",
  avfLinuxHelperBin: "//core/apps/desktop/src-tauri/src:ctx-avf-linux-helper",
});
const TARGET_SPECS = Object.freeze({
  "darwin-aarch64": Object.freeze({
    platformLabel: "//tools/bazel/platforms:darwin_arm64",
  }),
  "darwin-x86_64": Object.freeze({
    platformLabel: "//tools/bazel/platforms:darwin_x86_64",
  }),
  "linux-aarch64": Object.freeze({
    platformLabel: "//tools/bazel/platforms:linux_arm64",
  }),
  "linux-x86_64": Object.freeze({
    platformLabel: "//tools/bazel/platforms:linux_x86_64",
  }),
});

function repoRoots() {
  const coreRoot = path.resolve(__dirname, "..");
  return {
    coreRoot,
    repoRoot: path.resolve(coreRoot, ".."),
  };
}

function parseArgs(argv) {
  const args = {
    command: PREPARE_DESKTOP_SIDECARS_COMMAND,
    profile: DEFAULT_PROFILE,
    targetKey: "",
  };
  const [command = PREPARE_DESKTOP_SIDECARS_COMMAND, ...rest] = argv;
  args.command = String(command || "").trim() || PREPARE_DESKTOP_SIDECARS_COMMAND;
  for (let index = 0; index < rest.length; index += 1) {
    const arg = rest[index];
    if (arg === "--profile") {
      args.profile = String(rest[index + 1] || "").trim();
      index += 1;
      continue;
    }
    if (arg === "--target-key") {
      args.targetKey = String(rest[index + 1] || "").trim();
      index += 1;
      continue;
    }
    throw new Error(`unsupported arg: ${arg}`);
  }
  if (![PREPARE_DESKTOP_SIDECARS_COMMAND, PRINT_DESKTOP_SIDECAR_ENV_COMMAND].includes(args.command)) {
    throw new Error(`unsupported ctx-http Bazel command: ${args.command}`);
  }
  if (!["debug", "release"].includes(args.profile)) {
    throw new Error(`unsupported --profile '${args.profile}'`);
  }
  if (args.targetKey && !TARGET_SPECS[args.targetKey]) {
    throw new Error(`unsupported ctx-http Bazel target '${args.targetKey}'`);
  }
  return args;
}

function buildBazelCommandContext(env = process.env) {
  const { coreRoot, repoRoot } = repoRoots();
  const layout = resolveCtxCacheLayout({ cwd: coreRoot, env });
  fs.mkdirSync(layout.tmpDir, { recursive: true });
  fs.mkdirSync(layout.bazelOutputUserRoot, { recursive: true });
  fs.mkdirSync(layout.bazelDiskCacheDir, { recursive: true });
  fs.mkdirSync(layout.bazelRepositoryCacheDir, { recursive: true });
  const bazelEnv = {
    ...env,
    TMPDIR: layout.tmpDir,
  };
  const directRunBazelVersion = resolveDirectRunBazelVersion({ repoRoot, env: bazelEnv });
  if (directRunBazelVersion) {
    bazelEnv.USE_BAZEL_VERSION = directRunBazelVersion;
  }
  return {
    bazelBinary: bazeliskBinaryPath({ repoRoot, env: bazelEnv }),
    bazelCommandArgs: [
      `--disk_cache=${layout.bazelDiskCacheDir}`,
      `--repository_cache=${layout.bazelRepositoryCacheDir}`,
    ],
    env: bazelEnv,
    repoRoot,
    startupArgs: [`--output_user_root=${layout.bazelOutputUserRoot}`],
  };
}

function runChecked(command, args, options, failureMessage, spawnSyncImpl = childProcess.spawnSync) {
  const result = spawnSyncImpl(command, args, options);
  if (result.error) {
    throw result.error;
  }
  if (typeof result.status === "number" && result.status !== 0) {
    throw new Error(failureMessage || `${command} ${args.join(" ")} failed with status ${result.status}`);
  }
  if (result.signal) {
    throw new Error(failureMessage || `${command} ${args.join(" ")} terminated with signal ${result.signal}`);
  }
  return result;
}

function buildBazelPlatformArgs(targetKey = "") {
  const normalizedTargetKey = String(targetKey || "").trim();
  if (!normalizedTargetKey) {
    return [];
  }
  const targetSpec = TARGET_SPECS[normalizedTargetKey];
  if (!targetSpec) {
    throw new Error(`unsupported ctx-http Bazel target '${targetKey}'`);
  }
  return [`--platforms=${targetSpec.platformLabel}`];
}

function shouldResolveAvfLinuxHelper(targetKey = "", platform = process.platform) {
  const normalizedTargetKey = String(targetKey || "").trim();
  if (normalizedTargetKey) {
    return normalizedTargetKey.startsWith("darwin-");
  }
  return platform === "darwin";
}

function buildTargetsViaBazel(
  targets,
  {
    env = process.env,
    targetKey = "",
    quietStdout = false,
    spawnSyncImpl = childProcess.spawnSync,
    withHostJobBudgetImpl = withHostJobBudget,
  } = {},
) {
  const {
    bazelBinary,
    bazelCommandArgs,
    env: bazelEnv,
    repoRoot,
    startupArgs,
  } = buildBazelCommandContext(env);
  withHostJobBudgetImpl({
    budgetKey: HOST_HEAVY_BUDGET_KEY,
    command: `bazel build ${targets.join(" ")}`,
    cwd: repoRoot,
    env: bazelEnv,
  }, () => {
    runChecked(
      bazelBinary,
      [
        ...startupArgs,
        "build",
        ...bazelCommandArgs,
        ...buildBazelPlatformArgs(targetKey),
        ...buildBuildBuddyAuthArgs(bazelEnv),
        ...targets,
      ],
      {
        cwd: repoRoot,
        env: bazelEnv,
        stdio: quietStdout ? ["ignore", "ignore", "inherit"] : "inherit",
      },
      `bazel build ${targets.join(" ")} failed`,
      spawnSyncImpl,
    );
  });
}

function parseBazelOutputPaths(stdout, { repoRoot, targets }) {
  const lines = String(stdout || "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);

  const resolved = new Map();
  for (const line of lines) {
    const separatorIndex = line.indexOf("|");
    if (separatorIndex <= 0) {
      throw new Error(`unexpected Bazel cquery output line: ${line}`);
    }
    const label = line.slice(0, separatorIndex).trim().replace(/^@+/, "");
    const outputPath = line.slice(separatorIndex + 1).trim();
    if (!outputPath) {
      throw new Error(`expected exactly one Bazel output for ${label}, got 0`);
    }
    if (resolved.has(label)) {
      throw new Error(`expected exactly one Bazel output for ${label}, got multiple`);
    }
    resolved.set(label, path.resolve(repoRoot, outputPath));
  }

  for (const target of targets) {
    if (!resolved.has(target)) {
      throw new Error(`missing Bazel output for ${target}`);
    }
  }

  return resolved;
}

function resolveBazelOutputPaths(
  targets,
  {
    env = process.env,
    targetKey = "",
    spawnSyncImpl = childProcess.spawnSync,
    withHostJobBudgetImpl = withHostJobBudget,
  } = {},
) {
  const {
    bazelBinary,
    bazelCommandArgs,
    env: bazelEnv,
    repoRoot,
    startupArgs,
  } = buildBazelCommandContext(env);
  const queryExpression = `set(${targets.join(" ")})`;
  let result = null;
  withHostJobBudgetImpl({
    budgetKey: HOST_HEAVY_BUDGET_KEY,
    command: `bazel cquery ${targets.join(" ")}`,
    cwd: repoRoot,
    env: bazelEnv,
  }, () => {
    result = runChecked(
      bazelBinary,
      [
        ...startupArgs,
        "cquery",
        ...bazelCommandArgs,
        ...buildBazelPlatformArgs(targetKey),
        "--output=starlark",
        "--starlark:expr=str(target.label) + \"|\" + \"\\n\".join([f.path for f in target.files.to_list()])",
        ...buildBuildBuddyAuthArgs(bazelEnv),
        queryExpression,
      ],
      {
        cwd: repoRoot,
        env: bazelEnv,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      },
      `bazel cquery ${targets.join(" ")} failed`,
      spawnSyncImpl,
    );
  });
  return parseBazelOutputPaths(result.stdout, { repoRoot, targets });
}

function resolveDesktopSidecarPaths({ env = process.env, targetKey = "" } = {}) {
  const targets = [DESKTOP_SIDECAR_TARGETS.ctxBin, DESKTOP_SIDECAR_TARGETS.ctxMcpBin];
  const resolveAvfLinuxHelper = shouldResolveAvfLinuxHelper(targetKey, env.CTX_HTTP_BAZEL_HOST_PLATFORM);
  if (resolveAvfLinuxHelper) {
    targets.push(DESKTOP_SIDECAR_TARGETS.avfLinuxHelperBin);
  }
  buildTargetsViaBazel(targets, { env, targetKey, quietStdout: true });
  const resolvedPaths = resolveBazelOutputPaths(targets, { env, targetKey });
  return {
    ctxBinPath: resolvedPaths.get(DESKTOP_SIDECAR_TARGETS.ctxBin),
    ctxMcpBinPath: resolvedPaths.get(DESKTOP_SIDECAR_TARGETS.ctxMcpBin),
    avfLinuxHelperBinPath: resolveAvfLinuxHelper
      ? resolvedPaths.get(DESKTOP_SIDECAR_TARGETS.avfLinuxHelperBin)
      : "",
  };
}

function buildDesktopSyncEnv({
  env = process.env,
  ctxBinPath,
  ctxMcpBinPath,
  avfLinuxHelperBinPath = "",
  desktopWebDist = "",
  profile,
} = {}) {
  return {
    ...env,
    CTX_DESKTOP_CTX_BIN: ctxBinPath,
    CTX_DESKTOP_CTX_MCP_BIN: ctxMcpBinPath,
    CTX_DESKTOP_SYNC_BUNDLES: "0",
    CTX_DESKTOP_SYNC_PROFILE: profile,
    ...(String(avfLinuxHelperBinPath || "").trim()
      ? { CTX_DESKTOP_AVF_LINUX_HELPER_BIN: String(avfLinuxHelperBinPath).trim() }
      : {}),
    ...(String(desktopWebDist || "").trim() ? { CTX_DESKTOP_WEB_DIST: String(desktopWebDist).trim() } : {}),
  };
}

function resolveDesktopWebDist({ env = process.env, profile } = {}) {
  const configured = String(env.CTX_DESKTOP_WEB_DIST || "").trim();
  if (configured) {
    return configured;
  }
  const { coreRoot } = repoRoots();
  const desktopVersion = readDesktopVersion(coreRoot);
  return ensureWebDistArtifact({
    coreRoot,
    env,
    appVersion: desktopVersion,
    variant: `desktop-sidecars-${profile}`,
  }).distDir;
}

function syncDesktopResources({
  env = process.env,
  profile,
  ctxBinPath,
  ctxMcpBinPath,
  avfLinuxHelperBinPath = "",
}) {
  const { repoRoot } = repoRoots();
  const desktopWebDist = resolveDesktopWebDist({ env, profile });
  runChecked(
    "node",
    ["core/scripts/desktop_sync_resources.cjs", "--profile", profile],
    {
      cwd: repoRoot,
      env: buildDesktopSyncEnv({
        env,
        ctxBinPath,
        ctxMcpBinPath,
        avfLinuxHelperBinPath,
        desktopWebDist,
        profile,
      }),
      stdio: "inherit",
    },
    "desktop sidecar sync failed",
  );
}

function prepareDesktopSidecars({ env = process.env, profile = DEFAULT_PROFILE } = {}) {
  const targetKey = String(env.CTX_HTTP_BAZEL_TARGET_KEY || "").trim();
  const { ctxBinPath, ctxMcpBinPath, avfLinuxHelperBinPath } = resolveDesktopSidecarPaths({ env, targetKey });
  syncDesktopResources({
    env,
    profile,
    ctxBinPath,
    ctxMcpBinPath,
    avfLinuxHelperBinPath,
  });
  return {
    ctxBinPath,
    ctxMcpBinPath,
    avfLinuxHelperBinPath,
  };
}

function shellQuote(value) {
  return `'${String(value || "").replace(/'/g, `'\"'\"'`)}'`;
}

function renderDesktopSidecarEnv({ ctxBinPath, ctxMcpBinPath, avfLinuxHelperBinPath = "" }) {
  return [
    `export CTX_DESKTOP_CTX_BIN=${shellQuote(ctxBinPath)}`,
    `export CTX_DESKTOP_CTX_MCP_BIN=${shellQuote(ctxMcpBinPath)}`,
    ...(String(avfLinuxHelperBinPath || "").trim()
      ? [`export CTX_DESKTOP_AVF_LINUX_HELPER_BIN=${shellQuote(avfLinuxHelperBinPath)}`]
      : []),
  ].join("\n");
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.command === PREPARE_DESKTOP_SIDECARS_COMMAND) {
    prepareDesktopSidecars({
      env: {
        ...process.env,
        ...(args.targetKey ? { CTX_HTTP_BAZEL_TARGET_KEY: args.targetKey } : {}),
      },
      profile: args.profile,
    });
    return;
  }
  const sidecarPaths = resolveDesktopSidecarPaths({
    env: process.env,
    targetKey: args.targetKey,
  });
  process.stdout.write(`${renderDesktopSidecarEnv(sidecarPaths)}\n`);
}

if (require.main === module) {
  main();
}

module.exports = {
  BAZEL_PILOT_SCRIPT_PATH,
  DEFAULT_PROFILE,
  DESKTOP_SIDECAR_TARGETS,
  PREPARE_DESKTOP_SIDECARS_COMMAND,
  PRINT_DESKTOP_SIDECAR_ENV_COMMAND,
  TARGET_SPECS,
  buildBazelPlatformArgs,
  buildBazelCommandContext,
  buildDesktopSyncEnv,
  buildTargetsViaBazel,
  parseBazelOutputPaths,
  parseArgs,
  prepareDesktopSidecars,
  renderDesktopSidecarEnv,
  resolveDesktopWebDist,
  resolveBazelOutputPaths,
  resolveDesktopSidecarPaths,
  repoRoots,
  shouldResolveAvfLinuxHelper,
};
