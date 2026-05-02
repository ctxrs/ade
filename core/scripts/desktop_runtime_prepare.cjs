#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const childProcess = require("node:child_process");
const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const { withFileLockSync } = require("./lib/file_lock.cjs");
const {
  DESKTOP_PREPARE_BUDGET_KEY,
  HOST_HEAVY_BUDGET_KEY,
  withHostJobBudget,
} = require("./lib/host_job_budget.cjs");
const { computeWebDistCacheKey, ensureWebDistArtifact } = require("./lib/web_dist_cache.cjs");
const { readDesktopVersion } = require("./desktop_version.cjs");
const { resolveDefaultLockPath, validateRuntimeLock } = require("./runtime_lock_validate.cjs");

const coreRoot = path.resolve(__dirname, "..");
const bundlesDir = path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles");
const manifestPath = path.join(bundlesDir, "manifest.json");
const effectiveManifestPath = path.join(bundlesDir, "runtime_manifest.effective.json");
const runtimeStatePath = path.join(bundlesDir, "runtime_state.json");
const overridesPath = path.join(coreRoot, "..", ".ctx", "local", "runtime_overrides.json");
const destBinDir = path.join(coreRoot, "apps", "desktop", "src-tauri", "bin");
const destWebDistDir = path.join(coreRoot, "apps", "desktop", "src-tauri", "web", "dist");

const PROFILE_VALUES = new Set(["parity", "override", "source-all"]);
const PREP_STATE_VERSION = 3;
const PREP_FINGERPRINT_VERSION = 1;
const DEFAULT_IGNORED_DIR_NAMES = new Set([
  ".git",
  "coverage",
  "dist",
  "node_modules",
  "playwright-report",
  "target",
  "test-results",
]);
const DESKTOP_SOURCE_IGNORED_DIR_NAMES = new Set([
  ...DEFAULT_IGNORED_DIR_NAMES,
  "bin",
  "bundles",
  "web",
]);
const PREP_ENV_PREFIXES = ["CTX_BUNDLE_"];
const PREP_ENV_KEYS = new Set([
  "CTX_AVF_LINUX_GUEST_RUNTIME_DIR",
  "CTX_AVF_LINUX_GUEST_RUNTIME_VERSION",
  "CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD",
  "CTX_DESKTOP_SYNC_BUNDLES",
  "CTX_DESKTOP_WEB_DIST",
  "CTX_RUNTIME_OVERRIDES_PATH",
  "CTX_RUNTIME_PROFILE",
]);

function trimValue(value) {
  return String(value ?? "").trim();
}

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

const resolveProfile = () => {
  const raw = trimValue(process.env.CTX_RUNTIME_PROFILE || "parity") || "parity";
  if (!PROFILE_VALUES.has(raw)) {
    console.error(`error: invalid CTX_RUNTIME_PROFILE='${raw}' (expected parity|override|source-all)`);
    process.exit(1);
  }
  return raw;
};

const sha256File = (filePath) => {
  const data = fs.readFileSync(filePath);
  return crypto.createHash("sha256").update(data).digest("hex");
};

function updateHashWithFile(hash, absolutePath, displayPath, stats) {
  hash.update(`file:${displayPath}:${stats.mode}:${stats.size}\n`);
  hash.update(fs.readFileSync(absolutePath));
  hash.update("\n");
}

function updateHashWithSymlink(hash, absolutePath, displayPath) {
  hash.update(`symlink:${displayPath}:${fs.readlinkSync(absolutePath)}\n`);
}

function updateHashWithMissingPath(hash, displayPath) {
  hash.update(`missing:${displayPath}\n`);
}

