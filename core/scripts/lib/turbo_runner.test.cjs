const assert = require("node:assert/strict");
const test = require("node:test");

const { HOST_HEAVY_BUDGET_KEY } = require("./host_job_budget.cjs");
const { buildTurboRunArgs, runTurbo } = require("./turbo_runner.cjs");

test("buildTurboRunArgs keeps task ordering and includes cache settings", () => {
  const args = buildTurboRunArgs({
    env: {
      TURBO_CACHE_DIR: "/tmp/ctx-turbo-cache",
      TURBO_CACHE_MODE: "local:rw,remote:rw",
      CTX_VERIFY_TURBO_CONCURRENCY: "7",
    },
    taskNames: ["rust:crate:clippy:ctx-http", "rust:crate:test:ctx-http"],
    extraArgs: ["--filter=ctx-monorepo"],
  });

  assert.deepEqual(args, [
    "run",
    "rust:crate:clippy:ctx-http",
    "rust:crate:test:ctx-http",
    "--filter=ctx-monorepo",
    "--cache-dir=/tmp/ctx-turbo-cache",
    "--output-logs=errors-only",
    "--log-order=grouped",
    "--concurrency=7",
    "--ui=stream",
    "--cache=local:rw,remote:rw",
  ]);
});

test("buildTurboRunArgs lets an explicit concurrency override serialize a task batch", () => {
  const args = buildTurboRunArgs({
    env: {
      TURBO_CACHE_DIR: "/tmp/ctx-turbo-cache",
      CTX_VERIFY_TURBO_CONCURRENCY: "7",
    },
    taskNames: ["rust:ctx-http:test:all"],
    concurrencyOverride: 1,
  });

  assert.ok(args.includes("--concurrency=1"));
});

test("runTurbo executes under the shared host-heavy budget", () => {
  const budgetCalls = [];
  const spawnCalls = [];

  runTurbo({
    coreRoot: "/repo/core",
    env: {
      TURBO_CACHE_DIR: "/tmp/ctx-turbo-cache",
    },
    taskNames: ["rust:crate:test:ctx-http"],
    resolveTurboBinaryImpl: () => "/repo/core/node_modules/.bin/turbo",
    spawnSyncImpl: (command, args, options) => {
      spawnCalls.push({ command, args, options });
      return { status: 0 };
    },
    withHostJobBudgetImpl: (options, fn) => {
      budgetCalls.push(options);
      return fn();
    },
  });

  assert.equal(budgetCalls.length, 1);
  assert.equal(budgetCalls[0].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.equal(spawnCalls.length, 1);
});
