const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");
const crypto = require("crypto");
const { shouldBundleRemoteDaemons } = require("./desktop_sync_resources_remote_daemon_policy.cjs");

const args = process.argv.slice(2);
const profileIdx = args.indexOf("--profile");
const profile = profileIdx !== -1 ? args[profileIdx + 1] : "debug";
const syncBundlesEnabled = !["0", "false", "no", "off"].includes(
  String(process.env.CTX_DESKTOP_SYNC_BUNDLES || "1").trim().toLowerCase(),
);
const runtimeProfile = String(process.env.CTX_RUNTIME_PROFILE || "parity").trim().toLowerCase();
const sourceAllRuntimeProfile = runtimeProfile === "source-all";

const coreRoot = path.resolve(__dirname, "..");
const desktopTauriRoot = path.join(coreRoot, "apps", "desktop", "src-tauri");
const providerMatrixPath = path.join(coreRoot, "crates", "ctx-http", "src", "provider_matrix.json");
const destBinDir = path.join(desktopTauriRoot, "bin");
const destWebDistDir = path.join(desktopTauriRoot, "web", "dist");
const destBundleDir = path.join(desktopTauriRoot, "bundles");
const bundleScript = path.join(coreRoot, "..", "scripts", "ensure_bundled_harnesses.sh");
const harnessRuntimeRs = path.join(coreRoot, "crates", "ctx-http", "src", "harness_runtime.rs");
const runtimeLockPath = path.join(destBundleDir, "runtime_lock.v2.json");
const hostManifestOs = process.platform === "darwin" ? "macos" : process.platform === "win32" ? "windows" : "linux";
const hostManifestArch = process.arch === "arm64" ? "aarch64" : process.arch === "x64" ? "x86_64" : process.arch;
const parityProviderTargets = [
  { os: "macos", arch: hostManifestArch },
  { os: "linux", arch: "aarch64" },
  { os: "linux", arch: "x86_64" },
];
const parityRuntimeTargets = [
  { os: "macos", arch: hostManifestArch },
  { os: "linux", arch: "aarch64" },
  { os: "linux", arch: "x86_64" },
];
const parityImageTargets = [
  { os: "linux", arch: "aarch64" },
  { os: "linux", arch: "x86_64" },
];

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

