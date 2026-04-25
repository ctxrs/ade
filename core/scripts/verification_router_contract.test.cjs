const assert = require("node:assert/strict");
const test = require("node:test");

const {
  MERGE_READY_COMMAND,
  buildOverlayCommands,
  buildVerificationPlan,
  runVerificationPlan,
} = require("./run_verification_router.cjs");

test("verify:touched adds source invariants to the targeted Rust gate plan", () => {
  const plan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["core/crates/ctx-provider-accounts/src/lib.rs"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm rust:turbo:check",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/crates/ctx-provider-accounts/src/lib.rs",
  ]);
});

test("verify:affected broadens the canonical Rust leaf beyond verify:touched", () => {
  const plan = buildVerificationPlan({
    intent: "affected",
    base: "origin/main",
    changedFiles: ["core/crates/ctx-provider-accounts/src/lib.rs"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm rust:turbo:check",
    "node scripts/ctx_http_suite_task.cjs --suite provider-auth",
    "node scripts/ctx_http_suite_task.cjs --suite provider-runtime-simulated",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --crate ctx-provider-accounts",
  ]);
});

test("verify:affected routes supabase migrations through the dedicated invariant check", () => {
  const plan = buildVerificationPlan({
    intent: "affected",
    base: "origin/main",
    changedFiles: ["supabase/migrations/20260421000000_test.sql"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm supabase:migrations:check",
  ]);
});

test("verify:broader keeps web escalations changed-aware", () => {
  const plan = buildVerificationPlan({
    intent: "broader",
    base: "origin/main",
    changedFiles: ["core/apps/web/src/state/providerOnboardingCoordinator.ts"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm bazel:web:unit:non-pretext:foundation:state",
    "pnpm bazel:web:e2e:premerge",
  ]);
});

test("verify:affected routes extracted layout package changes to the dedicated pretext slice", () => {
  const plan = buildVerificationPlan({
    intent: "affected",
    base: "origin/main",
    changedFiles: ["core/packages/session-thread-layout/src/sessionMarkdownContract.ts"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm bazel:web:pretext:measurement",
  ]);
});

test("verify:touched routes root-level web shell files to the workbench surface app shard", () => {
  const plan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["core/apps/web/src/main.tsx"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm bazel:web:unit:non-pretext:workbench:surface:app",
  ]);
});

test("verify:touched routes session workbench files to the workbench surface session shard", () => {
  const plan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["core/apps/web/src/pages/sessionView/SessionWorkbenchPane.tsx"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm bazel:web:unit:non-pretext:workbench:surface:session",
  ]);
});

test("verify:touched routes workbench shell files to the dedicated shell shard", () => {
  const plan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["core/apps/web/src/pages/workbenchShell/WorkbenchPage.shell.tsx"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm bazel:web:unit:non-pretext:workbench:shell",
  ]);
});

test("verify:touched routes non-pretext web fixture changes to the foundation shard", () => {
  const plan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["core/apps/web/src/testdata/projectionEquivalenceFixtures.ts"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm bazel:web:unit:non-pretext:foundation:state",
  ]);
});

test("verify:touched routes foundation utility changes to the shared foundation shard", () => {
  const plan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["core/apps/web/src/utils/codeTokenLinks.ts"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm bazel:web:unit:non-pretext:foundation:shared",
  ]);
});

