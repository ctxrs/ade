#!/usr/bin/env node

const path = require("node:path");

const {
  buildCtxCacheEnv,
  formatShellExports,
} = require("./lib/cache_roots.cjs");

function usage() {
  console.error(
    "usage: print_ctx_cache_env.cjs [--mode workspace|verify-quick] [--format shell|json] [--cwd <dir>] [--mkdir]",
  );
}

function main() {
  const args = process.argv.slice(2);
  let mode = "workspace";
  let format = "shell";
  let cwd = path.resolve(__dirname, "..");
  let mkdir = false;

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    switch (arg) {
      case "--mode":
        mode = args[index + 1] || "";
        index += 1;
        break;
      case "--format":
        format = args[index + 1] || "";
        index += 1;
        break;
      case "--cwd":
        cwd = path.resolve(args[index + 1] || "");
        index += 1;
        break;
      case "--mkdir":
        mkdir = true;
        break;
      case "-h":
      case "--help":
        usage();
        process.exit(0);
        break;
      default:
        console.error(`error: unknown option '${arg}'`);
        usage();
        process.exit(2);
    }
  }

  if (!new Set(["workspace", "verify-quick"]).has(mode)) {
    console.error(`error: unsupported mode '${mode}'`);
    process.exit(2);
  }
  if (!new Set(["shell", "json"]).has(format)) {
    console.error(`error: unsupported format '${format}'`);
    process.exit(2);
  }

  const { env, layout, cargoTargetDir } = buildCtxCacheEnv({
    cwd,
    env: process.env,
    mode,
    mkdir,
  });
  env.CTX_RUST_CACHE_SOURCE = "print_ctx_cache_env";
  const payload = {
    CTX_EXTERNAL_CACHE_ROOT: env.CTX_EXTERNAL_CACHE_ROOT,
    CTX_INTERNAL_VOLATILE_ROOT: env.CTX_INTERNAL_VOLATILE_ROOT,
    CTX_PREFERRED_VOLATILE_ROOT: env.CTX_PREFERRED_VOLATILE_ROOT,
    CTX_VOLATILE_ROOT: env.CTX_VOLATILE_ROOT,
    CTX_VOLATILE_ROOT_MODE: env.CTX_VOLATILE_ROOT_MODE,
    CTX_VOLATILE_TARGETS_DIR: env.CTX_VOLATILE_TARGETS_DIR,
    CTX_VOLATILE_ARTIFACTS_DIR: env.CTX_VOLATILE_ARTIFACTS_DIR,
    CTX_VOLATILE_TMPDIR: env.CTX_VOLATILE_TMPDIR,
    ...(env.TMPDIR ? { TMPDIR: env.TMPDIR } : {}),
    ...(env.TMP ? { TMP: env.TMP } : {}),
    ...(env.TEMP ? { TEMP: env.TEMP } : {}),
    CTX_VOLATILE_CACHE_DIR: env.CTX_VOLATILE_CACHE_DIR,
    CARGO_TARGET_DIR: cargoTargetDir,
    CTX_VERIFY_CARGO_TARGET_DIR: env.CTX_VERIFY_CARGO_TARGET_DIR,
    CTX_RUST_CACHE_SOURCE: env.CTX_RUST_CACHE_SOURCE,
    CTX_RUST_CACHE_MODE: env.CTX_RUST_CACHE_MODE,
    CTX_RUST_CACHE_SCOPE_KEY: env.CTX_RUST_CACHE_SCOPE_KEY,
    CTX_RUST_CACHE_REPO_SLUG: env.CTX_RUST_CACHE_REPO_SLUG,
    CTX_RUST_CACHE_TARGET_DIR: env.CTX_RUST_CACHE_TARGET_DIR,
    CTX_RUST_CACHE_VERIFY_TARGET_DIR: env.CTX_RUST_CACHE_VERIFY_TARGET_DIR,
    CTX_RUST_CACHE_VOLATILE_ROOT_MODE: env.CTX_RUST_CACHE_VOLATILE_ROOT_MODE,
    CTX_RUST_CACHE_SCCACHE: env.CTX_RUST_CACHE_SCCACHE,
    CTX_BAZEL_DISK_CACHE_DIR: env.CTX_BAZEL_DISK_CACHE_DIR,
    CTX_BAZEL_REPOSITORY_CACHE_DIR: env.CTX_BAZEL_REPOSITORY_CACHE_DIR,
    CTX_BAZEL_OUTPUT_USER_ROOT: env.CTX_BAZEL_OUTPUT_USER_ROOT,
    CTX_BUNDLE_CACHE_DIR: env.CTX_BUNDLE_CACHE_DIR,
    PLAYWRIGHT_BROWSERS_PATH: env.PLAYWRIGHT_BROWSERS_PATH,
    SCCACHE_DIR: env.SCCACHE_DIR,
    ...(env.SCCACHE_BASEDIRS ? { SCCACHE_BASEDIRS: env.SCCACHE_BASEDIRS } : {}),
    ...(env.SCCACHE_SERVER_UDS ? { SCCACHE_SERVER_UDS: env.SCCACHE_SERVER_UDS } : {}),
    ...(env.SCCACHE_BUCKET ? { SCCACHE_BUCKET: env.SCCACHE_BUCKET } : {}),
    ...(env.SCCACHE_ENDPOINT ? { SCCACHE_ENDPOINT: env.SCCACHE_ENDPOINT } : {}),
    ...(env.SCCACHE_REGION ? { SCCACHE_REGION: env.SCCACHE_REGION } : {}),
    ...(env.SCCACHE_S3_KEY_PREFIX ? { SCCACHE_S3_KEY_PREFIX: env.SCCACHE_S3_KEY_PREFIX } : {}),
    ...(env.SCCACHE_S3_USE_SSL ? { SCCACHE_S3_USE_SSL: env.SCCACHE_S3_USE_SSL } : {}),
    ...(env.CARGO_INCREMENTAL ? { CARGO_INCREMENTAL: env.CARGO_INCREMENTAL } : {}),
    ...(env.RUSTFLAGS ? { RUSTFLAGS: env.RUSTFLAGS } : {}),
    ...(env.CARGO_HOME ? { CARGO_HOME: env.CARGO_HOME } : {}),
    ...(env.SCCACHE_PATH ? { SCCACHE_PATH: env.SCCACHE_PATH } : {}),
    ...(env.RUSTC_WRAPPER ? { RUSTC_WRAPPER: env.RUSTC_WRAPPER } : {}),
  };

  if (format === "json") {
    process.stdout.write(
      `${JSON.stringify({ mode, cargo_target_dir: cargoTargetDir, layout, env: payload }, null, 2)}\n`,
    );
    return;
  }

  process.stdout.write(`${formatShellExports(payload)}\n`);
}

main();
