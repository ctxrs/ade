#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { readDesktopVersion } = require("./desktop_version.cjs");
const { resolveCargoTargetDir } = require("./lib/cargo_target_dir.cjs");

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
  cargoTargetDir,
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

  const baseEnv = {
    ...process.env,
    CARGO_TARGET_DIR: cargoTargetDir,
  };
  const steps = [];
  const shouldPrepareAvfGuestRuntime =
    platform === "darwin" && arch === "arm64" && syncBundles !== "0";
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

  if (config.buildWeb) {
    steps.push({
      command: "pnpm",
      args: ["-C", "apps/web", "build"],
      env: {
        ...baseEnv,
        VITE_CTX_APP_VERSION: desktopVersion,
      },
    });
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
  const cargoTargetDir = resolveCargoTargetDir({ cwd: coreRoot });
  const desktopVersion = readDesktopVersion(coreRoot);
  const steps = createPrepSteps({
    mode,
    cargoTargetDir,
    desktopVersion,
    arch: process.arch,
  });
  for (const step of steps) {
    runStep(step);
  }
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
