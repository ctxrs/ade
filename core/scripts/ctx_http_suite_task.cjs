#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const {
  buildCtxHttpSuiteCommands,
  getCtxHttpSuiteNames,
} = require("./lib/ctx_http_suites.cjs");

const DEFAULT_CTX_HTTP_LOCAL_TEST_JOBS = "1";
const DEFAULT_CTX_HTTP_BAZEL_JOBS = "2";
const CTX_HTTP_SINGLE_ACTION_BAZEL_JOB_SUITES = new Set([
  "all",
  "base",
  "unit-tests-api",
  "unit-tests-daemon-and-scheduler",
  "unit-tests-lib",
  "unit-tests-lib-session-head-large",
  "unit-tests-merge-queue",
  "unit-tests-provider-and-settings",
  "unit-tests-workspace-runtime",
]);

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

function defaultBazelJobsForSuites(suites) {
  return suites.some((suite) => CTX_HTTP_SINGLE_ACTION_BAZEL_JOB_SUITES.has(suite))
    ? "1"
    : DEFAULT_CTX_HTTP_BAZEL_JOBS;
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
  if (!String(env.CTX_BAZEL_LOCAL_TEST_JOBS ?? "").trim()) {
    env.CTX_BAZEL_LOCAL_TEST_JOBS = DEFAULT_CTX_HTTP_LOCAL_TEST_JOBS;
  }
  if (!String(env.CTX_BAZEL_JOBS ?? "").trim()) {
    env.CTX_BAZEL_JOBS = defaultBazelJobsForSuites(args.suites);
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
  defaultBazelJobsForSuites,
  parseArgs,
};
