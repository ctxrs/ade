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
    suites: [],
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--suite") {
      const suiteName = argv[index + 1] || "";
      if (!suiteName || suiteName.startsWith("--")) {
        throw new Error("--suite requires a suite name");
      }
      args.suites.push(suiteName);
      index += 1;
    } else if (arg === "--list") {
      args.list = true;
    } else {
      throw new Error(`unknown arg: ${arg}`);
    }
  }
  if (args.suites.length === 0) {
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

function buildTaskPlan({
  argv,
  cwd,
  env: sourceEnv,
  mkdir,
}) {
  const args = parseArgs(argv);
  const commands = buildCtxHttpSuiteCommands(args.suites);
  const isBatchSelection = args.suites.length > 1 || args.suites.includes("all");
  if (args.list) {
    return {
      args,
      commands,
      env: null,
      isBatchSelection,
    };
  }
  const { env } = buildCtxCacheEnv({
    cwd,
    env: sourceEnv,
    mode: "workspace",
    mkdir,
  });
  if (!String(env.RUST_TEST_THREADS ?? "").trim()) {
    env.RUST_TEST_THREADS = "1";
  }
  return {
    args,
    commands,
    env,
    isBatchSelection,
  };
}

function main() {
  const coreRoot = path.resolve(__dirname, "..");
  const plan = buildTaskPlan({
    argv: process.argv.slice(2),
    cwd: coreRoot,
    env: process.env,
    mkdir: true,
  });
  if (plan.args.list) {
    for (const command of plan.commands) {
      console.log([command.command, ...command.args].join(" "));
    }
    return;
  }
  if (plan.isBatchSelection) {
    process.stderr.write(`CTX_HTTP_SUITE_BATCH ${JSON.stringify({ suites: plan.args.suites })}\n`);
  }

  for (const command of plan.commands) {
    run(command.command, command.args, {
      cwd: coreRoot,
      env: plan.env,
    });
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  buildTaskPlan,
  parseArgs,
};
