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

test("agent-default escalates high-risk web state changes to the canonical premerge browser suite", () => {
  const plan = buildExecutionPlan({
    profileId: "agent-default",
    changedFiles: ["core/apps/web/src/state/providerOnboardingCoordinator.ts"],
    touchedOnly: true,
  });

  assert.deepEqual(plan.commands, [
    "pnpm bazel:web:test",
    "pnpm test:e2e:premerge",
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
    ["bash ../scripts/buildkite/run_release_finalize.sh"],
  );
  assert.deepEqual(
    buildExecutionPlan({ profileId: "stable-promotion", touchedOnly: false, changedFiles: [] }).commands,
    ["bash ../scripts/buildkite/run_release_finalize.sh"],
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
