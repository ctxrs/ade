const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  DEFAULT_BUILD_TARGETS,
  DEFAULT_TEST_TARGETS,
  RUST_CLIPPY_BAZEL_ARGS,
  bazeliskBinaryPath,
  commandExists,
  buildBuildBuddyAuthArgs,
  buildBazelPilotSpawn,
  buildBazelPilotInvocation,
  buildBazelPilotSummary,
  formatBazelPilotSummaryLine,
  formatSpawnFailureMessage,
  parseArgs,
  parseBatchMode,
  parseBazelTestTimeoutSeconds,
  parsePositiveIntegerEnv,
  parseRemoteExecutionMode,
  resolveLocalTestJobs,
  resolveRemoteExecutionMode,
  resolvePhaseBudgetKey,
  resolveBazeliskCommand,
  runBazelPilotInvocationPhases,
} = require("./run_bazel_pilot.cjs");
const { HOST_HEAVY_BUDGET_KEY } = require("./lib/host_job_budget.cjs");
const PARENT_BAZEL_ENV_KEYS = [
  "CTX_BAZEL_DISK_CACHE_DIR",
  "CTX_BAZEL_JOBS",
  "CTX_BAZEL_OUTPUT_USER_ROOT",
  "CTX_BAZEL_REMOTE_EXECUTION",
  "CTX_BAZEL_REPOSITORY_CACHE_DIR",
  "CTX_CACHE_SCOPE_KEY",
  "CTX_CACHE_ENV_MANAGED_TURBO_CACHE_DIR",
  "TURBO_API",
  "TURBO_CACHE_DIR",
  "TURBO_TEAM",
  "TURBO_TOKEN",
];

for (const key of PARENT_BAZEL_ENV_KEYS) {
  delete process.env[key];
}

test("bazel pilot defaults to the expanded Rust slice test targets", () => {
  assert.deepEqual(parseArgs([]), {
    command: "test",
    rustClippy: false,
    targets: DEFAULT_TEST_TARGETS,
  });
});

test("bazel pilot build defaults to the expanded Rust slice library targets", () => {
  assert.deepEqual(parseArgs(["build"]), {
    command: "build",
    rustClippy: false,
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

test("bazel pilot parses rust clippy build mode", () => {
  assert.deepEqual(parseArgs(["build", "--rust-clippy", "//core/crates/ctx-core:lib"]), {
    command: "build",
    rustClippy: true,
    targets: ["//core/crates/ctx-core:lib"],
  });
  assert.throws(
    () => parseArgs(["test", "--rust-clippy", "//core/crates/ctx-core:unit_tests"]),
    /--rust-clippy is only supported/,
  );
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

test("bazel pilot rust clippy mode forwards rules_rust aspect args", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["build", "--rust-clippy", "//core/crates/ctx-core:lib"],
    env: {
      ...process.env,
      CTX_BAZEL_REMOTE_EXECUTION: "off",
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-clippy",
      CTX_SESSION_ID: "bazel-clippy-session",
    },
  });

  assert.equal(invocation.rustClippy, true);
  for (const arg of RUST_CLIPPY_BAZEL_ARGS) {
    assert.equal(invocation.commandArgs.includes(arg), true);
    assert.equal(invocation.phases[0].commandArgs.includes(arg), true);
  }
  assert.deepEqual(invocation.targets, ["//core/crates/ctx-core:lib"]);
});

test("bazel pilot scrubs retired Turbo env before spawning Bazel", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/packages/session-supervisor-core:unit_tests"],
    env: {
      ...process.env,
      CTX_BAZEL_REMOTE_EXECUTION: "off",
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-no-turbo",
      CTX_SESSION_ID: "bazel-no-turbo-session",
      CTX_CACHE_ENV_MANAGED_TURBO_CACHE_DIR: "1",
      TURBO_API: "https://example.invalid",
      TURBO_CACHE_DIR: "/tmp/legacy-turbo",
      TURBO_TEAM: "legacy-team",
      TURBO_TOKEN: "legacy-token",
    },
  });

  assert.equal(invocation.env.CTX_CACHE_ENV_MANAGED_TURBO_CACHE_DIR, undefined);
  assert.equal(invocation.env.TURBO_API, undefined);
  assert.equal(invocation.env.TURBO_CACHE_DIR, undefined);
  assert.equal(invocation.env.TURBO_TEAM, undefined);
  assert.equal(invocation.env.TURBO_TOKEN, undefined);
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
  assert.equal(parseRemoteExecutionMode("", { platform: "darwin" }), "cache");
  assert.equal(parseRemoteExecutionMode("", { platform: "linux" }), "linux");
  assert.equal(parseRemoteExecutionMode("cache"), "cache");
  assert.equal(parseRemoteExecutionMode("off"), "off");
  assert.equal(parseRemoteExecutionMode("false"), "off");
  assert.equal(parseRemoteExecutionMode("1"), "all");
  assert.equal(parseRemoteExecutionMode("true"), "all");
  assert.equal(parseRemoteExecutionMode("linux"), "linux");
  assert.equal(parseRemoteExecutionMode("darwin"), "darwin");
  assert.equal(parseRemoteExecutionMode("macos"), "darwin");
});

