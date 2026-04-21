const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
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
  parseBatchMode,
  parsePositiveIntegerEnv,
  parseRemoteExecutionMode,
  resolvePhaseBudgetKey,
  resolveBazeliskCommand,
  runBazelPilotInvocationPhases,
} = require("./run_bazel_pilot.cjs");
const { HOST_HEAVY_BUDGET_KEY } = require("./lib/host_job_budget.cjs");

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

test("bazel pilot run separates Bazel targets from post-run args", () => {
  assert.deepEqual(parseArgs(["run", "//core/apps/web:lint", "--", "--fix"]), {
    command: "run",
    targets: ["//core/apps/web:lint"],
    runArgs: ["--fix"],
  });
});

test("bazel pilot invocation stays on the volatile cache layout", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test"],
    env: {
      ...process.env,
      CTX_BAZEL_REMOTE_EXECUTION: "off",
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
  assert.equal(invocation.phases[0].commandArgs.some((entry) => entry.startsWith("--profile=")), false);
  assert.equal(invocation.phases[0].commandArgs.some((entry) => entry.startsWith("--build_event_json_file=")), false);
  assert.equal(invocation.phases[0].commandArgs.some((entry) => entry.startsWith("--invocation_id=")), false);
});

test("bazel pilot batch mode defaults on darwin and can be overridden explicitly", () => {
  assert.equal(parseBatchMode(undefined, { platform: "darwin" }), true);
  assert.equal(parseBatchMode(undefined, { platform: "linux" }), false);
  assert.equal(parseBatchMode("1", { platform: "linux" }), true);
  assert.equal(parseBatchMode("0", { platform: "darwin" }), false);
});

test("bazel pilot remote execution mode parsing supports cache, off, all, linux, and darwin", () => {
  const defaultMode =
    process.platform === "darwin" ? "cache" : process.platform === "linux" ? "linux" : "off";
  assert.equal(parseRemoteExecutionMode(undefined), defaultMode);
  assert.equal(parseRemoteExecutionMode(""), defaultMode);
  assert.equal(parseRemoteExecutionMode("cache"), "cache");
  assert.equal(parseRemoteExecutionMode("off"), "off");
  assert.equal(parseRemoteExecutionMode("false"), "off");
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

test("bazel pilot auth args accept the documented BuildBuddy API key env name", () => {
  assert.deepEqual(buildBuildBuddyAuthArgs({ BUILD_BUDDY_API_KEY: "api-key-456" }), [
    "--remote_header=x-buildbuddy-api-key=api-key-456",
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
      BUILD_BUDDY_API_KEY: "buildbuddy-ci-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "all");
  assert.equal(invocation.buildBuddyEnabled, true);
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "remote");
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-cache"), true);
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-rbe"), true);
  assert.equal(
    invocation.phases[0].commandArgs.includes("--remote_header=x-buildbuddy-api-key=buildbuddy-ci-key"),
    true,
  );
  assert.equal(
    invocation.phases[0].commandArgs.some((entry) => entry.startsWith("--invocation_id=")),
    true,
  );
  assert.equal(typeof invocation.phases[0].buildBuddy?.invocationUrl, "string");
});

test("bazel pilot forwards local test job caps for test invocations", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/crates/ctx-http:bin_tests"],
    env: {
      ...process.env,
      CTX_BAZEL_REMOTE_EXECUTION: "off",
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-local-test-jobs",
      CTX_SESSION_ID: "bazel-local-test-jobs-session",
      CTX_BAZEL_LOCAL_TEST_JOBS: "3",
    },
  });

  assert.equal(invocation.commandArgs.includes("--local_test_jobs=3"), true);
  assert.equal(invocation.phases[0].commandArgs.includes("--local_test_jobs=3"), true);
});

test("bazel pilot marks only local phases as host-budgeted work", () => {
  const invocation = buildBazelPilotInvocation({
    argv: [
      "build",
      "//core/crates/ctx-provider-accounts:lib",
      "//core/crates/ctx-lsp:ctx-lsp-test-server",
    ],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-budgeted",
      CTX_SESSION_ID: "bazel-budget-session",
      CTX_BAZEL_REMOTE_EXECUTION: "linux",
      BUILD_BUDDY_API_KEY: "buildbuddy-linux-key",
    },
  });

  assert.equal(resolvePhaseBudgetKey(invocation.phases[0]), null);
  assert.equal(resolvePhaseBudgetKey(invocation.phases[1]), HOST_HEAVY_BUDGET_KEY);
});

