#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const defaultBundleDir = path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles");
const defaultManifestPath = path.join(defaultBundleDir, "manifest.json");
const defaultLockPath = path.join(defaultBundleDir, "runtime_lock.v2.json");

const AVF_RUNTIME_ID = "avf-linux-guest";
const TARGET_OS = "macos";
const TARGET_ARCH = "aarch64";

const isNonEmptyString = (value) => typeof value === "string" && value.trim().length > 0;

const usage = () => {
  process.stdout.write(
    [
      "Usage:",
      "  node core/scripts/avf_runtime_lock_freshness.cjs --runtime-dir <prepared-runtime-dir>",
      "    [--bundle-dir <bundles-dir>]",
      "    [--manifest-path <manifest.json>]",
      "    [--lock-path <runtime_lock.v2.json>]",
      "    [--allow-managed-runtime]",
      "",
      "Checks that the prepared AVF Linux guest runtime built from current source matches",
      "the bundled desktop manifest/runtime-lock metadata for macOS Apple Silicon.",
      "",
    ].join("\n"),
  );
};

const parseArgs = (argv) => {
  const cli = {
    runtimeDir: "",
    bundleDir: defaultBundleDir,
    manifestPath: defaultManifestPath,
    lockPath: defaultLockPath,
    allowManagedRuntime: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--runtime-dir") {
      cli.runtimeDir = String(argv[++i] || "").trim();
      continue;
    }
    if (arg === "--bundle-dir") {
      cli.bundleDir = String(argv[++i] || "").trim();
      continue;
    }
    if (arg === "--manifest-path") {
      cli.manifestPath = String(argv[++i] || "").trim();
      continue;
    }
    if (arg === "--lock-path") {
      cli.lockPath = String(argv[++i] || "").trim();
      continue;
    }
    if (arg === "--allow-managed-runtime") {
      cli.allowManagedRuntime = true;
      continue;
    }
    if (arg === "-h" || arg === "--help") {
      usage();
      process.exit(0);
    }
    throw new Error(`unknown argument: ${arg}`);
  }
  if (!isNonEmptyString(cli.runtimeDir)) {
    throw new Error("--runtime-dir is required");
  }
  return cli;
};

const readJsonFile = (filePath) => JSON.parse(fs.readFileSync(filePath, "utf8"));

const parseVersionFile = (raw) => {
  const out = {};
  for (const line of String(raw || "").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    const key = trimmed.slice(0, eq).trim();
    const value = trimmed.slice(eq + 1).trim();
    if (key) out[key] = value;
  }
  return out;
};

const requiredVersionKeys = Object.freeze([
  "version",
  "rootfs-sha256",
  "kernel-sha256",
  "initrd-sha256",
  "guest-agent-sha256",
  "egress-proxy-sha256",
  "container-stack-sha256",
]);

const expectedBundleRoot = (version) => path.posix.join(
  "runtimes",
  AVF_RUNTIME_ID,
  TARGET_OS,
  TARGET_ARCH,
  version,
);

const normalizeUriBasename = (value) => {
  if (!isNonEmptyString(value)) return "";
  try {
    return path.posix.basename(new URL(value).pathname);
  } catch {
    return path.posix.basename(String(value).trim());
  }
};

const findManifestRuntimeEntry = (manifest) =>
  Array.isArray(manifest?.runtimes)
    ? manifest.runtimes.find(
      (entry) =>
        entry
        && entry.id === AVF_RUNTIME_ID
        && entry.os === TARGET_OS
        && entry.arch === TARGET_ARCH,
    ) || null
    : null;

const findRuntimeComponent = (lock) =>
  Array.isArray(lock?.components)
    ? lock.components.find(
      (component) =>
        component
        && component.kind === "runtime"
        && component.id === AVF_RUNTIME_ID
        && component.os === TARGET_OS
        && component.arch === TARGET_ARCH,
    ) || null
    : null;

const findCiSourceByBasename = (component, basename) =>
  Array.isArray(component?.sources)
    ? component.sources.find(
      (source) =>
        source
        && source.source_type === "ci"
        && normalizeUriBasename(source.uri) === basename,
    ) || null
    : null;

const findHelperSource = (component, helperKey) => {
  const helper = component?.helpers?.[helperKey];
  return helper && typeof helper === "object" ? helper : null;
};

