const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const args = process.argv.slice(2);
const profileIdx = args.indexOf("--profile");
const profile = profileIdx !== -1 ? args[profileIdx + 1] : "release";

const coreRoot = path.resolve(__dirname, "..");
const desktopTauriRoot = path.join(coreRoot, "apps", "desktop", "src-tauri");
const destBinDir = path.join(desktopTauriRoot, "bin");
const destWebDistDir = path.join(desktopTauriRoot, "web", "dist");
const destBundleDir = path.join(desktopTauriRoot, "bundles");
const bundleScript = path.join(coreRoot, "scripts", "ensure_bundled_harnesses.sh");

const targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

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
  if (!fs.existsSync(filePath)) return;
  try {
    fs.chmodSync(filePath, 0o755);
  } catch (e) {
    console.warn(`warn: failed to chmod +x ${filePath}: ${e?.message ?? e}`);
  }
};

const resetBundleDir = () => {
  fs.mkdirSync(destBundleDir, { recursive: true });
  const keep = new Set(["README.md", ".gitkeep"]);
  for (const entry of fs.readdirSync(destBundleDir)) {
    if (keep.has(entry)) continue;
    fs.rmSync(path.join(destBundleDir, entry), { recursive: true, force: true });
  }
};

const shouldSyncBundles = () => {
  const flag = String(process.env.CTX_DESKTOP_SYNC_BUNDLES || "").trim();
  if (flag) return flag === "1" || flag.toLowerCase() === "true";
  return false;
};

const resolveArchEnv = (baseKey, arch) => {
  const key = arch === "aarch64" ? "AARCH64" : "X86_64";
  const alt = arch === "aarch64" ? "ARM64" : "AMD64";
  return (
    process.env[`${baseKey}_${key}`] ||
    process.env[`${baseKey}_${alt}`] ||
    process.env[baseKey]
  );
};

const runBundleForArch = (arch, append) => {
  const env = {
    ...process.env,
    CTX_BUNDLE_DIR: destBundleDir,
    CTX_BUNDLE_OS: "macos",
    CTX_BUNDLE_ARCH: arch,
  };
  if (append) env.CTX_BUNDLE_APPEND = "1";

  if (env.CTX_BUNDLE_PODMAN === "1") {
    const archiveUrl = resolveArchEnv("PODMAN_ARCHIVE_URL", arch);
    const archivePath = resolveArchEnv("PODMAN_ARCHIVE_PATH", arch);
    const binRel = resolveArchEnv("PODMAN_BIN_REL", arch);
    const extractSubdir = resolveArchEnv("PODMAN_EXTRACT_SUBDIR", arch);
    if (archiveUrl) env.PODMAN_ARCHIVE_URL = archiveUrl;
    if (archivePath) env.PODMAN_ARCHIVE_PATH = archivePath;
    if (binRel) env.PODMAN_BIN_REL = binRel;
    if (extractSubdir) env.PODMAN_EXTRACT_SUBDIR = extractSubdir;
  }

  const res = childProcess.spawnSync(bundleScript, {
    env,
    stdio: "inherit",
  });
  if (res.status !== 0) {
    throw new Error(`bundle script failed for ${arch} (${res.status ?? "unknown"})`);
  }
};

const syncBundles = () => {
  if (!shouldSyncBundles()) return null;
  if (!fs.existsSync(bundleScript)) {
    throw new Error(`missing bundle script: ${bundleScript}`);
  }
  resetBundleDir();
  runBundleForArch("aarch64", false);
  runBundleForArch("x86_64", true);
  return destBundleDir;
};

const copyTargetSidecar = (name, target) => {
  const src = path.join(coreRoot, "target", target, profile, name);

  // Tauri expects `externalBin` sidecars to be named `bin/<name>-<target>` when building with `--target`.
  const dest = path.join(destBinDir, `${name}-${target}`);

  if (!fs.existsSync(src)) {
    throw new Error(`missing sidecar for ${target}: ${src} (did you build --target ${target}?)`);
  }

  fs.mkdirSync(destBinDir, { recursive: true });
  fs.copyFileSync(src, dest);
  ensureExecutable(dest);
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
  if (process.platform !== "darwin") {
    throw new Error("desktop_sync_resources_universal_macos is intended to run on macOS (darwin)");
  }
  if (profile !== "debug" && profile !== "release") {
    throw new Error(`invalid --profile ${profile}; expected debug|release`);
  }
  if (!fs.existsSync(desktopTauriRoot)) {
    throw new Error(`missing desktop tauri project dir: ${desktopTauriRoot}`);
  }

  fs.mkdirSync(destBinDir, { recursive: true });
  for (const entry of fs.readdirSync(destBinDir)) {
    if (entry === ".gitkeep") continue;
    fs.rmSync(path.join(destBinDir, entry), { recursive: true, force: true });
  }

  const copied = {
    sidecars: {
      ctx: Object.fromEntries(targets.map((t) => [t, copyTargetSidecar("ctx", t)])),
      ctxMcp: Object.fromEntries(targets.map((t) => [t, copyTargetSidecar("ctx-mcp", t)])),
    },
    webDist: copyWebDist(),
    bundles: syncBundles(),
  };

  console.log("desktop_sync_resources_universal_macos:", copied);
};

main();
