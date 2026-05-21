const {
  getAllCtxHttpSuiteCheckinFanoutTargets,
  getCtxHttpSuiteTargets,
} = require("./ctx_http_suites.cjs");
const {
  WEB_LINUX_RBE_SAFE_BAZEL_TEST_TARGETS,
  WEB_SMOKE_BAZEL_TARGETS,
} = require("./web_smoke_bazel_targets.cjs");

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
    "//core/crates/ctx-avf-linux-runtime:unit_tests_avf_linux_vm",
  ],
  "ctx-core": [
    "//core/crates/ctx-core:unit_tests",
    "//core/crates/ctx-core:workspace_payload_corpus",
  ],
  "ctx-daemon": [
    "//core/crates/ctx-daemon:unit_tests_daemon",
    "//core/crates/ctx-daemon:unit_tests_execution_effective",
    "//core/crates/ctx-daemon:unit_tests_merge_queue",
    "//core/crates/ctx-daemon:unit_tests_merge_queue_enabled_workspace_resume_after_open",
    "//core/crates/ctx-daemon:unit_tests_scheduler",
    "//core/crates/ctx-daemon:unit_tests_storage_guard",
    "//core/crates/ctx-daemon:unit_tests_vcs_hooks",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_container_status_avf",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_keeps_avf_workspace_container_ready",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_prepare_starts_cached_container",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_reclaim_ctx_harness_container",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_reclaim_idle_machine",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_reclaim_idle_runtime_with_containers",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_reclaim_idle_runtime_with_parked_containers",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_recreates_machine_for_memory_profile_change",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_reuses_running_container",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_running_unreachable_machine_reconfiguration",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_starts_avf_workspace_vm",
    "//core/crates/ctx-daemon:unit_tests_workspace_runtime_unknown_machine_state_engine_unreachable",
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
  "ctx-http-auth": ["//core/crates/ctx-http-auth:unit_tests"],
  "ctx-http-test-support": ["//core/crates/ctx-http-test-support:unit_tests"],
  "ctx-crp-protocol": ["//core/crates/ctx-crp-protocol:unit_tests"],
  "ctx-llm-relay-authority": ["//core/crates/ctx-llm-relay-authority:unit_tests"],
  "ctx-llm-relay-contract": ["//core/crates/ctx-llm-relay-contract:unit_tests"],
  "ctx-load-test": ["//core/tools/load-test:unit_tests"],
  "ctx-managed-installs": ["//core/crates/ctx-managed-installs:unit_tests"],
  "ctx-mcp": [
    "//core/crates/ctx-mcp:mcp_contracts",
    "//core/crates/ctx-mcp:subagent_tools",
  ],
  "ctx-mcp-auth": ["//core/crates/ctx-mcp-auth:unit_tests"],
  "ctx-mcp-command": ["//core/crates/ctx-mcp-command:unit_tests"],
  "ctx-merge-queue": ["//core/crates/ctx-merge-queue:unit_tests"],
  "ctx-mobile-access-service": ["//core/crates/ctx-mobile-access-service:unit_tests"],
  "ctx-observability": ["//core/crates/ctx-observability:unit_tests"],
  "ctx-org-policy": ["//core/crates/ctx-org-policy:unit_tests"],
  "ctx-provider-runtime": ["//core/crates/ctx-provider-runtime:unit_tests"],
  "ctx-repo-onboarding-service": ["//core/crates/ctx-repo-onboarding-service:unit_tests"],
  "ctx-worktree-bootstrap-service": ["//core/crates/ctx-worktree-bootstrap-service:unit_tests"],
  "ctx-worktree-vcs-service": ["//core/crates/ctx-worktree-vcs-service:unit_tests"],
  "ctx-transport-runtime": ["//core/crates/ctx-transport-runtime:unit_tests"],
  "ctx-linux-sandbox-runtime": ["//core/crates/ctx-linux-sandbox-runtime:unit_tests"],
  "ctx-provider-accounts": ["//core/crates/ctx-provider-accounts:unit_tests"],
  "ctx-providers": ["//core/crates/ctx-providers:unit_tests"],
  "ctx-provider-install": ["//core/crates/ctx-provider-install:unit_tests"],
  "ctx-provider-matrix": ["//core/crates/ctx-provider-matrix:unit_tests"],
  "ctx-provider-auth-import": ["//core/crates/ctx-provider-auth-import:unit_tests"],
  "ctx-resource-utilization": ["//core/crates/ctx-resource-utilization:unit_tests"],
  "ctx-route-contracts": ["//core/crates/ctx-route-contracts:unit_tests"],
  "ctx-run-archive-service": ["//core/crates/ctx-run-archive-service:unit_tests"],
  "ctx-run-scheduler": ["//core/crates/ctx-run-scheduler:unit_tests"],
  "ctx-runtime-assets": ["//core/crates/ctx-runtime-assets:unit_tests"],
  "ctx-sandbox-contract": ["//core/crates/ctx-sandbox-contract:unit_tests"],
  "ctx-sandbox-container-runtime": ["//core/crates/ctx-sandbox-container-runtime:unit_tests"],
  "ctx-sandbox-materialization": ["//core/crates/ctx-sandbox-materialization:unit_tests"],
  "ctx-session-artifacts": ["//core/crates/ctx-session-artifacts:unit_tests"],
  "ctx-session-message-service": ["//core/crates/ctx-session-message-service:unit_tests"],
  "ctx-session-runtime": ["//core/crates/ctx-session-runtime:unit_tests"],
  "ctx-session-service": ["//core/crates/ctx-session-service:unit_tests"],
  "ctx-session-title-service": ["//core/crates/ctx-session-title-service:unit_tests"],
  "ctx-session-vcs-service": ["//core/crates/ctx-session-vcs-service:unit_tests"],
  "ctx-subagent-service": ["//core/crates/ctx-subagent-service:unit_tests"],
  "ctx-session-tools": ["//core/crates/ctx-session-tools:unit_tests"],
  "ctx-settings-model": ["//core/crates/ctx-settings-model:unit_tests"],
  "ctx-settings-service": ["//core/crates/ctx-settings-service:unit_tests"],
  "ctx-store": [
    "//core/crates/ctx-store:sqlite_hardening",
    "//core/crates/ctx-store:unit_tests",
  ],
  "ctx-storage-admission": ["//core/crates/ctx-storage-admission:unit_tests"],
  "ctx-task-service": ["//core/crates/ctx-task-service:unit_tests"],
  "ctx-update-service": ["//core/crates/ctx-update-service:unit_tests"],
  "ctx-tunnel-control-plane": ["//core/crates/ctx-tunnel-control-plane:unit_tests"],
  "ctx-tunnel-relay": ["//core/crates/ctx-tunnel-relay:unit_tests"],
  "ctx-tunnel-router": ["//core/crates/ctx-tunnel-router:unit_tests"],
  "ctx-tunnel-store": ["//core/crates/ctx-tunnel-store:unit_tests"],
  "ctx-workspace-attachments": ["//core/crates/ctx-workspace-attachments:unit_tests"],
  "ctx-workspace-config": ["//core/crates/ctx-workspace-config:unit_tests"],
  "ctx-worktree-data-plane": ["//core/crates/ctx-worktree-data-plane:unit_tests"],
  "ctx-workspace-container": ["//core/crates/ctx-workspace-container:unit_tests"],
  "ctx-workspace-active-snapshot": ["//core/crates/ctx-workspace-active-snapshot:unit_tests"],
  "ctx-workspace-stream-service": ["//core/crates/ctx-workspace-stream-service:unit_tests"],
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
  "ctx-daemon": ["//core/crates/ctx-daemon:lib"],
  "ctx-http": ["//core/crates/ctx-http:ctx", "//core/crates/ctx-http:lib"],
  "ctx-http-auth": ["//core/crates/ctx-http-auth:lib"],
  "ctx-http-test-support": ["//core/crates/ctx-http-test-support:lib"],
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
  "ctx-crp-protocol": ["//core/crates/ctx-crp-protocol:lib"],
  "ctx-llm-relay-authority": ["//core/crates/ctx-llm-relay-authority:lib"],
  "ctx-llm-relay-contract": ["//core/crates/ctx-llm-relay-contract:lib"],
  "ctx-load-test": ["//core/tools/load-test:ctx-load-test"],
  "ctx-merge-queue": ["//core/crates/ctx-merge-queue:lib"],
  "ctx-mcp-auth": ["//core/crates/ctx-mcp-auth:lib"],
  "ctx-mcp-command": ["//core/crates/ctx-mcp-command:lib"],
  "ctx-mobile-access-service": ["//core/crates/ctx-mobile-access-service:lib"],
  "ctx-managed-installs": ["//core/crates/ctx-managed-installs:lib"],
  "ctx-observability": ["//core/crates/ctx-observability:lib"],
  "ctx-org-policy": ["//core/crates/ctx-org-policy:lib"],
  "ctx-provider-runtime": ["//core/crates/ctx-provider-runtime:lib"],
  "ctx-repo-onboarding-service": ["//core/crates/ctx-repo-onboarding-service:lib"],
  "ctx-transport-runtime": ["//core/crates/ctx-transport-runtime:lib"],
  "ctx-linux-sandbox-runtime": ["//core/crates/ctx-linux-sandbox-runtime:lib"],
  "ctx-provider-accounts": ["//core/crates/ctx-provider-accounts:lib"],
  "ctx-providers": ["//core/crates/ctx-providers:lib"],
  "ctx-provider-install": ["//core/crates/ctx-provider-install:lib"],
  "ctx-provider-matrix": ["//core/crates/ctx-provider-matrix:lib"],
  "ctx-provider-auth-import": ["//core/crates/ctx-provider-auth-import:lib"],
  "ctx-resource-utilization": ["//core/crates/ctx-resource-utilization:lib"],
  "ctx-route-contracts": ["//core/crates/ctx-route-contracts:lib"],
  "ctx-run-archive-service": ["//core/crates/ctx-run-archive-service:lib"],
  "ctx-run-scheduler": ["//core/crates/ctx-run-scheduler:lib"],
  "ctx-runtime-assets": ["//core/crates/ctx-runtime-assets:lib"],
  "ctx-sandbox-contract": ["//core/crates/ctx-sandbox-contract:lib"],
  "ctx-sandbox-container-runtime": ["//core/crates/ctx-sandbox-container-runtime:lib"],
  "ctx-sandbox-materialization": ["//core/crates/ctx-sandbox-materialization:lib"],
  "ctx-session-artifacts": ["//core/crates/ctx-session-artifacts:lib"],
  "ctx-session-message-service": ["//core/crates/ctx-session-message-service:lib"],
  "ctx-session-runtime": ["//core/crates/ctx-session-runtime:lib"],
  "ctx-session-service": ["//core/crates/ctx-session-service:lib"],
  "ctx-session-title-service": ["//core/crates/ctx-session-title-service:lib"],
  "ctx-session-vcs-service": ["//core/crates/ctx-session-vcs-service:lib"],
  "ctx-subagent-service": ["//core/crates/ctx-subagent-service:lib"],
  "ctx-session-tools": ["//core/crates/ctx-session-tools:lib"],
  "ctx-settings-model": ["//core/crates/ctx-settings-model:lib"],
  "ctx-settings-service": ["//core/crates/ctx-settings-service:lib"],
  "ctx-mcp": ["//core/crates/ctx-mcp:ctx-mcp"],
  "ctx-store": ["//core/crates/ctx-store:lib"],
  "ctx-storage-admission": ["//core/crates/ctx-storage-admission:lib"],
  "ctx-task-service": ["//core/crates/ctx-task-service:lib"],
  "ctx-update-service": ["//core/crates/ctx-update-service:lib"],
  "ctx-tunnel-control-plane": ["//core/crates/ctx-tunnel-control-plane:ctx-tunnel-control-plane"],
  "ctx-tunnel-relay": ["//core/crates/ctx-tunnel-relay:ctx-tunnel-relay"],
  "ctx-tunnel-router": ["//core/crates/ctx-tunnel-router:ctx-tunnel-router"],
  "ctx-tunnel-store": [
    "//core/crates/ctx-tunnel-store:ctx-tunnel-cleanup",
    "//core/crates/ctx-tunnel-store:lib",
  ],
  "ctx-workspace-attachments": ["//core/crates/ctx-workspace-attachments:lib"],
  "ctx-workspace-config": ["//core/crates/ctx-workspace-config:lib"],
  "ctx-worktree-data-plane": ["//core/crates/ctx-worktree-data-plane:lib"],
  "ctx-workspace-container": ["//core/crates/ctx-workspace-container:lib"],
  "ctx-workspace-active-snapshot": ["//core/crates/ctx-workspace-active-snapshot:lib"],
  "ctx-workspace-stream-service": ["//core/crates/ctx-workspace-stream-service:lib"],
  "ctx-worktree-bootstrap-service": ["//core/crates/ctx-worktree-bootstrap-service:lib"],
  "ctx-worktree-vcs-service": ["//core/crates/ctx-worktree-vcs-service:lib"],
  "ctx-workspace-runtime": ["//core/crates/ctx-workspace-runtime:lib"],
});

