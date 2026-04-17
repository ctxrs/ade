#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const { runTurbo } = require("./lib/turbo_runner.cjs");

const coreRoot = path.resolve(__dirname, "..");

function applyVerifyQuickDefaults(env, platform = process.platform) {
  if (!String(env.CARGO_INCREMENTAL ?? "").trim()) {
    env.CARGO_INCREMENTAL = "0";
  }
  if (!String(env.RUST_TEST_THREADS ?? "").trim()) {
    env.RUST_TEST_THREADS = "1";
  }
  if (!String(env.CTX_BAZEL_REMOTE_EXECUTION ?? "").trim()) {
    if (platform === "darwin") {
      env.CTX_BAZEL_REMOTE_EXECUTION = "cache";
    } else if (platform === "linux") {
      env.CTX_BAZEL_REMOTE_EXECUTION = "linux";
    }
  }
  if (platform === "darwin") {
    if (!String(env.CTX_BAZEL_BATCH ?? "").trim()) {
      env.CTX_BAZEL_BATCH = "0";
    }
    if (!String(env.CTX_BAZEL_JOBS ?? "").trim()) {
      env.CTX_BAZEL_JOBS = "1";
    }
  }
}

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
  applyVerifyQuickDefaults(env);

  run("pnpm", ["source:file-size:enforce"], env);
  run(
    "node",
    [
      "--test",
      "scripts/buildbuddy_workflow_contract.test.cjs",
      "scripts/buildkite_pipeline_contract.test.cjs",
      "scripts/buildkite_hetzner_contract.test.cjs",
      "scripts/buildbuddy_run_job_contract.test.cjs",
      "scripts/buildbuddy_desktop_system_parity_contract.test.cjs",
      "scripts/buildbuddy_release_preflight_contract.test.cjs",
      "scripts/buildbuddy_macos_publish_prereqs.test.cjs",
      "scripts/affected_tests_contract.test.cjs",
      "scripts/bundled_harnesses_docker_contracts_bazel_contract.test.cjs",
      "scripts/ctx_http_bazel_contract.test.cjs",
      "scripts/desktop_ipc_bazel_contract.test.cjs",
      "scripts/ctx_harness_dockerfile_contract.test.cjs",
      "scripts/install_site_bazel_contract.test.cjs",
      "scripts/release_bundle_contracts_bazel_contract.test.cjs",
      "scripts/web_dist_bazel_contract.test.cjs",
      "scripts/updates_failure_safety_bazel_contract.test.cjs",
      "scripts/run_workspace_task_contract.test.cjs",
      "scripts/rust_crate_task.test.cjs",
      "scripts/lib/bazel_rust_targets.test.cjs",
      "scripts/lib/ctx_http_suites.test.cjs",
    ],
    env,
  );
  run("bash", ["../scripts/tests/provider_deps_cross_target_contract.sh"], env);
  run("pnpm", ["bazel:install-site:test"], env);
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

if (require.main === module) {
  main();
}

module.exports = {
  applyVerifyQuickDefaults,
};
