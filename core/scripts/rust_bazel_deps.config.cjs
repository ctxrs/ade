const GENERATED_DEP_CONSUMER_CRATES = Object.freeze([
  "ctx-bundled-assets",
  "ctx-core",
  "ctx-crp-protocol",
  "ctx-events",
  "ctx-fs",
  "ctx-harness-setup",
  "ctx-llm-relay-authority",
  "ctx-llm-relay-contract",
  "ctx-mcp-command",
  "ctx-mobile-access-service",
  "ctx-daemon",
  "ctx-http-auth",
  "ctx-mcp-auth",
  "ctx-http-test-support",
  "ctx-observability",
  "ctx-org-policy",
  "ctx-provider-install",
  "ctx-provider-accounts",
  "ctx-provider-matrix",
  "ctx-resource-utilization",
  "ctx-route-contracts",
  "ctx-run-archive-service",
  "ctx-run-scheduler",
  "ctx-runtime-assets",
  "ctx-sandbox-contract",
  "ctx-session-artifacts",
  "ctx-session-message-service",
  "ctx-session-runtime",
  "ctx-session-service",
  "ctx-session-title-service",
  "ctx-session-vcs-service",
  "ctx-subagent-service",
  "ctx-session-tools",
  "ctx-storage-admission",
  "ctx-update-service",
  "ctx-settings-model",
  "ctx-settings-service",
  "ctx-workspace-attachments",
  "ctx-workspace-active-snapshot",
  "ctx-worktree-vcs-service",
]);

const MANUAL_CRATES = Object.freeze({
  "ctx-http": Object.freeze({
    owner: "build-graph",
    rationale: "Large custom Bazel surface with split tests, binaries, feature variants, and compile data.",
  }),
  "ctx-load-test": Object.freeze({
    owner: "build-graph",
    rationale: "Workspace package lives under core/tools and is outside the first generated core/crates rollout.",
  }),
});

const PROC_MACRO_DEPS = Object.freeze({
  "async-trait": Object.freeze({
    owner: "build-graph",
    rationale: "rules_rust requires this direct proc macro dependency in proc_macro_deps.",
  }),
});

module.exports = Object.freeze({
  version: 1,
  generatedOutputPath: "tools/bazel/rust_deps.generated.bzl",
  generatedDepConsumerCrates: GENERATED_DEP_CONSUMER_CRATES,
  manualCrates: MANUAL_CRATES,
  procMacroDeps: PROC_MACRO_DEPS,
});