test("bazel pilot disables BuildBuddy by default for oversized Darwin web tests", () => {
  assert.equal(resolveRemoteExecutionMode({
    command: "test",
    platform: "darwin",
    targets: ["//core/apps/web:unit_tests_non_pretext"],
  }), "off");
  assert.equal(resolveRemoteExecutionMode({
    command: "test",
    platform: "darwin",
    targets: ["//core/apps/web/e2e:premerge_required"],
  }), "off");
  assert.equal(resolveRemoteExecutionMode({
    command: "test",
    platform: "darwin",
    targets: ["//core/packages/session-supervisor-core:unit_tests"],
  }), "cache");
  assert.equal(resolveRemoteExecutionMode({
    command: "test",
    env: { CTX_BAZEL_REMOTE_EXECUTION: "cache" },
    platform: "darwin",
    targets: ["//core/apps/web:unit_tests_non_pretext"],
  }), "cache");
  assert.equal(resolveRemoteExecutionMode({
    command: "test",
    platform: "linux",
    targets: ["//core/apps/web:unit_tests_non_pretext"],
  }), "linux");
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

test("bazel pilot serializes the oversized web non-pretext target on Darwin by default", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/apps/web:unit_tests_non_pretext"],
    platform: "darwin",
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-darwin-web-non-pretext",
      CTX_SESSION_ID: "bazel-darwin-web-non-pretext-session",
    },
  });

  assert.equal(resolveLocalTestJobs({
    command: "test",
    platform: "darwin",
    targets: ["//core/apps/web:unit_tests_non_pretext"],
  }), 1);
  assert.equal(invocation.remoteExecutionMode, "off");
  assert.equal(invocation.commandArgs.includes("--local_test_jobs=1"), true);
  assert.equal(invocation.phases[0].commandArgs.includes("--local_test_jobs=1"), true);
});

test("bazel pilot keeps Linux web non-pretext target parallel unless explicitly capped", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/apps/web:unit_tests_non_pretext"],
    platform: "linux",
    env: {
      ...process.env,
      CTX_BAZEL_REMOTE_EXECUTION: "off",
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-linux-web-non-pretext",
      CTX_SESSION_ID: "bazel-linux-web-non-pretext-session",
    },
  });

  assert.equal(resolveLocalTestJobs({
    command: "test",
    platform: "linux",
    targets: ["//core/apps/web:unit_tests_non_pretext"],
  }), null);
  assert.equal(invocation.commandArgs.some((entry) => entry.startsWith("--local_test_jobs=")), false);
  assert.equal(
    invocation.phases[0].commandArgs.some((entry) => entry.startsWith("--local_test_jobs=")),
    false,
  );
});

