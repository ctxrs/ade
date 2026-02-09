const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const args = process.argv.slice(2);
const profileIdx = args.indexOf("--profile");
const profile = profileIdx !== -1 ? args[profileIdx + 1] : "debug";

const coreRoot = path.resolve(__dirname, "..");
const desktopTauriRoot = path.join(coreRoot, "apps", "desktop", "src-tauri");
const destBinDir = path.join(desktopTauriRoot, "bin");
const destWebDistDir = path.join(desktopTauriRoot, "web", "dist");
const destBundleDir = path.join(desktopTauriRoot, "bundles");
const bundleScript = path.join(coreRoot, "..", "scripts", "ensure_bundled_harnesses.sh");

const isWindows = process.platform === "win32";
const binExt = isWindows ? ".exe" : "";

const resolveCargoTargetDir = () => {
  const env = process.env.CARGO_TARGET_DIR;
  if (env && String(env).trim()) return String(env).trim();
  return path.join(coreRoot, "target");
};

const copyDirRecursive = (srcDir, destDir) => {
  fs.mkdirSync(destDir, { recursive: true });
  for (const entry of fs.readdirSync(srcDir, { withFileTypes: true })) {
    const srcPath = path.join(srcDir, entry.name);
    const destPath = path.join(destDir, entry.name);
    if (entry.isDirectory()) {
      copyDirRecursive(srcPath, destPath);
      continue;
    }
    if (entry.isSymbolicLink()) {
      const linkTarget = fs.readlinkSync(srcPath);
      fs.symlinkSync(linkTarget, destPath);
      continue;
    }
    fs.copyFileSync(srcPath, destPath);
  }
};

const ensureExecutable = (filePath) => {
  if (isWindows) return;
  if (!fs.existsSync(filePath)) return;
  try {
    fs.chmodSync(filePath, 0o755);
  } catch (e) {
    console.warn(`warn: failed to chmod +x ${filePath}: ${e?.message ?? e}`);
  }
};

const resetBundleDir = () => {
  fs.mkdirSync(destBundleDir, { recursive: true });
  // Keep lightweight repo-tracked resources that are used at runtime (and ignore rules).
  const keep = new Set(["README.md", ".gitkeep", ".gitignore", "lucide-settings.svg"]);
  for (const entry of fs.readdirSync(destBundleDir)) {
    if (keep.has(entry)) continue;
    fs.rmSync(path.join(destBundleDir, entry), { recursive: true, force: true });
  }
};

const shouldSyncBundles = () => {
  const flag = String(process.env.CTX_DESKTOP_SYNC_BUNDLES || "").trim();
  if (flag) return flag === "1" || flag.toLowerCase() === "true";
  // For release builds, we want deterministic, self-contained bundles by default.
  return profile === "release";
};

