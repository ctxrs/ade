#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");

const { buildExecutionPlan } = require("./lib/test_taxonomy/execution.cjs");

test("agent-default fans out ctx-http shared changes into suite-level commands", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/crates/ctx-http/src/api/mod.rs"],
    touchedOnly: true,
  });

  assert.ok(!plan.selectedEntries.some((entry) =>
    entry.id === "build-graph.rust-turbo-check" || entry.entrypointType === "rust-crate-gate",
  ));

  assert.deepEqual(plan.commands, [
    "node scripts/ctx_http_suite_task.cjs --suite attachments-routing --suite provider-auth --suite provider-runtime-simulated --suite repo-vcs --suite sandbox-runtime-simulated --suite scheduler-runtime --suite subagents-control --suite turns-terminal --suite updates-release --suite workspace-stream",
  ]);
});

test("agent-default keeps non-ctx-http Rust crate changes on the taxonomy-backed Rust gate", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/crates/ctx-provider-accounts/src/lib.rs"],
    touchedOnly: true,
  });

  assert.ok(plan.selectedEntries.some((entry) => entry.id === "build-graph.rust-turbo-check"));
  assert.ok(plan.selectedEntries.some((entry) => entry.id === "provider-auth.rust-gate.ctx-provider-accounts"));

  assert.deepEqual(plan.commands, [
    "pnpm rust:turbo:check",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/crates/ctx-provider-accounts/src/lib.rs",
  ]);
});

test("agent-default affected selection expands a Rust leaf into downstream dependency-truthful suites", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/crates/ctx-provider-accounts/src/lib.rs"],
    selectionMode: "affected",
  });

  assert.deepEqual(plan.commands, [
    "pnpm rust:turbo:check",
    "node scripts/ctx_http_suite_task.cjs --suite provider-auth --suite provider-runtime-simulated",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --crate ctx-provider-accounts",
  ]);
});

test("agent-default workspace-level Rust inputs select the taxonomy-backed Rust gate set", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/Cargo.lock"],
    touchedOnly: true,
  });

  assert.ok(plan.selectedEntries.some((entry) => entry.id === "build-graph.rust-turbo-check"));
  assert.ok(plan.selectedEntries.some((entry) => entry.id === "provider-auth.rust-gate.ctx-provider-accounts"));
  assert.ok(plan.selectedEntries.some((entry) => entry.id === "repo-vcs.rust-gate.ctx-merge-queue"));

  assert.deepEqual(plan.commands, [
    "pnpm rust:turbo:check",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/Cargo.lock",
  ]);
});

test("agent-default escalates high-risk web state changes to the canonical premerge browser suite", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/state/providerOnboardingCoordinator.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:foundation:state",
    "pnpm bazel:web:e2e:premerge",
  ]);
});

test("agent-default keeps settings-only web changes off the pretext measurement slice", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/pages/settings/SettingsPage.tsx"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:settings-setup",
  ]);
});

test("agent-default routes root-level web shell files to the workbench surface app shard", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/main.tsx"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:workbench:surface:app",
  ]);
});

test("agent-default routes session workbench files to the workbench surface session shard", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/pages/sessionView/SessionWorkbenchPane.tsx"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:workbench:surface:session",
  ]);
});

test("agent-default routes workbench shell files to the dedicated shell shard", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/pages/workbenchShell/WorkbenchPage.shell.tsx"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:workbench:shell",
  ]);
});

test("agent-default routes non-pretext web fixture changes to the foundation shard", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/testdata/projectionEquivalenceFixtures.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:foundation:state",
  ]);
});

test("agent-default routes foundation utility changes to the shared foundation shard", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/utils/codeTokenLinks.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:foundation:shared",
  ]);
});

test("agent-default routes pretext measurement source changes to the dedicated unit slice", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/pages/sessionThread/sessionMarkdownInlineMeasurement.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:pretext:measurement",
  ]);
});

test("agent-default touched selection keeps extracted layout package changes on the direct package unit gate", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/packages/session-thread-layout/src/sessionMarkdownContract.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:thread-layout",
  ]);
});

test("agent-default affected selection expands extracted layout package changes into app consumer coverage", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/packages/session-thread-layout/src/sessionMarkdownContract.ts"],
    selectionMode: "affected",
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:thread-layout",
    "pnpm bazel:web:pretext:measurement",
  ]);
});

test("agent-default touched selection keeps extracted supervisor package changes on the direct package unit gate", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/packages/session-supervisor-core/src/sessionSubscriptionPlan.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:supervisor-core",
  ]);
});

test("agent-default affected selection expands extracted supervisor package changes into app consumer coverage", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/packages/session-supervisor-core/src/sessionSubscriptionPlan.ts"],
    selectionMode: "affected",
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:supervisor-core",
    "pnpm bazel:web:unit:non-pretext",
  ]);
});

