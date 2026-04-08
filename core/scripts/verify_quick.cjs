#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");

const coreRoot = path.resolve(__dirname, "..");

function run(command, args, env) {
  const result = childProcess.spawnSync(command, args, {
    cwd: coreRoot,
    env,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function main() {
  const { env } = buildCtxCacheEnv({
    cwd: coreRoot,
    env: process.env,
    mode: "verify-quick",
    mkdir: true,
  });
  if (!String(env.CARGO_INCREMENTAL ?? "").trim()) {
    env.CARGO_INCREMENTAL = "0";
  }
  if (!String(env.RUST_TEST_THREADS ?? "").trim()) {
    env.RUST_TEST_THREADS = "1";
  }

  run("pnpm", ["source:file-size:enforce"], env);
  run("pnpm", ["desktop:ipc:check"], env);
  run("pnpm", ["-C", "apps/web", "lint"], env);
  run(
    "turbo",
    [
      "run",
      "rust:fmt",
      "rust:panic-traps",
      "rust:clippy",
      "rust:test",
      "supabase:migrations:check",
      "supabase:functions:check",
      "typecheck",
      "any:enforce",
      "--filter=ctx-monorepo",
      "--filter=ctx-web",
      `--cache-dir=${env.TURBO_CACHE_DIR}`,
      "--output-logs=errors-only",
      "--log-order=grouped",
      `--concurrency=${env.CTX_VERIFY_TURBO_CONCURRENCY || "4"}`,
      "--ui=stream",
      `--cache=${env.TURBO_CACHE_MODE || "local:rw"}`,
    ],
    env,
  );
}

main();
