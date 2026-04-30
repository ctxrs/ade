const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");
const { spawnSync } = require("node:child_process");
const { buildTaskPlan } = require("./ctx_http_suite_task.cjs");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(__dirname, "ctx_http_suite_task.cjs");
const PARENT_BAZEL_ENV_KEYS = [
  "CTX_BAZEL_JOBS",
  "CTX_BAZEL_LOCAL_TEST_JOBS",
  "RUST_TEST_THREADS",
];

for (const key of PARENT_BAZEL_ENV_KEYS) {
  delete process.env[key];
}

test("ctx-http suite task lists one batched Bazel command for multi-suite batches", () => {
  const result = spawnSync("node", [scriptPath, "--list", "--suite", "base", "--suite", "provider-auth"], {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: "pipe",
  });

  assert.equal(result.status, 0);
  assert.equal(
    result.stdout.trim(),
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:base //core/crates/ctx-http:provider-auth",
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

test("ctx-http suite task lists the all meta-suite as one batched Bazel command", () => {
  const result = spawnSync("node", [scriptPath, "--list", "--suite", "all"], {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: "pipe",
  });

  assert.equal(result.status, 0);
  const stdoutLines = result.stdout.trim().split("\n");
  assert.equal(stdoutLines.length, 1);
  assert.equal(stdoutLines[0].includes("//core/crates/ctx-http:base"), false);
  assert.equal(stdoutLines[0].includes("//core/crates/ctx-http:unit-tests-api"), true);
  assert.equal(stdoutLines[0].includes("//core/crates/ctx-http:bin_tests"), true);
  assert.equal(stdoutLines[0].includes("//core/crates/ctx-http:doc_tests"), true);
  assert.equal(
    stdoutLines[0].includes("//core/crates/ctx-http:workspace_active_snapshot_http"),
    true,
  );
  assert.equal(stdoutLines[0].includes("//core/crates/ctx-http:workspace-stream"), false);
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
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:base", "//core/crates/ctx-http:provider-auth"],
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
