#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const { runTurbo } = require("./lib/turbo_runner.cjs");

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
  run("node", ["--test", "scripts/buildbuddy_workflow_contract.test.cjs"], env);
  run("pnpm", ["desktop:ipc:check"], env);
  run("pnpm", ["bazel:web:any:enforce"], env);
  run("pnpm", ["bazel:web:lint"], env);
  run("pnpm", ["rust:fmt"], env);
  run("pnpm", ["rust:panic-traps"], env);
  run("pnpm", ["rust:turbo:check"], env);
  run(
    "node",
    [
      "scripts/run_rust_gate.cjs",
      "--mode",
      "verify-quick",
      "--all",
      "--clippy",
      "--test-strategy",
      "mixed",
    ],
    env,
  );
  runTurbo({
    coreRoot,
    env,
    taskNames: [
      "supabase:migrations:check",
      "supabase:functions:check",
    ],
    extraArgs: ["--filter=ctx-monorepo", "--filter=ctx-web"],
  });
  run("pnpm", ["bazel:web:typecheck"], env);
}

main();