const hasManagedProviderTarget = (providerId, os, arch) => {
  if (!fs.existsSync(providerMatrixPath)) return false;
  let parsed;
  try {
    parsed = JSON.parse(fs.readFileSync(providerMatrixPath, "utf8"));
  } catch {
    return false;
  }
  const providers = Array.isArray(parsed?.providers) ? parsed.providers : [];
  const provider = providers.find((entry) => entry && entry.id === providerId);
  if (!provider || typeof provider !== "object") return false;
  const managedInstall =
    provider.managed_install && typeof provider.managed_install === "object"
      ? provider.managed_install
      : null;
  const targets =
    managedInstall && managedInstall.targets && typeof managedInstall.targets === "object"
      ? managedInstall.targets
      : null;
  if (!targets) return false;
  const legacyKey = `${os}-${arch}`;
  const slashKey = `${os}/${arch}`;
  return Boolean(targets[legacyKey] || targets[slashKey]);
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
      )}. Ensure parity bundle generation includes all required targets.`
    );
  }
};

const assertBundledRuntimeTargets = (bundleDir, runtimeId, targets) => {
  const manifest = readBundleManifest(bundleDir);
  const runtimes = Array.isArray(manifest?.runtimes) ? manifest.runtimes : [];
  const missing = [];
  for (const target of targets) {
    const found = runtimes.some(
      (r) =>
        r &&
        r.id === runtimeId &&
        r.os === target.os &&
        r.arch === target.arch &&
        typeof r.root === "string" &&
        r.root.trim().length > 0 &&
        typeof r.bin === "string" &&
        r.bin.trim().length > 0,
    );
    if (!found) missing.push(`${target.os}/${target.arch}`);
  }
  if (missing.length > 0) {
    throw new Error(
      `bundle manifest missing ${runtimeId} runtime targets: ${missing.join(
        ", ",
      )}. Container/provider startup requires bundled host+linux runtimes.`,
    );
  }
};

const assertBundledHarnessImageTargets = (bundleDir, expectedImage, targets) => {
  const manifest = readBundleManifest(bundleDir);
  const images = Array.isArray(manifest?.images) ? manifest.images : [];
  for (const target of targets) {
    const entry = images.find(
      (img) =>
        img &&
        img.id === "ctx-harness" &&
        img.os === target.os &&
        img.arch === target.arch &&
        img.image === expectedImage &&
        typeof img.tar === "string" &&
        img.tar.trim().length > 0,
    );
    if (!entry) {
      throw new Error(
        `bundle manifest missing ctx-harness ${target.os}/${target.arch} image '${expectedImage}'. ` +
          "Set CTX_BUNDLE_HARNESS_IMAGE=both for parity bundles.",
      );
    }
    const tarPath = path.isAbsolute(entry.tar) ? entry.tar : path.join(bundleDir, entry.tar);
    if (!fs.existsSync(tarPath)) {
      throw new Error(
        `bundle manifest references missing ctx-harness image tar for ${target.os}/${target.arch}: ${tarPath}. ` +
          "Re-run desktop bundle sync with CTX_BUNDLE_HARNESS_IMAGE=both.",
      );
    }
  }
};

const readRuntimeLock = () => {
  if (!fs.existsSync(runtimeLockPath)) {
    throw new Error(`missing runtime lock for parity enforcement: ${runtimeLockPath}`);
  }
  try {
    return JSON.parse(fs.readFileSync(runtimeLockPath, "utf8"));
  } catch (error) {
    throw new Error(`failed to parse runtime lock ${runtimeLockPath}: ${error?.message ?? error}`);
  }
};

const readRuntimeLockRequiredProviderIds = () => {
  const lock = readRuntimeLock();
  const ids = Array.isArray(lock?.required?.provider_ids)
    ? lock.required.provider_ids
        .map((entry) => String(entry || "").trim())
        .filter((entry) => entry.length > 0)
    : [];
  if (ids.length === 0) {
    throw new Error(`runtime lock ${runtimeLockPath} has no required.provider_ids`);
  }
  return [...new Set(ids)].sort();
};

const readPinnedPodmanConfig = ({ os, arch }) => {
  const lock = readRuntimeLock();
  const components = Array.isArray(lock?.components) ? lock.components : [];
  const component = components.find(
    (entry) => entry?.kind === "runtime" && entry?.id === "podman" && entry?.os === os && entry?.arch === arch,
  );
  if (!component) {
    throw new Error(`runtime lock ${runtimeLockPath} missing runtime/podman component for ${os}/${arch}`);
  }
  const version = String(component.version || "").trim();
  if (!version) {
    throw new Error(`runtime lock podman component ${os}/${arch} missing version`);
  }
  const sources = Array.isArray(component.sources) ? component.sources : [];
  const archiveSource = sources.find((source) => {
    const sourceType = String(source?.source_type || "").trim();
    const uri = String(source?.uri || "").trim();
    return (sourceType === "vendor" || sourceType === "ci") && uri.startsWith("http");
  });
  if (!archiveSource) {
    throw new Error(
      `runtime lock podman component ${os}/${arch} missing vendor/ci archive source URI`,
    );
  }
  const archiveUrl = String(archiveSource.uri || "").trim();
  const archiveSha256 = String(archiveSource.sha256 || "").trim();
  if (!archiveSha256) {
    throw new Error(`runtime lock podman component ${os}/${arch} missing archive sha256`);
  }
  const binRel = String(component.bin || "").trim() || "usr/bin/podman";
  const extractSubdir = String(component.extract_subdir || "").trim();
  const gvproxyUrl = String(component?.helpers?.gvproxy?.uri || "").trim();
  const gvproxySha256 = String(component?.helpers?.gvproxy?.sha256 || "").trim();
  const vfkitUrl = String(component?.helpers?.vfkit?.uri || "").trim();
  const vfkitSha256 = String(component?.helpers?.vfkit?.sha256 || "").trim();
  if (!gvproxyUrl || !gvproxySha256 || !vfkitUrl || !vfkitSha256) {
    throw new Error(
      `runtime lock podman component ${os}/${arch} missing helper URIs/sha256 for gvproxy or vfkit`,
    );
  }

  return {
    version,
    archiveUrl,
    archiveSha256,
    binRel,
    extractSubdir,
    gvproxyUrl,
    gvproxySha256,
    vfkitUrl,
    vfkitSha256,
  };
};

const applyPinnedEnv = (env, key, value) => {
  if (!value) return;
  if (env[key] && env[key] !== value) {
    throw new Error(
      `${key} is pinned by runtime lock and cannot be overridden (expected '${value}', got '${env[key]}')`,
    );
  }
  env[key] = value;
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

const sha256File = (filePath) => {
  const data = fs.readFileSync(filePath);
  return crypto.createHash("sha256").update(data).digest("hex");
};

const commandExists = (name) => {
  const res = childProcess.spawnSync(name, ["--version"], { stdio: "ignore" });
  return res.status === 0;
};

const runtimeProbeArgs = (runtime) => {
  if (runtime === "docker") return ["info", "--format", "{{.ServerVersion}}"];
  if (runtime === "podman") return ["info", "--format", "json"];
  return null;
};

const runtimeUsable = (runtime) => {
  const probeArgs = runtimeProbeArgs(runtime);
  if (!probeArgs) return true;
  const res = childProcess.spawnSync(runtime, probeArgs, { stdio: "ignore" });
  return res.status === 0;
};

const resolveContainerRuntime = () => {
  const requested = String(process.env.CTX_BUNDLE_REMOTE_DAEMON_RUNTIME || "").trim();
  if (requested) {
    if (!commandExists(requested)) {
      throw new Error(
        `CTX_BUNDLE_REMOTE_DAEMON_RUNTIME='${requested}' is not available. Install it or unset the variable.`,
      );
    }
    if (!runtimeUsable(requested)) {
      throw new Error(
        `CTX_BUNDLE_REMOTE_DAEMON_RUNTIME='${requested}' is installed but not usable. Ensure the runtime daemon/service is running, or unset CTX_BUNDLE_REMOTE_DAEMON_RUNTIME.`,
      );
    }
    return requested;
  }

  const runtimeChecks = [
    { name: "docker", installed: commandExists("docker"), usable: false },
    { name: "podman", installed: commandExists("podman"), usable: false },
  ];
  for (const check of runtimeChecks) {
    if (check.installed) check.usable = runtimeUsable(check.name);
  }

  const usableRuntime = runtimeChecks.find((check) => check.installed && check.usable);
  if (usableRuntime) return usableRuntime.name;

  const installedButUnusable = runtimeChecks
    .filter((check) => check.installed && !check.usable)
    .map((check) => check.name);
  if (installedButUnusable.length > 0) {
    throw new Error(
      `remote daemon bundling found container runtime(s) in PATH but none are usable (${installedButUnusable.join(
        ", ",
      )}). Ensure the runtime daemon/service is running (for example, start Docker Desktop or podman machine) and rerun desktop prep.`,
    );
  }

  throw new Error(
    "remote daemon bundling requires docker or podman in PATH. Install a container runtime and rerun desktop prep.",
  );
};

const resolveRemoteDaemonBuilderImage = () => {
  const requested = String(process.env.CTX_BUNDLE_REMOTE_DAEMON_IMAGE || "").trim();
  if (requested) return requested;
  // Prefer a conservative glibc baseline so bundled daemon binaries run on a wider
  // range of Linux remotes (including our Debian bookworm fixture).
  return "rust:1.88-bookworm";
};

const upsertManifestDaemons = (bundleDir, daemonEntries) => {
  const manifestPath = path.join(bundleDir, "manifest.json");
  const manifest = readBundleManifest(bundleDir);
  const existing = Array.isArray(manifest?.daemons) ? manifest.daemons : [];
  const keep = existing.filter(
    (entry) =>
      !daemonEntries.some(
        (next) =>
          next.id === entry?.id && next.os === entry?.os && next.arch === entry?.arch,
      ),
  );
  const merged = [...keep, ...daemonEntries].sort((a, b) =>
    `${a.id}::${a.os}::${a.arch}`.localeCompare(`${b.id}::${b.os}::${b.arch}`),
  );
  manifest.daemons = merged;
  fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
};

const bundleRemoteDaemons = (bundleDir) => {
  const runtime = resolveContainerRuntime();
  const builderImage = resolveRemoteDaemonBuilderImage();
  const daemonsDir = path.join(bundleDir, "daemons");
  fs.mkdirSync(daemonsDir, { recursive: true });
  const cacheRoot = path.join(
    process.env.HOME || coreRoot,
    ".cache",
    "cargo",
    "ctx-monorepo",
    "desktop-remote-daemons",
  );
  const cargoRegistryCache = path.join(cacheRoot, "registry");
  const cargoGitCache = path.join(cacheRoot, "git");
  fs.mkdirSync(cargoRegistryCache, { recursive: true });
  fs.mkdirSync(cargoGitCache, { recursive: true });

  const targets = [
    {
      arch: "x86_64",
      platform: "linux/amd64",
      rustTarget: "x86_64-unknown-linux-gnu",
      fileName: "ctx-daemon-linux-x86_64",
    },
    {
      arch: "aarch64",
      platform: "linux/arm64",
      rustTarget: "aarch64-unknown-linux-gnu",
      fileName: "ctx-daemon-linux-aarch64",
    },
  ];

  const daemonEntries = [];
  for (const target of targets) {
    const targetCache = path.join(cacheRoot, "target", target.rustTarget);
    fs.mkdirSync(targetCache, { recursive: true });
    const outPath = path.join(daemonsDir, target.fileName);
    const buildCmd =
      "set -euo pipefail; " +
      "export PATH=\"/usr/local/cargo/bin:$PATH\"; " +
      `rustup target add ${target.rustTarget} >/dev/null 2>&1 || true; ` +
      `cargo build --manifest-path /src/Cargo.toml -p ctx-http --release --target ${target.rustTarget}; ` +
      `install -Dm0755 /target/${target.rustTarget}/release/ctx /out/${target.fileName}`;
    const args = [
      "run",
      "--rm",
      "--platform",
      target.platform,
      "-v",
      `${coreRoot}:/src`,
      "-v",
      `${daemonsDir}:/out`,
      "-v",
      `${targetCache}:/target`,
      "-v",
      `${cargoRegistryCache}:/usr/local/cargo/registry`,
      "-v",
      `${cargoGitCache}:/usr/local/cargo/git`,
      "-w",
      "/src",
      "-e",
      "CARGO_TARGET_DIR=/target",
      builderImage,
      "bash",
      "-lc",
      buildCmd,
    ];
    const res = childProcess.spawnSync(runtime, args, { stdio: "inherit" });
    if (res.status !== 0) {
      throw new Error(
        `failed to build bundled remote daemon for linux/${target.arch} using ${runtime} (${res.status ?? "unknown"})`,
      );
    }
    ensureExecutable(outPath);
    daemonEntries.push({
      id: "ctx-daemon",
      os: "linux",
      arch: target.arch,
      sha256: sha256File(outPath),
      bin: `daemons/${target.fileName}`,
    });
  }

  upsertManifestDaemons(bundleDir, daemonEntries);
};

const resetBundleDir = () => {
  fs.mkdirSync(destBundleDir, { recursive: true });
  // Keep lightweight repo-tracked resources that are used at runtime (and ignore rules).
  const keep = new Set([
    "README.md",
    ".gitkeep",
    ".gitignore",
    "lucide-settings.svg",
    "runtime_lock.v1.json",
    "runtime_lock.v2.json",
  ]);
  for (const entry of fs.readdirSync(destBundleDir)) {
    if (keep.has(entry)) continue;
    fs.rmSync(path.join(destBundleDir, entry), { recursive: true, force: true });
  }
  // Tauri resource globs include bundles/images/**/*; keep at least one stable file
  // so packaging doesn't fail when no generated bundle assets are present yet.
  const providersDir = path.join(destBundleDir, "providers");
  fs.mkdirSync(providersDir, { recursive: true });
  fs.writeFileSync(
    path.join(providersDir, "placeholder.txt"),
    "ctx desktop placeholder provider bundle\n",
    "utf8",
  );
  const runtimesDir = path.join(destBundleDir, "runtimes");
  fs.mkdirSync(runtimesDir, { recursive: true });
  fs.writeFileSync(
    path.join(runtimesDir, "placeholder.txt"),
    "ctx desktop placeholder runtime bundle\n",
    "utf8",
  );
  const imagesDir = path.join(destBundleDir, "images");
  fs.mkdirSync(imagesDir, { recursive: true });
  fs.writeFileSync(
    path.join(imagesDir, "placeholder.txt"),
    "ctx desktop placeholder; generated by desktop_sync_resources.cjs\n",
    "utf8",
  );
  const daemonsDir = path.join(destBundleDir, "daemons");
  fs.mkdirSync(daemonsDir, { recursive: true });
  fs.writeFileSync(
    path.join(daemonsDir, "placeholder.txt"),
    "ctx desktop placeholder daemon bundle\n",
    "utf8",
  );
};

const writePlaceholderBundleManifest = () => {
  resetBundleDir();
  const manifestPath = path.join(destBundleDir, "manifest.json");
  const placeholder = {
    version: 1,
    providers: [],
    runtimes: [],
    images: [],
    daemons: [],
  };
  fs.writeFileSync(manifestPath, `${JSON.stringify(placeholder, null, 2)}\n`, "utf8");
};

const syncBundles = () => {
  if (!fs.existsSync(bundleScript)) {
    throw new Error(`missing bundle script: ${bundleScript}`);
  }
  resetBundleDir();
  const env = {
    ...process.env,
    CTX_BUNDLE_DIR: destBundleDir,
    CTX_BUNDLE_DEPENDENCY_AWARE_RUNTIMES: "0",
  };
  // keep harness/provider bundle builds on their own target dirs; forwarding the
  // desktop CARGO_TARGET_DIR can make adapter binary resolution brittle.
  delete env.CARGO_TARGET_DIR;
  // Parity contract requires both Linux image targets for local container and remote flows.
  env.CTX_BUNDLE_HARNESS_IMAGE = env.CTX_BUNDLE_HARNESS_IMAGE || "both";
  // For desktop builds on macOS we want container mode to work out-of-box without relying on
  // system Podman installs (PATH). Bundle Podman (plus helper binaries) deterministically.
  if (process.platform === "darwin") {
    env.CTX_BUNDLE_PODMAN = env.CTX_BUNDLE_PODMAN || "1";
    if (env.CTX_BUNDLE_PODMAN === "1") {
      const pinnedPodman = readPinnedPodmanConfig({ os: hostManifestOs, arch: hostManifestArch });
      applyPinnedEnv(env, "PODMAN_VERSION", pinnedPodman.version);
      applyPinnedEnv(env, "PODMAN_ARCHIVE_URL", pinnedPodman.archiveUrl);
      applyPinnedEnv(env, "PODMAN_ARCHIVE_SHA256", pinnedPodman.archiveSha256);
      applyPinnedEnv(env, "PODMAN_BIN_REL", pinnedPodman.binRel);
      applyPinnedEnv(env, "PODMAN_EXTRACT_SUBDIR", pinnedPodman.extractSubdir);
      applyPinnedEnv(env, "PODMAN_GVPROXY_URL", pinnedPodman.gvproxyUrl);
      applyPinnedEnv(env, "PODMAN_GVPROXY_SHA256", pinnedPodman.gvproxySha256);
      applyPinnedEnv(env, "PODMAN_VFKIT_URL", pinnedPodman.vfkitUrl);
      applyPinnedEnv(env, "PODMAN_VFKIT_SHA256", pinnedPodman.vfkitSha256);
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
    const requiredProviderIds = readRuntimeLockRequiredProviderIds();
    const codexLinuxArmManagedAvailable = hasManagedProviderTarget("codex", "linux", "aarch64");
    const linuxTargets = [
      {
        arch: "aarch64",
        buildCodexCrp: sourceAllRuntimeProfile || !codexLinuxArmManagedAvailable,
      },
      { arch: "x86_64", buildCodexCrp: false },
    ];
    const linuxProviders = requiredProviderIds.join(",");
    for (const target of linuxTargets) {
      const linuxEnv = {
        ...env,
        CTX_BUNDLE_APPEND: "1",
        CTX_BUNDLE_OS: "linux",
        CTX_BUNDLE_ARCH: target.arch,
        CTX_BUNDLE_ONLY_PROVIDERS: linuxProviders,
        CTX_BUNDLE_SKIP_RUNTIMES: "0",
        CTX_BUNDLE_SKIP_IMAGES: "0",
        CTX_BUNDLE_INCLUDE_BRIDGE: "1",
        CTX_BUNDLE_LOCAL_ADAPTERS: "auto",
        CTX_BUNDLE_BUILD_LOCAL_ADAPTERS: "0",
        CTX_BUNDLE_HARNESS_IMAGE: "1",
        CTX_BUNDLE_PODMAN: "0",
      };
      if (target.buildCodexCrp && !linuxEnv.CTX_BUNDLE_BUILD_CODEX_CRP) {
        // Source-all profile explicitly opts into building codex-crp locally.
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
    for (const providerId of requiredProviderIds) {
      assertBundledProviderTargets(destBundleDir, providerId, parityProviderTargets);
    }
    assertBundledRuntimeTargets(destBundleDir, "node", parityRuntimeTargets);
    assertBundledRuntimeTargets(destBundleDir, "python", parityRuntimeTargets);
    if (env.CTX_BUNDLE_PODMAN === "1") {
      assertBundledRuntimeTargets(destBundleDir, "podman", [{ os: hostManifestOs, arch: hostManifestArch }]);
    }
  }

  if (profile === "debug" || profile === "release") {
    const expectedImage = readRustStringConst(harnessRuntimeRs, "DEFAULT_CONTAINER_IMAGE");
    assertBundledHarnessImageTargets(destBundleDir, expectedImage, parityImageTargets);
  }

  if (shouldBundleRemoteDaemons(process.env)) {
    // Zero-config remote bootstrap requires shipping managed Linux daemon binaries
    // for both arches in every desktop bundle.
    bundleRemoteDaemons(destBundleDir);
  } else {
    console.log(
      "desktop_sync_resources: skipping remote daemon bundle build (CTX_BUNDLE_REMOTE_DAEMONS=0)",
    );
  }

  return destBundleDir;
};

const verifyExistingBundles = () => {
  const manifestPath = path.join(destBundleDir, "manifest.json");
  if (!fs.existsSync(manifestPath)) {
    throw new Error(
      `missing existing bundle manifest: ${manifestPath}. Run CTX_RUNTIME_PROFILE=source-all pnpm -C core desktop:runtime:prepare to materialize bundled artifacts.`,
    );
  }
  readBundleManifest(destBundleDir);
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
    bundles: syncBundlesEnabled ? syncBundles() : verifyExistingBundles(),
  };

  console.log("desktop_sync_resources:", copied);
};

main();
