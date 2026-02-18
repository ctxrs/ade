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
const harnessRuntimeRs = path.join(coreRoot, "crates", "ctx-http", "src", "harness_runtime.rs");

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

const readRustStringConst = (filePath, constName) => {
  if (!fs.existsSync(filePath)) {
    throw new Error(`missing source file for const ${constName}: ${filePath}`);
  }
  const source = fs.readFileSync(filePath, "utf8");
  const pattern = new RegExp(`const\\s+${constName}[^=]*=\\s*"([^"]+)"`);
  const match = source.match(pattern);
  if (!match || !match[1]) {
    throw new Error(`failed to resolve const ${constName} from ${filePath}`);
  }
  return match[1];
};

const readBundleManifest = (bundleDir) => {
  const manifestPath = path.join(bundleDir, "manifest.json");
  if (!fs.existsSync(manifestPath)) {
    throw new Error(`missing bundle manifest: ${manifestPath}`);
  }
  try {
    return JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  } catch (e) {
    throw new Error(`failed to parse bundle manifest ${manifestPath}: ${e?.message ?? e}`);
  }
};

const assertBundledProviderTargets = (bundleDir, providerId, targets) => {
  const manifest = readBundleManifest(bundleDir);
  const providers = Array.isArray(manifest?.providers) ? manifest.providers : [];
  const missing = [];
  for (const target of targets) {
    const found = providers.some(
      (p) =>
        p &&
        p.id === providerId &&
        p.os === target.os &&
        p.arch === target.arch &&
        typeof p.command === "string" &&
        p.command.trim().length > 0
    );
    if (!found) missing.push(`${target.os}/${target.arch}`);
  }
  if (missing.length > 0) {
    throw new Error(
      `bundle manifest missing ${providerId} targets: ${missing.join(
        ", "
      )}. Ensure cross-arch codex bundling is configured for release packaging.`
    );
  }
};

const localLinuxArchForImageGuard = () => {
  if (process.arch === "arm64") return "aarch64";
  if (process.arch === "x64") return "x86_64";
  return null;
};

