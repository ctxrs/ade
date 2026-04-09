#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const {
  buildCtxHttpSuiteCommands,
  getCtxHttpSuiteNames,
} = require("./lib/ctx_http_suites.cjs");

function parseArgs(argv) {
  const args = {
    list: false,
    suite: "",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--suite") {
      args.suite = argv[index + 1] || "";
      index += 1;
    } else if (arg === "--list") {
      args.list = true;
    } else {
      throw new Error(`unknown arg: ${arg}`);
    }
  }
  if (!args.suite) {
    throw new Error(`--suite is required; expected one of ${getCtxHttpSuiteNames({ includeAll: true }).join(", ")}`);
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

function main() {
  const args = parseArgs(process.argv.slice(2));
  const commands = buildCtxHttpSuiteCommands(args.suite);
  if (args.list) {
    for (const command of commands) {
      console.log([command.command, ...command.args].join(" "));
    }
    return;
  }

  const coreRoot = path.resolve(__dirname, "..");
  const { env } = buildCtxCacheEnv({
    cwd: coreRoot,
    env: process.env,
    mode: "workspace",
    mkdir: true,
  });
  if (!String(env.RUST_TEST_THREADS ?? "").trim()) {
    env.RUST_TEST_THREADS = "1";
  }

  for (const command of commands) {
    run(command.command, command.args, {
      cwd: coreRoot,
      env,
    });
  }
}

main();
