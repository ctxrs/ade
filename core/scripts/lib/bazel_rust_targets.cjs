const { getCtxHttpSuiteTargets } = require("./ctx_http_suites.cjs");

function sortUnique(values) {
  return [...new Set(values)].filter(Boolean).sort();
}

function flattenTargetMapping(mapping) {
  return sortUnique(Object.values(mapping).flat());
}

const BAZEL_TEST_TARGETS_BY_CRATE = Object.freeze({
  "ctx-avf-linux-guest-agent": ["//core/crates/ctx-avf-linux-guest-agent:unit_tests"],
  "codex-crp": ["//core/crates/codex-crp:unit_tests"],
  "ctx-avf-linux-runtime": [
    "//core/crates/ctx-avf-linux-runtime:helper_path_test_support",
    "//core/crates/ctx-avf-linux-runtime:unit_tests",
  ],
  "ctx-core": [
    "//core/crates/ctx-core:unit_tests",
    "//core/crates/ctx-core:workspace_payload_corpus",
  ],
  "ctx-bundled-assets": ["//core/crates/ctx-bundled-assets:unit_tests"],
  "ctx-client": ["//core/crates/ctx-client:unit_tests"],
  "ctx-desktop-ipc": ["//core/crates/ctx-desktop-ipc:typescript_check_test"],
  "ctx-docs-mirror": ["//core/crates/ctx-docs-mirror:unit_tests"],
  "ctx-egress-proxy": ["//core/crates/ctx-egress-proxy:unit_tests"],
  "ctx-execution-runtime": ["//core/crates/ctx-execution-runtime:unit_tests"],
  "ctx-events": ["//core/crates/ctx-events:unit_tests"],
  "ctx-fs": ["//core/crates/ctx-fs:unit_tests"],
  "ctx-harness-setup": ["//core/crates/ctx-harness-setup:unit_tests"],
  "ctx-harness-runtime": ["//core/crates/ctx-harness-runtime:unit_tests"],
  "ctx-harness-sources": ["//core/crates/ctx-harness-sources:unit_tests"],
  "ctx-http": getCtxHttpSuiteTargets("all"),
  "ctx-load-test": ["//core/tools/load-test:unit_tests"],
  "ctx-managed-installs": ["//core/crates/ctx-managed-installs:unit_tests"],
  "ctx-mcp": [
    "//core/crates/ctx-mcp:mcp_contracts",
    "//core/crates/ctx-mcp:subagent_tools",
  ],
  "ctx-merge-queue": ["//core/crates/ctx-merge-queue:unit_tests"],
  "ctx-provider-runtime": ["//core/crates/ctx-provider-runtime:unit_tests"],
  "ctx-workspace-services": ["//core/crates/ctx-workspace-services:unit_tests"],
  "ctx-transport-runtime": ["//core/crates/ctx-transport-runtime:unit_tests"],
  "ctx-linux-sandbox-runtime": ["//core/crates/ctx-linux-sandbox-runtime:unit_tests"],
  "ctx-provider-accounts": ["//core/crates/ctx-provider-accounts:unit_tests"],
  "ctx-providers": ["//core/crates/ctx-providers:unit_tests"],
  "ctx-provider-install": ["//core/crates/ctx-provider-install:unit_tests"],
  "ctx-provider-matrix": ["//core/crates/ctx-provider-matrix:unit_tests"],
  "ctx-provider-auth-import": ["//core/crates/ctx-provider-auth-import:unit_tests"],
  "ctx-runtime-assets": ["//core/crates/ctx-runtime-assets:unit_tests"],
  "ctx-sandbox-contract": ["//core/crates/ctx-sandbox-contract:unit_tests"],
  "ctx-sandbox-container-runtime": ["//core/crates/ctx-sandbox-container-runtime:unit_tests"],
  "ctx-sandbox-materialization": ["//core/crates/ctx-sandbox-materialization:unit_tests"],
  "ctx-session-tools": ["//core/crates/ctx-session-tools:unit_tests"],
  "ctx-store": [
    "//core/crates/ctx-store:sqlite_hardening",
    "//core/crates/ctx-store:unit_tests",
  ],
  "ctx-storage-admission": ["//core/crates/ctx-storage-admission:unit_tests"],
  "ctx-tunnel-control-plane": ["//core/crates/ctx-tunnel-control-plane:unit_tests"],
  "ctx-tunnel-relay": ["//core/crates/ctx-tunnel-relay:unit_tests"],
  "ctx-tunnel-router": ["//core/crates/ctx-tunnel-router:unit_tests"],
  "ctx-worker-protocol": ["//core/crates/ctx-worker-protocol:unit_tests"],
  "ctx-worker-shim": ["//core/crates/ctx-worker-shim:unit_tests"],
  "ctx-workspace-config": ["//core/crates/ctx-workspace-config:unit_tests"],
  "ctx-worktree-data-plane": ["//core/crates/ctx-worktree-data-plane:unit_tests"],
  "ctx-workspace-container": ["//core/crates/ctx-workspace-container:unit_tests"],
  "ctx-workspace-active-snapshot": ["//core/crates/ctx-workspace-active-snapshot:unit_tests"],
  "ctx-workspace-runtime": [
    "//core/crates/ctx-workspace-runtime:unit_tests",
    "//core/crates/ctx-workspace-runtime:workspace_runtime_crash_recovery",
  ],
});

