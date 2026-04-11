#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");

function parseArgs(argv) {
  const args = {
    crate: "",
    task: "",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--crate") {
      args.crate = argv[index + 1] || "";
      index += 1;
    } else if (arg === "--task") {
      args.task = argv[index + 1] || "";
      index += 1;
    } else {
      throw new Error(`unknown arg: ${arg}`);
    }
  }
  if (!args.crate) {
    throw new Error("--crate is required");
  }
  if (!args.task) {
    throw new Error("--task is required");
  }
  return args;
}

function run(command, args, options) {
  const result = childProcess.spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function ensureCargoTargetScaffolding(env) {
  const targetDir = String(env.CARGO_TARGET_DIR || "").trim();
  if (!targetDir) {
    return;
  }
  for (const relPath of [
    ".",
    "debug",
    "debug/deps",
    "debug/build",
    "debug/examples",
    "debug/incremental",
  ]) {
    fs.mkdirSync(path.join(targetDir, relPath), { recursive: true });
  }
}

function main() {
  const { crate, task } = parseArgs(process.argv.slice(2));
  const coreRoot = path.resolve(__dirname, "..");
  const { env } = buildCtxCacheEnv({
    cwd: coreRoot,
    env: process.env,
    mode: "workspace",
    mkdir: true,
  });
  ensureCargoTargetScaffolding(env);

  if (task === "clippy") {
    run(
      "cargo",
      ["clippy", "-p", crate, "--all-targets", "--all-features", "--", "-D", "warnings"],
      {
        cwd: coreRoot,
        env,
      },
    );
    return;
  }

  if (task === "test") {
    run("cargo", ["test", "-q", "-p", crate], {
      cwd: coreRoot,
      env,
    });
    return;
  }

  if (task === "nextest") {
    if (!String(env.NEXTEST_TEST_THREADS ?? "").trim()) {
      env.NEXTEST_TEST_THREADS = "2";
    }
    run("cargo", ["nextest", "run", "--no-tests", "pass", "-p", crate], {
      cwd: coreRoot,
      env,
    });
    return;
  }

  throw new Error(`unknown task: ${task}`);
}

main();
