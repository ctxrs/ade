#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");

function usage() {
  console.error(
    "usage: run_with_ctx_cache_env.cjs [--mode workspace|verify-quick] [--cwd <dir>] -- <command> [args...]",
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
