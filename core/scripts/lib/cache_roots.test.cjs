const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  buildCtxCacheEnv,
  formatShellExports,
  LEGACY_TURBO_ENV_KEYS,
  resolveCtxCacheLayout,
  resolveRepoScopeKey,
  resolveSccacheDir,
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
  assert.equal(env.CARGO_INCREMENTAL, "0");
  assert.equal(env.CARGO_HOME, undefined);
  assert.equal(env.CTX_RUST_CACHE_MODE, "workspace");
  assert.equal(env.CTX_RUST_CACHE_SCOPE_KEY, scopeKey);
  assert.equal(env.CTX_RUST_CACHE_REPO_SLUG, "ctx-monorepo");
  assert.equal(env.CTX_RUST_CACHE_TARGET_DIR, env.CARGO_TARGET_DIR);
  assert.equal(env.CTX_RUST_CACHE_VERIFY_TARGET_DIR, env.CTX_VERIFY_CARGO_TARGET_DIR);
  assert.equal(env.CTX_RUST_CACHE_VOLATILE_ROOT_MODE, "explicit");
  assert.equal(env.CTX_RUST_CACHE_SCCACHE, "unconfigured");
  assert.equal(env.CTX_BAZEL_DISK_CACHE_DIR, path.join(volatileRoot, "cache", "bazel-disk", "ctx-monorepo"));
  assert.equal(
    env.CTX_BAZEL_REPOSITORY_CACHE_DIR,
    path.join(volatileRoot, "cache", "bazel-repository", "ctx-monorepo"),
  );
  assert.equal(
    env.CTX_BAZEL_OUTPUT_USER_ROOT,
    path.join(volatileRoot, "targets", "bazel", scopeKey),
  );
  assert.equal(env.CTX_BUNDLE_CACHE_DIR, path.join(volatileRoot, "cache", "bundles"));
  assert.equal(env.PLAYWRIGHT_BROWSERS_PATH, path.join(volatileRoot, "cache", "playwright"));
  assert.equal(layout.bazelDiskCacheDir, path.join(volatileRoot, "cache", "bazel-disk", "ctx-monorepo"));
  assert.equal(
    layout.bazelRepositoryCacheDir,
    path.join(volatileRoot, "cache", "bazel-repository", "ctx-monorepo"),
  );
  assert.equal(layout.bazelOutputUserRoot, path.join(volatileRoot, "targets", "bazel", scopeKey));
  assert.equal(layout.bundleCacheDir, path.join(volatileRoot, "cache", "bundles"));
  assert.equal(layout.volatileRootMode, "explicit");
  assert.equal(env.RUSTC_WRAPPER, undefined);
  assert.equal(env.SCCACHE_PATH, undefined);
});

test("buildCtxCacheEnv scrubs retired Turbo cache environment", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: path.join(os.tmpdir(), "ctx-cache-roots-no-turbo"),
      CTX_SESSION_ID: "no-turbo-session",
      CTX_CACHE_ENV_MANAGED_TURBO_CACHE_DIR: "1",
      TURBO_API: "https://example.invalid",
      TURBO_CACHE_DIR: "/tmp/old-turbo-cache",
      TURBO_TEAM: "legacy-team",
      TURBO_TOKEN: "legacy-token",
    },
  });

  for (const key of LEGACY_TURBO_ENV_KEYS) {
    assert.equal(env[key], undefined);
  }
});

test("buildCtxCacheEnv derives a per-shell session scope when none is provided", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-derived-session");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
    },
  });

  assert.match(String(env.CTX_RUST_CACHE_SCOPE_KEY), /^ppid-\d+$/);
  assert.equal(
    env.CARGO_TARGET_DIR,
    path.join(volatileRoot, "targets", "ctx-monorepo", env.CTX_RUST_CACHE_SCOPE_KEY),
  );
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
  assert.equal(env.SCCACHE_NO_DAEMON, "1");
  if (process.platform !== "win32") {
    assert.equal(env.SCCACHE_SERVER_UDS, resolveSccacheServerUds(env.CARGO_TARGET_DIR));
  }
});

