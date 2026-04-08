#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const childProcess = require("node:child_process");
const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const { readDesktopVersion } = require("./desktop_version.cjs");
const { ensureWebDistArtifact } = require("./lib/web_dist_cache.cjs");
const { resolveDefaultLockPath, validateRuntimeLock } = require("./runtime_lock_validate.cjs");

const coreRoot = path.resolve(__dirname, "..");
const bundlesDir = path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles");
const manifestPath = path.join(bundlesDir, "manifest.json");
const effectiveManifestPath = path.join(bundlesDir, "runtime_manifest.effective.json");
const runtimeStatePath = path.join(bundlesDir, "runtime_state.json");
const overridesPath = path.join(coreRoot, "..", ".ctx", "local", "runtime_overrides.json");

const PROFILE_VALUES = new Set(["parity", "override", "source-all"]);

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
  const raw = String(process.env.CTX_RUNTIME_PROFILE || "parity").trim() || "parity";
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
}) => {
  const payload = {
    version: 2,
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
    overrides_applied: overridesApplied,
  };
  fs.writeFileSync(runtimeStatePath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  return payload;
};

const main = () => {
  const profile = resolveProfile();
  const { env: prepEnv, cargoTargetDir } = buildCtxCacheEnv({
    cwd: coreRoot,
    env: process.env,
    mode: "workspace",
    mkdir: true,
  });
  const desktopVersion = readDesktopVersion(coreRoot);
  const lockPath = resolveDefaultLockPath();
  const prepMode = profile === "source-all" ? "source-all" : "parity-with-existing-bundles";

  if (profile === "source-all") {
    run("pnpm", ["desktop:prep"], { env: prepEnv });
  } else {
    ensureParityPrep({ prepEnv, desktopVersion });
  }

  const validation = validateRuntimeLock({
    lockPath,
    manifestPath,
    profile,
    overridesPath: process.env.CTX_RUNTIME_OVERRIDES_PATH || overridesPath,
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
  });

  console.log(
    `desktop_runtime_prepare: profile=${profile} lock=v${validation.lockVersion} lock_sha=${state.runtime_lock.sha256 ?? "missing"} effective_manifest_sha=${state.effective_manifest.sha256 ?? "missing"}`,
  );
};

main();