test("agent-default selects narrow pretext E2E Bazel labels for touched parity specs", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/e2e/workbench-pretext-wrap-rules.spec.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "node scripts/run_bazel_pilot.cjs test //core/apps/web/e2e:pretext_wrap_rules",
  ]);
});

test("agent-default affected selection adds ctx-http base compile truth for scheduler runtime changes", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/crates/ctx-http/src/scheduler/runtime/event_loop.rs"],
    selectionMode: "affected",
  });

  assert.deepEqual(plan.commands, [
    "node scripts/ctx_http_suite_task.cjs --suite base --suite scheduler-runtime",
  ]);
});

test("agent-default affected selection keeps shared turn execution paths on both scheduler runtime and terminal suites", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/crates/ctx-http/src/api/execution.rs"],
    selectionMode: "affected",
  });

  assert.deepEqual(plan.commands, [
    "node scripts/ctx_http_suite_task.cjs --suite base --suite scheduler-runtime --suite turns-terminal",
  ]);
});

test("agent-default affected selection keeps direct ctx-http suite test edits on suite truth plus base compile truth", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/crates/ctx-http/tests/subscription_accounts_api.rs"],
    selectionMode: "affected",
  });

  assert.deepEqual(plan.commands, [
    "node scripts/ctx_http_suite_task.cjs --suite base --suite provider-auth",
  ]);
});

test("agent-default selects a narrow web E2E Bazel label for touched browser specs", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/e2e/workbench-unarchive-visible.spec.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "node scripts/run_bazel_pilot.cjs test //core/apps/web/e2e:workbench_unarchive_visible",
  ]);
});

test("agent-default escalates direct agent-full web E2E specs to their suite label", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/e2e/workbench-providers-bootstrap.spec.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "node scripts/run_bazel_pilot.cjs test //core/apps/web/e2e:premerge_required",
  ]);
});

test("agent-default routes shared web E2E Bazel macro changes to the canonical premerge suite", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/e2e/web_e2e_test.bzl"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:e2e:premerge",
  ]);
});

test("agent-default routes shared web E2E runtime changes without fanning out to every suite", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/scripts/run-e2e-bazel-runtime.mjs"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:misc",
    "pnpm bazel:web:e2e:premerge",
  ]);
});

test("agent-default routes shared web E2E server changes through unit and premerge gates", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/scripts/start-e2e-server.mjs"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:misc",
    "pnpm bazel:web:e2e:premerge",
  ]);
});

test("agent-default routes shared Playwright config changes through unit and premerge gates", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/playwright.shared.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:workbench:surface:app",
    "pnpm bazel:web:e2e:premerge",
  ]);
});

test("agent-default routes shared Playwright config tests to web unit tests", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/playwright.shared.test.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:unit:non-pretext:workbench:surface:app",
  ]);
});