const BAZEL_BUILD_TARGETS_BY_CRATE = Object.freeze({
  "ctx-avf-linux-helper": ["//core/apps/desktop/src-tauri/src:ctx-avf-linux-helper"],
  "ctx-avf-linux-guest-agent": ["//core/crates/ctx-avf-linux-guest-agent:ctx-avf-linux-guest-agent"],
  "codex-crp": ["//core/crates/codex-crp:codex-crp"],
  "ctx-avf-linux-runtime": ["//core/crates/ctx-avf-linux-runtime:lib"],
  "ctx-core": ["//core/crates/ctx-core:lib"],
  "ctx-http": ["//core/crates/ctx-http:ctx", "//core/crates/ctx-http:lib"],
  "ctx-bundled-assets": ["//core/crates/ctx-bundled-assets:lib"],
  "ctx-client": ["//core/crates/ctx-client:lib"],
  "ctx-desktop-ipc": ["//core/crates/ctx-desktop-ipc:lib"],
  "ctx-docs-mirror": ["//core/crates/ctx-docs-mirror:ctx-docs-mirror"],
  "ctx-egress-proxy": ["//core/crates/ctx-egress-proxy:ctx-egress-proxy"],
  "ctx-execution-runtime": ["//core/crates/ctx-execution-runtime:lib"],
  "ctx-events": ["//core/crates/ctx-events:lib"],
  "ctx-fs": ["//core/crates/ctx-fs:lib"],
  "ctx-harness-setup": ["//core/crates/ctx-harness-setup:lib"],
  "ctx-harness-runtime": ["//core/crates/ctx-harness-runtime:lib"],
  "ctx-harness-sources": ["//core/crates/ctx-harness-sources:lib"],
  "ctx-load-test": ["//core/tools/load-test:ctx-load-test"],
  "ctx-merge-queue": ["//core/crates/ctx-merge-queue:lib"],
  "ctx-managed-installs": ["//core/crates/ctx-managed-installs:lib"],
  "ctx-provider-runtime": ["//core/crates/ctx-provider-runtime:lib"],
  "ctx-workspace-services": ["//core/crates/ctx-workspace-services:lib"],
  "ctx-transport-runtime": ["//core/crates/ctx-transport-runtime:lib"],
  "ctx-linux-sandbox-runtime": ["//core/crates/ctx-linux-sandbox-runtime:lib"],
  "ctx-provider-accounts": ["//core/crates/ctx-provider-accounts:lib"],
  "ctx-providers": ["//core/crates/ctx-providers:lib"],
  "ctx-provider-install": ["//core/crates/ctx-provider-install:lib"],
  "ctx-provider-matrix": ["//core/crates/ctx-provider-matrix:lib"],
  "ctx-provider-auth-import": ["//core/crates/ctx-provider-auth-import:lib"],
  "ctx-runtime-assets": ["//core/crates/ctx-runtime-assets:lib"],
  "ctx-sandbox-contract": ["//core/crates/ctx-sandbox-contract:lib"],
  "ctx-sandbox-container-runtime": ["//core/crates/ctx-sandbox-container-runtime:lib"],
  "ctx-sandbox-materialization": ["//core/crates/ctx-sandbox-materialization:lib"],
  "ctx-session-tools": ["//core/crates/ctx-session-tools:lib"],
  "ctx-mcp": ["//core/crates/ctx-mcp:ctx-mcp"],
  "ctx-store": ["//core/crates/ctx-store:lib"],
  "ctx-storage-admission": ["//core/crates/ctx-storage-admission:lib"],
  "ctx-tunnel-control-plane": ["//core/crates/ctx-tunnel-control-plane:ctx-tunnel-control-plane"],
  "ctx-tunnel-relay": ["//core/crates/ctx-tunnel-relay:ctx-tunnel-relay"],
  "ctx-tunnel-router": ["//core/crates/ctx-tunnel-router:ctx-tunnel-router"],
  "ctx-worker-protocol": ["//core/crates/ctx-worker-protocol:lib"],
  "ctx-worker-shim": ["//core/crates/ctx-worker-shim:ctx-worker-shim"],
  "ctx-workspace-config": ["//core/crates/ctx-workspace-config:lib"],
  "ctx-worktree-data-plane": ["//core/crates/ctx-worktree-data-plane:lib"],
  "ctx-workspace-container": ["//core/crates/ctx-workspace-container:lib"],
  "ctx-workspace-active-snapshot": ["//core/crates/ctx-workspace-active-snapshot:lib"],
  "ctx-workspace-runtime": ["//core/crates/ctx-workspace-runtime:lib"],
});