test("bazel pilot invocation runner applies the host-heavy budget to local phases", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/crates/ctx-http:provider-auth"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-runner-budget",
      CTX_SESSION_ID: "bazel-runner-budget-session",
      CTX_BAZEL_REMOTE_EXECUTION: "off",
    },
  });
  const budgetCalls = [];
  const spawnCalls = [];

  runBazelPilotInvocationPhases(invocation, {
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

test("bazel pilot runner writes telemetry summaries and redacts BuildBuddy headers", () => {
  const volatileRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazel-pilot-telemetry-"));
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/crates/ctx-http:provider-auth"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_SESSION_ID: "bazel-telemetry-session",
      CTX_BAZEL_REMOTE_EXECUTION: "1",
      BUILD_BUDDY_API_KEY: "buildbuddy-redact-me",
      CTX_VERIFY_PARENT_ENTRYPOINT: "verify:affected",
      CTX_VERIFY_PARENT_RUN_ID: "router-run-1",
    },
  });

  runBazelPilotInvocationPhases(invocation, {
    spawnSyncImpl: () => ({ status: 0 }),
    withHostJobBudgetImpl: (_options, fn) => fn(),
  });

  const summary = JSON.parse(fs.readFileSync(invocation.telemetry.summaryPath, "utf8"));
  assert.equal(summary.kind, "bazel");
  assert.equal(summary.success, true);
  assert.equal(summary.remoteExecutionMode, "all");
  assert.equal(summary.buildBuddyEnabled, true);
  assert.equal(summary.parentEntrypoint, "verify:affected");
  assert.equal(summary.parentRunId, "router-run-1");
  assert.equal(summary.phases.length, 1);
  assert.equal(summary.buildBuddyInvocations.length, 1);
  assert.equal(summary.buildBuddyInvocations[0].phaseName, "remote");
  assert.match(summary.buildBuddyInvocations[0].invocationUrl, /https:\/\/app\.buildbuddy\.io\/invocation\//u);
  assert.equal(
    summary.phases[0].commandArgs.includes("--remote_header=x-buildbuddy-api-key=<redacted>"),
    true,
  );
  assert.match(summary.phases[0].buildBuddyInvocationUrl, /https:\/\/app\.buildbuddy\.io\/invocation\//u);
  assert.equal(fs.existsSync(invocation.telemetry.hostSamplesPath), true);
});

test("bazel pilot does not record dead BuildBuddy links when Bazel never starts", () => {
  const volatileRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazel-pilot-spawn-failure-"));
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/crates/ctx-http:provider-auth"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_SESSION_ID: "bazel-spawn-failure-session",
      CTX_BAZEL_REMOTE_EXECUTION: "1",
      BUILD_BUDDY_API_KEY: "buildbuddy-key",
    },
  });
  const enoent = new Error("spawnSync bazelisk ENOENT");
  enoent.code = "ENOENT";

  runBazelPilotInvocationPhases(invocation, {
    exitImpl: () => {},
    logErrorImpl: () => {},
    spawnSyncImpl: () => ({ error: enoent }),
    withHostJobBudgetImpl: (_options, fn) => fn(),
  });

  const summary = JSON.parse(fs.readFileSync(invocation.telemetry.summaryPath, "utf8"));
  assert.deepEqual(summary.buildBuddyInvocations, []);
  assert.equal("buildBuddyInvocationUrl" in summary.phases[0], false);
});

test("bazel pilot includes generated run-script time in recorded phase durations", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["run", "//core/apps/web:lint", "--", "--fix"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazel-pilot-run-duration-")),
      CTX_SESSION_ID: "bazel-run-duration-session",
      CTX_BAZEL_REMOTE_EXECUTION: "off",
    },
  });

  const durations = [0, 500, 1000, 1750];
  const originalNow = Date.now;
  Date.now = () => durations.shift();

  try {
    runBazelPilotInvocationPhases(invocation, {
      spawnSyncImpl: (_command, args) => {
        if (args.some((entry) => String(entry).startsWith("--script_path="))) {
          return { status: 0 };
        }
        return { status: 0 };
      },
      withHostJobBudgetImpl: (_options, fn) => fn(),
    });
  } finally {
    Date.now = originalNow;
  }

  const summary = JSON.parse(fs.readFileSync(invocation.telemetry.summaryPath, "utf8"));
  assert.equal(summary.phases.length, 1);
  assert.equal(summary.phases[0].runScriptDurationMs, 750);
  assert.equal(summary.phases[0].durationMs, 1250);
});