test("bazel pilot applies a bounded Bazel test timeout for verification parent runs", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/apps/web:unit_tests_non_pretext_settings_setup"],
    env: {
      ...process.env,
      CTX_BAZEL_REMOTE_EXECUTION: "off",
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-verify-timeout",
      CTX_SESSION_ID: "bazel-verify-timeout-session",
      CTX_VERIFY_PARENT_ENTRYPOINT: "verify:affected",
    },
  });

  assert.equal(parseBazelTestTimeoutSeconds(invocation.env), 1200);
  assert.equal(invocation.bazelTestTimeoutSeconds, 1200);
  assert.equal(invocation.commandArgs.includes("--test_timeout=1200"), true);
  assert.equal(invocation.phases[0].commandArgs.includes("--test_timeout=1200"), true);
});

test("bazel pilot leaves direct Bazel test commands unbounded by default", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/apps/web:unit_tests_non_pretext_settings_setup"],
    env: {
      ...process.env,
      CTX_BAZEL_REMOTE_EXECUTION: "off",
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-direct-timeout",
      CTX_SESSION_ID: "bazel-direct-timeout-session",
      CTX_BAZEL_TEST_TIMEOUT_SECONDS: "",
      CTX_VERIFY_PARENT_ENTRYPOINT: "",
    },
  });

  assert.equal(invocation.bazelTestTimeoutSeconds, null);
  assert.equal(invocation.commandArgs.some((entry) => entry.startsWith("--test_timeout=")), false);
});

test("bazel pilot accepts explicit Bazel test timeout overrides and disables", () => {
  assert.equal(parseBazelTestTimeoutSeconds({ CTX_BAZEL_TEST_TIMEOUT_SECONDS: "45" }), 45);
  assert.equal(parseBazelTestTimeoutSeconds({ CTX_BAZEL_TEST_TIMEOUT_SECONDS: "0" }), null);
  assert.equal(parseBazelTestTimeoutSeconds({ CTX_BAZEL_TEST_TIMEOUT_SECONDS: "off" }), null);

  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/apps/web:unit_tests_non_pretext_settings_setup"],
    env: {
      ...process.env,
      CTX_BAZEL_REMOTE_EXECUTION: "off",
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-explicit-timeout",
      CTX_SESSION_ID: "bazel-explicit-timeout-session",
      CTX_BAZEL_TEST_TIMEOUT_SECONDS: "45",
      CTX_VERIFY_PARENT_ENTRYPOINT: "",
    },
  });

  assert.equal(invocation.commandArgs.includes("--test_timeout=45"), true);
});

test("bazel pilot rejects invalid Bazel test timeout overrides", () => {
  assert.throws(
    () => parseBazelTestTimeoutSeconds({ CTX_BAZEL_TEST_TIMEOUT_SECONDS: "soon" }),
    /CTX_BAZEL_TEST_TIMEOUT_SECONDS/u,
  );
  assert.throws(
    () =>
      buildBazelPilotInvocation({
        argv: ["test", "//core/apps/web:unit_tests_non_pretext_settings_setup"],
        env: {
          ...process.env,
          CTX_BAZEL_REMOTE_EXECUTION: "off",
          CTX_BAZEL_TEST_TIMEOUT_SECONDS: "1.5",
        },
      }),
    /positive integer/u,
  );
});

