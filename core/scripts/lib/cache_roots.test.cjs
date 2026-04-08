const assert = require("node:assert/strict");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  buildCtxCacheEnv,
  formatShellExports,
  resolveCtxCacheLayout,
  resolveRepoScopeKey,
  resolveSccacheServerUds,
  resolveVolatileSelection,
} = require("./cache_roots.cjs");

test("resolveRepoScopeKey returns a stable non-empty scope key", () => {
  const coreRoot = path.resolve(__dirname, "..", "..");
  const scopeKey = resolveRepoScopeKey(coreRoot);

  assert.equal(typeof scopeKey, "string");
  assert.equal(scopeKey.length > 0, true);
});

test("resolveRepoScopeKey prefers explicit and session-scoped cache keys over worktree ids", () => {
  const coreRoot = path.resolve(__dirname, "..", "..");

  assert.equal(
    resolveRepoScopeKey(coreRoot, {
      CTX_CACHE_SCOPE_KEY: "shared-agent-slot",
      CTX_SESSION_ID: "session-123",
    }),
    "shared-agent-slot",
  );
  assert.equal(
    resolveRepoScopeKey(coreRoot, {
      CTX_SESSION_ID: "session-123",
    }),
    "session-123",
  );
  assert.equal(
    resolveRepoScopeKey(coreRoot, {
      CODEX_THREAD_ID: "thread/unsafe value",
    }),
    "thread-unsafe-value",
  );
});

test("resolveVolatileSelection prefers explicit volatile roots", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const explicitRoot = path.join(os.tmpdir(), "ctx-cache-roots-explicit");
  const selection = resolveVolatileSelection({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: explicitRoot,
    },
  });

  assert.equal(selection.volatileRoot, explicitRoot);
  assert.equal(selection.volatileRootMode, "explicit");
});

test("resolveCtxCacheLayout falls back to internal volatile storage when external cache root is unavailable", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const scopeKey = resolveRepoScopeKey(cwd, {});
  const internalRoot = path.join(os.tmpdir(), "ctx-cache-roots-internal");
  const layout = resolveCtxCacheLayout({
    cwd,
    env: {
      HOME: os.homedir(),
      CTX_EXTERNAL_CACHE_ROOT: path.join(os.tmpdir(), "ctx-cache-roots-missing", "external"),
      CTX_INTERNAL_VOLATILE_ROOT: internalRoot,
    },
  });

  assert.equal(layout.volatileRootMode, "internal-fallback");
  assert.equal(layout.volatileRoot, internalRoot);
  assert.equal(layout.workspaceCargoTargetDir, path.join(internalRoot, "targets", "ctx-monorepo", scopeKey));
  assert.equal(layout.verifyCargoTargetDir, layout.workspaceCargoTargetDir);
});

test("buildCtxCacheEnv sets shared cache defaults and keeps verify quick on the workspace target by default", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const scopeKey = "session-scope";
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-env");
  const { env, layout } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_SESSION_ID: scopeKey,
    },
  });

  assert.equal(env.CARGO_TARGET_DIR, path.join(volatileRoot, "targets", "ctx-monorepo", scopeKey));
  assert.equal(env.CTX_VERIFY_CARGO_TARGET_DIR, env.CARGO_TARGET_DIR);
  assert.equal(env.CARGO_HOME, undefined);
  assert.equal(env.TURBO_CACHE_DIR, path.join(volatileRoot, "cache", "turbo", "ctx-monorepo"));
  assert.equal(env.CTX_BUNDLE_CACHE_DIR, path.join(volatileRoot, "cache", "bundles"));
  assert.equal(env.PLAYWRIGHT_BROWSERS_PATH, path.join(volatileRoot, "cache", "playwright"));
  assert.equal(layout.bundleCacheDir, path.join(volatileRoot, "cache", "bundles"));
  assert.equal(layout.volatileRootMode, "explicit");
});

test("buildCtxCacheEnv normalizes sccache inputs when sccache is active", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-sccache");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      RUSTC_WRAPPER: "/opt/homebrew/bin/sccache",
    },
  });

  assert.equal(env.CARGO_INCREMENTAL, "0");
  assert.match(String(env.SCCACHE_BASEDIRS), new RegExp(path.join(volatileRoot, "targets").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.match(String(env.SCCACHE_BASEDIRS), new RegExp(cwd.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.match(String(env.RUSTFLAGS), /--remap-path-prefix=.*=\/ctx-workspace/);
  assert.match(String(env.RUSTFLAGS), /--remap-path-prefix=.*=\/ctx-volatile/);
  if (process.platform !== "win32") {
    assert.equal(env.SCCACHE_SERVER_UDS, resolveSccacheServerUds(env.CARGO_TARGET_DIR));
  }
});

test("buildCtxCacheEnv preserves explicit incremental and existing path normalization inputs", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-sccache-existing");
  const existingBaseDir = path.join(os.tmpdir(), "existing-basedir");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      RUSTC_WRAPPER: "/opt/homebrew/bin/sccache",
      CARGO_INCREMENTAL: "1",
      SCCACHE_BASEDIRS: existingBaseDir,
      RUSTFLAGS: "--cfg existing_flag",
    },
  });

  assert.equal(env.CARGO_INCREMENTAL, "1");
  assert.match(String(env.SCCACHE_BASEDIRS), new RegExp(existingBaseDir.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.match(String(env.RUSTFLAGS), /--cfg existing_flag/);
  assert.match(String(env.RUSTFLAGS), /--remap-path-prefix=.*=\/ctx-workspace/);
});

test("buildCtxCacheEnv honors explicit verify target overrides", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-verify");
  const verifyTargetDir = path.join(volatileRoot, "targets", "ctx-monorepo", "verify-custom");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_VERIFY_CARGO_TARGET_DIR: verifyTargetDir,
    },
    mode: "verify-quick",
  });

  assert.equal(env.CARGO_TARGET_DIR, verifyTargetDir);
  assert.equal(env.CTX_VERIFY_CARGO_TARGET_DIR, verifyTargetDir);
});

test("buildCtxCacheEnv can opt into a volatile cargo home", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-cargo-home");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_USE_VOLATILE_CARGO_HOME: "1",
    },
  });

  assert.equal(env.CARGO_HOME, path.join(volatileRoot, "cache", "cargo-home"));
  assert.equal(env.CTX_BUNDLE_CACHE_DIR, path.join(volatileRoot, "cache", "bundles"));
});

test("formatShellExports produces stable export lines", () => {
  const rendered = formatShellExports({
    BETA: "two words",
    ALPHA: "one",
  });

  assert.equal(rendered, "export ALPHA='one'\nexport BETA='two words'");
});
