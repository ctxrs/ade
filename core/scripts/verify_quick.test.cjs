const test = require("node:test");
const assert = require("node:assert/strict");

const { applyVerifyQuickDefaults } = require("./verify_quick.cjs");

test("applyVerifyQuickDefaults enables linux RBE defaults on darwin", () => {
  const env = {};
  applyVerifyQuickDefaults(env, "darwin");

  assert.deepEqual(env, {
    CARGO_INCREMENTAL: "0",
    RUST_TEST_THREADS: "1",
    CTX_BAZEL_REMOTE_EXECUTION: "linux",
    CTX_BAZEL_BATCH: "0",
    CTX_BAZEL_JOBS: "1",
  });
});

test("applyVerifyQuickDefaults enables linux remote Bazel defaults without darwin batch overrides", () => {
  const env = {};
  applyVerifyQuickDefaults(env, "linux");

  assert.deepEqual(env, {
    CARGO_INCREMENTAL: "0",
    RUST_TEST_THREADS: "1",
    CTX_BAZEL_REMOTE_EXECUTION: "linux",
  });
});

test("applyVerifyQuickDefaults preserves explicit overrides", () => {
  const env = {
    CARGO_INCREMENTAL: "1",
    RUST_TEST_THREADS: "8",
    CTX_BAZEL_REMOTE_EXECUTION: "all",
    CTX_BAZEL_BATCH: "1",
    CTX_BAZEL_JOBS: "4",
  };
  applyVerifyQuickDefaults(env, "darwin");

  assert.deepEqual(env, {
    CARGO_INCREMENTAL: "1",
    RUST_TEST_THREADS: "8",
    CTX_BAZEL_REMOTE_EXECUTION: "all",
    CTX_BAZEL_BATCH: "1",
    CTX_BAZEL_JOBS: "4",
  });
});