test("release-contracts touched-only selection owns legacy release script changes", () => {
  const plan = buildExecutionPlan({
    profileId: "release-contracts",
    changedFiles: ["scripts/buildbuddy/run_release_contracts.sh"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm release:bundle:contracts:linux-x86_64",
    "pnpm bazel:desktop:runtime:lock:check-matrix",
    "pnpm bazel:desktop:runtime:lock:validate",
    "pnpm bazel:desktop:check:versions",
  ]);
});

test("mac-preview touched-only selection resolves to the dedicated Apple Silicon preview wrapper", () => {
  const plan = buildExecutionPlan({
    profileId: "mac-preview",
    changedFiles: ["core/apps/desktop/src-tauri/src/main.rs"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "bash ../scripts/buildkite/run_mac_preview_build.sh",
  ]);
});

test("nightly-breadth is the explicit scheduled anomaly and fuzz union", () => {
  const plan = buildExecutionPlan({
    profileId: "nightly-breadth",
    touchedOnly: false,
    changedFiles: [],
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:anomaly:ctx-http:fault-matrix",
    "pnpm bazel:anomaly:ctx-http:hot-endpoints-no-db",
    "pnpm bazel:anomaly:ctx-store:fault-injection",
    "pnpm bazel:fuzz:providers",
    "pnpm bazel:fuzz:mcp",
    "pnpm bazel:fuzz:workspace-payloads",
    "pnpm bazel:fuzz:release-manifests",
    "pnpm bazel:fuzz:desktop-ipc",
  ]);
});

test("nightly helper profiles keep the scheduled nightly slices Linux-only and bounded", () => {
  assert.deepEqual(
    buildExecutionPlan({ profileId: "nightly-linux-anomaly", touchedOnly: false, changedFiles: [] }).commands,
    [
      "pnpm bazel:anomaly:ctx-http:fault-matrix",
      "pnpm bazel:anomaly:ctx-http:hot-endpoints-no-db",
      "pnpm bazel:anomaly:ctx-store:fault-injection",
    ],
  );
  assert.deepEqual(
    buildExecutionPlan({ profileId: "nightly-linux-fuzz", touchedOnly: false, changedFiles: [] }).commands,
    [
      "pnpm bazel:fuzz:providers",
      "pnpm bazel:fuzz:mcp",
      "pnpm bazel:fuzz:workspace-payloads",
      "pnpm bazel:fuzz:release-manifests",
      "pnpm bazel:fuzz:desktop-ipc",
    ],
  );
});

test("release publish-mode profiles stay on the artifact-only finalize boundary", () => {
  assert.deepEqual(
    buildExecutionPlan({ profileId: "canary-proof", touchedOnly: false, changedFiles: [] }).commands,
    [
      "bash ../scripts/buildkite/run_release_finalize.sh",
      "bash ../scripts/tests/updater_linux_release_truth.sh",
      "bash ../scripts/tests/updater_remote_daemon_e2e.sh",
    ],
  );
  assert.deepEqual(
    buildExecutionPlan({ profileId: "stable-promotion", touchedOnly: false, changedFiles: [] }).commands,
    ["bash ../scripts/buildkite/run_release_stable_promote.sh"],
  );
});

test("nightly benchmark evidence stays on the dedicated wrapper entrypoint", () => {
  assert.deepEqual(
    buildExecutionPlan({ profileId: "nightly-benchmark-evidence", touchedOnly: false, changedFiles: [] }).commands,
    ["bash ../scripts/buildkite/run_nightly_benchmark_evidence.sh"],
  );
});

test("direct pipeline helper profiles resolve to explicit package-script commands", () => {
  assert.deepEqual(
    buildExecutionPlan({ profileId: "install-bootstrap-contracts", touchedOnly: false, changedFiles: [] }).commands,
    ["pnpm install:bootstrap:contracts"],
  );
  assert.deepEqual(
    buildExecutionPlan({ profileId: "provider-auth-validate", touchedOnly: false, changedFiles: [] }).commands,
    ["pnpm bazel:provider-auth:validate"],
  );
  assert.deepEqual(
    buildExecutionPlan({ profileId: "desktop-system-parity-contracts", touchedOnly: false, changedFiles: [] }).commands,
    [
      "pnpm bazel:desktop:runtime:lock:check-matrix",
      "pnpm bazel:desktop:check:versions",
      "pnpm bazel:desktop:provider-matrix:contracts",
      "pnpm bazel:desktop:launch-mode:contracts",
      "pnpm bazel:desktop:bundle:contracts",
      "pnpm bazel:bundled-harness:dependency:contracts",
      "pnpm bazel:provider-auth:validate",
      "pnpm bazel:desktop:e2e:preflight:contracts",
      "pnpm bazel:desktop:sync-resources:contracts",
      "pnpm bazel:providers:e2e:bundle:contracts",
      "pnpm bazel:providers:linux-arm:contracts",
      "pnpm bazel:tauri:tools:lock:contracts",
      "pnpm bazel:desktop:deps:contracts",
    ],
  );
  assert.deepEqual(
    buildExecutionPlan({ profileId: "release-updater-web-e2e", touchedOnly: false, changedFiles: [] }).commands,
    [
      "pnpm bazel:web:e2e:release:retry",
    ],
  );
  assert.deepEqual(
    buildExecutionPlan({ profileId: "release-updater-smoke", touchedOnly: false, changedFiles: [] }).commands,
    [
      "pnpm verify:e2e:updater:smoke:native",
    ],
  );
});

test("checkin promotion gate includes broad stable cacheable basics", () => {
  const plan = buildExecutionPlan({
    profileId: "checkin",
  });
  const entryIds = new Set(plan.selectedEntries.map((entry) => entry.id));

  for (const requiredEntryId of [
    "repo-contracts.source-file-size",
    "repo-contracts.testing-taxonomy-check",
    "build-graph.rust-turbo-check",
    "ctx-http.base",
    "web-workbench.web-typecheck",
    "web-workbench.session-supervisor-core-unit-tests",
    "web-workbench.web-premerge-required",
    "repo-contracts.buildkite-pipeline",
  ]) {
    assert.equal(entryIds.has(requiredEntryId), true, `missing checkin entry ${requiredEntryId}`);
  }

  assert.ok(plan.commands.includes("pnpm source:file-size:enforce"));
  assert.ok(plan.commands.includes("pnpm testing:taxonomy:check"));
  assert.ok(plan.commands.includes("pnpm rust:turbo:check"));
  assert.ok(plan.commands.includes("node scripts/ctx_http_suite_task.cjs --suite attachments-routing"));
  assert.ok(plan.commands.includes("node scripts/ctx_http_suite_task.cjs --suite base"));
  assert.ok(plan.commands.includes("node scripts/ctx_http_suite_task.cjs --suite provider-auth"));
  assert.equal(
    plan.commands.some((command) => command.includes("--suite attachments-routing --suite base")),
    false,
  );
  assert.ok(plan.commands.includes("pnpm bazel:web:typecheck"));
  assert.ok(plan.commands.includes("pnpm bazel:web:e2e:premerge"));
  assert.ok(plan.commands.includes("pnpm bazel:buildkite:pipeline:test"));
});
