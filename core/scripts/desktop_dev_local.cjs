#!/usr/bin/env node

const childProcess = require("node:child_process");
const os = require("node:os");
const path = require("node:path");
const fs = require("node:fs");
const net = require("node:net");

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

const envFlagEnabled = (value, defaultValue = false) => {
  const normalized = String(value ?? "").trim().toLowerCase();
  if (!normalized) return defaultValue;
  return !["0", "false", "no", "off"].includes(normalized);
};

const normalizeManifestOs = (raw) => {
  if (raw === "darwin" || raw === "macos") return "macos";
  if (raw === "win32" || raw === "windows") return "windows";
  if (raw === "linux") return "linux";
  return String(raw || "").trim();
};

const normalizeManifestArch = (raw) => {
  if (raw === "arm64" || raw === "aarch64") return "aarch64";
  if (raw === "x64" || raw === "x86_64" || raw === "amd64") return "x86_64";
  return String(raw || "").trim();
};

const bundledPodmanDeclared = (manifestPath, platform = process.platform, arch = process.arch) => {
  try {
    if (!manifestPath || !fs.existsSync(manifestPath)) return false;
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    const runtimes = Array.isArray(manifest?.runtimes) ? manifest.runtimes : [];
    const expectedOs = normalizeManifestOs(platform);
    const expectedArch = normalizeManifestArch(arch);
    return runtimes.some((entry) =>
      entry
      && entry.id === "podman"
      && entry.os === expectedOs
      && entry.arch === expectedArch
      && typeof entry.root === "string"
      && entry.root.trim().length > 0
      && typeof entry.bin === "string"
      && entry.bin.trim().length > 0);
  } catch {
    return false;
  }
};

const shouldEnableSystemPodmanFallback = ({
  bundlePodmanEnv,
  allowSystemPodmanEnv,
  effectiveManifestPath,
  platform = process.platform,
  arch = process.arch,
}) => {
  if (typeof allowSystemPodmanEnv === "string" && allowSystemPodmanEnv.trim().length > 0) {
    return false;
  }
  if (envFlagEnabled(bundlePodmanEnv, true) === false) {
    return true;
  }
  return !bundledPodmanDeclared(effectiveManifestPath, platform, arch);
};

const portIsAvailable = (host, port) =>
  new Promise((resolve) => {
    const server = net.createServer();
    server.unref();
    server.once("error", () => {
      resolve(false);
    });
    server.listen({ host, port, exclusive: true }, () => {
      server.close(() => resolve(true));
    });
  });

const resolveWebDevPort = async (host) => {
  const explicitPort = String(process.env.CTX_DESKTOP_WEB_DEV_PORT || "").trim();
  if (explicitPort) return explicitPort;
  const startPort = 5197;
  const endPort = 5247;
  for (let candidate = startPort; candidate <= endPort; candidate += 1) {
    // Verify localhost availability too because devUrl points at 127.0.0.1.
    const available = await portIsAvailable(host, candidate);
    const localhostAvailable = host === "127.0.0.1" ? available : await portIsAvailable("127.0.0.1", candidate);
    if (available && localhostAvailable) {
      return String(candidate);
    }
  }
  return String(startPort);
};

const main = async () => {
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
  const useBundledWeb = envFlagEnabled(process.env.CTX_DESKTOP_USE_BUNDLED_WEB, false);
  const webDevHost = String(process.env.CTX_DESKTOP_WEB_DEV_HOST || "127.0.0.1").trim() || "127.0.0.1";
  const webDevPort = await resolveWebDevPort(webDevHost);
  const webDevUrlDefault = `http://${webDevHost}:${webDevPort}`;
  const devUrl = String(process.env.CTX_DESKTOP_WEB_DEV_URL || webDevUrlDefault).trim() || webDevUrlDefault;
  const beforeDevCommand = String(
    process.env.CTX_DESKTOP_WEB_BEFORE_DEV_COMMAND
      || `CTX_DEV_HTTP=1 pnpm -C ../web exec vite --host ${webDevHost} --port ${webDevPort} --strictPort`,
  ).trim();
  const tauriConfigOverride = JSON.stringify({
    build: {
      // In desktop dev, default to the live webapp dev server for source maps + non-minified errors.
      beforeDevCommand: useBundledWeb ? "" : beforeDevCommand,
      devUrl: useBundledWeb ? null : devUrl,
    },
  });

  // Canonical desktop dev flow: prepare runtime, validate bundled lock contract, then launch.
  run("node", ["scripts/desktop_runtime_prepare.cjs"], { env: prepEnv });
  if (shouldEnableSystemPodmanFallback({
    bundlePodmanEnv: process.env.CTX_BUNDLE_PODMAN,
    allowSystemPodmanEnv: process.env.CTX_ALLOW_SYSTEM_PODMAN,
    effectiveManifestPath,
  })) {
    tauriEnv.CTX_ALLOW_SYSTEM_PODMAN = "1";
  }
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

if (require.main === module) {
  main().catch((error) => {
    console.error(`desktop_dev_local failed: ${error?.message ?? String(error)}`);
    process.exit(1);
  });
}

module.exports = {
  envFlagEnabled,
  normalizeManifestOs,
  normalizeManifestArch,
  bundledPodmanDeclared,
  shouldEnableSystemPodmanFallback,
};
