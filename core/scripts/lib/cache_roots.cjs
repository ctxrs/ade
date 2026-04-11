const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const DEFAULT_EXTERNAL_CACHE_ROOT = "/Volumes/ctx-cache";
const DEFAULT_INTERNAL_VOLATILE_ROOT = path.join(os.homedir(), ".ctx", "volatile");
const DEFAULT_REPO_CACHE_SLUG = "ctx-monorepo";
const CACHE_ROOT_MARKER_FILE = ".ctx-volatile-root.json";

function trimValue(value) {
  return String(value ?? "").trim();
}

function parseEnabledFlag(value) {
  return ["1", "true", "yes", "on"].includes(trimValue(value).toLowerCase());
}

function splitEnvPathList(value) {
  const delimiter = process.platform === "win32" ? ";" : ":";
  return trimValue(value)
    .split(delimiter)
    .map((entry) => trimValue(entry))
    .filter((entry) => entry.length > 0);
}

function formatEnvPathList(entries) {
  const delimiter = process.platform === "win32" ? ";" : ":";
  return entries.join(delimiter);
}

function uniqueNormalizedPaths(entries) {
  const result = [];
  const seen = new Set();
  for (const entry of entries) {
    const normalized = path.normalize(String(entry ?? ""));
    if (!normalized || seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    result.push(normalized);
  }
  return result;
}

function appendEnvPathListValue(targetEnv, key, entries) {
  const merged = uniqueNormalizedPaths([
    ...splitEnvPathList(targetEnv[key]),
    ...entries,
  ]);
  if (merged.length > 0) {
    targetEnv[key] = formatEnvPathList(merged);
  }
}

function appendSpaceSeparatedFlags(targetEnv, key, flags) {
  const existing = trimValue(targetEnv[key]);
  const merged = existing ? [existing] : [];
  for (const flag of flags) {
    const normalizedFlag = trimValue(flag);
    if (!normalizedFlag) {
      continue;
    }
    if (merged.some((entry) => entry.includes(normalizedFlag))) {
      continue;
    }
    merged.push(normalizedFlag);
  }
  if (merged.length > 0) {
    targetEnv[key] = merged.join(" ");
  }
}

function resolveSccacheServerUds(targetPath) {
  const hash = crypto.createHash("sha1").update(path.resolve(targetPath)).digest("hex").slice(0, 12);
  return path.join("/tmp", `ctx-sccache-${hash}.sock`);
}

function setDefaultEnvValue(targetEnv, key, value) {
  if (!trimValue(targetEnv[key])) {
    targetEnv[key] = value;
  }
}

function setDefaultEnvValueIfPresent(targetEnv, key, value) {
  const normalized = trimValue(value);
  if (normalized) {
    setDefaultEnvValue(targetEnv, key, normalized);
  }
}

function setDerivedPathEnvValue(targetEnv, key, value, { cwd, explicitVolatileRoot }) {
  const currentValue = trimValue(targetEnv[key]);
  if (!currentValue) {
    targetEnv[key] = value;
    return;
  }
  if (!explicitVolatileRoot) {
    return;
  }
  const resolvedCurrentValue = resolveConfiguredPath(currentValue, { cwd });
  if (!isPathInsideRoot(resolvedCurrentValue, explicitVolatileRoot)) {
    targetEnv[key] = value;
  }
}

function resolveConfiguredPath(configured, { cwd = process.cwd() } = {}) {
  const normalized = trimValue(configured);
  if (!normalized) {
    return "";
  }
  return path.isAbsolute(normalized) ? path.normalize(normalized) : path.resolve(cwd, normalized);
}

function execGit(command, cwd) {
  return childProcess
    .execSync(command, {
      cwd,
      stdio: ["ignore", "pipe", "ignore"],
    })
    .toString()
    .trim();
}

function sanitizeScopeKey(value) {
  const normalized = trimValue(value).replace(/[^A-Za-z0-9._-]+/g, "-");
  return normalized.replace(/^-+|-+$/g, "");
}

function resolveRepoScopeKey(cwd = process.cwd(), env = process.env) {
  const explicitScopeKey = sanitizeScopeKey(env.CTX_CACHE_SCOPE_KEY);
  if (explicitScopeKey) {
    return explicitScopeKey;
  }

  const sessionScopeKey = sanitizeScopeKey(env.CTX_SESSION_ID) || sanitizeScopeKey(env.CODEX_THREAD_ID);
  if (sessionScopeKey) {
    return sessionScopeKey;
  }

  try {
    const rawGitDir = execGit("git rev-parse --git-dir", cwd);
    const gitDirName = path.basename(rawGitDir);
    if (gitDirName && gitDirName !== ".git") {
      return gitDirName;
    }
  } catch {}

  try {
    const repoRoot = execGit("git rev-parse --show-toplevel", cwd);
    const repoName = path.basename(repoRoot);
    if (repoName) {
      return repoName;
    }
  } catch {}

  return "default";
}

function findNearestExistingPath(targetPath) {
  let current = path.resolve(targetPath);
  while (!fs.existsSync(current)) {
    const parent = path.dirname(current);
    if (parent === current) {
      return current;
    }
    current = parent;
  }
  return current;
}

function isWritablePath(targetPath) {
  const existingPath = findNearestExistingPath(targetPath);
  try {
    fs.accessSync(existingPath, fs.constants.W_OK);
    return true;
  } catch {
    return false;
  }
}

function resolveVolatileSelection({ cwd = process.cwd(), env = process.env } = {}) {
  const explicitVolatileRoot = resolveConfiguredPath(env.CTX_VOLATILE_ROOT, { cwd });
  const externalCacheRoot =
    resolveConfiguredPath(env.CTX_EXTERNAL_CACHE_ROOT, { cwd }) || DEFAULT_EXTERNAL_CACHE_ROOT;
  const preferredVolatileRoot =
    resolveConfiguredPath(env.CTX_PREFERRED_VOLATILE_ROOT, { cwd })
    || path.join(externalCacheRoot, "volatile");
  const internalVolatileRoot =
    resolveConfiguredPath(env.CTX_INTERNAL_VOLATILE_ROOT, { cwd }) || DEFAULT_INTERNAL_VOLATILE_ROOT;

  if (explicitVolatileRoot) {
    return {
      volatileRoot: explicitVolatileRoot,
      volatileRootMode: "explicit",
      externalCacheRoot,
      preferredVolatileRoot,
      internalVolatileRoot,
      externalAvailable: preferredVolatileRoot === explicitVolatileRoot,
    };
  }

  const preferredProbePath = trimValue(env.CTX_PREFERRED_VOLATILE_ROOT)
    ? path.dirname(preferredVolatileRoot)
    : externalCacheRoot;
  const externalAvailable =
    fs.existsSync(preferredProbePath) && isWritablePath(preferredProbePath);

  return {
    volatileRoot: externalAvailable ? preferredVolatileRoot : internalVolatileRoot,
    volatileRootMode: externalAvailable ? "preferred-external" : "internal-fallback",
    externalCacheRoot,
    preferredVolatileRoot,
    internalVolatileRoot,
    externalAvailable,
  };
}

function resolveDefaultSubdir({ explicitValue, baseDir, fallbackSegments, cwd }) {
  return resolveConfiguredPath(explicitValue, { cwd }) || path.join(baseDir, ...fallbackSegments);
}

function isPathInsideRoot(candidatePath, rootPath) {
  const relative = path.relative(rootPath, candidatePath);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function resolveCtxCacheLayout({ cwd = process.cwd(), env = process.env } = {}) {
  const scopeKey = resolveRepoScopeKey(cwd, env);
  const {
    volatileRoot,
    volatileRootMode,
    externalCacheRoot,
    preferredVolatileRoot,
    internalVolatileRoot,
    externalAvailable,
  } = resolveVolatileSelection({ cwd, env });
  const explicitVolatileRoot = trimValue(env.CTX_VOLATILE_ROOT);

  const sanitizeExplicitSubdir = (explicitValue, fallbackSegments) => {
    const configuredPath = resolveConfiguredPath(explicitValue, { cwd });
    if (!configuredPath) {
      return path.join(volatileRoot, ...fallbackSegments);
    }
    if (explicitVolatileRoot && !isPathInsideRoot(configuredPath, volatileRoot)) {
      return path.join(volatileRoot, ...fallbackSegments);
    }
    return configuredPath;
  };
  const sanitizeExplicitPath = (explicitValue, fallbackPath) => {
    const configuredPath = resolveConfiguredPath(explicitValue, { cwd });
    if (!configuredPath) {
      return fallbackPath;
    }
    if (explicitVolatileRoot && !isPathInsideRoot(configuredPath, volatileRoot)) {
      return fallbackPath;
    }
    return configuredPath;
  };

  const targetsDir = sanitizeExplicitSubdir(env.CTX_VOLATILE_TARGETS_DIR, ["targets"]);
  const artifactsDir = sanitizeExplicitSubdir(env.CTX_VOLATILE_ARTIFACTS_DIR, ["artifacts"]);
  const tmpDir = sanitizeExplicitSubdir(env.CTX_VOLATILE_TMPDIR, ["tmp"]);
  const cacheDir = sanitizeExplicitSubdir(env.CTX_VOLATILE_CACHE_DIR, ["cache"]);
  const cargoHome = sanitizeExplicitPath(env.CARGO_HOME, path.join(cacheDir, "cargo-home"));
  const sccacheDir = sanitizeExplicitPath(env.SCCACHE_DIR, path.join(cacheDir, "sccache"));
  const turboCacheDir = sanitizeExplicitPath(
    env.TURBO_CACHE_DIR,
    path.join(cacheDir, "turbo", DEFAULT_REPO_CACHE_SLUG),
  );
  const bundleCacheDir = sanitizeExplicitPath(
    env.CTX_BUNDLE_CACHE_DIR,
    path.join(cacheDir, "bundles"),
  );
  const playwrightBrowsersPath = sanitizeExplicitPath(
    env.PLAYWRIGHT_BROWSERS_PATH,
    path.join(cacheDir, "playwright"),
  );
  const workspaceCargoTargetDir =
    resolveConfiguredPath(env.CARGO_TARGET_DIR || env.CTX_E2E_CARGO_TARGET_DIR, { cwd })
    || path.join(targetsDir, DEFAULT_REPO_CACHE_SLUG, scopeKey);
  const verifyCargoTargetDir =
    resolveConfiguredPath(env.CTX_VERIFY_CARGO_TARGET_DIR, { cwd })
    || workspaceCargoTargetDir;

  return {
    scopeKey,
    repoCacheSlug: DEFAULT_REPO_CACHE_SLUG,
    volatileRoot,
    volatileRootMode,
    externalCacheRoot,
    preferredVolatileRoot,
    internalVolatileRoot,
    externalAvailable,
    targetsDir,
    artifactsDir,
    tmpDir,
    cacheDir,
    cargoHome,
    sccacheDir,
    turboCacheDir,
    bundleCacheDir,
    playwrightBrowsersPath,
    workspaceCargoTargetDir,
    verifyCargoTargetDir,
  };
}

function writeCacheRootMarker(rootDir, layout) {
  const markerPath = path.join(rootDir, CACHE_ROOT_MARKER_FILE);
  const payload = {
    version: 1,
    repo_cache_slug: layout.repoCacheSlug,
    scope_key: layout.scopeKey,
    volatile_root_mode: layout.volatileRootMode,
    updated_at: new Date().toISOString(),
  };
  fs.writeFileSync(markerPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
}

function ensureCacheLayout(layout, { includeTargets = true } = {}) {
  const requiredDirs = [
    layout.volatileRoot,
    layout.targetsDir,
    layout.artifactsDir,
    layout.tmpDir,
    layout.cacheDir,
    layout.cargoHome,
    layout.sccacheDir,
    layout.turboCacheDir,
    layout.bundleCacheDir,
    layout.playwrightBrowsersPath,
  ];
  if (includeTargets) {
    requiredDirs.push(layout.workspaceCargoTargetDir, layout.verifyCargoTargetDir);
  }
  for (const dir of requiredDirs) {
    fs.mkdirSync(dir, { recursive: true });
  }
  writeCacheRootMarker(layout.volatileRoot, layout);
}

function resolveAvailableSccachePath(env = process.env) {
  const configuredPath = trimValue(env.SCCACHE_PATH);
  if (configuredPath) {
    return configuredPath;
  }
  try {
    const probeCommand = process.platform === "win32" ? "where" : "which";
    const result = childProcess.spawnSync(probeCommand, ["sccache"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
    if (result.status === 0) {
      return trimValue(String(result.stdout).split(/\r?\n/)[0]);
    }
  } catch {}
  // Fall back: if RUSTC_WRAPPER basename looks like sccache, use it as the sccache path
  const configuredWrapper = trimValue(env.RUSTC_WRAPPER);
  if (configuredWrapper && path.basename(configuredWrapper).toLowerCase().includes("sccache")) {
    return configuredWrapper;
  }
  return "";
}

function buildR2Endpoint(accountId, jurisdiction) {
  const normalizedAccountId = trimValue(accountId);
  if (!normalizedAccountId) {
    return "";
  }
  const normalizedJurisdiction = trimValue(jurisdiction).toLowerCase();
  if (normalizedJurisdiction === "eu") {
    return `https://${normalizedAccountId}.eu.r2.cloudflarestorage.com`;
  }
  if (normalizedJurisdiction === "fedramp") {
    return `https://${normalizedAccountId}.fedramp.r2.cloudflarestorage.com`;
  }
  return `https://${normalizedAccountId}.r2.cloudflarestorage.com`;
}

function buildCtxCacheEnv({
  cwd = process.cwd(),
  env = process.env,
  mode = "workspace",
  mkdir = false,
} = {}) {
  const useVolatileCargoHome = parseEnabledFlag(env.CTX_USE_VOLATILE_CARGO_HOME);

  function buildForLayout(layout) {
    const resolvedEnv = { ...env };
    const cargoTargetDir =
      mode === "verify-quick" ? layout.verifyCargoTargetDir : layout.workspaceCargoTargetDir;
    const explicitVolatileRoot = trimValue(env.CTX_VOLATILE_ROOT)
      ? layout.volatileRoot
      : "";

    setDefaultEnvValue(resolvedEnv, "CTX_EXTERNAL_CACHE_ROOT", layout.externalCacheRoot);
    setDefaultEnvValue(resolvedEnv, "CTX_INTERNAL_VOLATILE_ROOT", layout.internalVolatileRoot);
    setDefaultEnvValue(resolvedEnv, "CTX_PREFERRED_VOLATILE_ROOT", layout.preferredVolatileRoot);
    setDefaultEnvValue(resolvedEnv, "CTX_VOLATILE_ROOT", layout.volatileRoot);
    setDefaultEnvValue(resolvedEnv, "CTX_VOLATILE_ROOT_MODE", layout.volatileRootMode);
    setDerivedPathEnvValue(resolvedEnv, "CTX_VOLATILE_TARGETS_DIR", layout.targetsDir, {
      cwd,
      explicitVolatileRoot,
    });
    setDerivedPathEnvValue(resolvedEnv, "CTX_VOLATILE_ARTIFACTS_DIR", layout.artifactsDir, {
      cwd,
      explicitVolatileRoot,
    });
    setDerivedPathEnvValue(resolvedEnv, "CTX_VOLATILE_TMPDIR", layout.tmpDir, {
      cwd,
      explicitVolatileRoot,
    });
    setDerivedPathEnvValue(resolvedEnv, "CTX_VOLATILE_CACHE_DIR", layout.cacheDir, {
      cwd,
      explicitVolatileRoot,
    });
    setDerivedPathEnvValue(resolvedEnv, "SCCACHE_DIR", layout.sccacheDir, {
      cwd,
      explicitVolatileRoot,
    });
    setDerivedPathEnvValue(resolvedEnv, "TURBO_CACHE_DIR", layout.turboCacheDir, {
      cwd,
      explicitVolatileRoot,
    });
    setDerivedPathEnvValue(resolvedEnv, "CTX_BUNDLE_CACHE_DIR", layout.bundleCacheDir, {
      cwd,
      explicitVolatileRoot,
    });
    setDerivedPathEnvValue(
      resolvedEnv,
      "PLAYWRIGHT_BROWSERS_PATH",
      layout.playwrightBrowsersPath,
      {
        cwd,
        explicitVolatileRoot,
      },
    );
    setDerivedPathEnvValue(resolvedEnv, "CARGO_TARGET_DIR", cargoTargetDir, {
      cwd,
      explicitVolatileRoot,
    });
    setDerivedPathEnvValue(
      resolvedEnv,
      "CTX_VERIFY_CARGO_TARGET_DIR",
      layout.verifyCargoTargetDir,
      {
        cwd,
        explicitVolatileRoot,
      },
    );
    if (useVolatileCargoHome) {
      setDerivedPathEnvValue(resolvedEnv, "CARGO_HOME", layout.cargoHome, {
        cwd,
        explicitVolatileRoot,
      });
    }

    const turboApi = trimValue(env.TURBO_API);
    const turboToken = trimValue(env.TURBO_TOKEN);
    const turboTeam = trimValue(env.TURBO_TEAM);
    if (!trimValue(resolvedEnv.TURBO_CACHE_MODE) && turboApi && turboToken && turboTeam) {
      resolvedEnv.TURBO_CACHE_MODE = "local:rw,remote:rw";
    }

    const remoteSccacheBucket =
      trimValue(env.CTX_SCCACHE_R2_BUCKET) || trimValue(env.SCCACHE_BUCKET);
    const remoteSccacheAccountId = trimValue(env.CTX_SCCACHE_R2_ACCOUNT_ID);
    const remoteSccacheEndpoint =
      trimValue(env.CTX_SCCACHE_R2_ENDPOINT)
      || trimValue(env.SCCACHE_ENDPOINT)
      || buildR2Endpoint(remoteSccacheAccountId, env.CTX_SCCACHE_R2_JURISDICTION);
    const remoteSccacheKeyPrefix =
      trimValue(env.CTX_SCCACHE_R2_KEY_PREFIX)
      || trimValue(env.SCCACHE_S3_KEY_PREFIX)
      || `sccache/${layout.repoCacheSlug}`;
    const remoteSccacheAccessKeyId = trimValue(env.CTX_SCCACHE_R2_ACCESS_KEY_ID);
    const remoteSccacheSecretAccessKey = trimValue(env.CTX_SCCACHE_R2_SECRET_ACCESS_KEY);
    const remoteSccacheSessionToken = trimValue(env.CTX_SCCACHE_R2_SESSION_TOKEN);
    if (remoteSccacheBucket) {
      setDefaultEnvValueIfPresent(resolvedEnv, "SCCACHE_BUCKET", remoteSccacheBucket);
      setDefaultEnvValueIfPresent(
        resolvedEnv,
        "SCCACHE_REGION",
        trimValue(env.CTX_SCCACHE_R2_REGION) || trimValue(env.SCCACHE_REGION) || "auto",
      );
      setDefaultEnvValueIfPresent(resolvedEnv, "SCCACHE_ENDPOINT", remoteSccacheEndpoint);
      setDefaultEnvValueIfPresent(resolvedEnv, "SCCACHE_S3_KEY_PREFIX", remoteSccacheKeyPrefix);
      setDefaultEnvValueIfPresent(
        resolvedEnv,
        "SCCACHE_S3_USE_SSL",
        trimValue(env.SCCACHE_S3_USE_SSL) || "true",
      );
      if (remoteSccacheAccessKeyId) {
        resolvedEnv.AWS_ACCESS_KEY_ID = remoteSccacheAccessKeyId;
      } else {
        setDefaultEnvValueIfPresent(
          resolvedEnv,
          "AWS_ACCESS_KEY_ID",
          trimValue(env.AWS_ACCESS_KEY_ID),
        );
      }
      if (remoteSccacheSecretAccessKey) {
        resolvedEnv.AWS_SECRET_ACCESS_KEY = remoteSccacheSecretAccessKey;
      } else {
        setDefaultEnvValueIfPresent(
          resolvedEnv,
          "AWS_SECRET_ACCESS_KEY",
          trimValue(env.AWS_SECRET_ACCESS_KEY),
        );
      }
      if (remoteSccacheSessionToken) {
        resolvedEnv.AWS_SESSION_TOKEN = remoteSccacheSessionToken;
      } else {
        setDefaultEnvValueIfPresent(
          resolvedEnv,
          "AWS_SESSION_TOKEN",
          trimValue(env.AWS_SESSION_TOKEN),
        );
      }
    }

    const sccachePath = resolveAvailableSccachePath(resolvedEnv);
    if (sccachePath && !trimValue(resolvedEnv.RUSTC_WRAPPER)) {
      resolvedEnv.RUSTC_WRAPPER = sccachePath;
      setDefaultEnvValue(resolvedEnv, "SCCACHE_PATH", sccachePath);
    }
    const effectiveWrapper = trimValue(resolvedEnv.RUSTC_WRAPPER);
    const wrapperIsSccache =
      sccachePath &&
      effectiveWrapper &&
      (effectiveWrapper === sccachePath ||
        path.basename(effectiveWrapper).toLowerCase().includes("sccache"));
    if (wrapperIsSccache) {
      setDefaultEnvValue(resolvedEnv, "SCCACHE_NO_DAEMON", "1");
      if (process.platform !== "win32" && !trimValue(resolvedEnv.SCCACHE_SERVER_UDS)) {
        resolvedEnv.SCCACHE_SERVER_UDS = resolveSccacheServerUds(cargoTargetDir);
      }
      appendEnvPathListValue(resolvedEnv, "SCCACHE_BASEDIRS", [
        path.resolve(cwd),
        path.resolve(cargoTargetDir),
      ]);
      setDefaultEnvValue(resolvedEnv, "CARGO_INCREMENTAL", "0");
      appendSpaceSeparatedFlags(resolvedEnv, "RUSTFLAGS", [
        `--remap-path-prefix=${path.resolve(cwd)}=/ctx-workspace`,
        `--remap-path-prefix=${path.resolve(layout.volatileRoot)}=/ctx-volatile`,
      ]);
    }

    return {
      env: resolvedEnv,
      layout,
      cargoTargetDir: resolvedEnv.CARGO_TARGET_DIR,
    };
  }

  const initialLayout = resolveCtxCacheLayout({ cwd, env });
  let result = buildForLayout(initialLayout);

  if (mkdir) {
    try {
      ensureCacheLayout(result.layout);
      fs.mkdirSync(result.env.CARGO_TARGET_DIR, { recursive: true });
    } catch (error) {
      const hasExplicitVolatileRoot = Boolean(trimValue(env.CTX_VOLATILE_ROOT));
      const shouldFallback =
        !hasExplicitVolatileRoot && result.layout.volatileRootMode === "preferred-external";
      if (!shouldFallback) {
        throw error;
      }
      const fallbackLayout = {
        ...resolveCtxCacheLayout({
          cwd,
          env: {
            ...env,
            CTX_VOLATILE_ROOT: result.layout.internalVolatileRoot,
          },
        }),
        volatileRootMode: "internal-fallback",
      };
      result = buildForLayout(fallbackLayout);
      result.env.CTX_VOLATILE_ROOT_MODE = "internal-fallback";
      ensureCacheLayout(result.layout);
      fs.mkdirSync(result.env.CARGO_TARGET_DIR, { recursive: true });
    }
  }

  return result;
}

function shellQuote(value) {
  const normalized = String(value ?? "");
  return `'${normalized.replace(/'/g, `'\\''`)}'`;
}

function formatShellExports(envEntries) {
  return Object.entries(envEntries)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `export ${key}=${shellQuote(value)}`)
    .join("\n");
}

module.exports = {
  CACHE_ROOT_MARKER_FILE,
  DEFAULT_EXTERNAL_CACHE_ROOT,
  DEFAULT_INTERNAL_VOLATILE_ROOT,
  DEFAULT_REPO_CACHE_SLUG,
  buildCtxCacheEnv,
  ensureCacheLayout,
  formatShellExports,
  isWritablePath,
  resolveAvailableSccachePath,
  resolveConfiguredPath,
  resolveCtxCacheLayout,
  resolveRepoScopeKey,
  resolveSccacheServerUds,
  resolveVolatileSelection,
  writeCacheRootMarker,
};
