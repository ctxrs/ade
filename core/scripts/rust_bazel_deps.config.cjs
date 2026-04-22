const GENERATED_DEP_CONSUMER_CRATES = Object.freeze([
  "ctx-core",
  "ctx-crp-protocol",
  "ctx-events",
  "ctx-harness-setup",
  "ctx-provider-install",
  "ctx-runtime-assets",
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
  "ctx-worker-gateway": Object.freeze({
    owner: "build-graph",
    rationale: "Workspace member is manual-only and currently has no Bazel BUILD file.",
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