const validateAvfRuntimeFreshness = ({
  runtimeDir,
  bundleDir,
  manifest,
  lock,
  allowManagedRuntime = false,
}) => {
  const errors = [];
  const versionFilePath = path.join(runtimeDir, "version.txt");
  if (!fs.existsSync(versionFilePath)) {
    errors.push(`prepared runtime is missing version.txt: ${versionFilePath}`);
    return { ok: false, errors };
  }
  const versionInfo = parseVersionFile(fs.readFileSync(versionFilePath, "utf8"));
  for (const key of requiredVersionKeys) {
    if (!isNonEmptyString(versionInfo[key])) {
      errors.push(`prepared runtime version.txt is missing ${key}`);
    }
  }
  if (errors.length > 0) {
    return { ok: false, errors };
  }

  const manifestEntry = findManifestRuntimeEntry(manifest);
  if (!manifestEntry) {
    errors.push(`manifest.json is missing bundled runtime ${AVF_RUNTIME_ID} for ${TARGET_OS}/${TARGET_ARCH}`);
  }

  const runtimeComponent = findRuntimeComponent(lock);
  if (!runtimeComponent) {
    errors.push(`runtime_lock.v2.json is missing runtime component ${AVF_RUNTIME_ID} for ${TARGET_OS}/${TARGET_ARCH}`);
  }

  const expectedRoot = expectedBundleRoot(versionInfo.version);
  const bundledRuntimeDir = path.join(bundleDir, expectedRoot);
  const bundledRuntimeDirExists = fs.existsSync(bundledRuntimeDir)
    && fs.statSync(bundledRuntimeDir).isDirectory();
  if (!bundledRuntimeDirExists && !allowManagedRuntime) {
    errors.push(`bundled runtime directory is missing: ${bundledRuntimeDir}`);
  }

  if (manifestEntry) {
    if (manifestEntry.version !== versionInfo.version) {
      errors.push(
        `manifest version drift: expected ${versionInfo.version}, found ${String(manifestEntry.version || "")}`,
      );
    }
    if (manifestEntry.sha256 !== versionInfo["rootfs-sha256"]) {
      errors.push(
        `manifest rootfs sha drift: expected ${versionInfo["rootfs-sha256"]}, found ${String(manifestEntry.sha256 || "")}`,
      );
    }
    if (manifestEntry.root !== expectedRoot) {
      errors.push(`manifest root drift: expected ${expectedRoot}, found ${String(manifestEntry.root || "")}`);
    }
  }

  if (runtimeComponent) {
    if (runtimeComponent.version !== versionInfo.version) {
      errors.push(
        `runtime lock version drift: expected ${versionInfo.version}, found ${String(runtimeComponent.version || "")}`,
      );
    }
    const expectedHelperShas = new Map([
      ["kernel", versionInfo["kernel-sha256"]],
      ["initrd", versionInfo["initrd-sha256"]],
      ["guest-agent", versionInfo["guest-agent-sha256"]],
      ["egress-proxy", versionInfo["egress-proxy-sha256"]],
      ["container-stack", versionInfo["container-stack-sha256"]],
    ]);
    for (const [helperKey, expectedSha] of expectedHelperShas.entries()) {
      const source = findHelperSource(runtimeComponent, helperKey);
      if (!source) {
        errors.push(`runtime lock is missing helper source for ${helperKey}`);
        continue;
      }
      if (source.sha256 !== expectedSha) {
        errors.push(`runtime lock ${helperKey} sha drift: expected ${expectedSha}, found ${String(source.sha256 || "")}`);
      }
      if (!String(source.uri || "").includes(`/${versionInfo.version}/${TARGET_OS}/${TARGET_ARCH}/`)) {
        errors.push(`runtime lock ${helperKey} uri does not point at ${versionInfo.version}: ${String(source.uri || "")}`);
      }
    }
    const rootfsSource = findCiSourceByBasename(runtimeComponent, "rootfs.raw.zst");
    if (!rootfsSource) {
      errors.push("runtime lock is missing ci source for rootfs.raw.zst");
    } else if (!String(rootfsSource.uri || "").includes(`/${versionInfo.version}/${TARGET_OS}/${TARGET_ARCH}/`)) {
      errors.push(`runtime lock rootfs.raw.zst uri does not point at ${versionInfo.version}: ${String(rootfsSource.uri || "")}`);
    }
  }

  return {
    ok: errors.length === 0,
    errors,
    version: versionInfo.version,
    bundledRuntimeDir,
  };
};

const main = () => {
  const cli = parseArgs(process.argv.slice(2));
  const manifest = readJsonFile(path.resolve(cli.manifestPath));
  const lock = readJsonFile(path.resolve(cli.lockPath));
  const result = validateAvfRuntimeFreshness({
    runtimeDir: path.resolve(cli.runtimeDir),
    bundleDir: path.resolve(cli.bundleDir),
    manifest,
    lock,
    allowManagedRuntime: cli.allowManagedRuntime,
  });
  if (!result.ok) {
    for (const error of result.errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }
  process.stdout.write(
    `avf_runtime_lock_freshness: OK (${result.version}) bundled_dir=${result.bundledRuntimeDir}\n`,
  );
};

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`error: ${error?.message ?? String(error)}`);
    process.exit(1);
  }
}

module.exports = {
  expectedBundleRoot,
  parseVersionFile,
  validateAvfRuntimeFreshness,
};