function updateHashWithPath(hash, {
  absolutePath,
  displayPath,
  ignoredDirNames = DEFAULT_IGNORED_DIR_NAMES,
}) {
  if (!fs.existsSync(absolutePath)) {
    updateHashWithMissingPath(hash, displayPath);
    return;
  }

  const stats = fs.lstatSync(absolutePath);
  if (stats.isSymbolicLink()) {
    updateHashWithSymlink(hash, absolutePath, displayPath);
    return;
  }
  if (stats.isFile()) {
    updateHashWithFile(hash, absolutePath, displayPath, stats);
    return;
  }
  if (!stats.isDirectory()) {
    hash.update(`other:${displayPath}:${stats.mode}\n`);
    return;
  }

  hash.update(`dir:${displayPath}:${stats.mode}\n`);
  const entries = fs.readdirSync(absolutePath, { withFileTypes: true })
    .filter((entry) => !ignoredDirNames.has(entry.name))
    .sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    updateHashWithPath(hash, {
      absolutePath: path.join(absolutePath, entry.name),
      displayPath: path.posix.join(displayPath, entry.name),
      ignoredDirNames,
    });
  }
}

function resolveProviderMatrixPath(env = process.env) {
  const explicitPath = trimValue(env.CTX_BUNDLE_MATRIX_JSON);
  if (explicitPath) {
    return path.resolve(explicitPath);
  }
  return path.join(coreRoot, "crates", "ctx-provider-accounts", "src", "provider_matrix.json");
}

function buildPrepareEnvSnapshot(env = process.env) {
  const snapshot = {};
  const keys = Object.keys(env)
    .filter((key) => PREP_ENV_KEYS.has(key) || PREP_ENV_PREFIXES.some((prefix) => key.startsWith(prefix)))
    .sort();
  for (const key of keys) {
    snapshot[key] = trimValue(env[key]);
  }
  return snapshot;
}

function resolveWebDistDescriptor({
  coreRoot: coreRootPath,
  env = process.env,
  desktopVersion,
  computeWebDistCacheKeyImpl = computeWebDistCacheKey,
} = {}) {
  const explicitWebDist = trimValue(env.CTX_DESKTOP_WEB_DIST);
  if (explicitWebDist) {
    return {
      source: "explicit",
      path: path.resolve(coreRootPath, explicitWebDist),
    };
  }
  return {
    source: "artifact-cache",
    cache_key: computeWebDistCacheKeyImpl({
      coreRoot: coreRootPath,
      env,
      appVersion: desktopVersion,
      variant: "desktop-runtime-parity",
    }),
  };
}

function buildPrepareTrackedEntries({
  coreRoot: coreRootPath,
  env = process.env,
  lockPath,
  overridesPath: overridesPathValue,
  webDistDescriptor,
} = {}) {
  const entries = [
    { absolutePath: lockPath, displayPath: "runtime-lock" },
    { absolutePath: overridesPathValue, displayPath: "runtime-overrides" },
    { absolutePath: path.join(coreRootPath, "package.json"), displayPath: "core-package.json" },
    { absolutePath: path.join(coreRootPath, "pnpm-lock.yaml"), displayPath: "pnpm-lock.yaml" },
    { absolutePath: path.join(coreRootPath, "apps", "desktop", "package.json"), displayPath: "desktop-package.json" },
    { absolutePath: path.join(coreRootPath, "apps", "desktop", "src-tauri"), displayPath: "desktop-src-tauri", ignoredDirNames: DESKTOP_SOURCE_IGNORED_DIR_NAMES },
    { absolutePath: path.join(coreRootPath, "crates", "ctx-http"), displayPath: "crate-ctx-http" },
    { absolutePath: path.join(coreRootPath, "crates", "ctx-mcp"), displayPath: "crate-ctx-mcp" },
    { absolutePath: path.join(coreRootPath, "crates", "ctx-sandbox-container-runtime"), displayPath: "crate-ctx-sandbox-container-runtime" },
    { absolutePath: resolveProviderMatrixPath(env), displayPath: "provider-matrix" },
    { absolutePath: path.join(coreRootPath, "scripts", "desktop_check_versions.cjs"), displayPath: "desktop-check-versions-script" },
    { absolutePath: path.join(coreRootPath, "scripts", "desktop_sync_resources.cjs"), displayPath: "desktop-sync-resources-script" },
    { absolutePath: path.join(coreRootPath, "scripts", "desktop_sync_resources_remote_daemon_policy.cjs"), displayPath: "desktop-sync-remote-daemon-policy-script" },
    { absolutePath: path.join(coreRootPath, "scripts", "runtime_lock_validate.cjs"), displayPath: "runtime-lock-validate-script" },
    { absolutePath: path.join(coreRootPath, "scripts", "lib", "web_dist_cache.cjs"), displayPath: "web-dist-cache-script" },
    { absolutePath: path.join(coreRootPath, "scripts", "prepare_avf_linux_guest_runtime.sh"), displayPath: "prepare-avf-linux-guest-runtime-script" },
    { absolutePath: path.join(coreRootPath, "..", "scripts", "ensure_bundled_harnesses.sh"), displayPath: "ensure-bundled-harnesses-script" },
  ];

  const guestRuntimeDir = trimValue(env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR);
  if (guestRuntimeDir) {
    entries.push({
      absolutePath: path.resolve(guestRuntimeDir),
      displayPath: "avf-linux-guest-runtime",
    });
  }

  if (webDistDescriptor.source === "explicit") {
    entries.push({
      absolutePath: webDistDescriptor.path,
      displayPath: "desktop-web-dist",
    });
  }

  return entries;
}

