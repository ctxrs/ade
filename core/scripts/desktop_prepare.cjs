#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const { readDesktopVersion } = require("./desktop_version.cjs");
const { resolveCargoTargetDir } = require("./lib/cargo_target_dir.cjs");
const { ensureWebDistArtifact } = require("./lib/web_dist_cache.cjs");

const coreRoot = path.resolve(__dirname, "..");

const PREP_MODES = {
  dev: {
    cargoProfile: "debug",
    syncProfile: "debug",
    buildWeb: false,
    checkVersions: false,
  },
  "debug-build": {
    cargoProfile: "debug",
    syncProfile: "debug",
    buildWeb: true,
    checkVersions: true,
  },
  "release-build": {
    cargoProfile: "release",
    syncProfile: "release",
    buildWeb: true,
    checkVersions: true,
  },
};

function fail(message) {
  console.error(`desktop_prepare failed: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  let mode = "debug-build";
  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--mode") {
      mode = argv[i + 1] || "";
      i += 1;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      console.log("Usage: node scripts/desktop_prepare.cjs [--mode dev|debug-build|release-build]");
      process.exit(0);
    }
    fail(`unknown argument: ${arg}`);
  }
  if (!PREP_MODES[mode]) {
    fail(`invalid --mode '${mode}' (expected dev|debug-build|release-build)`);
  }
  return { mode };
}

function createPrepSteps({
  mode,
  prepEnv,
  cargoTargetDir = resolveCargoTargetDir({ cwd: coreRoot, env: prepEnv }),
  desktopWebDist = "",
  desktopVersion,
  syncBundles = process.env.CTX_DESKTOP_SYNC_BUNDLES || (mode === "dev" ? "0" : "1"),
  platform = process.platform,
  arch = process.arch,
}) {
  const config = PREP_MODES[mode];
  const release = config.cargoProfile === "release";
  const cargoArgs = ["build", "-p", "ctx-http", "-p", "ctx-mcp"];
  const avfArgs = [
    "build",
    "--manifest-path",
    "apps/desktop/src-tauri/Cargo.toml",
    "--bin",
    "ctx-avf-linux-helper",
  ];
  if (release) {
    cargoArgs.push("--release");
    avfArgs.push("--release");
  }

  const baseEnv = { ...prepEnv, CARGO_TARGET_DIR: cargoTargetDir };
  if (
    !baseEnv.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD
    && process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD
  ) {
    baseEnv.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD =
      process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD;
  }
  const steps = [];
  const allowManagedAvfRuntimeMissingLocalPayload =
    String(process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD || "").trim() === "1";
  const shouldPrepareAvfGuestRuntime =
    platform === "darwin"
    && arch === "arm64"
    && syncBundles !== "0"
    && !allowManagedAvfRuntimeMissingLocalPayload;
  const avfGuestRuntimeDir = shouldPrepareAvfGuestRuntime
    ? path.join(cargoTargetDir, "desktop-avf-linux-guest-runtime")
    : null;

  if (config.checkVersions) {
    steps.push({
      command: "node",
      args: ["scripts/desktop_check_versions.cjs"],
      env: baseEnv,
    });
  }

  steps.push({
    command: "cargo",
    args: cargoArgs,
    env: baseEnv,
  });

  if (platform === "darwin") {
    steps.push({
      command: "cargo",
      args: avfArgs,
      env: {
        ...baseEnv,
        CTX_DESKTOP_SKIP_TAURI_BUILD: "1",
      },
    });
  }

  if (config.buildWeb && !desktopWebDist) {
    throw new Error(`desktopWebDist is required for ${mode}`);
  }

  if (shouldPrepareAvfGuestRuntime) {
    steps.push({
      command: "bash",
      args: [
        "scripts/prepare_avf_linux_guest_runtime.sh",
        "--output-dir",
        avfGuestRuntimeDir,
        "--arch",
        "arm64",
        "--force",
      ],
      env: baseEnv,
    });
  }

  steps.push({
    command: "node",
    args: ["scripts/desktop_sync_resources.cjs", "--profile", config.syncProfile],
    env: {
      ...baseEnv,
      CTX_DESKTOP_SYNC_BUNDLES: syncBundles,
      ...(desktopWebDist ? { CTX_DESKTOP_WEB_DIST: desktopWebDist } : {}),
      ...(avfGuestRuntimeDir
        ? { CTX_AVF_LINUX_GUEST_RUNTIME_DIR: avfGuestRuntimeDir }
        : {}),
    },
  });

  return steps;
}

function runStep(step) {
  const result = childProcess.spawnSync(step.command, step.args, {
    cwd: coreRoot,
    stdio: "inherit",
    env: step.env,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function main(argv = process.argv) {
  const { mode } = parseArgs(argv);
  const { env: prepEnv, cargoTargetDir } = buildCtxCacheEnv({
    cwd: coreRoot,
    env: process.env,
    mode: "workspace",
    mkdir: true,
  });
  const desktopVersion = readDesktopVersion(coreRoot);
  const desktopWebDist = PREP_MODES[mode].buildWeb
    ? ensureWebDistArtifact({
      coreRoot,
      env: prepEnv,
      appVersion: desktopVersion,
      variant: `desktop-${mode}`,
    }).distDir
    : trimDesktopWebDist(process.env.CTX_DESKTOP_WEB_DIST);
  const steps = createPrepSteps({
    mode,
    prepEnv,
    cargoTargetDir,
    desktopWebDist,
    desktopVersion,
    arch: process.arch,
  });
  for (const step of steps) {
    runStep(step);
  }
}

function trimDesktopWebDist(value) {
  return String(value ?? "").trim();
}

if (require.main === module) {
  main();
}

module.exports = {
  PREP_MODES,
  createPrepSteps,
  main,
  parseArgs,
};
