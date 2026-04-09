const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  DEFAULT_TEST_TARGETS,
  buildBazelPilotInvocation,
  parseArgs,
} = require("./run_bazel_pilot.cjs");

test("bazel pilot defaults to the provider-auth slice test targets", () => {
  assert.deepEqual(parseArgs([]), {
    command: "test",
    targets: DEFAULT_TEST_TARGETS,
  });
});

test("bazel pilot invocation stays on the volatile cache layout", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot",
      CTX_SESSION_ID: "bazel-test-session",
    },
  });

  assert.equal(invocation.repoRoot, path.resolve(__dirname, "..", ".."));
  assert.equal(
    invocation.startupArgs.includes(
      "--output_user_root=/tmp/ctx-bazel-pilot/targets/bazel/bazel-test-session",
    ),
    true,
  );
  assert.equal(
    invocation.commandArgs.includes(
      "--disk_cache=/tmp/ctx-bazel-pilot/cache/bazel-disk/ctx-monorepo",
    ),
    true,
  );
  assert.equal(
    invocation.commandArgs.includes(
      "--repository_cache=/tmp/ctx-bazel-pilot/cache/bazel-repository/ctx-monorepo",
    ),
    true,
  );
  assert.equal(invocation.env.TMPDIR, "/tmp/ctx-bazel-pilot/tmp");
});
