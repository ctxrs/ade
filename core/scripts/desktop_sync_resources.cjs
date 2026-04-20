const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");
const crypto = require("crypto");
const { shouldBundleRemoteDaemons } = require("./desktop_sync_resources_remote_daemon_policy.cjs");
const { resolveBuildMode, resolveDesktopBuildIdentity } = require("./lib/desktop_build_identity.cjs");
const { parseBoolish, resolveBoolishFlag } = require("./lib/boolish.cjs");
const { resolveCargoTargetDir } = require("./lib/cargo_target_dir.cjs");
const { resolveDesktopWebDistSource } = require("./lib/web_dist_cache.cjs");
const { validateRuntimeLock } = require("./runtime_lock_validate.cjs");

const args = process.argv.slice(2);
const profileIdx = args.indexOf("--profile");
const profile = profileIdx !== -1 ? args[profileIdx + 1] : "debug";
const syncBundlesEnabled = resolveBoolishFlag(process.env.CTX_DESKTOP_SYNC_BUNDLES, true, "CTX_DESKTOP_SYNC_BUNDLES");
const allowManagedAvfRuntimeMissingLocalPayload = resolveBoolishFlag(
  process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD,
  false,
  "CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD",
);

const coreRoot = path.resolve(__dirname, "..");
const desktopTauriRoot = path.join(coreRoot, "apps", "desktop", "src-tauri");
const destBinDir = path.join(desktopTauriRoot, "bin");
const destWebDistDir = path.join(desktopTauriRoot, "web", "dist");
const destBundleDir = path.join(desktopTauriRoot, "bundles");
const ctxMcpCargoTomlPath = path.join(coreRoot, "crates", "ctx-mcp", "Cargo.toml");
const avfLinuxHelperEntitlementsPath = path.join(
  desktopTauriRoot,
  "ctx-avf-linux-helper.entitlements",
);
const bundleScript = path.join(coreRoot, "..", "scripts", "ensure_bundled_harnesses.sh");
const sandboxContainerRuntimeRs = path.join(
  coreRoot,
  "crates",
  "ctx-sandbox-container-runtime",
  "src",
  "lib.rs",
);
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
const CTX_MCP_RUNTIME_ID = "ctx-mcp";
const AVF_LINUX_GUEST_RUNTIME_ID = "avf-linux-guest";
const AVF_LINUX_GUEST_ROOTFS_NAME = "rootfs.raw";
const AVF_LINUX_GUEST_KERNEL_REL = path.join("helpers", "kernel");
const AVF_LINUX_GUEST_INITRD_REL = path.join("helpers", "initrd");
const AVF_LINUX_GUEST_AGENT_REL = path.join("helpers", "guest-agent");
const AVF_LINUX_EGRESS_PROXY_REL = path.join("helpers", "egress-proxy");
const AVF_LINUX_CONTAINER_STACK_REL = path.join("helpers", "container-stack.tar.gz");
const MANIFEST_FILENAME = "manifest.json";
const EFFECTIVE_MANIFEST_FILENAME = "runtime_manifest.effective.json";
const ARTIFACT_IDENTITY_FILENAME = "artifact_identity.json";
const PROVIDER_MATRIX_FILENAME = "provider_matrix.json";
const RUNTIME_LOCK_V2_FILENAME = "runtime_lock.v2.json";
const RUNTIME_LOCK_V1_FILENAME = "runtime_lock.v1.json";
const defaultRuntimeOverridesPath = path.join(coreRoot, "..", ".ctx", "local", "runtime_overrides.json");

const isWindows = process.platform === "win32";
const binExt = isWindows ? ".exe" : "";

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

const resolveBundleCacheRoot = (cacheKey, env = process.env) => {
  const normalizedKey = String(cacheKey || "").trim();
  if (!normalizedKey) {
    throw new Error("cacheKey is required");
  }

  const explicitRoot = String(env.CTX_DESKTOP_BUNDLE_CACHE_ROOT || "").trim();
  if (explicitRoot) {
    return path.join(explicitRoot, normalizedKey);
  }

  const cargoTargetDir = String(env.CARGO_TARGET_DIR || "").trim()
    || resolveCargoTargetDir({ cwd: coreRoot, env });
  if (cargoTargetDir) {
    return path.join(cargoTargetDir, "desktop-sync-cache", normalizedKey);
  }

  return path.join(
    env.HOME || coreRoot,
    ".cache",
    "cargo",
    "ctx-monorepo",
    normalizedKey,
  );
};

const ensureContainerCacheDir = (dir, hostOs = hostManifestOs) => {
  fs.mkdirSync(dir, { recursive: true });
  if (hostOs !== "macos") {
    return dir;
  }
  try {
    fs.chmodSync(dir, 0o777);
  } catch {}
  return dir;
};

const parseAvfLinuxGuestRuntimeVersion = (raw) => {
  const text = String(raw || "");
  if (!text.trim()) return "";

  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

  if (lines.length === 0) return "";

  const versionEntry = lines.find((line) => line.startsWith("version="));
  if (versionEntry) {
    return versionEntry.slice("version=".length).trim();
  }

  const bareVersion = lines.find((line) => !line.includes("="));
  return bareVersion || "";
};

const resolveBundledRuntimeIds = (runtimeIds) => {
  const ids = Array.isArray(runtimeIds)
    ? runtimeIds
        .map((entry) => String(entry || "").trim())
        .filter((entry) => entry.length > 0)
    : [];
  return [...new Set(ids)].sort();
};

const shouldBundleLinuxCtxMcpRuntime = (platform = process.platform) =>
  platform === "darwin" || platform === "linux";

const readCargoPackageVersion = (cargoTomlPath) => {
  if (!fs.existsSync(cargoTomlPath)) {
    throw new Error(`missing Cargo.toml for version lookup: ${cargoTomlPath}`);
  }
  const source = fs.readFileSync(cargoTomlPath, "utf8");
  const match = source.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!match || !match[1]) {
    throw new Error(`failed to resolve package version from ${cargoTomlPath}`);
  }
  return match[1].trim();
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

const readDefaultContainerImage = () =>
  readRustStringConst(sandboxContainerRuntimeRs, "DEFAULT_CONTAINER_IMAGE");

