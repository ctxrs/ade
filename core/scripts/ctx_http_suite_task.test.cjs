const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(__dirname, "ctx_http_suite_task.cjs");

test("ctx-http suite task lists sequential Bazel commands for multi-suite batches", () => {
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

test("ctx-http suite task lists the all meta-suite as sequential Bazel commands", () => {
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

test("ctx-http suite task executes batched selections sequentially without widening local test fanout", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-http-suite-task-"));
  const capturePath = path.join(tempDir, "capture.jsonl");
  const fakeNodePath = path.join(tempDir, "node");
  fs.writeFileSync(
    fakeNodePath,
    [
      `#!${process.execPath}`,
      "const fs = require('node:fs');",
      `const capturePath = ${JSON.stringify(capturePath)};`,
      "fs.appendFileSync(capturePath, JSON.stringify({",
      "  args: process.argv.slice(2),",
      "  bazelJobs: process.env.CTX_BAZEL_JOBS || '',",
      "  localTestJobs: process.env.CTX_BAZEL_LOCAL_TEST_JOBS || '',",
      "  rustTestThreads: process.env.RUST_TEST_THREADS || '',",
      "}) + '\\n');",
    ].join("\n"),
    { mode: 0o755 },
  );

  const result = spawnSync(process.execPath, [scriptPath, "--suite", "base", "--suite", "provider-auth"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${tempDir}:${process.env.PATH}`,
    },
    stdio: "pipe",
  });

  assert.equal(result.status, 0);
  assert.match(result.stderr, /CTX_HTTP_SUITE_BATCH/u);

  const calls = fs.readFileSync(capturePath, "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  assert.deepEqual(calls, [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:base"],
      bazelJobs: "2",
      localTestJobs: "",
      rustTestThreads: "1",
    },
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:provider-auth"],
      bazelJobs: "2",
      localTestJobs: "",
      rustTestThreads: "1",
    },
  ]);
});

test("ctx-http suite task preserves an explicit Bazel job cap", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-http-suite-task-"));
  const capturePath = path.join(tempDir, "capture.jsonl");
  const fakeNodePath = path.join(tempDir, "node");
  fs.writeFileSync(
    fakeNodePath,
    [
      `#!${process.execPath}`,
      "const fs = require('node:fs');",
      `const capturePath = ${JSON.stringify(capturePath)};`,
      "fs.appendFileSync(capturePath, JSON.stringify({",
      "  args: process.argv.slice(2),",
      "  bazelJobs: process.env.CTX_BAZEL_JOBS || '',",
      "}) + '\\n');",
    ].join("\n"),
    { mode: 0o755 },
  );

  const result = spawnSync(process.execPath, [scriptPath, "--suite", "base"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      CTX_BAZEL_JOBS: "3",
      PATH: `${tempDir}:${process.env.PATH}`,
    },
    stdio: "pipe",
  });

  assert.equal(result.status, 0);
  const calls = fs.readFileSync(capturePath, "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  assert.deepEqual(calls, [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", "//core/crates/ctx-http:base"],
      bazelJobs: "3",
    },
  ]);
});
