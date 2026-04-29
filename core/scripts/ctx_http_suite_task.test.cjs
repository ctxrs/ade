const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");
const { spawnSync } = require("node:child_process");
const { buildTaskPlan } = require("./ctx_http_suite_task.cjs");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(__dirname, "ctx_http_suite_task.cjs");

test("ctx-http suite task lists one Bazel command per suite for multi-suite batches", () => {
  const result = spawnSync("node", [scriptPath, "--list", "--suite", "base", "--suite", "provider-auth"], {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: "pipe",
  });

  assert.equal(result.status, 0);
  assert.equal(
    result.stdout.trim(),
    [
      "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:base",
      "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:provider-auth",
    ].join("\n"),
  );
});

test("ctx-http suite task rejects a trailing --suite without a value", () => {
  const result = spawnSync("node", [scriptPath, "--suite", "base", "--suite"], {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: "pipe",
  });

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /--suite requires a suite name/u);
});

test("ctx-http suite task lists the all meta-suite as suite-scoped Bazel commands", () => {
  const result = spawnSync("node", [scriptPath, "--list", "--suite", "all"], {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: "pipe",
  });

  assert.equal(result.status, 0);
  const stdoutLines = result.stdout.trim().split("\n");
  assert.equal(stdoutLines.length > 2, true);
  assert.equal(stdoutLines[0], "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:base");
  assert.equal(
    stdoutLines.includes("node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:workspace-stream"),
    true,
  );
});

test("ctx-http suite task executes batched selections per suite without forcing a global job cap", () => {
  const plan = buildTaskPlan({
    argv: ["--suite", "base", "--suite", "provider-auth"],
    cwd: repoRoot,
    env: {
      PATH: process.env.PATH ?? "",
    },
    mkdir: false,
  });

  assert.equal(plan.isBatchSelection, true);
  assert.deepEqual(plan.commands, [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:base"],
      command: "node",
    },
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:provider-auth"],
      command: "node",
    },
  ]);
  assert.equal(String(plan.env.CTX_BAZEL_JOBS ?? ""), "");
  assert.equal(plan.env.CTX_BAZEL_LOCAL_TEST_JOBS, "1");
  assert.equal(plan.env.RUST_TEST_THREADS, "1");
});

test("ctx-http suite task preserves explicit Bazel job caps", () => {
  const plan = buildTaskPlan({
    argv: ["--suite", "base"],
    cwd: repoRoot,
    env: {
      CTX_BAZEL_JOBS: "3",
      CTX_BAZEL_LOCAL_TEST_JOBS: "2",
      PATH: process.env.PATH ?? "",
    },
    mkdir: false,
  });

  assert.deepEqual(plan.commands, [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:base"],
      command: "node",
    },
  ]);
  assert.equal(plan.isBatchSelection, false);
  assert.equal(plan.env.CTX_BAZEL_JOBS, "3");
  assert.equal(plan.env.CTX_BAZEL_LOCAL_TEST_JOBS, "2");
  assert.equal(plan.env.RUST_TEST_THREADS, "1");
});