const readBundleManifest = (bundleDir) => {
  const manifestPath = path.join(bundleDir, MANIFEST_FILENAME);
  if (!fs.existsSync(manifestPath)) {
    throw new Error(`missing bundle manifest: ${manifestPath}`);
  }
  try {
    return JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  } catch (e) {
    throw new Error(`failed to parse bundle manifest ${manifestPath}: ${e?.message ?? e}`);
  }
};

const upsertManifestRuntimes = (bundleDir, runtimeEntries) => {
  const manifestPath = path.join(bundleDir, MANIFEST_FILENAME);
  const manifest = readBundleManifest(bundleDir);
  const existing = Array.isArray(manifest?.runtimes) ? manifest.runtimes : [];
  const keep = existing.filter(
    (entry) =>
      !runtimeEntries.some(
        (next) =>
          next.id === entry?.id && next.os === entry?.os && next.arch === entry?.arch,
      ),
  );
  const merged = [...keep, ...runtimeEntries].sort((a, b) =>
    `${a.id}::${a.os}::${a.arch}`.localeCompare(`${b.id}::${b.os}::${b.arch}`),
  );
  manifest.runtimes = merged;
  fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
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

const hasManagedDownloadSource = (component) => {
  const sources = Array.isArray(component?.sources) ? component.sources : [];
  return sources.some((source) => {
    const sourceType = String(source?.source_type || "").trim();
    const uri = String(source?.uri || "").trim();
    const sha256 = String(source?.sha256 || "").trim();
    return (sourceType === "ci" || sourceType === "vendor") && uri.length > 0 && sha256.length > 0;
  });
};

const findLockedComponent = (lock, kind, id, osValue, archValue) => {
  const components = Array.isArray(lock?.components) ? lock.components : [];
  return components.find((component) =>
    component
    && component.kind === kind
    && component.id === id
    && component.os === osValue
    && component.arch === archValue
    && String(component.variant || "default").trim() === "default"
  );
};

const assertManagedAvfRuntimeComponent = (lock, hostOs, hostArch) => {
  const component = findLockedComponent(lock, "runtime", AVF_LINUX_GUEST_RUNTIME_ID, hostOs, hostArch);
  if (!component || !hasManagedDownloadSource(component)) {
    throw new Error(
      `runtime lock missing managed AVF guest runtime source for ${hostOs}/${hostArch}; run pnpm -C core desktop:prep:release`,
    );
  }
  const helpers = component.helpers || {};
  for (const helperName of ["kernel", "initrd", "guest-agent", "egress-proxy", "container-stack"]) {
    const helper = helpers[helperName];
    if (!String(helper?.uri || "").trim() || !String(helper?.sha256 || "").trim()) {
      throw new Error(
        `runtime lock missing AVF helper metadata for ${helperName} (${hostOs}/${hostArch}); run pnpm -C core desktop:prep:release`,
      );
    }
  }
};

const readRuntimeLock = (bundleDir = destBundleDir) => {
  const runtimeLockPath = path.join(bundleDir, RUNTIME_LOCK_V2_FILENAME);
  if (!fs.existsSync(runtimeLockPath)) {
    throw new Error(`missing runtime lock for parity enforcement: ${runtimeLockPath}`);
  }
  try {
    return JSON.parse(fs.readFileSync(runtimeLockPath, "utf8"));
  } catch (error) {
    throw new Error(`failed to parse runtime lock ${runtimeLockPath}: ${error?.message ?? error}`);
  }
};

const resolveRuntimeLockPath = (bundleDir = destBundleDir) => {
  const v2Path = path.join(bundleDir, RUNTIME_LOCK_V2_FILENAME);
  if (fs.existsSync(v2Path)) {
    return v2Path;
  }
  return path.join(bundleDir, RUNTIME_LOCK_V1_FILENAME);
};

const writeEffectiveBundleManifest = (
  bundleDir = destBundleDir,
  profileValue = process.env.CTX_RUNTIME_PROFILE || "parity",
) => {
  const manifestPath = path.join(bundleDir, MANIFEST_FILENAME);
  const runtimeLockPath = resolveRuntimeLockPath(bundleDir);
  const validation = validateRuntimeLock({
    lockPath: runtimeLockPath,
    manifestPath,
    profile: profileValue,
    overridesPath: process.env.CTX_RUNTIME_OVERRIDES_PATH || defaultRuntimeOverridesPath,
  });
  const filteredErrors = filterManagedAvfLocalPayloadErrors({
    errors: validation.errors,
    bundleDir,
    hostOs: hostManifestOs,
    hostArch: hostManifestArch,
  });
  if (filteredErrors.length > 0) {
    throw new Error(
      `runtime lock validation failed while writing ${EFFECTIVE_MANIFEST_FILENAME}: ${filteredErrors.join("; ")}`,
    );
  }
  const effectiveManifestPath = path.join(bundleDir, EFFECTIVE_MANIFEST_FILENAME);
  fs.writeFileSync(
    effectiveManifestPath,
    `${JSON.stringify(validation.effectiveManifest, null, 2)}\n`,
    "utf8",
  );
  return effectiveManifestPath;
};

const parseManagedAvfLocalPayloadValidationTarget = (error) => {
  const text = String(error || "");
  const match = text.match(
    /^runtime (?:root avf-linux-guest|bin avf-linux-guest|helper avf-linux-guest\/[^ ]+) \(([^/]+)\/([^)]+)\) missing (?:directory|file):/,
  );
  if (!match) return null;
  return {
    os: match[1],
    arch: match[2],
  };
};

const filterManagedAvfLocalPayloadErrors = ({
  errors,
  bundleDir = destBundleDir,
  hostOs = hostManifestOs,
  hostArch = hostManifestArch,
  allowManagedRuntime = allowManagedAvfRuntimeMissingLocalPayload,
} = {}) => {
  const list = Array.isArray(errors) ? errors : [];
  if (!allowManagedRuntime || list.length === 0 || hostOs !== "macos") {
    return list;
  }
  let lock = null;
  try {
    lock = readRuntimeLock(bundleDir);
  } catch {
    return list;
  }
  return list.filter((error) => {
    const target = parseManagedAvfLocalPayloadValidationTarget(error);
    if (!target || target.os !== "macos") {
      return true;
    }
    try {
      assertManagedAvfRuntimeComponent(lock, target.os, target.arch);
      return false;
    } catch {
      return true;
    }
  });
};