test("bazel pilot duration includes host-budget queue time for local phases", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/crates/ctx-http:provider-auth"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazel-pilot-budget-wait-")),
      CTX_SESSION_ID: "bazel-budget-wait-session",
      CTX_BAZEL_REMOTE_EXECUTION: "off",
    },
  });

  runBazelPilotInvocationPhases(invocation, {
    spawnSyncImpl: () => ({ status: 0 }),
    withHostJobBudgetImpl: (_options, fn) => {
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50);
      return fn();
    },
  });

  const summary = JSON.parse(fs.readFileSync(invocation.telemetry.summaryPath, "utf8"));
  assert.equal(summary.phases.length, 1);
  assert.ok(summary.phases[0].durationMs >= 50);
});

test("bazel pilot still writes a thin summary when host-budget acquisition throws", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/crates/ctx-http:provider-auth"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazel-pilot-budget-throw-")),
      CTX_SESSION_ID: "bazel-budget-throw-session",
      CTX_BAZEL_REMOTE_EXECUTION: "off",
    },
  });

  runBazelPilotInvocationPhases(invocation, {
    exitImpl: () => {},
    logErrorImpl: () => {},
    withHostJobBudgetImpl: () => {
      throw new Error("budget unavailable");
    },
  });

  const summary = JSON.parse(fs.readFileSync(invocation.telemetry.summaryPath, "utf8"));
  assert.equal(summary.success, false);
  assert.equal(summary.phases.length, 1);
  assert.equal(summary.phases[0].command, "budget:host-heavy");
  assert.equal(summary.phases[0].status, 1);
  assert.match(summary.phases[0].error, /budget unavailable/u);
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
      BUILD_BUDDY_API_KEY: "buildbuddy-linux-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "linux");
  assert.deepEqual(
    invocation.phases.map((phase) => ({
      name: phase.name,
      targets: phase.targets,
      hasBuildBuddyCache: phase.commandArgs.includes("--config=buildbuddy-cache"),
      hasLinuxConfig: phase.commandArgs.includes("--config=buildbuddy-linux-rbe"),
      hasBuildBuddyHeader: phase.commandArgs.includes(
        "--remote_header=x-buildbuddy-api-key=buildbuddy-linux-key",
      ),
    })),
    [
      {
        name: "linux-rbe",
        targets: ["//core/crates/ctx-provider-accounts:lib"],
        hasBuildBuddyCache: true,
        hasLinuxConfig: true,
        hasBuildBuddyHeader: true,
      },
      {
        name: "local",
        targets: ["//core/crates/ctx-lsp:ctx-lsp-test-server"],
        hasBuildBuddyCache: true,
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
      BUILD_BUDDY_API_KEY: "buildbuddy-linux-run-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "linux");
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "local");
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-cache"), true);
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
      BUILD_BUDDY_API_KEY: "buildbuddy-darwin-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "darwin");
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "darwin-rbe");
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-cache"), true);
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-darwin-rbe"), true);
  assert.equal(
    invocation.phases[0].commandArgs.includes("--remote_header=x-buildbuddy-api-key=buildbuddy-darwin-key"),
    true,
  );
});

test("bazel pilot defaults to cache-only on darwin and linux RBE on linux", () => {
  const expectedMode =
    process.platform === "darwin" ? "cache" : process.platform === "linux" ? "linux" : "off";
  const invocation = buildBazelPilotInvocation({
    argv: ["build", "//core/crates/ctx-core:lib"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-default-rbe",
      CTX_SESSION_ID: "bazel-default-rbe-session",
      BUILD_BUDDY_API_KEY: "buildbuddy-default-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, expectedMode);
  if (expectedMode === "off") {
    assert.equal(invocation.phases[0].name, "local");
    return;
  }
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-cache"), true);
  if (expectedMode === "cache") {
    assert.equal(invocation.phases[0].name, "local");
    assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-darwin-rbe"), false);
    assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-linux-rbe"), false);
    return;
  }
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-linux-rbe"), true);
});

test("bazel pilot cache mode keeps execution local while preserving BuildBuddy cache auth", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["build", "//core/crates/ctx-core:lib"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-cache-mode",
      CTX_SESSION_ID: "bazel-cache-mode-session",
      CTX_BAZEL_REMOTE_EXECUTION: "cache",
      BUILD_BUDDY_API_KEY: "buildbuddy-cache-key",
    },
  });

  assert.equal(invocation.remoteExecutionMode, "cache");
  assert.equal(invocation.phases.length, 1);
  assert.equal(invocation.phases[0].name, "local");
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-cache"), true);
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-darwin-rbe"), false);
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-linux-rbe"), false);
  assert.equal(
    invocation.phases[0].commandArgs.includes("--remote_header=x-buildbuddy-api-key=buildbuddy-cache-key"),
    true,
  );
});

