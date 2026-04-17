const assert = require("node:assert/strict");
const test = require("node:test");
const path = require("node:path");

const { applyTaskScopedEnv, resolveTaskCargoTargetDir } = require("./rust_crate_task.cjs");

test("resolveTaskCargoTargetDir keeps the shared target dir for non-clippy tasks", () => {
  const env = {
    CARGO_TARGET_DIR: "/tmp/ctx-target",
  };

  assert.equal(
    resolveTaskCargoTargetDir({ env, crate: "ctx-http", task: "test" }),
    "/tmp/ctx-target",
  );
  assert.equal(
    resolveTaskCargoTargetDir({ env, crate: "ctx-http", task: "nextest" }),
    "/tmp/ctx-target",
  );
});

test("resolveTaskCargoTargetDir isolates clippy by crate under the shared target root", () => {
  const env = {
    CARGO_TARGET_DIR: "/tmp/ctx-target",
  };

  assert.equal(
    resolveTaskCargoTargetDir({ env, crate: "ctx-http", task: "clippy" }),
    path.join("/tmp/ctx-target", "clippy", "ctx-http"),
  );
});

test("applyTaskScopedEnv rewrites only the task-local cargo target dir", () => {
  const env = {
    CARGO_TARGET_DIR: "/tmp/ctx-target",
    CTX_VERIFY_CARGO_TARGET_DIR: "/tmp/ctx-target",
  };

  applyTaskScopedEnv({ env, crate: "ctx-mcp", task: "clippy" });

  assert.deepEqual(env, {
    CARGO_TARGET_DIR: path.join("/tmp/ctx-target", "clippy", "ctx-mcp"),
    CTX_VERIFY_CARGO_TARGET_DIR: "/tmp/ctx-target",
  });
});