const assertRuntimeTargetsAvailable = (bundleDir, runtimeId, targets) => {
  const manifest = readBundleManifest(bundleDir);
  const runtimeLock = readRuntimeLock(bundleDir);
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
    if (found) continue;
    if (runtimeId === AVF_LINUX_GUEST_RUNTIME_ID && target.os === "macos") {
      try {
        assertManagedAvfRuntimeComponent(runtimeLock, target.os, target.arch);
        continue;
      } catch (_error) {
        missing.push(`${target.os}/${target.arch}`);
        continue;
      }
    }
    const component = findLockedComponent(runtimeLock, "runtime", runtimeId, target.os, target.arch);
    if (!component || !hasManagedDownloadSource(component)) {
      missing.push(`${target.os}/${target.arch}`);
    }
  }
  if (missing.length > 0) {
    throw new Error(
      `bundle/runtime lock missing ${runtimeId} runtime targets: ${missing.join(
        ", ",
      )}. Container/provider startup requires either bundled runtime payloads or managed runtime-lock sources.`,
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

const normalizeTargetToken = (token, hostValue) => {
  const trimmed = String(token || "").trim();
  if (!trimmed) return null;
  if (trimmed === "host") return hostValue;
  return trimmed;
};

const readRuntimeLockRequiredIds = (kind) => {
  const lock = readRuntimeLock();
  const key = `${kind}_ids`;
  const ids = Array.isArray(lock?.required?.[key])
    ? lock.required[key]
        .map((entry) => String(entry || "").trim())
        .filter((entry) => entry.length > 0)
    : [];
  return [...new Set(ids)].sort();
};

const readRuntimeLockRequiredTargets = (kind, fallbackTargets) => {
  const lock = readRuntimeLock();
  const configured = Array.isArray(lock?.required?.targets?.[kind])
    ? lock.required.targets[kind]
    : [];
  if (configured.length === 0) {
    return fallbackTargets;
  }
  const seen = new Set();
  const targets = [];
  for (const raw of configured) {
    const [rawOs, rawArch] = String(raw || "").split("/");
    const os = normalizeTargetToken(rawOs, hostManifestOs);
    const arch = normalizeTargetToken(rawArch, hostManifestArch);
    if (!os || !arch) continue;
    const key = `${os}/${arch}`;
    if (seen.has(key)) continue;
    seen.add(key);
    targets.push({ os, arch });
  }
  return targets.length > 0 ? targets : fallbackTargets;
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

const SHA256_CHUNK_SIZE = 8 * 1024 * 1024;

const sha256File = (filePath) => {
  const hash = crypto.createHash("sha256");
  const fd = fs.openSync(filePath, "r");
  const buffer = Buffer.allocUnsafe(SHA256_CHUNK_SIZE);
  try {
    while (true) {
      const bytesRead = fs.readSync(fd, buffer, 0, buffer.length, null);
      if (bytesRead === 0) break;
      hash.update(bytesRead === buffer.length ? buffer : buffer.subarray(0, bytesRead));
    }
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest("hex");
};

const commandExists = (name) => {
  const res = childProcess.spawnSync(name, ["--version"], { stdio: "ignore" });
  return res.status === 0;
};

const runtimeProbeArgs = (runtime) => {
  if (runtime === "docker") return ["info", "--format", "{{.ServerVersion}}"];
  if (runtime === "nerdctl") return ["info"];
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
    { name: "nerdctl", installed: commandExists("nerdctl"), usable: false },
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
      )}). Ensure the runtime daemon/service is running (for example, start Docker Desktop or containerd) and rerun desktop prep.`,
    );
  }

  throw new Error(
    "remote daemon bundling requires docker or nerdctl in PATH. Install a container runtime and rerun desktop prep.",
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

const stageAvfLinuxGuestRuntime = (bundleDir) => {
  const rawSourceDir = String(process.env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR || "").trim();
  if (!rawSourceDir) {
    return null;
  }

  const sourceDir = path.resolve(rawSourceDir);
  if (!fs.existsSync(sourceDir) || !fs.statSync(sourceDir).isDirectory()) {
    throw new Error(
      `CTX_AVF_LINUX_GUEST_RUNTIME_DIR must point to a directory: ${sourceDir}`,
    );
  }

  const rootfsPath = path.join(sourceDir, AVF_LINUX_GUEST_ROOTFS_NAME);
  const kernelPath = path.join(sourceDir, AVF_LINUX_GUEST_KERNEL_REL);
  const initrdPath = path.join(sourceDir, AVF_LINUX_GUEST_INITRD_REL);
  const guestAgentPath = path.join(sourceDir, AVF_LINUX_GUEST_AGENT_REL);
  const egressProxyPath = path.join(sourceDir, AVF_LINUX_EGRESS_PROXY_REL);
  const containerStackPath = path.join(sourceDir, AVF_LINUX_CONTAINER_STACK_REL);
  for (const requiredPath of [
    rootfsPath,
    kernelPath,
    initrdPath,
    guestAgentPath,
    egressProxyPath,
    containerStackPath,
  ]) {
    if (!fs.existsSync(requiredPath) || !fs.statSync(requiredPath).isFile()) {
      throw new Error(
        `AVF Linux guest runtime is incomplete; missing required file: ${requiredPath}`,
      );
    }
  }

  const explicitVersion = String(process.env.CTX_AVF_LINUX_GUEST_RUNTIME_VERSION || "").trim();
  const versionFile = path.join(sourceDir, "version.txt");
  const fileVersion = fs.existsSync(versionFile)
    ? parseAvfLinuxGuestRuntimeVersion(fs.readFileSync(versionFile, "utf8"))
    : "";
  const version = explicitVersion || fileVersion || "local";

  const runtimeParentRel = path.join(
    "runtimes",
    AVF_LINUX_GUEST_RUNTIME_ID,
    hostManifestOs,
    hostManifestArch,
  );
  const runtimeParentDir = path.join(bundleDir, runtimeParentRel);
  fs.rmSync(runtimeParentDir, { recursive: true, force: true });

  const runtimeRootRel = path.join(runtimeParentRel, version);
  const runtimeRootDir = path.join(bundleDir, runtimeRootRel);
  copyDirRecursive(sourceDir, runtimeRootDir);

  const bundledRootfsPath = path.join(runtimeRootDir, AVF_LINUX_GUEST_ROOTFS_NAME);
  const bundledKernelPath = path.join(runtimeRootDir, AVF_LINUX_GUEST_KERNEL_REL);
  const bundledInitrdPath = path.join(runtimeRootDir, AVF_LINUX_GUEST_INITRD_REL);
  const bundledGuestAgentPath = path.join(runtimeRootDir, AVF_LINUX_GUEST_AGENT_REL);
  const bundledEgressProxyPath = path.join(runtimeRootDir, AVF_LINUX_EGRESS_PROXY_REL);
  const bundledContainerStackPath = path.join(runtimeRootDir, AVF_LINUX_CONTAINER_STACK_REL);
  for (const requiredPath of [
    bundledRootfsPath,
    bundledKernelPath,
    bundledInitrdPath,
    bundledGuestAgentPath,
    bundledEgressProxyPath,
    bundledContainerStackPath,
  ]) {
    if (!fs.existsSync(requiredPath) || !fs.statSync(requiredPath).isFile()) {
      throw new Error(
        `staged AVF Linux guest runtime is incomplete; missing required file: ${requiredPath}`,
      );
    }
  }

  upsertManifestRuntimes(bundleDir, [
    {
      id: AVF_LINUX_GUEST_RUNTIME_ID,
      version,
      os: hostManifestOs,
      arch: hostManifestArch,
      sha256: sha256File(bundledRootfsPath),
      root: runtimeRootRel,
      bin: AVF_LINUX_GUEST_ROOTFS_NAME,
    },
  ]);

  return {
    sourceDir,
    version,
    runtimeRootDir,
    rootfsPath: bundledRootfsPath,
    kernelPath: bundledKernelPath,
    initrdPath: bundledInitrdPath,
    guestAgentPath: bundledGuestAgentPath,
    egressProxyPath: bundledEgressProxyPath,
    containerStackPath: bundledContainerStackPath,
  };
};

const buildRemoteDaemonContainerArgs = ({
  runtime,
  builderImage,
  coreDir,
  daemonsDir,
  targetCache,
  cargoHome,
  rustupHome,
  target,
  hostOs = hostManifestOs,
}) => {
  const preparedDaemonsDir = ensureContainerCacheDir(daemonsDir, hostOs);
  const buildCmd =
    "set -euo pipefail; " +
    "export CARGO_HOME=\"/cargo-home\"; " +
    "export RUSTUP_HOME=\"/rustup-home\"; " +
    "export PATH=\"$CARGO_HOME/bin:/usr/local/cargo/bin:$PATH\"; " +
    "mkdir -p /out; " +
    "rustup toolchain install stable --profile minimal --no-self-update >/dev/null 2>&1 || true; " +
    `rustup target add --toolchain stable ${target.rustTarget} >/dev/null 2>&1 || true; ` +
    `cargo +stable build --manifest-path /src/Cargo.toml -p ctx-http --release --target ${target.rustTarget}; ` +
    `install -Dm0755 /target/${target.rustTarget}/release/ctx /out/${target.fileName}`;
  return [
    runtime,
    [
      "run",
      "--rm",
      "--platform",
      target.platform,
      "-v",
      `${coreDir}:/src`,
      "-v",
      `${preparedDaemonsDir}:/out`,
      "-v",
      `${targetCache}:/target`,
      "-v",
      `${cargoHome}:/cargo-home`,
      "-v",
      `${rustupHome}:/rustup-home`,
      "-w",
      "/src",
      "-e",
      "CARGO_TARGET_DIR=/target",
      "-e",
      "CARGO_HOME=/cargo-home",
      "-e",
      "RUSTUP_HOME=/rustup-home",
      builderImage,
      "bash",
      "-lc",
      buildCmd,
    ],
  ];
};

const linuxBundleTargetForArch = (arch) => {
  switch (arch) {
    case "aarch64":
      return {
        arch,
        platform: "linux/arm64",
        rustTarget: "aarch64-unknown-linux-gnu",
      };
    case "x86_64":
      return {
        arch,
        platform: "linux/amd64",
        rustTarget: "x86_64-unknown-linux-gnu",
      };
    default:
      throw new Error(`unsupported host arch for bundled linux ctx-mcp runtime: ${arch}`);
  }
};

const buildLinuxCtxMcpContainerArgs = ({
  runtime,
  builderImage,
  coreDir,
  runtimesDir,
  targetCache,
  cargoHome,
  rustupHome,
  target,
  runtimeVersion,
  hostOs = hostManifestOs,
}) => {
  const preparedRuntimesDir = ensureContainerCacheDir(runtimesDir, hostOs);
  ensureContainerCacheDir(path.join(preparedRuntimesDir, "runtimes"), hostOs);
  const runtimeRootRel = path.posix.join(
    "runtimes",
    CTX_MCP_RUNTIME_ID,
    "linux",
    target.arch,
    runtimeVersion,
  );
  const buildCmd =
    "set -euo pipefail; " +
    "export CARGO_HOME=\"/cargo-home\"; " +
    "export RUSTUP_HOME=\"/rustup-home\"; " +
    "export PATH=\"$CARGO_HOME/bin:/usr/local/cargo/bin:$PATH\"; " +
    "mkdir -p /out; " +
    "rustup toolchain install stable --profile minimal --no-self-update >/dev/null 2>&1 || true; " +
    `rustup target add --toolchain stable ${target.rustTarget} >/dev/null 2>&1 || true; ` +
    `cargo +stable build --manifest-path /src/Cargo.toml -p ctx-mcp --release --target ${target.rustTarget}; ` +
    `install -Dm0755 /target/${target.rustTarget}/release/ctx-mcp /out/${runtimeRootRel}/ctx-mcp; ` +
    `chmod -R 0777 /out/runtimes/${CTX_MCP_RUNTIME_ID}`;
  return [
    runtime,
    [
      "run",
      "--rm",
      "--platform",
      target.platform,
      "-v",
      `${coreDir}:/src`,
      "-v",
      `${preparedRuntimesDir}:/out`,
      "-v",
      `${targetCache}:/target`,
      "-v",
      `${cargoHome}:/cargo-home`,
      "-v",
      `${rustupHome}:/rustup-home`,
      "-w",
      "/src",
      "-e",
      "CARGO_TARGET_DIR=/target",
      "-e",
      "CARGO_HOME=/cargo-home",
      "-e",
      "RUSTUP_HOME=/rustup-home",
      builderImage,
      "bash",
      "-lc",
      buildCmd,
    ],
  ];
};

const bundleRemoteDaemons = (bundleDir) => {
  const runtime = resolveContainerRuntime();
  const builderImage = resolveRemoteDaemonBuilderImage();
  const daemonsDir = path.join(bundleDir, "daemons");
  fs.mkdirSync(daemonsDir, { recursive: true });
  const cacheRoot = resolveBundleCacheRoot("desktop-remote-daemons");
  const cargoHome = ensureContainerCacheDir(path.join(cacheRoot, "cargo-home"));
  const rustupHome = ensureContainerCacheDir(path.join(cacheRoot, "rustup-home"));

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
    const targetCache = ensureContainerCacheDir(path.join(cacheRoot, "target", target.rustTarget));
    const outPath = path.join(daemonsDir, target.fileName);
    const [spawnCmd, args] = buildRemoteDaemonContainerArgs({
      runtime,
      builderImage,
      coreDir: coreRoot,
      daemonsDir,
      targetCache,
      cargoHome,
      rustupHome,
      target,
    });
    const res = childProcess.spawnSync(spawnCmd, args, { stdio: "inherit" });
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

const bundleLinuxCtxMcpRuntime = (bundleDir) => {
  const runtime = resolveContainerRuntime();
  const builderImage = resolveRemoteDaemonBuilderImage();
  const runtimesDir = bundleDir;
  fs.mkdirSync(runtimesDir, { recursive: true });
  const runtimeVersion = readCargoPackageVersion(ctxMcpCargoTomlPath);
  const cacheRoot = resolveBundleCacheRoot(
    path.join("desktop-bundled-runtimes", CTX_MCP_RUNTIME_ID),
  );
  const cargoHome = ensureContainerCacheDir(path.join(cacheRoot, "cargo-home"));
  const rustupHome = ensureContainerCacheDir(path.join(cacheRoot, "rustup-home"));

  const target = linuxBundleTargetForArch(hostManifestArch);
  const targetCache = ensureContainerCacheDir(path.join(cacheRoot, "target", target.rustTarget));

  const [spawnCmd, args] = buildLinuxCtxMcpContainerArgs({
    runtime,
    builderImage,
    coreDir: coreRoot,
    runtimesDir,
    targetCache,
    cargoHome,
    rustupHome,
    target,
    runtimeVersion,
  });
  const res = childProcess.spawnSync(spawnCmd, args, { stdio: "inherit" });
  if (res.status !== 0) {
    throw new Error(
      `failed to build bundled linux ctx-mcp runtime for ${target.arch} using ${runtime} (${res.status ?? "unknown"})`,
    );
  }

  const runtimeRootRel = path.join("runtimes", CTX_MCP_RUNTIME_ID, "linux", target.arch, runtimeVersion);
  const outPath = path.join(bundleDir, runtimeRootRel, "ctx-mcp");
  ensureExecutable(outPath);
  upsertManifestRuntimes(bundleDir, [
    {
      id: CTX_MCP_RUNTIME_ID,
      version: runtimeVersion,
      os: "linux",
      arch: target.arch,
      sha256: sha256File(outPath),
      root: runtimeRootRel,
      bin: "ctx-mcp",
    },
  ]);
};

const resetBundleDir = (bundleDir = destBundleDir) => {
  fs.mkdirSync(bundleDir, { recursive: true });
  // Keep lightweight repo-tracked resources that are used at runtime (and ignore rules).
  const keep = new Set([
    "README.md",
    ".gitkeep",
    ".gitignore",
    "lucide-settings.svg",
    "tauri_tools_lock.v1.json",
    "runtime_lock.v1.json",
    "runtime_lock.v2.json",
  ]);
  for (const entry of fs.readdirSync(bundleDir)) {
    if (keep.has(entry)) continue;
    fs.rmSync(path.join(bundleDir, entry), {
      recursive: true,
      force: true,
      maxRetries: 5,
      retryDelay: 50,
    });
  }
  // Tauri resource globs include bundles/images/**/*; keep at least one stable file
  // so packaging doesn't fail when no generated bundle assets are present yet.
  const providersDir = path.join(bundleDir, "providers");
  fs.mkdirSync(providersDir, { recursive: true });
  fs.writeFileSync(
    path.join(providersDir, "placeholder.txt"),
    "ctx desktop placeholder provider bundle\n",
    "utf8",
  );
  const runtimesDir = path.join(bundleDir, "runtimes");
  fs.mkdirSync(runtimesDir, { recursive: true });
  fs.writeFileSync(
    path.join(runtimesDir, "placeholder.txt"),
    "ctx desktop placeholder runtime bundle\n",
    "utf8",
  );
  const imagesDir = path.join(bundleDir, "images");
  fs.mkdirSync(imagesDir, { recursive: true });
  fs.writeFileSync(
    path.join(imagesDir, "placeholder.txt"),
    "ctx desktop placeholder; generated by desktop_sync_resources.cjs\n",
    "utf8",
  );
  const daemonsDir = path.join(bundleDir, "daemons");
  fs.mkdirSync(daemonsDir, { recursive: true });
  fs.writeFileSync(
    path.join(daemonsDir, "placeholder.txt"),
    "ctx desktop placeholder daemon bundle\n",
    "utf8",
  );
};

const writePlaceholderBundleManifest = (bundleDir = destBundleDir) => {
  resetBundleDir(bundleDir);
  const manifestPath = path.join(bundleDir, "manifest.json");
  const effectiveManifestPath = path.join(bundleDir, EFFECTIVE_MANIFEST_FILENAME);
  const placeholder = {
    version: 1,
    providers: [],
    runtimes: [],
    images: [],
    daemons: [],
  };
  fs.writeFileSync(manifestPath, `${JSON.stringify(placeholder, null, 2)}\n`, "utf8");
  fs.rmSync(effectiveManifestPath, { force: true });
};

const resolveArtifactIdentityMode = (env = process.env) => {
  const explicitMode = String(env.CTX_DESKTOP_BUILD_MODE || "").trim();
  if (explicitMode) {
    return resolveBuildMode({ env, requestedMode: explicitMode });
  }
  if (profile === "debug") {
    return "packaged";
  }
  if (String(env.RELEASE_CHANNEL || "").trim() === "e2e") {
    return "e2e";
  }
  if (
    String(env.RELEASE_CHANNEL || "").trim()
    || String(env.RELEASE_SOURCE_COMMIT || "").trim()
    || String(env.RELEASE_VERSION || "").trim()
    || String(env.CTX_RELEASE_EFFECTIVE_VERSION || "").trim()
  ) {
    return "release";
  }
  return "packaged";
};

const writeArtifactIdentity = (bundleDir = destBundleDir, env = process.env) => {
  const identity = resolveDesktopBuildIdentity({
    coreRoot,
    env,
    mode: resolveArtifactIdentityMode(env),
  });
  const artifactIdentityPath = path.join(bundleDir, ARTIFACT_IDENTITY_FILENAME);
  fs.writeFileSync(artifactIdentityPath, `${JSON.stringify(identity, null, 2)}\n`, "utf8");
  return {
    artifactIdentityPath,
    identity,
  };
};

const resolveBundleProviderMatrixSource = (env = process.env) => {
  const explicitPath = String(env.CTX_BUNDLE_MATRIX_JSON || "").trim();
  if (explicitPath) {
    return path.resolve(explicitPath);
  }
  return path.join(coreRoot, "crates", "ctx-provider-accounts", "src", "provider_matrix.json");
};

const writeBundledProviderManifest = (bundleDir = destBundleDir, env = process.env) => {
  const sourcePath = resolveBundleProviderMatrixSource(env);
  if (!fs.existsSync(sourcePath)) {
    throw new Error(`missing provider manifest for bundle sync: ${sourcePath}`);
  }
  const targetPath = path.join(bundleDir, PROVIDER_MATRIX_FILENAME);
  fs.copyFileSync(sourcePath, targetPath);
  return targetPath;
};

const ensureCargoBinOnPath = (env) => {
  const resolvedEnv = { ...env };
  const cargoHome = String(resolvedEnv.CARGO_HOME || "").trim();
  if (!cargoHome) {
    return resolvedEnv;
  }
  const cargoBinDir = path.join(cargoHome, "bin");
  const pathDelimiter = path.delimiter;
  const currentPath = String(resolvedEnv.PATH || "");
  const pathEntries = currentPath.split(pathDelimiter).filter(Boolean);
  if (!pathEntries.includes(cargoBinDir)) {
    resolvedEnv.PATH = currentPath
      ? `${cargoBinDir}${pathDelimiter}${currentPath}`
      : cargoBinDir;
  }
  return resolvedEnv;
};

const syncBundles = () => {
  if (!fs.existsSync(bundleScript)) {
    throw new Error(`missing bundle script: ${bundleScript}`);
  }
  writePlaceholderBundleManifest();
  const env = ensureCargoBinOnPath({
    ...process.env,
    CTX_BUNDLE_DIR: destBundleDir,
    CTX_BUNDLE_DEPENDENCY_AWARE_RUNTIMES: "0",
  });
  Object.assign(env, resolvePrimaryBundleTargetEnv({ env }));
  const requiredProviderIds = readRuntimeLockRequiredIds("provider");
  const requiredRuntimeIds = readRuntimeLockRequiredIds("runtime");
  const bundledRuntimeIds = resolveBundledRuntimeIds(requiredRuntimeIds);
  const requiredImageIds = readRuntimeLockRequiredIds("image");
  const requiredProviderTargets = readRuntimeLockRequiredTargets("provider", parityProviderTargets);
  const requiredRuntimeTargets = readRuntimeLockRequiredTargets("runtime", parityRuntimeTargets);
  const requiredImageTargets = readRuntimeLockRequiredTargets("image", parityImageTargets);
  // keep harness/provider bundle builds on their own target dirs; forwarding the
  // desktop CARGO_TARGET_DIR can make adapter binary resolution brittle.
  delete env.CARGO_TARGET_DIR;
  env.CTX_BUNDLE_ONLY_PROVIDERS = env.CTX_BUNDLE_ONLY_PROVIDERS
    || (requiredProviderIds.length > 0 ? requiredProviderIds.join(",") : "__none__");
  if (bundledRuntimeIds.length === 0) {
    env.CTX_BUNDLE_SKIP_RUNTIMES = env.CTX_BUNDLE_SKIP_RUNTIMES || "1";
  }
  const requiresHarnessImage = requiredImageIds.includes("ctx-harness");
  if (!env.CTX_BUNDLE_HARNESS_IMAGE) {
    env.CTX_BUNDLE_HARNESS_IMAGE = profile === "source-all" && requiresHarnessImage ? "both" : "0";
  }
  const bundleHarnessImageMode = String(env.CTX_BUNDLE_HARNESS_IMAGE || "").trim().toLowerCase();
  const bundleHarnessImages = parseBoolish(bundleHarnessImageMode) === true
    || bundleHarnessImageMode === "both"
    || bundleHarnessImageMode === "all";
  if (requiredImageIds.length === 0 || !bundleHarnessImages) {
    env.CTX_BUNDLE_SKIP_IMAGES = env.CTX_BUNDLE_SKIP_IMAGES || "1";
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
    const linuxProviderTargets = requiredProviderTargets.filter((target) => target.os === "linux");
    const linuxRuntimeTargets = requiredRuntimeTargets.filter((target) => target.os === "linux");
    const linuxImageTargets = requiredImageTargets.filter((target) => target.os === "linux");
    const linuxArchSet = new Set([
      ...(requiredProviderIds.length > 0 ? linuxProviderTargets.map((target) => target.arch) : []),
      ...(bundledRuntimeIds.length > 0 ? linuxRuntimeTargets.map((target) => target.arch) : []),
      ...(requiredImageIds.length > 0 ? linuxImageTargets.map((target) => target.arch) : []),
    ]);
    const linuxArchTargets = [...linuxArchSet].map((arch) => ({ arch }));
    const linuxProviders = requiredProviderIds.length > 0 ? requiredProviderIds.join(",") : "__none__";

    for (const target of linuxArchTargets) {
      const needsLinuxRuntime = bundledRuntimeIds.length > 0
        && linuxRuntimeTargets.some((entry) => entry.arch === target.arch);
      const needsLinuxImage = requiredImageIds.length > 0
        && linuxImageTargets.some((entry) => entry.arch === target.arch);
      const shouldBundleLinuxImage = bundleHarnessImages && needsLinuxImage;
      const linuxEnv = {
        ...env,
        CTX_BUNDLE_APPEND: "1",
        CTX_BUNDLE_OS: "linux",
        CTX_BUNDLE_ARCH: target.arch,
        CTX_BUNDLE_ONLY_PROVIDERS: linuxProviders,
        CTX_BUNDLE_SKIP_RUNTIMES: needsLinuxRuntime ? "0" : "1",
        CTX_BUNDLE_SKIP_IMAGES: shouldBundleLinuxImage ? "0" : "1",
        CTX_BUNDLE_INCLUDE_BRIDGE: "1",
        CTX_BUNDLE_LOCAL_ADAPTERS: "off",
        CTX_BUNDLE_BUILD_LOCAL_ADAPTERS: "0",
        CTX_BUNDLE_HARNESS_IMAGE: shouldBundleLinuxImage ? "1" : "0",
      };
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
      assertBundledProviderTargets(destBundleDir, providerId, requiredProviderTargets);
    }
    for (const runtimeId of bundledRuntimeIds) {
      assertRuntimeTargetsAvailable(destBundleDir, runtimeId, requiredRuntimeTargets);
    }
  }

  const stagedAvfGuestRuntime = stageAvfLinuxGuestRuntime(destBundleDir);
  for (const runtimeId of bundledRuntimeIds) {
    assertRuntimeTargetsAvailable(destBundleDir, runtimeId, requiredRuntimeTargets);
  }

  if (
    (profile === "debug" || profile === "release")
    && requiredImageIds.includes("ctx-harness")
    && bundleHarnessImages
  ) {
    const expectedImage = readDefaultContainerImage();
    assertBundledHarnessImageTargets(destBundleDir, expectedImage, requiredImageTargets);
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
  if (shouldBundleLinuxCtxMcpRuntime()) {
    bundleLinuxCtxMcpRuntime(destBundleDir);
  }

  const effectiveManifestPath = writeEffectiveBundleManifest(destBundleDir);
  const artifactIdentity = writeArtifactIdentity(destBundleDir);
  const bundledProviderManifestPath = writeBundledProviderManifest(destBundleDir, process.env);

  return {
    bundledProviderManifestPath,
    bundleDir: destBundleDir,
    stagedAvfGuestRuntime,
    effectiveManifestPath,
    artifactIdentity,
  };
};

const verifyExistingBundles = () => {
  const manifestPath = path.join(destBundleDir, MANIFEST_FILENAME);
  if (!fs.existsSync(manifestPath)) {
    throw new Error(
      `missing existing bundle manifest: ${manifestPath}. Run CTX_RUNTIME_PROFILE=source-all pnpm -C core desktop:runtime:prepare to materialize bundled artifacts.`,
    );
  }
  readBundleManifest(destBundleDir);
  writeEffectiveBundleManifest(destBundleDir);
  writeArtifactIdentity(destBundleDir);
  writeBundledProviderManifest(destBundleDir, process.env);
  return destBundleDir;
};

function fallbackHostTarget({ platform = process.platform, arch = process.arch } = {}) {
  if (platform === "darwin" && arch === "arm64") {
    return "aarch64-apple-darwin";
  }
  if (platform === "darwin" && arch === "x64") {
    return "x86_64-apple-darwin";
  }
  if (platform === "linux" && arch === "arm64") {
    return "aarch64-unknown-linux-gnu";
  }
  if (platform === "linux" && arch === "x64") {
    return "x86_64-unknown-linux-gnu";
  }
  if (platform === "win32" && arch === "arm64") {
    return "aarch64-pc-windows-msvc";
  }
  if (platform === "win32" && arch === "x64") {
    return "x86_64-pc-windows-msvc";
  }
  return null;
}

function bundleTargetFromRustTriple(targetTriple) {
  switch (String(targetTriple || "").trim()) {
    case "aarch64-apple-darwin":
      return { os: "macos", arch: "aarch64" };
    case "x86_64-apple-darwin":
      return { os: "macos", arch: "x86_64" };
    case "aarch64-unknown-linux-gnu":
      return { os: "linux", arch: "aarch64" };
    case "x86_64-unknown-linux-gnu":
      return { os: "linux", arch: "x86_64" };
    case "aarch64-pc-windows-msvc":
      return { os: "windows", arch: "aarch64" };
    case "x86_64-pc-windows-msvc":
      return { os: "windows", arch: "x86_64" };
    default:
      return null;
  }
}

const resolveHostTarget = ({
  env = process.env,
  platform = process.platform,
  arch = process.arch,
  execSyncImpl = childProcess.execSync,
} = {}) => {
  const envTarget = env.CARGO_BUILD_TARGET || env.TAURI_ENV_TARGET_TRIPLE;
  if (envTarget) return envTarget;
  try {
    const info = execSyncImpl("rustc -vV", { encoding: "utf8" });
    const match = info.match(/^host:\s+(.+)$/m);
    if (match) {
      return match[1].trim();
    }
  } catch {
    // Fall back to the known Node platform/arch mapping below.
  }
  return fallbackHostTarget({ platform, arch });
};

function resolvePrimaryBundleTargetEnv({ env = process.env } = {}) {
  const target = bundleTargetFromRustTriple(resolveHostTarget({ env }));
  if (!target) {
    return {};
  }
  const resolved = {};
  if (!String(env.CTX_BUNDLE_OS || "").trim()) {
    resolved.CTX_BUNDLE_OS = target.os;
  }
  if (!String(env.CTX_BUNDLE_ARCH || "").trim()) {
    resolved.CTX_BUNDLE_ARCH = target.arch;
  }
  return resolved;
}

const copySidecarBinary = ({
  sourceDir,
  sourcePath = "",
  destDir,
  sourceName,
  destName = sourceName,
  targetTriple = null,
  binExtOverride = binExt,
  platform = process.platform,
  spawnSyncImpl = childProcess.spawnSync,
  helperEntitlementsPath = avfLinuxHelperEntitlementsPath,
}) => {
  const src = String(sourcePath || "").trim()
    ? path.resolve(String(sourcePath))
    : path.join(sourceDir, `${sourceName}${binExtOverride}`);
  const dest = path.join(destDir, `${destName}${binExtOverride}`);
  const destTarget = targetTriple ? path.join(destDir, `${destName}-${targetTriple}${binExtOverride}`) : null;
  if (!String(sourcePath || "").trim() && !fs.existsSync(src)) {
    if (platform === "darwin" && sourceName === "ctx-avf-linux-helper") {
      const args = [
        "build",
        "--manifest-path",
        path.join("apps", "desktop", "src-tauri", "Cargo.toml"),
        "--bin",
        "ctx-avf-linux-helper",
      ];
      if (profile === "release") {
        args.push("--release");
      }
      const res = spawnSyncImpl("cargo", args, {
        cwd: coreRoot,
        env: {
          ...process.env,
          CTX_DESKTOP_SKIP_TAURI_BUILD: "1",
          CARGO_TARGET_DIR: resolveCargoTargetDir({ cwd: coreRoot }),
        },
        stdio: "inherit",
      });
      if (res.status !== 0) {
        throw new Error(
          `failed to build ctx-avf-linux-helper before desktop resource sync (${res.status ?? "unknown"})`,
        );
      }
    }
  }
  if (!fs.existsSync(src)) {
    throw new Error(`missing sidecar: ${src} (did you run cargo build or provide an explicit CTX_DESKTOP_*_BIN override?)`);
  }
  fs.mkdirSync(destDir, { recursive: true });
  const stageAndPublish = (finalPath) => {
    const stagedPath = path.join(
      destDir,
      `.${path.basename(finalPath)}.${process.pid}.${crypto.randomUUID()}.tmp`,
    );
    try {
      fs.copyFileSync(src, stagedPath);
      ensureExecutable(stagedPath);
      if (platform === "darwin" && sourceName === "ctx-avf-linux-helper") {
        const res = spawnSyncImpl(
          "/usr/bin/codesign",
          ["--force", "--sign", "-", "--entitlements", helperEntitlementsPath, stagedPath],
          { stdio: "inherit" },
        );
        if (res.status !== 0) {
          throw new Error(
            `failed to codesign ctx-avf-linux-helper for desktop resource sync (${res.status ?? "unknown"})`,
          );
        }
      }
      fs.renameSync(stagedPath, finalPath);
    } catch (error) {
      fs.rmSync(stagedPath, { force: true });
      throw error;
    }
  };
  stageAndPublish(dest);
  if (destTarget) {
    stageAndPublish(destTarget);
  }
  if (platform === "darwin" && sourceName === "ctx-avf-linux-helper") {
    for (const pathToSign of [dest, destTarget].filter(Boolean)) {
      if (!fs.existsSync(pathToSign)) {
        throw new Error(`missing signed AVF helper sidecar at ${pathToSign}`);
      }
      const res = spawnSyncImpl(
        "/usr/bin/codesign",
        ["--verify", "--verbose=1", pathToSign],
        { stdio: "ignore" },
      );
      if (res.status !== 0) {
        throw new Error(`signed AVF helper sidecar failed verification at ${pathToSign}`);
      }
    }
  }
  return { src, dest, destTarget };
};

const copySidecar = (sourceName, destName = sourceName, explicitSourcePath = "") => {
  const { dest } = copySidecarBinary({
    sourceDir: path.join(resolveCargoTargetDir({ cwd: coreRoot }), profile),
    sourcePath: explicitSourcePath,
    destDir: destBinDir,
    sourceName,
    destName,
    targetTriple: resolveHostTarget(),
  });
  return dest;
};

const copyWebDist = () => {
  const srcDist = resolveDesktopWebDistSource(coreRoot, process.env);
  if (!fs.existsSync(srcDist)) {
    throw new Error(`missing web dist: ${srcDist}`);
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
    ctxDaemon: copySidecar("ctx", "ctx-daemon", process.env.CTX_DESKTOP_CTX_BIN || ""),
    ctxMcp: copySidecar("ctx-mcp", "ctx-mcp", process.env.CTX_DESKTOP_CTX_MCP_BIN || ""),
    avfLinuxHelper: process.platform === "darwin"
      ? copySidecar("ctx-avf-linux-helper", "ctx-avf-linux-helper", process.env.CTX_DESKTOP_AVF_LINUX_HELPER_BIN || "")
      : null,
    webDist: copyWebDist(),
    bundleInfo: syncBundlesEnabled
      ? syncBundles()
      : { bundleDir: verifyExistingBundles(), stagedAvfGuestRuntime: null },
  };
  copied.bundles = copied.bundleInfo.bundleDir;
  if (copied.bundleInfo.stagedAvfGuestRuntime) {
    copied.avfLinuxGuestRuntime = copied.bundleInfo.stagedAvfGuestRuntime.runtimeRootDir;
  }
  delete copied.bundleInfo;

  console.log("desktop_sync_resources:", copied);
};

if (require.main === module) {
  main();
} else {
  module.exports = {
    __desktopSyncResourcesTestHooks: {
      assertRuntimeTargetsAvailable,
      bundleTargetFromRustTriple,
      buildLinuxCtxMcpContainerArgs,
      buildRemoteDaemonContainerArgs,
      fallbackHostTarget,
      filterManagedAvfLocalPayloadErrors,
      readDefaultContainerImage,
      resetBundleDir,
      resolveHostTarget,
      resolvePrimaryBundleTargetEnv,
      resolveBundleCacheRoot,
      resolveArtifactIdentityMode,
      shouldBundleLinuxCtxMcpRuntime,
      writeBundledProviderManifest,
      writePlaceholderBundleManifest,
      writeEffectiveBundleManifest,
      writeArtifactIdentity,
      ensureCargoBinOnPath,
      ensureContainerCacheDir,
    },
    copySidecarBinary,
    parseAvfLinuxGuestRuntimeVersion,
    resolveBundledRuntimeIds,
    stageAvfLinuxGuestRuntime,
  };
}