function buildPrepareFingerprint({
  coreRoot: coreRootPath,
  env = process.env,
  profile,
  desktopVersion,
  lockPath,
  overridesPath: overridesPathValue,
  cargoTargetDir,
  computeWebDistCacheKeyImpl = computeWebDistCacheKey,
} = {}) {
  const envSnapshot = buildPrepareEnvSnapshot(env);
  const webDistDescriptor = resolveWebDistDescriptor({
    coreRoot: coreRootPath,
    env,
    desktopVersion,
    computeWebDistCacheKeyImpl,
  });
  const trackedEntries = buildPrepareTrackedEntries({
    coreRoot: coreRootPath,
    env,
    lockPath,
    overridesPath: overridesPathValue,
    webDistDescriptor,
  });
  const hash = crypto.createHash("sha256");
  hash.update(`desktop-runtime-prepare:v${PREP_FINGERPRINT_VERSION}\n`);
  hash.update(`profile:${profile}\n`);
  hash.update(`desktopVersion:${desktopVersion}\n`);
  hash.update(`cargoTargetDir:${cargoTargetDir}\n`);
  hash.update(`env:${JSON.stringify(envSnapshot, Object.keys(envSnapshot).sort())}\n`);
  hash.update(`webDist:${JSON.stringify(webDistDescriptor)}\n`);
  for (const entry of trackedEntries) {
    updateHashWithPath(hash, entry);
  }
  return {
    envSnapshot,
    fingerprint: hash.digest("hex"),
    trackedEntries: trackedEntries.map((entry) => entry.displayPath),
    webDistDescriptor,
  };
}

function readRuntimeState(statePath = runtimeStatePath) {
  if (!fs.existsSync(statePath)) {
    return null;
  }
  try {
    return JSON.parse(fs.readFileSync(statePath, "utf8"));
  } catch {
    return null;
  }
}

function resolvePrepareRequiredOutputs({
  platform = process.platform,
} = {}) {
  const requiredOutputs = [
    manifestPath,
    effectiveManifestPath,
    destWebDistDir,
    path.join(destBinDir, `ctx-daemon${process.platform === "win32" ? ".exe" : ""}`),
    path.join(destBinDir, `ctx-mcp${process.platform === "win32" ? ".exe" : ""}`),
  ];
  if (platform === "darwin") {
    requiredOutputs.push(path.join(destBinDir, "ctx-avf-linux-helper"));
  }
  return requiredOutputs;
}

function resolveDesktopPrepareLockPath(layout) {
  const lockKey = crypto.createHash("sha1").update(path.resolve(bundlesDir)).digest("hex").slice(0, 12);
  return path.join(layout.cacheDir, "locks", "desktop-runtime-prepare", `${lockKey}.lock`);
}