function getBazelClippyTargetsForCrates(crateNames) {
  // Keep clippy on explicit labels so Linux RBE partitioning can keep known-safe
  // targets remote. Package-wide :all labels do not match the remote-safe allowlist.
  return getBazelBuildTargetsForCrates(crateNames);
}

const LINUX_RBE_UNSAFE_BAZEL_TEST_TARGETS = Object.freeze(
  new Set([
    // This target pulls a Darwin-hosted Rust toolchain helper, which cannot execute on the
    // Linux BuildBuddy workers used by the linux-rbe pool.
    "//core/crates/ctx-avf-linux-guest-agent:unit_tests",
    // Suite aliases hide their concrete children from the Linux RBE partitioner.
    // Route the flattened ctx-http children instead so only known unsafe leaves spill local.
    ...getCtxHttpSuiteTargets("all").filter(
      (target) => !getAllCtxHttpSuiteCheckinFanoutTargets().includes(target),
    ),
  ]),
);

function buildLinuxRbeSafeBazelTestTargets({
  unsafeTargets = LINUX_RBE_UNSAFE_BAZEL_TEST_TARGETS,
} = {}) {
  const unsafeTargetSet = unsafeTargets instanceof Set ? unsafeTargets : new Set(unsafeTargets || []);
  return sortUnique([
    ...flattenTargetMapping(BAZEL_TEST_TARGETS_BY_CRATE),
    ...getAllCtxHttpSuiteCheckinFanoutTargets(),
    ...WEB_LINUX_RBE_SAFE_BAZEL_TEST_TARGETS,
    ...WEB_SMOKE_BAZEL_TARGETS,
  ]).filter((target) => !unsafeTargetSet.has(target));
}

