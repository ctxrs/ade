const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  DEFAULT_BUILD_TARGETS,
  DEFAULT_TEST_TARGETS,
  bazeliskBinaryPath,
  commandExists,
  buildBuildBuddyAuthArgs,
  buildBazelPilotSpawn,
  buildBazelPilotInvocation,
  formatSpawnFailureMessage,
  parseArgs,
  parseRemoteExecutionMode,
  resolveBazeliskCommand,
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

test("bazel pilot remote execution mode parsing supports off, all, linux, and darwin", () => {
  assert.equal(parseRemoteExecutionMode(undefined), "off");
  assert.equal(parseRemoteExecutionMode(""), "off");
  assert.equal(parseRemoteExecutionMode("1"), "all");
  assert.equal(parseRemoteExecutionMode("true"), "all");
  assert.equal(parseRemoteExecutionMode("linux"), "linux");
  assert.equal(parseRemoteExecutionMode("darwin"), "darwin");
  assert.equal(parseRemoteExecutionMode("macos"), "darwin");
});

test("bazel pilot auth args stay empty without a BuildBuddy API key", () => {
  assert.deepEqual(buildBuildBuddyAuthArgs({}), []);
});

test("bazel pilot auth args forward the BuildBuddy API key as a remote header", () => {
  assert.deepEqual(buildBuildBuddyAuthArgs({ BUILDBUDDY_API_KEY: "api-key-123" }), [
    "--remote_header=x-buildbuddy-api-key=api-key-123",
  ]);
});

test("bazel pilot invocation enables full BuildBuddy remote execution when requested", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["build"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-rbe",
      CTX_SESSION_ID: "bazel-rbe-session",
      CTX_BAZEL_REMOTE_EXECUTION: "1",
      BUILDBUDDY_API_KEY: "buildbuddy-ci-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "all");
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "remote");
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-rbe"), true);
  assert.equal(
    invocation.phases[0].commandArgs.includes("--remote_header=x-buildbuddy-api-key=buildbuddy-ci-key"),
    true,
  );
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
      BUILDBUDDY_API_KEY: "buildbuddy-linux-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "linux");
  assert.deepEqual(
    invocation.phases.map((phase) => ({
      name: phase.name,
      targets: phase.targets,
      hasLinuxConfig: phase.commandArgs.includes("--config=buildbuddy-linux-rbe"),
      hasBuildBuddyHeader: phase.commandArgs.includes(
        "--remote_header=x-buildbuddy-api-key=buildbuddy-linux-key",
      ),
    })),
    [
      {
        name: "linux-rbe",
        targets: ["//core/crates/ctx-provider-accounts:lib"],
        hasLinuxConfig: true,
        hasBuildBuddyHeader: true,
      },
      {
        name: "local",
        targets: ["//core/crates/ctx-lsp:ctx-lsp-test-server"],
        hasLinuxConfig: false,
        hasBuildBuddyHeader: true,
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

test("bazel pilot darwin remote execution uses the darwin BuildBuddy config directly", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["build", "//core/crates/ctx-core:lib"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-darwin-rbe",
      CTX_SESSION_ID: "bazel-darwin-rbe-session",
      CTX_BAZEL_REMOTE_EXECUTION: "darwin",
      BUILDBUDDY_API_KEY: "buildbuddy-darwin-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "darwin");
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "darwin-rbe");
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-darwin-rbe"), true);
  assert.equal(
    invocation.phases[0].commandArgs.includes("--remote_header=x-buildbuddy-api-key=buildbuddy-darwin-key"),
    true,
  );
});

test("bazel pilot prefers the repo-managed bazelisk shim when it exists", () => {
  const repoRoot = "/tmp/ctx-monorepo";
  const expected = path.join(repoRoot, "core", "node_modules", ".bin", process.platform === "win32" ? "bazelisk.cmd" : "bazelisk");
  assert.equal(
    resolveBazeliskCommand({
      repoRoot,
      fileExists: (candidate) => candidate === expected,
      commandAvailable: () => false,
    }),
    expected,
  );
});

test("bazel pilot falls back to PATH bazelisk when the repo shim is absent", () => {
  assert.equal(
    resolveBazeliskCommand({
      repoRoot: "/tmp/ctx-monorepo",
      fileExists: () => false,
      commandAvailable: (candidate) => candidate === (process.platform === "win32" ? "bazelisk.cmd" : "bazelisk"),
    }),
    process.platform === "win32" ? "bazelisk.cmd" : "bazelisk",
  );
});

test("bazel pilot uses the resolved Bazelisk command in the spawn contract", () => {
  const spawn = buildBazelPilotSpawn({
    argv: ["run", "//core/apps/web:lint"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-binary",
      CTX_SESSION_ID: "bazel-binary-session",
    },
  });

  assert.equal(
    spawn.command,
    bazeliskBinaryPath({ repoRoot: path.resolve(__dirname, "..", "..") }),
  );
  assert.deepEqual(spawn.args, [
    "--output_user_root=/tmp/ctx-bazel-pilot-binary/targets/bazel/bazel-binary-session",
    "run",
    "--disk_cache=/tmp/ctx-bazel-pilot-binary/cache/bazel-disk/ctx-monorepo",
    "--repository_cache=/tmp/ctx-bazel-pilot-binary/cache/bazel-repository/ctx-monorepo",
    "//core/apps/web:lint",
  ]);
  assert.equal(spawn.options.env.BUILD_WORKSPACE_DIRECTORY, path.resolve(__dirname, "..", ".."));
});

test("bazel pilot missing shim failure tells the operator how to install it", () => {
  const error = new Error("spawnSync bazelisk ENOENT");
  error.code = "ENOENT";

  const message = formatSpawnFailureMessage(bazeliskBinaryPath(), error);

  assert.match(message, /Bazelisk is not installed/);
  assert.match(message, /pnpm -C core install --frozen-lockfile/);
});

test("bazel pilot commandExists returns false for missing commands", () => {
  assert.equal(commandExists("definitely-not-a-real-bazel-command-ctx"), false);
});