function canReusePreparedParity({
  profile,
  state,
  fingerprint,
  requiredOutputs,
} = {}) {
  if (profile === "source-all") {
    return false;
  }
  if (!state || state.version !== PREP_STATE_VERSION || state.profile !== profile) {
    return false;
  }
  if (trimValue(state?.prepare?.fingerprint) !== trimValue(fingerprint)) {
    return false;
  }
  if (state?.prepare?.fingerprint_version !== PREP_FINGERPRINT_VERSION) {
    return false;
  }
  return requiredOutputs.every((requiredPath) => fs.existsSync(requiredPath));
}

const ensureParityPrep = ({ prepEnv, desktopVersion }) => {
  run("node", ["scripts/desktop_check_versions.cjs"], { env: prepEnv });
  run("cargo", ["build", "-p", "ctx-http", "-p", "ctx-mcp"], { env: prepEnv });
  const desktopWebDist = ensureWebDistArtifact({
    coreRoot,
    env: prepEnv,
    appVersion: desktopVersion,
    variant: "desktop-runtime-parity",
  }).distDir;
  run("node", ["scripts/desktop_sync_resources.cjs", "--profile", "debug"], {
    env: {
      ...prepEnv,
      CTX_DESKTOP_WEB_DIST: desktopWebDist,
    },
  });
};

