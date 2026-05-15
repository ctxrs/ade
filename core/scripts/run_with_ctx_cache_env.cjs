#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");

function parseEnabledFlag(value) {
  return !["0", "false", "no", "off"].includes(String(value ?? "").trim().toLowerCase());
}

function usage() {
  console.error(
    "usage: run_with_ctx_cache_env.cjs [--mode workspace|verify-quick] [--cwd <dir>] -- <command> [args...]",
  );
}

function logCacheSummary(env) {
  if (!parseEnabledFlag(env.CTX_RUST_CACHE_LOG ?? "1")) {
    return;
  }
  console.error(
    "[ctx-cache] source=%s mode=%s scope=%s target=%s sccache=%s no_daemon=%s uds=%s tmp=%s volatile_root_mode=%s",
    env.CTX_RUST_CACHE_SOURCE || "run_with_ctx_cache_env",
    env.CTX_RUST_CACHE_MODE || "workspace",
    env.CTX_RUST_CACHE_SCOPE_KEY || "unknown",
    env.CARGO_TARGET_DIR || env.CTX_RUST_CACHE_TARGET_DIR || "unset",
    env.CTX_RUST_CACHE_SCCACHE || "unconfigured",
    env.SCCACHE_NO_DAEMON || "unset",
    env.SCCACHE_SERVER_UDS || "unset",
    env.TMPDIR || "unset",
    env.CTX_RUST_CACHE_VOLATILE_ROOT_MODE || env.CTX_VOLATILE_ROOT_MODE || "unset",
  );
}

function main() {
  const args = process.argv.slice(2);
  const separatorIndex = args.indexOf("--");
  if (separatorIndex === -1 || separatorIndex === args.length - 1) {
    usage();
    process.exit(2);
  }

  let mode = "workspace";
  let cwd = path.resolve(__dirname, "..");
  for (let index = 0; index < separatorIndex; index += 1) {
    const arg = args[index];
    switch (arg) {
      case "--mode":
        mode = args[index + 1] || "";
        index += 1;
        break;
      case "--cwd":
        cwd = path.resolve(args[index + 1] || "");
        index += 1;
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

  const command = args[separatorIndex + 1];
  const commandArgs = args.slice(separatorIndex + 2);
  const { env } = buildCtxCacheEnv({
    cwd,
    env: process.env,
    mode,
    mkdir: true,
  });
  env.CTX_RUST_CACHE_SOURCE = "run_with_ctx_cache_env";
  env.CTX_RUST_CACHE_WRAPPED = "1";
  logCacheSummary(env);

  const result = childProcess.spawnSync(command, commandArgs, {
    cwd,
    env,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  process.exit(result.status ?? 1);
}

main();
