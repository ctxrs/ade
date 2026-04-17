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

  assert.deepEqual(plan.commands, [
    "node scripts/ctx_http_suite_task.cjs --suite attachments-routing",
    "node scripts/ctx_http_suite_task.cjs --suite lsp",
    "node scripts/ctx_http_suite_task.cjs --suite provider-auth",
    "node scripts/ctx_http_suite_task.cjs --suite repo-vcs",
    "node scripts/ctx_http_suite_task.cjs --suite subagents-control",
    "node scripts/ctx_http_suite_task.cjs --suite turns-terminal",
    "node scripts/ctx_http_suite_task.cjs --suite updates-release",
    "node scripts/ctx_http_suite_task.cjs --suite workspace-stream",
  ]);
});

test("agent-default keeps non-catalogued Rust crate changes on the targeted Rust gate", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/crates/ctx-provider-accounts/src/lib.rs"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm rust:turbo:check",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/crates/ctx-provider-accounts/src/lib.rs",
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

test("releasetest touched-only selection owns legacy release script changes", () => {
  const plan = buildExecutionPlan({
    profileId: "releasetest",
    changedFiles: ["scripts/buildbuddy/run_release_contracts.sh"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm test:bundles:codex-archive-artifacts",
    "pnpm test:bundles:codex-provenance",
    "pnpm verify:e2e:updater:smoke:native",
  ]);
});