test("buildCtxCacheEnv replaces unsafe inherited sccache sockets with a short socket path", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(
    os.tmpdir(),
    "ctx-cache-roots-remote-real-xdg-fixture-with-deep-bundle-cache",
    "home",
    ".ctx",
    "volatile",
  );
  const targetDir = path.join(
    volatileRoot,
    "cache",
    "bundles",
    ".build",
    "cargo",
    "linux-x86_64",
  );
  const unsafeSocket = path.join(
    targetDir,
    "sccache",
    "server",
    "sccache.sock",
  );
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CARGO_TARGET_DIR: targetDir,
      RUSTC_WRAPPER: "/usr/bin/sccache",
      SCCACHE_SERVER_UDS: unsafeSocket,
    },
  });

  if (process.platform !== "win32") {
    assert.equal(env.SCCACHE_SERVER_UDS, resolveSccacheServerUds(targetDir));
    assert.equal(Buffer.byteLength(env.SCCACHE_SERVER_UDS) < 100, true);
  }
});

test("buildCtxCacheEnv shortens sccache dir when its default socket path would be unsafe", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(
    os.tmpdir(),
    "ctx-cache-roots-remote-real-xdg-fixture-with-deep-bundle-cache",
    "home",
    ".ctx",
    "volatile",
  );
  const targetDir = path.join(
    volatileRoot,
    "cache",
    "bundles",
    ".build",
    "cargo",
    "linux-x86_64",
  );
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CARGO_TARGET_DIR: targetDir,
      RUSTC_WRAPPER: "/usr/bin/sccache",
    },
  });

  if (process.platform !== "win32") {
    assert.equal(env.SCCACHE_DIR, resolveSccacheDir(targetDir));
    assert.equal(Buffer.byteLength(path.join(env.SCCACHE_DIR, "sccache.sock")) < 100, true);
    assert.equal(env.SCCACHE_SERVER_UDS, resolveSccacheServerUds(targetDir));
    assert.equal(Buffer.byteLength(env.SCCACHE_SERVER_UDS) < 100, true);
  }
});

test("buildCtxCacheEnv disables sccache entirely when CTX_DISABLE_SCCACHE is enabled", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-sccache-disabled");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_DISABLE_SCCACHE: "1",
      RUSTC_WRAPPER: "/opt/homebrew/bin/sccache",
      SCCACHE_PATH: "/opt/homebrew/bin/sccache",
      SCCACHE_NO_DAEMON: "1",
      SCCACHE_SERVER_UDS: "/tmp/ctx-sccache.sock",
    },
  });

  assert.equal(env.RUSTC_WRAPPER, undefined);
  assert.equal(env.SCCACHE_PATH, undefined);
  assert.equal(env.SCCACHE_NO_DAEMON, undefined);
  assert.equal(env.SCCACHE_SERVER_UDS, undefined);
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

test("buildCtxCacheEnv derives standard sccache S3 env from Cloudflare R2-specific inputs", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-r2");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      RUSTC_WRAPPER: "/opt/homebrew/bin/sccache",
      CTX_SCCACHE_R2_BUCKET: "test-sccache-bucket",
      CTX_SCCACHE_R2_ACCOUNT_ID: "test-r2-account-id",
      CTX_SCCACHE_R2_KEY_PREFIX: "sccache/dev",
      CTX_SCCACHE_R2_ACCESS_KEY_ID: "access-key-id",
      CTX_SCCACHE_R2_SECRET_ACCESS_KEY: "secret-access-key",
    },
  });

  assert.equal(env.SCCACHE_BUCKET, "test-sccache-bucket");
  assert.equal(env.SCCACHE_REGION, "auto");
  assert.equal(env.SCCACHE_ENDPOINT, "https://test-r2-account-id.r2.cloudflarestorage.com");
  assert.equal(env.SCCACHE_S3_KEY_PREFIX, "sccache/dev");
  assert.equal(env.SCCACHE_S3_USE_SSL, "true");
  assert.equal(env.AWS_ACCESS_KEY_ID, "access-key-id");
  assert.equal(env.AWS_SECRET_ACCESS_KEY, "secret-access-key");
});