test("bazel pilot marks only local phases as host-budgeted work", () => {
  const invocation = buildBazelPilotInvocation({
    argv: [
      "build",
      "//core/crates/ctx-provider-accounts:lib",
      "//core/crates/ctx-http:ctx",
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

test("bazel pilot keeps ordinary binary builds local in Linux partition mode", () => {
  const invocation = buildBazelPilotInvocation({
    argv: [
      "build",
      "//core/crates/ctx-core:lib",
      "//core/crates/ctx-http:ctx",
    ],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-ordinary-build-partition",
      CTX_SESSION_ID: "bazel-ordinary-build-partition-session",
      CTX_BAZEL_REMOTE_EXECUTION: "linux",
      BUILD_BUDDY_API_KEY: "buildbuddy-linux-key",
    },
  });

  assert.deepEqual(invocation.phases.map((phase) => phase.name), ["linux-rbe", "local"]);
  assert.deepEqual(invocation.phases[0].targets, ["//core/crates/ctx-core:lib"]);
  assert.deepEqual(invocation.phases[1].targets, ["//core/crates/ctx-http:ctx"]);
});

test("bazel pilot keeps clippy binary builds remote in Linux partition mode", () => {
  const invocation = buildBazelPilotInvocation({
    argv: [
      "build",
      "--rust-clippy",
      "//core/crates/ctx-core:lib",
      "//core/crates/ctx-http:ctx",
    ],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-clippy-build-partition",
      CTX_SESSION_ID: "bazel-clippy-build-partition-session",
      CTX_BAZEL_REMOTE_EXECUTION: "linux",
      BUILD_BUDDY_API_KEY: "buildbuddy-linux-key",
    },
  });

  assert.equal(invocation.rustClippy, true);
  assert.deepEqual(invocation.phases.map((phase) => phase.name), ["linux-rbe"]);
  assert.deepEqual(invocation.phases[0].targets, [
    "//core/crates/ctx-core:lib",
    "//core/crates/ctx-http:ctx",
  ]);
  assert.equal(invocation.phases[0].commandArgs.includes("--config=buildbuddy-linux-rbe"), true);
  for (const arg of RUST_CLIPPY_BAZEL_ARGS) {
    assert.equal(invocation.phases[0].commandArgs.includes(arg), true);
  }
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
  assert.equal(summary.runId, invocation.telemetry.runId);
  assert.equal(summary.phases.length, 1);
  assert.equal(summary.buildBuddyInvocations.length, 1);
  assert.equal(summary.buildBuddyInvocations[0].phaseName, "remote");
  assert.match(summary.buildBuddyInvocations[0].invocationUrl, /https:\/\/app\.buildbuddy\.io\/invocation\//u);
  assert.equal(
    summary.phases[0].commandArgs.includes("--remote_header=x-buildbuddy-api-key=<redacted>"),
    true,
  );
  assert.match(summary.phases[0].buildBuddyInvocationUrl, /https:\/\/app\.buildbuddy\.io\/invocation\//u);
  assert.equal(typeof summary.queueTimeMs, "number");
  assert.equal(typeof summary.remoteActionTimeMs, "number");
  assert.equal(typeof summary.runnerLocalOverheadMs, "number");
  assert.equal(fs.existsSync(invocation.telemetry.hostSamplesPath), true);
});

test("bazel pilot emits a machine-readable phase summary with local spill accounting", () => {
  const invocation = buildBazelPilotInvocation({
    argv: [
      "build",
      "//core/crates/ctx-provider-accounts:lib",
      "//core/crates/ctx-http:ctx",
    ],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-summary",
      CTX_SESSION_ID: "bazel-summary-session",
      CTX_BAZEL_REMOTE_EXECUTION: "linux",
      BUILD_BUDDY_API_KEY: "buildbuddy-linux-key",
    },
  });
  const emitted = [];

  runBazelPilotInvocationPhases(invocation, {
    emitSummaryImpl: (line) => emitted.push(line),
    spawnSyncImpl: () => ({ status: 0 }),
    withHostJobBudgetImpl: (_options, fn) => fn(),
  });

  assert.equal(emitted.length, 1);
  assert.match(emitted[0], /^CTX_BAZEL_PILOT_SUMMARY /u);
  const summary = JSON.parse(emitted[0].replace(/^CTX_BAZEL_PILOT_SUMMARY /u, ""));
  assert.equal(summary.remoteExecutionMode, "linux");
  assert.equal(summary.success, true);
  assert.equal(summary.runId, invocation.telemetry.runId);
  assert.equal(summary.phaseCount, 2);
  assert.equal(summary.remotePhaseCount, 1);
  assert.equal(summary.localPhaseCount, 1);
  assert.equal(summary.remoteTargetCount, 1);
  assert.equal(summary.localTargetCount, 1);
  assert.equal(summary.localSpill, true);
  assert.equal(typeof summary.queueTimeMs, "number");
  assert.equal(typeof summary.remoteActionTimeMs, "number");
  assert.equal(typeof summary.runnerLocalOverheadMs, "number");
  assert.deepEqual(summary.phases.map((phase) => phase.name), ["linux-rbe", "local"]);
});

test("bazel pilot default summary emitter writes to stderr instead of stdout", () => {
  const invocation = buildBazelPilotInvocation({
    argv: ["test", "//core/crates/ctx-core:unit_tests"],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazel-pilot-default-emitter-")),
      CTX_SESSION_ID: "bazel-default-emitter-session",
      CTX_BAZEL_REMOTE_EXECUTION: "off",
    },
  });
  const stdoutChunks = [];
  const stderrChunks = [];
  const originalStdoutWrite = process.stdout.write;
  const originalStderrWrite = process.stderr.write;

  process.stdout.write = ((chunk, encoding, callback) => {
    stdoutChunks.push(Buffer.isBuffer(chunk) ? chunk.toString("utf8") : String(chunk));
    if (typeof callback === "function") {
      callback();
    }
    return true;
  });
  process.stderr.write = ((chunk, encoding, callback) => {
    stderrChunks.push(Buffer.isBuffer(chunk) ? chunk.toString("utf8") : String(chunk));
    if (typeof callback === "function") {
      callback();
    }
    return true;
  });

  try {
    runBazelPilotInvocationPhases(invocation, {
      spawnSyncImpl: () => ({ status: 0 }),
      withHostJobBudgetImpl: (_options, fn) => fn(),
    });
  } finally {
    process.stdout.write = originalStdoutWrite;
    process.stderr.write = originalStderrWrite;
  }

  assert.equal(stdoutChunks.some((chunk) => chunk.includes("CTX_BAZEL_PILOT_SUMMARY")), false);
  assert.equal(stderrChunks.some((chunk) => chunk.includes("CTX_BAZEL_PILOT_SUMMARY")), true);
});

test("bazel pilot summary formatter is stable for parser consumption", () => {
  const line = formatBazelPilotSummaryLine(buildBazelPilotSummary({
    command: "test",
    env: {
      CTX_VERIFY_PARENT_RUN_ID: "router-run-1",
    },
    phases: [],
    remoteExecutionMode: "all",
    telemetry: {
      runId: "bazel-run-1",
    },
  }, [
    {
      durationMs: 1200,
      name: "remote",
      queueTimeMs: 200,
      status: 0,
      targets: ["//core/crates/ctx-core:unit_tests"],
    },
  ], 0));

  assert.match(line, /^CTX_BAZEL_PILOT_SUMMARY \{/u);
  const parsed = JSON.parse(line.replace(/^CTX_BAZEL_PILOT_SUMMARY /u, ""));
  assert.equal(parsed.command, "test");
  assert.equal(parsed.kind, "bazel");
  assert.equal(parsed.entrypoint, "run_bazel_pilot");
  assert.equal(parsed.runId, "bazel-run-1");
  assert.equal(parsed.parentRunId, "router-run-1");
  assert.equal(parsed.remoteExecutionMode, "all");
  assert.equal(parsed.durationMs, 1200);
  assert.equal(parsed.phaseCount, 1);
  assert.equal(parsed.remoteTargetCount, 1);
  assert.equal(parsed.queueTimeMs, 200);
  assert.equal(parsed.remoteActionTimeMs, 1000);
  assert.equal(parsed.runnerLocalOverheadMs, 200);
  assert.equal(parsed.localSpill, false);
});

test("bazel pilot reports local spill for linux remote mode with local-only execution", () => {
  const summary = buildBazelPilotSummary({
    command: "test",
    env: {},
    phases: [],
    remoteExecutionMode: "linux",
    telemetry: null,
  }, [
    {
      durationMs: 900,
      name: "local",
      queueTimeMs: 0,
      status: 0,
      targets: ["//core/apps/web:pretext_measurement_unit_tests"],
    },
  ], 0);

  assert.equal(summary.localPhaseCount, 1);
  assert.equal(summary.remotePhaseCount, 0);
  assert.equal(summary.localSpill, true);
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

  const durations = [0, 0, 500, 1000, 1750];
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
  assert.ok(summary.phases[0].queueTimeMs >= 50);
  assert.ok(summary.queueTimeMs >= 50);
  assert.ok(summary.runnerLocalOverheadMs >= 50);
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
      "//core/crates/ctx-http:ctx",
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
        targets: ["//core/crates/ctx-http:ctx"],
        hasBuildBuddyCache: true,
        hasLinuxConfig: false,
        hasBuildBuddyHeader: true,
      },
    ],
  );
});

test("bazel pilot linux remote execution keeps web-smoke verification fully remote", () => {
  const invocation = buildBazelPilotInvocation({
    argv: [
      "test",
      "//core/packages/session-supervisor-core:unit_tests",
      "//core/packages/session-thread-layout:unit_smoke",
      "//core/apps/web:unit_smoke",
    ],
    env: {
      ...process.env,
      CTX_VOLATILE_ROOT: "/tmp/ctx-bazel-pilot-web-smoke-linux-rbe",
      CTX_SESSION_ID: "bazel-web-smoke-linux-rbe-session",
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
        targets: [
          "//core/apps/web:unit_smoke",
          "//core/packages/session-supervisor-core:unit_tests",
          "//core/packages/session-thread-layout:unit_smoke",
        ],
        hasBuildBuddyCache: true,
        hasLinuxConfig: true,
        hasBuildBuddyHeader: true,
      },
    ],
  );
});

test("BuildBuddy Linux RBE configs pin target and host platforms to Linux", () => {
  const bazelrc = fs.readFileSync(path.resolve(__dirname, "..", "..", ".bazelrc"), "utf8");
  for (const configName of ["buildbuddy-rbe", "buildbuddy-linux-rbe"]) {
    assert.match(
      bazelrc,
      new RegExp(
        `^common:${configName} --platforms=//tools/bazel/platforms:linux_x86_64$`,
        "mu",
      ),
    );
    assert.match(
      bazelrc,
      new RegExp(
        `^common:${configName} --host_platform=//tools/bazel/platforms:linux_x86_64$`,
        "mu",
      ),
    );
  }
  assert.match(
    bazelrc,
    /^common:buildbuddy-linux-arm64-rbe --platforms=\/\/tools\/bazel\/platforms:linux_arm64$/mu,
  );
  assert.match(
    bazelrc,
    /^common:buildbuddy-linux-arm64-rbe --host_platform=\/\/tools\/bazel\/platforms:linux_arm64$/mu,
  );
});

test("BuildBuddy Darwin RBE configs pin target and host platforms to Darwin", () => {
  const bazelrc = fs.readFileSync(path.resolve(__dirname, "..", "..", ".bazelrc"), "utf8");
  for (const [configName, platformLabel] of [
    ["buildbuddy-darwin-rbe", "darwin_arm64"],
    ["buildbuddy-darwin-amd64-rbe", "darwin_x86_64"],
  ]) {
    assert.match(
      bazelrc,
      new RegExp(`^common:${configName} --platforms=//tools/bazel/platforms:${platformLabel}$`, "mu"),
    );
    assert.match(
      bazelrc,
      new RegExp(
        `^common:${configName} --host_platform=//tools/bazel/platforms:${platformLabel}$`,
        "mu",
      ),
    );
  }
});

test("BuildBuddy Linux amd64 RBE configs pin the executor container image and docker isolation", () => {
  const bazelrc = fs.readFileSync(path.resolve(__dirname, "..", "..", ".bazelrc"), "utf8");
  for (const configName of ["buildbuddy-rbe", "buildbuddy-linux-rbe"]) {
    assert.match(
      bazelrc,
      new RegExp(
        `^common:${configName} --remote_default_exec_properties=container-image=docker://localhost:5000/buildbuddy-executor:bookworm-amd64$`,
        "mu",
      ),
    );
    assert.match(
      bazelrc,
      new RegExp(
        `^common:${configName} --remote_default_exec_properties=workload-isolation-type=docker$`,
        "mu",
      ),
    );
    assert.match(
      bazelrc,
      new RegExp(
        `^common:${configName} --remote_default_exec_properties=dockerUser=root$`,
        "mu",
      ),
    );
  }
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
      CTX_BAZEL_REMOTE_EXECUTION: "",
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
      CTX_BAZEL_JOBS: "",
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