const writeRuntimeState = ({
  cargoTargetDir,
  profile,
  prepMode,
  lockPath,
  lockVersion,
  manifestPathValue,
  effectiveManifestPathValue,
  overridesApplied,
  prepareFingerprint,
  requiredOutputs,
  reusedPrep,
  trackedEntries,
  webDistDescriptor,
}) => {
  const payload = {
    version: PREP_STATE_VERSION,
    prepared_at: new Date().toISOString(),
    platform: `${process.platform}/${process.arch}`,
    profile,
    prep_mode: prepMode,
    cargo_target_dir: cargoTargetDir,
    lock_version: lockVersion,
    runtime_lock: {
      path: path.relative(coreRoot, lockPath),
      sha256: fs.existsSync(lockPath) ? sha256File(lockPath) : null,
    },
    bundles_manifest: {
      path: path.relative(coreRoot, manifestPathValue),
      sha256: fs.existsSync(manifestPathValue) ? sha256File(manifestPathValue) : null,
    },
    effective_manifest: {
      path: path.relative(coreRoot, effectiveManifestPathValue),
      sha256: fs.existsSync(effectiveManifestPathValue) ? sha256File(effectiveManifestPathValue) : null,
    },
    prepare: {
      fingerprint_version: prepareFingerprint ? PREP_FINGERPRINT_VERSION : null,
      fingerprint: prepareFingerprint,
      required_outputs: Array.isArray(requiredOutputs)
        ? requiredOutputs.map((requiredPath) => path.relative(coreRoot, requiredPath)).sort()
        : [],
      reused: Boolean(reusedPrep),
      tracked_entries: Array.isArray(trackedEntries) ? trackedEntries : [],
      web_dist: webDistDescriptor || null,
    },
    overrides_applied: overridesApplied,
  };
  fs.writeFileSync(runtimeStatePath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  return payload;
};

const main = () => {
  const profile = resolveProfile();
  const { env: prepEnv, cargoTargetDir, layout } = buildCtxCacheEnv({
    cwd: coreRoot,
    env: process.env,
    mode: "workspace",
    mkdir: true,
  });
  const desktopPrepareLockPath = resolveDesktopPrepareLockPath(layout);

  withFileLockSync(desktopPrepareLockPath, {
    metadata: {
      command: `desktop_runtime_prepare ${profile}`,
      cwd: coreRoot,
      leaseId: prepEnv.CTX_HOST_JOB_LEASE_ID || "",
      sessionId: prepEnv.CTX_SESSION_ID || "",
      threadId: prepEnv.CODEX_THREAD_ID || "",
    },
    staleMs: 20 * 60 * 1000,
    timeoutMs: 20 * 60 * 1000,
  }, () => {
    const desktopVersion = readDesktopVersion(coreRoot);
    const lockPath = resolveDefaultLockPath();
    const effectiveOverridesPath = process.env.CTX_RUNTIME_OVERRIDES_PATH || overridesPath;
    const prepMode = profile === "source-all" ? "source-all" : "parity-with-existing-bundles";
    const requiredOutputs = resolvePrepareRequiredOutputs();
    let prepareFingerprint = null;
    let trackedEntries = [];
    let webDistDescriptor = null;
    let reusedPrep = false;

    if (profile === "source-all") {
      withHostJobBudget({
        budgetKey: DESKTOP_PREPARE_BUDGET_KEY,
        command: "desktop_runtime_prepare source-all",
        cwd: coreRoot,
        env: prepEnv,
      }, () => {
        withHostJobBudget({
          budgetKey: HOST_HEAVY_BUDGET_KEY,
          command: "pnpm desktop:prep",
          cwd: coreRoot,
          env: prepEnv,
        }, () => {
          run("pnpm", ["desktop:prep"], { env: prepEnv });
        });
      });
    } else {
      const fingerprintInfo = buildPrepareFingerprint({
        coreRoot,
        env: prepEnv,
        profile,
        desktopVersion,
        lockPath,
        overridesPath: effectiveOverridesPath,
        cargoTargetDir,
      });
      prepareFingerprint = fingerprintInfo.fingerprint;
      trackedEntries = fingerprintInfo.trackedEntries;
      webDistDescriptor = fingerprintInfo.webDistDescriptor;
      reusedPrep = canReusePreparedParity({
        profile,
        state: readRuntimeState(),
        fingerprint: prepareFingerprint,
        requiredOutputs,
      });

      if (reusedPrep) {
        console.log(
          `desktop_runtime_prepare: reusing parity prep fingerprint=${prepareFingerprint.slice(0, 12)} profile=${profile}`,
        );
      } else {
        withHostJobBudget({
          budgetKey: DESKTOP_PREPARE_BUDGET_KEY,
          command: `desktop_runtime_prepare ${profile}`,
          cwd: coreRoot,
          env: prepEnv,
        }, () => {
          withHostJobBudget({
            budgetKey: HOST_HEAVY_BUDGET_KEY,
            command: `desktop_runtime_prepare materialize ${profile}`,
            cwd: coreRoot,
            env: prepEnv,
          }, () => {
            ensureParityPrep({ prepEnv, desktopVersion });
          });
        });
      }
    }

    const validation = validateRuntimeLock({
      lockPath,
      manifestPath,
      profile,
      overridesPath: effectiveOverridesPath,
    });

    if (!validation.ok) {
      for (const error of validation.errors) {
        console.error(`error: ${error}`);
      }
      if (profile !== "source-all") {
        console.error(
          "hint: run with CTX_RUNTIME_PROFILE=source-all once to materialize full local bundles if parity assets are missing",
        );
      }
      process.exit(1);
    }

    fs.writeFileSync(effectiveManifestPath, `${JSON.stringify(validation.effectiveManifest, null, 2)}\n`, "utf8");

    const state = writeRuntimeState({
      cargoTargetDir,
      profile,
      prepMode,
      lockPath,
      lockVersion: validation.lockVersion,
      manifestPathValue: manifestPath,
      effectiveManifestPathValue: effectiveManifestPath,
      overridesApplied: validation.appliedOverrides,
      prepareFingerprint,
      requiredOutputs,
      reusedPrep,
      trackedEntries,
      webDistDescriptor,
    });

    console.log(
      `desktop_runtime_prepare: profile=${profile} reused=${reusedPrep ? "1" : "0"} lock=v${validation.lockVersion} lock_sha=${state.runtime_lock.sha256 ?? "missing"} effective_manifest_sha=${state.effective_manifest.sha256 ?? "missing"}`,
    );
  });
};

if (require.main === module) {
  main();
}

module.exports = {
  buildPrepareEnvSnapshot,
  buildPrepareFingerprint,
  canReusePreparedParity,
  readRuntimeState,
  resolveDesktopPrepareLockPath,
  resolvePrepareRequiredOutputs,
  resolveProviderMatrixPath,
  resolveWebDistDescriptor,
};
