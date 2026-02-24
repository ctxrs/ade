#!/usr/bin/env node

const childProcess = require("node:child_process");
const os = require("node:os");
const path = require("node:path");
const fs = require("node:fs");

const { resolveLaunchMode } = require("./desktop_mode.cjs");

const coreRoot = path.resolve(__dirname, "..");

const run = (command, args, options = {}) => {
  const result = childProcess.spawnSync(command, args, {
    cwd: coreRoot,
    stdio: "inherit",
    ...options,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
};

const resolveGitDirName = () => {
  try {
    const raw = childProcess
      .execSync("git rev-parse --git-dir", { cwd: coreRoot, stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .trim();
    if (!raw) return "default";
    return path.basename(raw);
  } catch {
    return "default";
  }
};

const resolveCargoTargetDir = () => {
  const raw = String(process.env.CARGO_TARGET_DIR || "").trim();
  if (raw) return raw;
  return path.join(os.homedir(), ".cache", "cargo", "ctx-monorepo", resolveGitDirName());
};

const main = () => {
  const mode = resolveLaunchMode({ surface: "desktop" });
  const cargoTargetDir = resolveCargoTargetDir();
  const effectiveManifestPath = path.join(
    coreRoot,
    "apps",
    "desktop",
    "src-tauri",
    "bundles",
    "runtime_manifest.effective.json",
  );
  const prepEnv = { ...process.env, CARGO_TARGET_DIR: cargoTargetDir };
  const tauriEnv = {
    ...process.env,
    CTX_DESKTOP_CHANNEL: mode.channel,
    CTX_RUNTIME_PROFILE: mode.profile,
    CTX_LAUNCH_SURFACE: mode.surface,
    CTX_DESKTOP_DEV_BIN_DIR: path.join(cargoTargetDir, "debug"),
    CTX_BUNDLE_MANIFEST: effectiveManifestPath,
  };
  const tauriConfigOverride = JSON.stringify({
    build: {
      beforeDevCommand: "",
      devUrl: null,
    },
  });

  // Canonical desktop dev flow: prepare runtime, validate bundled lock contract, then launch.
  run("node", ["scripts/desktop_runtime_prepare.cjs"], { env: prepEnv });
  const runtimeStatePath = path.join(
    coreRoot,
    "apps",
    "desktop",
    "src-tauri",
    "bundles",
    "runtime_state.json",
  );
  if (fs.existsSync(runtimeStatePath)) {
    try {
      const state = JSON.parse(fs.readFileSync(runtimeStatePath, "utf8"));
      console.log(
        `desktop_mode_start: channel=${mode.channel} profile=${mode.profile} surface=${mode.surface} lock_sha=${state?.runtime_lock?.sha256 || "missing"} manifest_sha=${state?.effective_manifest?.sha256 || "missing"}`,
      );
    } catch (error) {
      console.warn(
        `desktop_mode_start: channel=${mode.channel} profile=${mode.profile} surface=${mode.surface} runtime_state=parse_error:${error?.message ?? String(error)}`,
      );
    }
  }
  run(
    "pnpm",
    ["-C", "apps/desktop", "exec", "tauri", "dev", "-c", tauriConfigOverride],
    { env: tauriEnv },
  );
};

main();