const syncBundles = () => {
  if (!shouldSyncBundles()) return null;
  if (!fs.existsSync(bundleScript)) {
    throw new Error(`missing bundle script: ${bundleScript}`);
  }
  resetBundleDir();
  const env = { ...process.env, CTX_BUNDLE_DIR: destBundleDir };
  // For release builds, bundle the default harness image tar so restricted networking works
  // without relying on registry pulls.
  if (profile === "release") {
    env.CTX_BUNDLE_HARNESS_IMAGE = env.CTX_BUNDLE_HARNESS_IMAGE || "both";
  }
  // For desktop releases we want container mode to work out-of-box without relying on system
  // Podman installs (PATH). Bundle Podman (plus its macOS helper binaries) deterministically.
  if (profile === "release" && process.platform === "darwin") {
    env.CTX_BUNDLE_PODMAN = env.CTX_BUNDLE_PODMAN || "1";
    if (env.CTX_BUNDLE_PODMAN === "1") {
      env.PODMAN_VERSION = env.PODMAN_VERSION || "5.7.1";
      if (!env.PODMAN_ARCHIVE_URL) {
        const arch = process.arch === "arm64" ? "arm64" : "amd64";
        env.PODMAN_ARCHIVE_URL = `https://github.com/containers/podman/releases/download/v${env.PODMAN_VERSION}/podman-remote-release-darwin_${arch}.zip`;
      }
      env.PODMAN_BIN_REL = env.PODMAN_BIN_REL || "usr/bin/podman";
    }
  }
  const res = childProcess.spawnSync(bundleScript, {
    env,
    stdio: "inherit",
  });
  if (res.status !== 0) {
    throw new Error(`bundle script failed (${res.status ?? "unknown"})`);
  }

  // Container mode runs Linux containers even on macOS/Windows. Bundle Linux provider
  // binaries too so "disk-isolated container" can work offline/out-of-box.
  if (profile === "release" && process.platform === "darwin") {
    for (const arch of ["aarch64", "x86_64"]) {
      const linuxEnv = {
        ...env,
        CTX_BUNDLE_APPEND: "1",
        CTX_BUNDLE_OS: "linux",
        CTX_BUNDLE_ARCH: arch,
        CTX_BUNDLE_ONLY_PROVIDERS: "codex-crp",
        CTX_BUNDLE_SKIP_RUNTIMES: "1",
        CTX_BUNDLE_SKIP_IMAGES: "1",
        CTX_BUNDLE_INCLUDE_BRIDGE: "0",
        CTX_BUNDLE_LOCAL_ADAPTERS: "off",
        CTX_BUNDLE_BUILD_LOCAL_ADAPTERS: "0",
        CTX_BUNDLE_HARNESS_IMAGE: "0",
        CTX_BUNDLE_PODMAN: "0",
      };
      const linuxRes = childProcess.spawnSync(bundleScript, {
        env: linuxEnv,
        stdio: "inherit",
      });
      if (linuxRes.status !== 0) {
        throw new Error(
          `bundle script failed for linux/${arch} (${linuxRes.status ?? "unknown"})`
        );
      }
    }
  }

  return destBundleDir;
};

const resolveHostTarget = () => {
  const envTarget = process.env.CARGO_BUILD_TARGET || process.env.TAURI_ENV_TARGET_TRIPLE;
  if (envTarget) return envTarget;
  try {
    const info = childProcess.execSync("rustc -vV", { encoding: "utf8" });
    const match = info.match(/^host:\s+(.+)$/m);
    return match ? match[1].trim() : null;
  } catch {
    return null;
  }
};

const copySidecar = (name) => {
  const src = path.join(resolveCargoTargetDir(), profile, `${name}${binExt}`);
  const dest = path.join(destBinDir, `${name}${binExt}`);
  const target = resolveHostTarget();
  const destTarget = target ? path.join(destBinDir, `${name}-${target}${binExt}`) : null;
  if (!fs.existsSync(src)) {
    throw new Error(`missing sidecar: ${src} (did you run cargo build?)`);
  }
  fs.mkdirSync(destBinDir, { recursive: true });
  fs.copyFileSync(src, dest);
  ensureExecutable(dest);
  if (destTarget) {
    fs.copyFileSync(src, destTarget);
    ensureExecutable(destTarget);
  }
  return dest;
};

const copyWebDist = () => {
  const srcDist = path.join(coreRoot, "apps", "web", "dist");
  if (!fs.existsSync(srcDist)) {
    throw new Error(`missing web dist: ${srcDist} (did you run pnpm -C apps/web build?)`);
  }

  fs.rmSync(destWebDistDir, { recursive: true, force: true });
  copyDirRecursive(srcDist, destWebDistDir);
  return destWebDistDir;
};

const main = () => {
  if (profile !== "debug" && profile !== "release") {
    throw new Error(`invalid --profile ${profile}; expected debug|release`);
  }
  if (!fs.existsSync(desktopTauriRoot)) {
    throw new Error(`missing desktop tauri project dir: ${desktopTauriRoot}`);
  }

  // Ensure deterministic contents.
  fs.mkdirSync(destBinDir, { recursive: true });
  for (const entry of fs.readdirSync(destBinDir)) {
    if (entry === ".gitkeep") continue;
    fs.rmSync(path.join(destBinDir, entry), { recursive: true, force: true });
  }

  const copied = {
    ctx: copySidecar("ctx"),
    ctxMcp: copySidecar("ctx-mcp"),
    webDist: copyWebDist(),
    bundles: syncBundles(),
  };

  console.log("desktop_sync_resources:", copied);
};

main();