test("buildCtxCacheEnv preserves explicit standard sccache env over derived R2 defaults", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-r2-explicit");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      RUSTC_WRAPPER: "/opt/homebrew/bin/sccache",
      CTX_SCCACHE_R2_BUCKET: "test-sccache-bucket",
      CTX_SCCACHE_R2_ACCOUNT_ID: "test-r2-account-id",
      SCCACHE_ENDPOINT: "https://example.invalid",
      SCCACHE_S3_KEY_PREFIX: "explicit/prefix",
      AWS_ACCESS_KEY_ID: "existing-id",
    },
  });

  assert.equal(env.SCCACHE_ENDPOINT, "https://example.invalid");
  assert.equal(env.SCCACHE_S3_KEY_PREFIX, "explicit/prefix");
  assert.equal(env.AWS_ACCESS_KEY_ID, "existing-id");
});

test("buildCtxCacheEnv prefers ctx-specific R2 credentials over ambient AWS credentials", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-r2-override");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      RUSTC_WRAPPER: "/opt/homebrew/bin/sccache",
      CTX_SCCACHE_R2_BUCKET: "test-sccache-bucket",
      CTX_SCCACHE_R2_ACCOUNT_ID: "test-r2-account-id",
      CTX_SCCACHE_R2_ACCESS_KEY_ID: "r2-access-key",
      CTX_SCCACHE_R2_SECRET_ACCESS_KEY: "r2-secret-key",
      CTX_SCCACHE_R2_SESSION_TOKEN: "r2-session-token",
      AWS_ACCESS_KEY_ID: "ambient-access-key",
      AWS_SECRET_ACCESS_KEY: "ambient-secret-key",
      AWS_SESSION_TOKEN: "ambient-session-token",
    },
  });

  assert.equal(env.AWS_ACCESS_KEY_ID, "r2-access-key");
  assert.equal(env.AWS_SECRET_ACCESS_KEY, "r2-secret-key");
  assert.equal(env.AWS_SESSION_TOKEN, "r2-session-token");
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

test("buildCtxCacheEnv preserves explicit cargo target overrides outside an explicit volatile root", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-explicit-volatile");
  const cargoTargetDir = path.join(os.tmpdir(), "ctx-cache-roots-external-target");
  const verifyTargetDir = path.join(os.tmpdir(), "ctx-cache-roots-external-verify");
  const { env, layout } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CARGO_TARGET_DIR: cargoTargetDir,
      CTX_VERIFY_CARGO_TARGET_DIR: verifyTargetDir,
    },
  });

  assert.equal(layout.workspaceCargoTargetDir, cargoTargetDir);
  assert.equal(layout.verifyCargoTargetDir, verifyTargetDir);
  assert.equal(env.CARGO_TARGET_DIR, cargoTargetDir);
  assert.equal(env.CTX_VERIFY_CARGO_TARGET_DIR, verifyTargetDir);
});

test("buildCtxCacheEnv falls back to CTX_E2E_CARGO_TARGET_DIR when CARGO_TARGET_DIR is unset", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-e2e-target");
  const cargoTargetDir = path.join(os.tmpdir(), "ctx-cache-roots-ctx-e2e-target");
  const { env, layout } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_E2E_CARGO_TARGET_DIR: cargoTargetDir,
    },
  });

  assert.equal(layout.workspaceCargoTargetDir, cargoTargetDir);
  assert.equal(env.CARGO_TARGET_DIR, cargoTargetDir);
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