test("verify:affected routes extracted supervisor package changes to direct package truth plus app coverage", () => {
  const plan = buildVerificationPlan({
    intent: "affected",
    base: "origin/main",
    changedFiles: ["core/packages/session-supervisor-core/src/sessionSubscriptionPlan.ts"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "pnpm bazel:web:unit:supervisor-core",
    "pnpm bazel:web:unit:non-pretext",
  ]);
});

test("verify:affected adds ctx-http base compile truth for canonical scheduler runtime changes", () => {
  const plan = buildVerificationPlan({
    intent: "affected",
    base: "origin/main",
    changedFiles: ["core/crates/ctx-http/src/scheduler/runtime/event_loop.rs"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "node scripts/ctx_http_suite_task.cjs --suite base",
    "node scripts/ctx_http_suite_task.cjs --suite scheduler-runtime",
  ]);
});

test("verify:affected keeps shared turn execution paths on both scheduler runtime and terminal suites", () => {
  const plan = buildVerificationPlan({
    intent: "affected",
    base: "origin/main",
    changedFiles: ["core/crates/ctx-http/src/api/execution.rs"],
  });

  assert.deepEqual(plan.commands, [
    "pnpm source:file-size:enforce",
    "node scripts/ctx_http_suite_task.cjs --suite base",
    "node scripts/ctx_http_suite_task.cjs --suite scheduler-runtime",
    "node scripts/ctx_http_suite_task.cjs --suite turns-terminal",
  ]);
});

test("verify:affected keeps direct ctx-http suite test edits on suite truth plus base compile truth", () => {
  const plan = buildVerificationPlan({
    intent: "affected",
    base: "origin/main",
    changedFiles: ["core/crates/ctx-http/tests/subscription_accounts_api.rs"],
  });

  assert.deepEqual(plan.commands, [
    "node scripts/ctx_http_suite_task.cjs --suite base",
    "node scripts/ctx_http_suite_task.cjs --suite provider-auth",
  ]);
});

test("verify:merge-ready uses the clean checkin profile gate", () => {
  assert.equal(MERGE_READY_COMMAND, "node scripts/run_test_taxonomy_profile.cjs --run --profile checkin");
});

test("verify:merge-ready keeps overlay invariants alongside the checkin profile", () => {
  assert.deepEqual(buildOverlayCommands(["supabase/migrations/20260421000000_test.sql"]), [
    "pnpm supabase:migrations:check",
  ]);
});

test("verify:touched becomes a no-op when there are no relevant changes", () => {
  const plan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["docs/notes.md"],
  });

  assert.deepEqual(plan.commands, []);
});

test("verify:touched keeps ignored docs/config files under core and supabase as no-ops", () => {
  const coreDocPlan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["core/README.md"],
  });
  const supabaseConfigPlan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["supabase/config.toml"],
  });

  assert.deepEqual(coreDocPlan.commands, []);
  assert.deepEqual(supabaseConfigPlan.commands, []);
});

test("verify:touched routes verification-tooling edits through the local tooling suite", () => {
  const plan = buildVerificationPlan({
    intent: "touched",
    base: "origin/main",
    changedFiles: ["core/scripts/run_verification_router.cjs"],
  });

  assert.deepEqual(plan.commands, [
    "node --test scripts/verification_git_changes.test.cjs scripts/verification_router_contract.test.cjs scripts/verification_run_store.test.cjs scripts/sdlc_verify_metrics_report.test.cjs scripts/run_bazel_pilot.test.cjs scripts/testing_tiers_contract.test.cjs scripts/test_taxonomy_execution_contract.test.cjs scripts/affected_tests_contract.test.cjs",
  ]);
});

test("verify router telemetry honors CTX_DISABLE_VERIFICATION_TELEMETRY and still runs commands", () => {
  const spawnCalls = [];

  runVerificationPlan({
    intent: "touched",
    entrypoint: "verify:touched",
    profileId: "agent-default",
    baseRef: "origin/main",
    changedFiles: ["supabase/migrations/20260421000000_test.sql"],
    mergeBase: "",
    overlayCommands: ["pnpm supabase:migrations:check"],
    taxonomyEntries: [],
    taxonomyCommands: [],
    commands: ["pnpm supabase:migrations:check"],
  }, {
    buildCtxCacheEnvImpl: () => ({
      env: {
        ...process.env,
        CTX_DISABLE_VERIFICATION_TELEMETRY: "1",
      },
      layout: {},
    }),
    createRunArtifactsImpl: () => {
      throw new Error("should not initialize telemetry when disabled");
    },
    spawnSyncImpl: (_command, _args, _options) => {
      spawnCalls.push(true);
      return { status: 0 };
    },
  });

  assert.equal(spawnCalls.length, 1);
});

