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
    "node scripts/ctx_http_suite_task.cjs --suite attachments-routing",
    "node scripts/ctx_http_suite_task.cjs --suite lsp",
    "node scripts/ctx_http_suite_task.cjs --suite provider-auth",
    "node scripts/ctx_http_suite_task.cjs --suite provider-runtime-simulated",
    "node scripts/ctx_http_suite_task.cjs --suite repo-vcs",
    "node scripts/ctx_http_suite_task.cjs --suite sandbox-runtime-simulated",
    "node scripts/ctx_http_suite_task.cjs --suite subagents-control",
    "node scripts/ctx_http_suite_task.cjs --suite turns-terminal",
    "node scripts/ctx_http_suite_task.cjs --suite updates-release",
    "node scripts/ctx_http_suite_task.cjs --suite workspace-stream",
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

test("agent-default escalates high-risk web state changes to browser truth", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/state/providerOnboardingCoordinator.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:test",
    "pnpm verify:e2e",
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

test("nightly-breadth splits provider auth live coverage by nightly slice", () => {
  const plan = buildExecutionPlan({
    profileId: "nightly-breadth",
    touchedOnly: false,
    changedFiles: [],
  });

  assert.match(plan.commands.join("\n"), /pnpm verify:desktop:provider-auth-matrix:auth-import/);
  assert.match(plan.commands.join("\n"), /pnpm verify:desktop:provider-auth-matrix:endpoint-write/);
  assert.match(plan.commands.join("\n"), /pnpm verify:desktop:provider-auth-matrix:oauth-subscription/);
  assert.doesNotMatch(plan.commands.join("\n"), /pnpm verify:desktop:provider-auth-matrix:nightly/);
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
