function sortUnique(values) {
  return [...new Set(values)].filter(Boolean).sort();
}

const BAZEL_TEST_TARGETS_BY_CRATE = Object.freeze({
  "ctx-avf-linux-runtime": ["//core/crates/ctx-avf-linux-runtime:unit_tests"],
  "ctx-core": [
    "//core/crates/ctx-core:unit_tests",
    "//core/crates/ctx-core:workspace_payload_corpus",
  ],
  "ctx-bundled-assets": ["//core/crates/ctx-bundled-assets:unit_tests"],
  "ctx-fs": ["//core/crates/ctx-fs:unit_tests"],
  "ctx-harness-setup": ["//core/crates/ctx-harness-setup:unit_tests"],
  "ctx-harness-runtime": ["//core/crates/ctx-harness-runtime:unit_tests"],
  "ctx-harness-sources": ["//core/crates/ctx-harness-sources:unit_tests"],
  "ctx-lsp": ["//core/crates/ctx-lsp:lsp_manager_smoke"],
  "ctx-linux-sandbox-runtime": ["//core/crates/ctx-linux-sandbox-runtime:unit_tests"],
  "ctx-provider-accounts": ["//core/crates/ctx-provider-accounts:unit_tests"],
  "ctx-provider-matrix": ["//core/crates/ctx-provider-matrix:unit_tests"],
  "ctx-provider-auth-import": ["//core/crates/ctx-provider-auth-import:unit_tests"],
  "ctx-providers": ["//core/crates/ctx-providers:unit_tests"],
  "ctx-runtime-assets": ["//core/crates/ctx-runtime-assets:unit_tests"],
  "ctx-sandbox-contract": ["//core/crates/ctx-sandbox-contract:unit_tests"],
  "ctx-sandbox-container-runtime": ["//core/crates/ctx-sandbox-container-runtime:unit_tests"],
  "ctx-sandbox-materialization": ["//core/crates/ctx-sandbox-materialization:unit_tests"],
  "ctx-session-tools": ["//core/crates/ctx-session-tools:unit_tests"],
  "ctx-storage-admission": ["//core/crates/ctx-storage-admission:unit_tests"],
  "ctx-worktree-data-plane": ["//core/crates/ctx-worktree-data-plane:unit_tests"],
  "ctx-workspace-container": ["//core/crates/ctx-workspace-container:unit_tests"],
  "ctx-workspace-active-snapshot": ["//core/crates/ctx-workspace-active-snapshot:unit_tests"],
});

const BAZEL_BUILD_TARGETS_BY_CRATE = Object.freeze({
  "ctx-avf-linux-runtime": ["//core/crates/ctx-avf-linux-runtime:lib"],
  "ctx-core": ["//core/crates/ctx-core:lib"],
  "ctx-bundled-assets": ["//core/crates/ctx-bundled-assets:lib"],
  "ctx-fs": ["//core/crates/ctx-fs:lib"],
  "ctx-harness-setup": ["//core/crates/ctx-harness-setup:lib"],
  "ctx-harness-runtime": ["//core/crates/ctx-harness-runtime:lib"],
  "ctx-harness-sources": ["//core/crates/ctx-harness-sources:lib"],
  "ctx-lsp": ["//core/crates/ctx-lsp:ctx-lsp-test-server", "//core/crates/ctx-lsp:lib"],
  "ctx-linux-sandbox-runtime": ["//core/crates/ctx-linux-sandbox-runtime:lib"],
  "ctx-provider-accounts": ["//core/crates/ctx-provider-accounts:lib"],
  "ctx-provider-matrix": ["//core/crates/ctx-provider-matrix:lib"],
  "ctx-provider-auth-import": ["//core/crates/ctx-provider-auth-import:lib"],
  "ctx-providers": ["//core/crates/ctx-providers:lib"],
  "ctx-runtime-assets": ["//core/crates/ctx-runtime-assets:lib"],
  "ctx-sandbox-contract": ["//core/crates/ctx-sandbox-contract:lib"],
  "ctx-sandbox-container-runtime": ["//core/crates/ctx-sandbox-container-runtime:lib"],
  "ctx-sandbox-materialization": ["//core/crates/ctx-sandbox-materialization:lib"],
  "ctx-session-tools": ["//core/crates/ctx-session-tools:lib"],
  "ctx-storage-admission": ["//core/crates/ctx-storage-admission:lib"],
  "ctx-worktree-data-plane": ["//core/crates/ctx-worktree-data-plane:lib"],
  "ctx-workspace-container": ["//core/crates/ctx-workspace-container:lib"],
  "ctx-workspace-active-snapshot": ["//core/crates/ctx-workspace-active-snapshot:lib"],
});

function getBazelCoveredCrates() {
  return Object.keys(BAZEL_TEST_TARGETS_BY_CRATE).sort();
}

function getBazelTargetsForCrates(mapping, crateNames) {
  const targets = [];
  for (const crateName of sortUnique(crateNames)) {
    const crateTargets = mapping[crateName];
    if (!crateTargets) {
      continue;
    }
    targets.push(...crateTargets);
  }
  return sortUnique(targets);
}

function getBazelTestTargetsForCrates(crateNames) {
  return getBazelTargetsForCrates(BAZEL_TEST_TARGETS_BY_CRATE, crateNames);
}

function getBazelBuildTargetsForCrates(crateNames) {
  return getBazelTargetsForCrates(BAZEL_BUILD_TARGETS_BY_CRATE, crateNames);
}

module.exports = {
  BAZEL_BUILD_TARGETS_BY_CRATE,
  BAZEL_TEST_TARGETS_BY_CRATE,
  getBazelBuildTargetsForCrates,
  getBazelCoveredCrates,
  getBazelTestTargetsForCrates,
};