const LINUX_RBE_UNSAFE_BAZEL_TEST_TARGETS = Object.freeze(
  new Set([
    // This target pulls a Darwin-hosted Rust toolchain helper, which cannot execute on the
    // Linux BuildBuddy workers used by the linux-rbe pool.
    "//core/crates/ctx-avf-linux-guest-agent:unit_tests",
  ]),
);

const LINUX_RBE_SAFE_BAZEL_TEST_TARGETS = Object.freeze(
  flattenTargetMapping(BAZEL_TEST_TARGETS_BY_CRATE).filter(
    (target) => !LINUX_RBE_UNSAFE_BAZEL_TEST_TARGETS.has(target),
  ),
);

// Keep Linux RBE build targets conservative on Mac hosts: libraries are safe to
// compile remotely, but host executables should stay local unless we explicitly
// decide the Linux output is what the caller wants.
const LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS = Object.freeze(
  flattenTargetMapping(BAZEL_BUILD_TARGETS_BY_CRATE).filter((target) => target.endsWith(":lib")),
);

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

function getLinuxRbeSafeBazelTargets(command) {
  if (command === "test") {
    return LINUX_RBE_SAFE_BAZEL_TEST_TARGETS;
  }
  if (command === "build") {
    return LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS;
  }
  return [];
}

function partitionBazelTargetsForLinuxRbe(command, targets) {
  const safeTargets = new Set(getLinuxRbeSafeBazelTargets(command));
  const remoteTargets = [];
  const localTargets = [];
  for (const target of sortUnique(targets)) {
    if (safeTargets.has(target)) {
      remoteTargets.push(target);
    } else {
      localTargets.push(target);
    }
  }
  return {
    remoteTargets,
    localTargets,
  };
}

module.exports = {
  BAZEL_BUILD_TARGETS_BY_CRATE,
  BAZEL_TEST_TARGETS_BY_CRATE,
  getBazelBuildTargetsForCrates,
  getBazelCoveredCrates,
  getLinuxRbeSafeBazelTargets,
  getBazelTestTargetsForCrates,
  LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS,
  LINUX_RBE_SAFE_BAZEL_TEST_TARGETS,
  partitionBazelTargetsForLinuxRbe,
};