const assertBundledHarnessImageForLocalArch = (bundleDir, expectedImage, arch) => {
  const manifest = readBundleManifest(bundleDir);
  const images = Array.isArray(manifest?.images) ? manifest.images : [];
  const entry = images.find(
    (img) =>
      img &&
      img.id === "ctx-harness" &&
      img.os === "linux" &&
      img.arch === arch &&
      img.image === expectedImage &&
      typeof img.tar === "string" &&
      img.tar.trim().length > 0,
  );
  if (!entry) {
    throw new Error(
      `bundle manifest missing ctx-harness linux/${arch} image '${expectedImage}'. ` +
        `Set CTX_BUNDLE_HARNESS_IMAGE=1 for debug builds (or both for release) before packaging.`,
    );
  }
  const tarPath = path.isAbsolute(entry.tar) ? entry.tar : path.join(bundleDir, entry.tar);
  if (!fs.existsSync(tarPath)) {
    throw new Error(
      `bundle manifest references missing ctx-harness image tar for linux/${arch}: ${tarPath}. ` +
        `Re-run desktop bundle sync with CTX_BUNDLE_HARNESS_IMAGE enabled.`,
    );
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
  // Tauri resource globs include bundles/images/**/*; keep at least one stable file
  // so packaging doesn't fail when no harness image tar is present yet.
  const imagesDir = path.join(destBundleDir, "images");
  fs.mkdirSync(imagesDir, { recursive: true });
  fs.writeFileSync(
    path.join(imagesDir, "placeholder.txt"),
    "ctx desktop placeholder; generated by desktop_sync_resources.cjs\n",
    "utf8",
  );
};

const shouldSyncBundles = () => {
  const flag = String(process.env.CTX_DESKTOP_SYNC_BUNDLES || "").trim();
  if (flag) return flag === "1" || flag.toLowerCase() === "true";
  // Desktop prep should include bundles by default so debug and release both exercise
  // the same runtime/provider packaging paths.
  return profile === "release" || profile === "debug";
};

const syncBundles = () => {
  if (!shouldSyncBundles()) return null;
  if (!fs.existsSync(bundleScript)) {
    throw new Error(`missing bundle script: ${bundleScript}`);
  }
  resetBundleDir();
  const env = { ...process.env, CTX_BUNDLE_DIR: destBundleDir };
  // Bundle default harness image tar for both debug and release so managed staging
  // works without registry pulls.
  if (profile === "release") {
    env.CTX_BUNDLE_HARNESS_IMAGE = env.CTX_BUNDLE_HARNESS_IMAGE || "both";
  } else if (profile === "debug") {
    env.CTX_BUNDLE_HARNESS_IMAGE = env.CTX_BUNDLE_HARNESS_IMAGE || "1";
  }
  // For desktop builds on macOS we want container mode to work out-of-box without relying on
  // system Podman installs (PATH). Bundle Podman (plus helper binaries) deterministically.
  if (process.platform === "darwin") {
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
  if (process.platform === "darwin") {
    const linuxTargets = [
      { arch: "aarch64", buildCodexCrp: true },
      { arch: "x86_64", buildCodexCrp: false },
    ];
    for (const target of linuxTargets) {
      const linuxEnv = {
        ...env,
        CTX_BUNDLE_APPEND: "1",
        CTX_BUNDLE_OS: "linux",
        CTX_BUNDLE_ARCH: target.arch,
        CTX_BUNDLE_ONLY_PROVIDERS: "codex,acp-crp-bridge,cursor,pi",
        CTX_BUNDLE_SKIP_RUNTIMES: "0",
        CTX_BUNDLE_SKIP_IMAGES: "1",
        CTX_BUNDLE_INCLUDE_BRIDGE: "1",
        CTX_BUNDLE_LOCAL_ADAPTERS: "auto",
        CTX_BUNDLE_BUILD_LOCAL_ADAPTERS: "0",
        CTX_BUNDLE_HARNESS_IMAGE: "0",
        CTX_BUNDLE_PODMAN: "0",
      };
      if (target.buildCodexCrp && !linuxEnv.CTX_BUNDLE_BUILD_CODEX_CRP) {
        // Managed codex artifacts currently publish linux/x86_64 only.
        // Build linux/aarch64 locally so Apple Silicon container sessions work.
        // Build-time container paths in ensure_bundled_harnesses.sh are Docker-only.
        linuxEnv.CTX_BUNDLE_BUILD_CODEX_CRP = "1";
      }
      const linuxRes = childProcess.spawnSync(bundleScript, {
        env: linuxEnv,
        stdio: "inherit",
      });
      if (linuxRes.status !== 0) {
        throw new Error(
          `bundle script failed for linux/${target.arch} (${linuxRes.status ?? "unknown"})`
        );
      }
    }
    assertBundledProviderTargets(destBundleDir, "codex", [
      { os: "linux", arch: "aarch64" },
      { os: "linux", arch: "x86_64" },
    ]);
    assertBundledProviderTargets(destBundleDir, "acp-crp-bridge", [
      { os: "linux", arch: "aarch64" },
      { os: "linux", arch: "x86_64" },
    ]);
    assertBundledProviderTargets(destBundleDir, "cursor", [
      { os: "linux", arch: "aarch64" },
      { os: "linux", arch: "x86_64" },
    ]);
    assertBundledProviderTargets(destBundleDir, "pi", [
      { os: "linux", arch: "aarch64" },
      { os: "linux", arch: "x86_64" },
    ]);
  }

  if (profile === "debug" || profile === "release") {
    const localLinuxArch = localLinuxArchForImageGuard();
    if (localLinuxArch) {
      const expectedImage = readRustStringConst(harnessRuntimeRs, "DEFAULT_CONTAINER_IMAGE");
      assertBundledHarnessImageForLocalArch(destBundleDir, expectedImage, localLinuxArch);
    } else {
      console.warn(
        `warn: skipping ctx-harness image manifest guard for unsupported host arch '${process.arch}'`,
      );
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