test("bazel pilot fails with an actionable error when BuildBuddy is enabled but the API key is unavailable", () => {
  assert.throws(
    () =>
      buildBazelPilotInvocation({
        argv: ["build", "//core/crates/ctx-core:lib"],
        env: {
          ...process.env,
          CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-missing-key",
          CTX_SESSION_ID: "bazel-missing-key-session",
          CTX_BAZEL_REMOTE_EXECUTION: "darwin",
        },
        buildBuddyApiKeyResolver: () => {
          throw new Error("infisical not logged in");
        },
      }),
    /CTX_BAZEL_REMOTE_EXECUTION=off/,
  );
});

test("bazel pilot falls back to the repo-managed bazelisk shim when no direct pnpm script exists", () => {
  const repoRoot = "/tmp/ctx-monorepo";
  const expected = path.join(repoRoot, "core", "node_modules", ".bin", process.platform === "win32" ? "bazelisk.cmd" : "bazelisk");
  assert.equal(
    resolveBazeliskCommand({
      repoRoot,
      fileExists: (candidate) => candidate === expected,
      pathRunnable: (candidate) => candidate === expected,
      readDir: () => [],
      commandAvailable: () => false,
    }),
    expected,
  );
});

test("bazel pilot prefers the direct pnpm Bazelisk script over the repo shim when both exist", () => {
  const repoRoot = "/tmp/ctx-monorepo";
  const repoShim = path.join(repoRoot, "core", "node_modules", ".bin", process.platform === "win32" ? "bazelisk.cmd" : "bazelisk");
  const expectedScript = path.join(
    repoRoot,
    "core",
    "node_modules",
    ".pnpm",
    "@bazel+bazelisk@1.28.1",
    "node_modules",
    "@bazel",
    "bazelisk",
    "bazelisk.js",
  );
  const existingPaths = new Set([
    repoShim,
    path.join(repoRoot, "core", "node_modules", ".pnpm"),
    expectedScript,
  ]);
  assert.equal(
    resolveBazeliskCommand({
      repoRoot,
      fileExists: (candidate) => existingPaths.has(candidate),
      pathRunnable: (candidate) => candidate === expectedScript || candidate === repoShim,
      readDir: () => ["@bazel+bazelisk@1.28.1"],
      commandAvailable: () => false,
    }),
    expectedScript,
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
      CTX_BAZEL_REMOTE_EXECUTION: "off",
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-binary",
      CTX_SESSION_ID: "bazel-binary-session",
    },
  });

  assert.equal(
    spawn.command,
    bazeliskBinaryPath({ repoRoot: path.resolve(__dirname, "..", "..") }),
  );
  assert.deepEqual(
    spawn.args.filter((entry) => !entry.startsWith("--invocation_id=")),
    [
      ...(process.platform === "darwin" ? ["--batch"] : []),
      "--output_user_root=/tmp/ctx-bazel-pilot-binary/targets/bazel/bazel-binary-session",
      "run",
      "--disk_cache=/tmp/ctx-bazel-pilot-binary/cache/bazel-disk/ctx-monorepo",
      "--repository_cache=/tmp/ctx-bazel-pilot-binary/cache/bazel-repository/ctx-monorepo",
      `--script_path=${spawn.runScriptPath}`,
      "//core/apps/web:lint",
    ],
  );
  assert.equal(spawn.args.some((entry) => entry.startsWith("--invocation_id=")), false);
  assert.equal(spawn.options.env.BUILD_WORKSPACE_DIRECTORY, path.resolve(__dirname, "..", ".."));
  assert.match(spawn.runScriptPath, /\/tmp\/ctx-bazel-pilot-binary\/tmp\/bazel-run-\d+-local\.sh$/);
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