test("verify router degrades gracefully when telemetry initialization fails", () => {
  const spawnCalls = [];
  const errors = [];

  runVerificationPlan({
    intent: "touched",
    entrypoint: "verify:touched",
    profileId: "agent-default",
    baseRef: "origin/main",
    changedFiles: ["supabase/migrations/20260421000000_test.sql"],
    mergeBase: "",
    overlayCommands: ["pnpm supabase:migrations:check"],
    taxonomyEntries: [],
    taxonomyCommands: [],
    commands: ["pnpm supabase:migrations:check"],
  }, {
    buildCtxCacheEnvImpl: () => ({
      env: {
        ...process.env,
        CTX_DISABLE_VERIFICATION_TELEMETRY: "0",
      },
      layout: {},
    }),
    createRunArtifactsImpl: () => {
      throw new Error("disk full");
    },
    logErrorImpl: (message) => {
      errors.push(message);
    },
    spawnSyncImpl: (_command, _args, _options) => {
      spawnCalls.push(true);
      return { status: 0 };
    },
  });

  assert.equal(spawnCalls.length, 1);
  assert.match(errors[0], /failed to initialize verification telemetry/u);
});

test("verify router links child Bazel runs back into the router summary", () => {
  const spawnEnvs = [];
  const finalized = [];

  runVerificationPlan({
    intent: "affected",
    entrypoint: "verify:affected",
    profileId: "agent-default",
    baseRef: "origin/main",
    changedFiles: ["core/crates/ctx-provider-accounts/src/lib.rs"],
    mergeBase: "abc123",
    overlayCommands: [],
    taxonomyEntries: ["rust-workspace"],
    taxonomyCommands: ["pnpm test:agent:minimal"],
    commands: ["pnpm test:agent:minimal"],
  }, {
    buildCtxCacheEnvImpl: () => ({
      env: {
        ...process.env,
        CTX_DISABLE_VERIFICATION_TELEMETRY: "0",
      },
      layout: {},
    }),
    createRunArtifactsImpl: () => ({
      runDir: "/tmp/router-run",
      runId: "router-run-1",
      hostSamplesPath: "/tmp/router-run/host-samples.jsonl",
    }),
    finalizeRunArtifactsImpl: (_artifacts, summary) => {
      finalized.push(summary);
    },
    mkdirSyncImpl: () => {},
    readIndexedSummariesImpl: () => ([
      {
        kind: "bazel",
        runId: "bazel-run-1",
        entrypoint: "run_bazel_pilot",
        durationMs: 1200,
        remoteExecutionMode: "cache",
        parentRunId: "router-run-1",
        targets: ["//core/crates/ctx-provider-accounts:lib"],
        buildBuddyInvocations: [
          {
            phaseName: "local",
            invocationId: "inv-123",
            invocationUrl: "https://app.buildbuddy.io/invocation/inv-123",
          },
        ],
      },
    ]),
    spawnSyncImpl: (_command, _args, options) => {
      spawnEnvs.push(options.env);
      return { status: 0 };
    },
  });

  assert.equal(spawnEnvs.length, 1);
  assert.equal(spawnEnvs[0].CTX_VERIFY_PARENT_ENTRYPOINT, "verify:affected");
  assert.equal(spawnEnvs[0].CTX_VERIFY_PARENT_RUN_ID, "router-run-1");
  assert.equal(finalized.length, 1);
  assert.deepEqual(finalized[0].childBazelRuns, [
    {
      runId: "bazel-run-1",
      entrypoint: "run_bazel_pilot",
      durationMs: 1200,
      remoteExecutionMode: "cache",
      targets: ["//core/crates/ctx-provider-accounts:lib"],
      buildBuddyInvocations: [
        {
          phaseName: "local",
          invocationId: "inv-123",
          invocationUrl: "https://app.buildbuddy.io/invocation/inv-123",
        },
      ],
    },
  ]);
});
