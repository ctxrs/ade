const assert = require("node:assert/strict");
const test = require("node:test");

const { buildTurboRunArgs } = require("./turbo_runner.cjs");

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
