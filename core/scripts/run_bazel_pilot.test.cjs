const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  DEFAULT_BUILD_TARGETS,
  DEFAULT_TEST_TARGETS,
  buildBazelPilotInvocation,
  parseArgs,
  parseRemoteExecutionMode,
} = require("./run_bazel_pilot.cjs");

test("bazel pilot defaults to the expanded Rust slice test targets", () => {
  assert.deepEqual(parseArgs([]), {
    command: "test",
    targets: DEFAULT_TEST_TARGETS,
  });
});

test("bazel pilot build defaults to the expanded Rust slice library targets", () => {
  assert.deepEqual(parseArgs(["build"]), {
    command: "build",
    targets: DEFAULT_BUILD_TARGETS,
  });
});

test("bazel pilot run requires explicit targets", () => {
  assert.deepEqual(parseArgs(["run"]), {
    command: "run",
    targets: [],
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
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "local");
});

test("bazel pilot remote execution mode parsing supports off, all, and linux", () => {
  assert.equal(parseRemoteExecutionMode(undefined), "off");
  assert.equal(parseRemoteExecutionMode(""), "off");
  assert.equal(parseRemoteExecutionMode("1"), "all");
  assert.equal(parseRemoteExecutionMode("true"), "all");
  assert.equal(parseRemoteExecutionMode("linux"), "linux");
});

test("bazel pilot invocation enables full BuildBuddy remote execution when requested", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["build"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-rbe",
      CTX_SESSION_ID: "bazel-rbe-session",
      CTX_BAZEL_REMOTE_EXECUTION: "1",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "all");
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "remote");
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-rbe"), true);
});

test("bazel pilot linux remote execution keeps lib builds remote and host executables local", () => {
  const invocation = buildBazelPilotInvocation({
    argv: [
      "build",
      "//core/crates/ctx-provider-accounts:lib",
      "//core/crates/ctx-lsp:ctx-lsp-test-server",
    ],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-linux-rbe",
      CTX_SESSION_ID: "bazel-linux-rbe-session",
      CTX_BAZEL_REMOTE_EXECUTION: "linux",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "linux");
  assert.deepEqual(
    invocation.phases.map((phase) => ({
      name: phase.name,
      targets: phase.targets,
      hasLinuxConfig: phase.commandArgs.includes("--config=buildbuddy-linux-rbe"),
    })),
    [
      {
        name: "linux-rbe",
        targets: ["//core/crates/ctx-provider-accounts:lib"],
        hasLinuxConfig: true,
      },
      {
        name: "local",
        targets: ["//core/crates/ctx-lsp:ctx-lsp-test-server"],
        hasLinuxConfig: false,
      },
    ],
  );
});

test("bazel pilot keeps run targets local even in linux remote execution mode", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["run", "//core/apps/web:unit_tests"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-linux-rbe-run",
      CTX_SESSION_ID: "bazel-linux-rbe-run-session",
      CTX_BAZEL_REMOTE_EXECUTION: "linux",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "linux");
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "local");
  assert.equal(
    invocation.phases[0].commandArgs.includes("--config=buildbuddy-linux-rbe"),
    false,
  );
});
