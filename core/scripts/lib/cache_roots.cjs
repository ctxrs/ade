"use strict";

const crypto = require("node:crypto");
const os = require("node:os");
const path = require("node:path");

const hasValue = (value) => String(value ?? "").trim().length > 0;

const resolvePath = (cwd, configured) => {
  const value = String(configured ?? "").trim();
  if (!value) return "";
  return path.isAbsolute(value) ? value : path.resolve(cwd, value);
};

const repoCacheSlug = (cwd) => {
  const hash = crypto.createHash("sha1").update(path.resolve(cwd)).digest("hex").slice(0, 12);
  return `ctx-${hash}`;
};

const resolveCtxCacheLayout = ({ cwd = process.cwd(), env = process.env } = {}) => {
  const resolvedCwd = path.resolve(cwd);
  const explicitRoot = resolvePath(resolvedCwd, env.CTX_VOLATILE_ROOT);
  const preferredRoot = resolvePath(resolvedCwd, env.CTX_VOLATILE_PREFERRED_ROOT);
  const rootDir =
    explicitRoot
    || preferredRoot
    || path.join(os.tmpdir(), repoCacheSlug(resolvedCwd), "volatile");
  const rootMode = explicitRoot
    ? "explicit"
    : preferredRoot
      ? "preferred-external"
      : "internal-fallback";

  const tmpDir = resolvePath(resolvedCwd, env.CTX_VOLATILE_TMPDIR) || path.join(rootDir, "tmp");
  const cacheDir = resolvePath(resolvedCwd, env.CTX_VOLATILE_CACHE_DIR) || path.join(rootDir, "cache");
  const targetsDir = resolvePath(resolvedCwd, env.CTX_VOLATILE_TARGETS_DIR) || path.join(rootDir, "targets");
  const artifactsDir =
    resolvePath(resolvedCwd, env.CTX_VOLATILE_ARTIFACTS_DIR) || path.join(rootDir, "artifacts");
  const cargoHomeDir = resolvePath(resolvedCwd, env.CARGO_HOME) || path.join(cacheDir, "cargo");
  const sccacheDir = resolvePath(resolvedCwd, env.SCCACHE_DIR) || path.join(cacheDir, "sccache");
  const playwrightBrowsersDir =
    resolvePath(resolvedCwd, env.PLAYWRIGHT_BROWSERS_PATH)
    || path.join(cacheDir, "playwright");

  return {
    rootDir,
    rootMode,
    tmpDir,
    cacheDir,
    targetsDir,
    artifactsDir,
    cargoHomeDir,
    sccacheDir,
    playwrightBrowsersDir,
  };
};

const buildCtxCacheEnv = ({ cwd = process.cwd(), env = process.env } = {}) => {
  const layout = resolveCtxCacheLayout({ cwd, env });
  const cacheEnv = {
    CARGO_HOME: env.CARGO_HOME || layout.cargoHomeDir,
    SCCACHE_DIR: env.SCCACHE_DIR || layout.sccacheDir,
    CTX_VOLATILE_ROOT: layout.rootDir,
    CTX_VOLATILE_ROOT_MODE: layout.rootMode,
    CTX_VOLATILE_TARGETS_DIR: layout.targetsDir,
    CTX_VOLATILE_ARTIFACTS_DIR: layout.artifactsDir,
    PLAYWRIGHT_BROWSERS_PATH: env.PLAYWRIGHT_BROWSERS_PATH || layout.playwrightBrowsersDir,
  };

  if (hasValue(env.SCCACHE_PATH)) {
    cacheEnv.SCCACHE_PATH = env.SCCACHE_PATH;
  }
  if (hasValue(env.RUSTC_WRAPPER)) {
    cacheEnv.RUSTC_WRAPPER = env.RUSTC_WRAPPER;
  }

  return { env: cacheEnv, layout };
};

module.exports = {
  buildCtxCacheEnv,
  resolveCtxCacheLayout,
};