test("buildCtxCacheEnv honors explicit Bazel cache path overrides inside an explicit volatile root", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-bazel-explicit");
  const explicitDiskCacheDir = path.join(volatileRoot, "custom", "bazel-disk");
  const explicitRepositoryCacheDir = path.join(volatileRoot, "custom", "bazel-repository");
  const explicitOutputUserRoot = path.join(volatileRoot, "custom", "bazel-output");
  const { env, layout } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_BAZEL_DISK_CACHE_DIR: explicitDiskCacheDir,
      CTX_BAZEL_REPOSITORY_CACHE_DIR: explicitRepositoryCacheDir,
      CTX_BAZEL_OUTPUT_USER_ROOT: explicitOutputUserRoot,
    },
  });

  assert.equal(env.CTX_BAZEL_DISK_CACHE_DIR, explicitDiskCacheDir);
  assert.equal(env.CTX_BAZEL_REPOSITORY_CACHE_DIR, explicitRepositoryCacheDir);
  assert.equal(env.CTX_BAZEL_OUTPUT_USER_ROOT, explicitOutputUserRoot);
  assert.equal(layout.bazelDiskCacheDir, explicitDiskCacheDir);
  assert.equal(layout.bazelRepositoryCacheDir, explicitRepositoryCacheDir);
  assert.equal(layout.bazelOutputUserRoot, explicitOutputUserRoot);
});

test("buildCtxCacheEnv falls back to the internal volatile root when preferred external layout creation fails", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const tempRoot = path.join(os.tmpdir(), "ctx-cache-roots-mkdir-fallback");
  const blockedPreferredRoot = path.join(tempRoot, "blocked-preferred-root");
  const internalRoot = path.join(tempRoot, "internal-root");

  fs.rmSync(tempRoot, { recursive: true, force: true });
  fs.mkdirSync(tempRoot, { recursive: true });
  fs.writeFileSync(blockedPreferredRoot, "blocked");

  const { env, layout } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_PREFERRED_VOLATILE_ROOT: blockedPreferredRoot,
      CTX_INTERNAL_VOLATILE_ROOT: internalRoot,
    },
    mkdir: true,
  });

  assert.equal(layout.volatileRoot, internalRoot);
  assert.equal(layout.volatileRootMode, "internal-fallback");
  assert.equal(env.CTX_VOLATILE_ROOT, internalRoot);
  assert.equal(env.CTX_VOLATILE_ROOT_MODE, "internal-fallback");
  assert.equal(fs.existsSync(path.join(internalRoot, ".ctx-volatile-root.json")), true);
});

test("buildCtxCacheEnv does not apply sccache env when RUSTC_WRAPPER is a non-sccache binary", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-non-sccache");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      RUSTC_WRAPPER: "/usr/bin/custom-wrapper",
      SCCACHE_PATH: "",
    },
  });

  assert.equal(env.RUSTC_WRAPPER, "/usr/bin/custom-wrapper");
  assert.equal(env.CARGO_INCREMENTAL, "0");
  assert.equal(env.CTX_RUST_CACHE_SCCACHE, "unconfigured");
  assert.equal(env.SCCACHE_NO_DAEMON, undefined);
  assert.equal(env.SCCACHE_BASEDIRS, undefined);
  assert.equal(env.SCCACHE_SERVER_UDS, undefined);
});

test("buildCtxCacheEnv applies sccache env when RUSTC_WRAPPER basename contains sccache", () => {
  const cwd = path.resolve(__dirname, "..", "..");
  const volatileRoot = path.join(os.tmpdir(), "ctx-cache-roots-sccache-basename");
  const { env } = buildCtxCacheEnv({
    cwd,
    env: {
      CTX_VOLATILE_ROOT: volatileRoot,
      RUSTC_WRAPPER: "/custom/path/sccache",
      SCCACHE_PATH: "/custom/path/sccache",
    },
  });

  assert.equal(env.CARGO_INCREMENTAL, "0");
  assert.equal(env.SCCACHE_NO_DAEMON, "1");
  assert.match(String(env.RUSTFLAGS), /--remap-path-prefix=.*=\/ctx-workspace/);
});

test("formatShellExports produces stable export lines", () => {
  const rendered = formatShellExports({
    BETA: "two words",
    ALPHA: "one",
  });

  assert.equal(rendered, "export ALPHA='one'\nexport BETA='two words'");
});