const LINUX_RBE_SAFE_BAZEL_TEST_TARGETS = Object.freeze(
  buildLinuxRbeSafeBazelTestTargets(),
);

// Keep Linux RBE build targets conservative on Mac hosts: libraries are safe to
// compile remotely, but host executables should stay local unless we explicitly
// decide the Linux output is what the caller wants.
const LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS = Object.freeze(
  flattenTargetMapping(BAZEL_BUILD_TARGETS_BY_CRATE).filter((target) => target.endsWith(":lib")),
);

const LINUX_RBE_SAFE_BAZEL_CLIPPY_TARGETS = Object.freeze(
  flattenTargetMapping(BAZEL_BUILD_TARGETS_BY_CRATE),
);

const LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS_WITH_TEST_BINARIES = Object.freeze(sortUnique([
  ...LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS,
  // `bazel build` of test targets only compiles the test binary. Targets that
  // are already safe to execute on Linux RBE are also safe to compile there.
  // Keep this promotion narrow: today only the ctx-http checkin warmup needs
  // test-binary builds before target-granular fanout runs the tests.
  ...getAllCtxHttpSuiteCheckinFanoutTargets(),
]));

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

function getLinuxRbeSafeBazelTargets(command, { rustClippy = false } = {}) {
  if (command === "test") {
    return LINUX_RBE_SAFE_BAZEL_TEST_TARGETS;
  }
  if (command === "build") {
    if (rustClippy) {
      return LINUX_RBE_SAFE_BAZEL_CLIPPY_TARGETS;
    }
    return LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS_WITH_TEST_BINARIES;
  }
  return [];
}

function partitionBazelTargetsForLinuxRbe(command, targets, options = {}) {
  const safeTargets = new Set(getLinuxRbeSafeBazelTargets(command, options));
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
  buildLinuxRbeSafeBazelTestTargets,
  getBazelBuildTargetsForCrates,
  getBazelClippyTargetsForCrates,
  getBazelCoveredCrates,
  getLinuxRbeSafeBazelTargets,
  getBazelTestTargetsForCrates,
  LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS,
  LINUX_RBE_SAFE_BAZEL_BUILD_TARGETS_WITH_TEST_BINARIES,
  LINUX_RBE_SAFE_BAZEL_CLIPPY_TARGETS,
  LINUX_RBE_SAFE_BAZEL_TEST_TARGETS,
  LINUX_RBE_UNSAFE_BAZEL_TEST_TARGETS,
  partitionBazelTargetsForLinuxRbe,
};
