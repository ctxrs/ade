const fs = require("node:fs");
const path = require("node:path");

const CTX_HTTP_SUITE_PREFIX = "rust:ctx-http:test:";
const CTX_HTTP_BAZEL_PACKAGE = "//core/crates/ctx-http";
const CTX_HTTP_SUITE_SCRIPT_INPUTS = [
  "crates/ctx-http/BUILD.bazel",
  "crates/ctx-http/ctx_http_bazel_tests.bzl",
  "scripts/ctx_http_suite_task.cjs",
  "scripts/lib/ctx_http_suites.cjs",
];
const MANUAL_ONLY_CTX_HTTP_TEST_FILES = new Set([
  "attachments_demo_react",
  "cloud_gateway_azure_e2e",
  "cloud_gateway_gcp_e2e",
]);
const CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS = [`${CTX_HTTP_BAZEL_PACKAGE}:manual-only`];
const CTX_HTTP_SHARED_SOURCE_GLOBS = [
  "crates/ctx-http/src/api/auth.rs",
  "crates/ctx-http/src/api/errors.rs",
  "crates/ctx-http/src/api/mod.rs",
  "crates/ctx-http/src/api/routes.rs",
  "crates/ctx-http/src/api/shared.rs",
  "crates/ctx-http/src/api/types.rs",
  "crates/ctx-http/src/async_util.rs",
  "crates/ctx-http/src/daemon.rs",
  "crates/ctx-http/src/daemon/**",
  "crates/ctx-http/src/lib.rs",
  "crates/ctx-http/src/logs.rs",
  "crates/ctx-http/src/telemetry.rs",
  "crates/ctx-http/src/test_support.rs",
];

const CTX_HTTP_SUITES = [
  {
    dependencyCrates: ["ctx-http"],
    name: "base",
    description: "ctx-http lib, bins, and doc tests",
    sourceGlobs: [],
    testFiles: [],
    type: "base",
  },
  {
    dependencyCrates: ["ctx-core", "ctx-events", "ctx-store", "ctx-workspace-active-snapshot"],
    name: "workspace-stream",
    description: "workspace snapshot, stream, cache, and replay behavior",
    sourceGlobs: [
      "crates/ctx-http/src/api/sessions/snapshot.rs",
      "crates/ctx-http/src/api/tasks/snapshot_state.rs",
      "crates/ctx-http/src/api/workspaces.rs",
      "crates/ctx-http/src/api/ws.rs",
      "crates/ctx-http/src/api/ws/**",
      "crates/ctx-http/src/daemon/workspaces/stream.rs",
      "crates/ctx-http/src/order_seq.rs",
    ],
    testFiles: [
      "cache_rehydration",
      "fault_matrix",
      "hot_endpoints_no_db",
      "replay_properties",
      "task_default_session_http",
      "workspace_active_snapshot_http",
      "workspace_stream_context_window_metrics",
      "workspace_stream_no_gaps_under_activity",
      "workspace_stream_stress_active_heads_lag",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-harness-sources",
      "ctx-provider-accounts",
      "ctx-provider-auth-import",
      "ctx-provider-install",
      "ctx-managed-installs",
      "ctx-providers",
      "ctx-store",
    ],
    name: "provider-auth",
    description: "provider install, auth callback, and account status flows",
    sourceGlobs: [
      "crates/ctx-http/src/api/provider_probe_auth.rs",
      "crates/ctx-http/src/api/providers.rs",
      "crates/ctx-http/src/api/providers/accounts.rs",
      "crates/ctx-http/src/api/providers/bootstrap.rs",
      "crates/ctx-http/src/api/providers/cursor_login.rs",
      "crates/ctx-http/src/api/providers/harness_config.rs",
      "crates/ctx-http/src/api/providers/imports.rs",
      "crates/ctx-http/src/api/providers/install.rs",
      "crates/ctx-http/src/api/providers/login.rs",
      "crates/ctx-http/src/api/providers/login/**",
      "crates/ctx-http/src/installer/provider_install.rs",
      "crates/ctx-http/src/provider_install_contract.rs",
    ],
    testFiles: [
      "acp_target_scoped_status",
      "codex_host_import_api",
      "codex_login_callback_api",
      "install_start_contract",
      "provider_current_ctx_version_regressions",
      "provider_target_scoped_installs",
      "subscription_accounts_api",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-execution-runtime",
      "ctx-harness-sources",
      "ctx-managed-installs",
      "ctx-provider-accounts",
      "ctx-providers",
      "ctx-provider-runtime",
      "ctx-store",
      "ctx-workspace-runtime",
    ],
    name: "provider-runtime-simulated",
    description: "provider runtime, model selection, and offline simulated scenarios",
    sourceGlobs: [
      "crates/ctx-http/src/api/provider_catalog.rs",
      "crates/ctx-http/src/api/provider_launch.rs",
      "crates/ctx-http/src/api/provider_launch/**",
      "crates/ctx-http/src/api/providers/probe.rs",
      "crates/ctx-http/src/api/providers/status.rs",
      "crates/ctx-http/src/api/sessions/models.rs",
      "crates/ctx-http/src/llm.rs",
      "crates/ctx-http/src/provider_guard.rs",
      "crates/ctx-http/src/provider_launch/**",
      "crates/ctx-http/src/provider_matrix.rs",
      "crates/ctx-http/src/provider_model_preferences.rs",
      "crates/ctx-http/src/provider_restart.rs",
      "crates/ctx-http/src/provider_usage.rs",
      "crates/ctx-http/src/workspace_provider_model_preferences.rs",
      "crates/ctx-http/src/workspace_runtime/**",
    ],
    testFiles: [
      "provider_probe_runtime_env",
      "provider_worker_reaping_offline",
      "provider_scenarios_offline",
      "session_model_api",
      "workspace_provider_model_preferences_http",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-execution-runtime",
      "ctx-harness-sources",
      "ctx-managed-installs",
      "ctx-provider-accounts",
      "ctx-providers",
      "ctx-provider-runtime",
      "ctx-store",
      "ctx-workspace-runtime",
    ],
    name: "provider-runtime-live",
    description: "provider runtime flows that require live-provider or bridge truth",
    sourceGlobs: [
      "crates/ctx-http/src/api/provider_catalog.rs",
      "crates/ctx-http/src/api/provider_launch.rs",
      "crates/ctx-http/src/api/provider_launch/**",
      "crates/ctx-http/src/api/providers/probe.rs",
      "crates/ctx-http/src/api/providers/status.rs",
      "crates/ctx-http/src/api/sessions/models.rs",
      "crates/ctx-http/src/llm.rs",
      "crates/ctx-http/src/provider_guard.rs",
      "crates/ctx-http/src/provider_launch/**",
      "crates/ctx-http/src/provider_matrix.rs",
      "crates/ctx-http/src/provider_model_preferences.rs",
      "crates/ctx-http/src/provider_restart.rs",
      "crates/ctx-http/src/provider_usage.rs",
      "crates/ctx-http/src/workspace_provider_model_preferences.rs",
      "crates/ctx-http/src/workspace_runtime/**",
    ],
    testFiles: [
      "acp_crp_bridge_tokens_e2e",
      "gemini_live_model_catalog",
      "live_provider_canary",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-events",
      "ctx-fs",
      "ctx-merge-queue",
      "ctx-store",
      "ctx-workspace-config",
      "ctx-workspace-services",
      "ctx-worktree-data-plane",
    ],
    name: "repo-vcs",
    description: "repo initialization, worktree state, merge queue, and VCS snapshots",
    sourceGlobs: [
      "crates/ctx-http/src/api/merge_queue_api.rs",
      "crates/ctx-http/src/api/repo.rs",
      "crates/ctx-http/src/api/sessions/diff_exec.rs",
      "crates/ctx-http/src/git_status.rs",
      "crates/ctx-http/src/git_status/**",
      "crates/ctx-http/src/git_status_watch.rs",
      "crates/ctx-http/src/merge_queue.rs",
      "crates/ctx-http/src/merge_queue/**",
      "crates/ctx-http/src/vcs_hooks.rs",
      "crates/ctx-http/src/workspace_config.rs",
      "crates/ctx-http/src/worktree_bootstrap.rs",
      "crates/ctx-http/src/worktree_data_plane.rs",
    ],
    testFiles: [
      "jj_merge_queue_basics",
      "merge_queue_isolation",
      "repo_clone_branch_and_safety",
      "repo_init_initial_commit",
      "repo_validate_destination",
      "session_diff_unavailable",
      "workspace_merge_queue_config_http",
      "workspace_execution_config_http",
      "worktree_archive_http",
      "worktree_vcs_snapshot",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-events",
      "ctx-store",
      "ctx-transport-runtime",
      "ctx-worker-protocol",
    ],
    name: "scheduler-runtime",
    description: "scheduler runtime, turn lifecycle, and stream backpressure flows",
    sourceGlobs: [
      "crates/ctx-http/src/api/execution.rs",
      "crates/ctx-http/src/api/sessions/control.rs",
      "crates/ctx-http/src/api/sessions/messages.rs",
      "crates/ctx-http/src/api/sessions/mod.rs",
      "crates/ctx-http/src/api/terminals.rs",
      "crates/ctx-http/src/api/terminals/**",
      "crates/ctx-http/src/completions.rs",
      "crates/ctx-http/src/ops_events.rs",
      "crates/ctx-http/src/order_seq.rs",
      "crates/ctx-http/src/scheduler.rs",
      "crates/ctx-http/src/scheduler/**",
    ],
    testFiles: [
      "assistant_chunk_stream_only",
      "assistant_message_persistence_faults",
      "noisy_output_backpressure",
      "turn_lifecycle_events",
      "turn_terminal_reconciliation",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-events",
      "ctx-store",
      "ctx-transport-runtime",
      "ctx-worker-protocol",
    ],
    name: "turns-terminal",
    description: "turn lifecycle, terminal, streaming, and message durability flows",
    sourceGlobs: [
      "crates/ctx-http/src/api/execution.rs",
      "crates/ctx-http/src/api/sessions/control.rs",
      "crates/ctx-http/src/api/sessions/messages.rs",
      "crates/ctx-http/src/api/sessions/mod.rs",
      "crates/ctx-http/src/api/sessions/titles_and_modes.rs",
      "crates/ctx-http/src/api/terminals.rs",
      "crates/ctx-http/src/api/terminals/**",
      "crates/ctx-http/src/api/web_sessions.rs",
      "crates/ctx-http/src/completions.rs",
      "crates/ctx-http/src/daemon/sessions/title_generation.rs",
      "crates/ctx-http/src/ops_events.rs",
      "crates/ctx-http/src/order_seq.rs",
      "crates/ctx-http/src/terminal_launch.rs",
      "crates/ctx-http/src/terminal_launch/**",
      "crates/ctx-http/src/terminals.rs",
      "crates/ctx-http/src/web_session_launch.rs",
      "crates/ctx-http/src/web_sessions.rs",
    ],
    testFiles: [
      "demo_seed_transcript_http",
      "message_idempotency",
      "terminal_workspace_stream_separation",
      "terminal_ws_reconnect",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-events",
      "ctx-managed-installs",
      "ctx-store",
      "ctx-transport-runtime",
      "ctx-worker-protocol",
      "ctx-workspace-services",
    ],
    name: "attachments-routing",
    description: "artifact uploads, attachment materialization, and route-scoping coverage",
    sourceGlobs: [
      "crates/ctx-http/src/api/artifacts.rs",
      "crates/ctx-http/src/api/demo.rs",
      "crates/ctx-http/src/attachments.rs",
      "crates/ctx-http/src/attachments/**",
      "crates/ctx-http/src/daemon/workspaces/attachments.rs",
      "crates/ctx-http/src/storage_guard.rs",
      "crates/ctx-http/src/worktree_data_plane.rs",
    ],
    testFiles: [
      "global_id_routing_http",
      "image_attachments_http_e2e",
      "workspace_attachments_local_canonical",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-events",
      "ctx-managed-installs",
      "ctx-store",
      "ctx-transport-runtime",
      "ctx-worker-protocol",
      "ctx-workspace-services",
    ],
    name: "subagents-control",
    description: "subagent orchestration, MCP/oracle, and title-path control flows",
    sourceGlobs: [
      "crates/ctx-http/src/api/mobile_access.rs",
      "crates/ctx-http/src/api/mobile_access/**",
      "crates/ctx-http/src/api/sessions/subagents.rs",
      "crates/ctx-http/src/api/sessions/subagents/**",
      "crates/ctx-http/src/daemon/sessions/subagents.rs",
      "crates/ctx-http/src/mcp_command.rs",
      "crates/ctx-http/src/oracle.rs",
      "crates/ctx-http/src/title_generation.rs",
      "crates/ctx-http/src/title_generation_local.rs",
    ],
    testFiles: [
      "subagent_mcp_http",
      "system_prompt_append_http",
      "title_generation_local",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-events",
      "ctx-managed-installs",
      "ctx-store",
      "ctx-transport-runtime",
      "ctx-worker-protocol",
      "ctx-workspace-services",
    ],
    name: "subagents-local-runtime",
    description: "real local title-generation runtime flows",
    sourceGlobs: [
      "crates/ctx-http/src/title_generation.rs",
      "crates/ctx-http/src/title_generation_local.rs",
    ],
    testFiles: [
      "title_generation_local_e2e",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-events",
      "ctx-managed-installs",
      "ctx-store",
      "ctx-transport-runtime",
      "ctx-worker-protocol",
      "ctx-workspace-services",
    ],
    name: "updates-release",
    description: "updates, manifests, release safety, and auxiliary response flows",
    sourceGlobs: [
      "crates/ctx-http/src/api/updates.rs",
      "crates/ctx-http/src/bundled_assets.rs",
      "crates/ctx-http/src/bundled_assets/**",
      "crates/ctx-http/src/updates.rs",
    ],
    testFiles: [
      "openai_responses_sse_stub",
      "release_manifest_corpus",
      "updates_appimage_apply_safety",
      "updates_failure_safety_checksum_mismatch",
      "updates_failure_safety_interrupted_transfer",
      "updates_failure_safety_manifest_parse",
      "updates_failure_safety_manifest_signature",
      "updates_failure_safety_missing_artifact",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-execution-runtime",
      "ctx-events",
      "ctx-fs",
      "ctx-providers",
      "ctx-store",
      "ctx-worker-protocol",
      "ctx-workspace-runtime",
    ],
    name: "sandbox-runtime-simulated",
    description: "sandbox/runtime recovery flows that stay in a simulated local world",
    sourceGlobs: [
      "crates/ctx-http/src/container_builder.rs",
      "crates/ctx-http/src/container_fs.rs",
      "crates/ctx-http/src/dictation_livekit.rs",
      "crates/ctx-http/src/disk_isolated.rs",
      "crates/ctx-http/src/disk_isolated_copy.rs",
      "crates/ctx-http/src/disk_isolated_sandbox.rs",
      "crates/ctx-http/src/disk_isolated_storage.rs",
      "crates/ctx-http/src/execution_effective.rs",
      "crates/ctx-http/src/execution_setup.rs",
      "crates/ctx-http/src/execution_setup/**",
      "crates/ctx-http/src/network_allowlist.rs",
      "crates/ctx-http/src/resource_governance.rs",
      "crates/ctx-http/src/resource_telemetry.rs",
      "crates/ctx-http/src/resource_utilization.rs",
      "crates/ctx-http/src/tool_cgroup.rs",
      "crates/ctx-http/src/workspace_runtime/**",
    ],
    testFiles: [
      "workspace_runtime_crash_recovery",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-execution-runtime",
      "ctx-events",
      "ctx-fs",
      "ctx-providers",
      "ctx-store",
      "ctx-worker-protocol",
      "ctx-workspace-runtime",
    ],
    name: "sandbox-runtime-container-e2e",
    description: "containerized sandbox and isolated filesystem end-to-end flows",
    sourceGlobs: [
      "crates/ctx-http/src/container_builder.rs",
      "crates/ctx-http/src/container_fs.rs",
      "crates/ctx-http/src/disk_isolated.rs",
      "crates/ctx-http/src/disk_isolated_copy.rs",
      "crates/ctx-http/src/disk_isolated_sandbox.rs",
      "crates/ctx-http/src/disk_isolated_storage.rs",
      "crates/ctx-http/src/execution_effective.rs",
      "crates/ctx-http/src/execution_setup.rs",
      "crates/ctx-http/src/execution_setup/**",
      "crates/ctx-http/src/network_allowlist.rs",
      "crates/ctx-http/src/workspace_runtime/**",
    ],
    testFiles: [
      "disk_isolated_sandbox_smoke",
      "disk_isolated_vcs_integrity",
      "harness_container_sandbox_e2e",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-execution-runtime",
      "ctx-events",
      "ctx-fs",
      "ctx-providers",
      "ctx-store",
      "ctx-worker-protocol",
      "ctx-workspace-runtime",
    ],
    name: "sandbox-runtime-resource-governance",
    description: "resource governance and cgroup/systemd enforcement flows",
    sourceGlobs: [
      "crates/ctx-http/src/execution_effective.rs",
      "crates/ctx-http/src/resource_governance.rs",
      "crates/ctx-http/src/resource_telemetry.rs",
      "crates/ctx-http/src/resource_utilization.rs",
      "crates/ctx-http/src/tool_cgroup.rs",
    ],
    testFiles: [
      "resource_governance_systemd_e2e",
    ],
    type: "integration",
  },
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-execution-runtime",
      "ctx-events",
      "ctx-fs",
      "ctx-providers",
      "ctx-store",
      "ctx-worker-protocol",
      "ctx-workspace-runtime",
    ],
    name: "sandbox-runtime-memory-leak",
    description: "sandbox/runtime memory pressure and leak detection flows",
    sourceGlobs: [
      "crates/ctx-http/src/execution_setup.rs",
      "crates/ctx-http/src/execution_setup/**",
      "crates/ctx-http/src/resource_telemetry.rs",
      "crates/ctx-http/src/resource_utilization.rs",
      "crates/ctx-http/src/workspace_runtime/**",
    ],
    testFiles: [
      "memory_leak_e2e",
    ],
    type: "integration",
  },
];

function getCtxHttpSuiteNames(options = {}) {
  const names = CTX_HTTP_SUITES.map((suite) => suite.name);
  if (options.includeAll) {
    return [...names, "all"];
  }
  return names;
}

function getCtxHttpSuiteTaskName(suiteName) {
  return `${CTX_HTTP_SUITE_PREFIX}${suiteName}`;
}

function getCtxHttpSuiteByName(suiteName) {
  if (suiteName === "all") {
    return {
      name: "all",
      description: "all ctx-http suite tasks",
      testFiles: [],
      type: "meta",
    };
  }
  return CTX_HTTP_SUITES.find((suite) => suite.name === suiteName) || null;
}

function listCtxHttpIntegrationTests(coreRoot) {
  const testsDir = path.join(coreRoot, "crates", "ctx-http", "tests");
  return fs
    .readdirSync(testsDir, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".rs"))
    .map((entry) => entry.name.replace(/\.rs$/u, ""))
    .sort();
}

function validateCtxHttpSuites(coreRoot) {
  const assignedByFile = new Map();
  for (const suite of CTX_HTTP_SUITES) {
    if (suite.type !== "integration") {
      continue;
    }
    for (const testFile of suite.testFiles) {
      const owners = assignedByFile.get(testFile) || [];
      owners.push(suite.name);
      assignedByFile.set(testFile, owners);
    }
  }

  const actualFiles = listCtxHttpIntegrationTests(coreRoot);
  const duplicates = [...assignedByFile.entries()]
    .filter(([, owners]) => owners.length > 1)
    .map(([testFile, owners]) => ({
      owners,
      testFile,
    }));
  const manualOnly = actualFiles.filter((testFile) => MANUAL_ONLY_CTX_HTTP_TEST_FILES.has(testFile));
  const missing = actualFiles.filter(
    (testFile) => !assignedByFile.has(testFile) && !MANUAL_ONLY_CTX_HTTP_TEST_FILES.has(testFile),
  );
  const unknown = [...assignedByFile.keys()].filter((testFile) => !actualFiles.includes(testFile));
  return {
    duplicates,
    manualOnly,
    missing,
    unknown,
  };
}

function normalizeCtxHttpSuiteSelection(suiteSelection) {
  const suiteNames = (Array.isArray(suiteSelection) ? suiteSelection : [suiteSelection])
    .map((suiteName) => String(suiteName || "").trim())
    .filter(Boolean);
  if (suiteNames.length === 0) {
    throw new Error(`missing ctx-http suite selection; expected one of ${getCtxHttpSuiteNames({ includeAll: true }).join(", ")}`);
  }
  if (suiteNames.includes("all") && suiteNames.length > 1) {
    throw new Error("ctx-http suite selection cannot mix 'all' with explicit suites");
  }
  for (const suiteName of suiteNames) {
    if (suiteName === "all") {
      continue;
    }
    if (!getCtxHttpSuiteByName(suiteName)) {
      throw new Error(`unknown ctx-http suite: ${suiteName}`);
    }
  }
  return suiteNames;
}

function buildCtxHttpSuiteCommands(suiteName) {
  const targets = getCtxHttpSuiteTargets(suiteName);
  return [{
    args: ["scripts/run_bazel_pilot.cjs", "test", ...targets],
    command: "node",
  }];
}

function getCtxHttpSuiteTarget(suiteName) {
  const suite = getCtxHttpSuiteByName(suiteName);
  if (!suite) {
    throw new Error(`unknown ctx-http suite: ${suiteName}`);
  }
  return `${CTX_HTTP_BAZEL_PACKAGE}:${suite.name}`;
}

function getCtxHttpSuiteTargets(suiteName) {
  const suiteNames = normalizeCtxHttpSuiteSelection(suiteName);
  if (suiteNames.length === 1 && suiteNames[0] === "all") {
    return CTX_HTTP_SUITES.map((suite) => getCtxHttpSuiteTarget(suite.name));
  }
  return suiteNames.map((entry) => getCtxHttpSuiteTarget(entry));
}

function buildCtxHttpSuiteTaskArgs(suiteName) {
  return normalizeCtxHttpSuiteSelection(suiteName)
    .flatMap((entry) => ["--suite", entry]);
}

module.exports = {
  CTX_HTTP_BAZEL_PACKAGE,
  CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS,
  CTX_HTTP_SUITES,
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITE_PREFIX,
  CTX_HTTP_SUITE_SCRIPT_INPUTS,
  MANUAL_ONLY_CTX_HTTP_TEST_FILES,
  buildCtxHttpSuiteCommands,
  buildCtxHttpSuiteTaskArgs,
  getCtxHttpSuiteTarget,
  getCtxHttpSuiteTargets,
  getCtxHttpSuiteByName,
  getCtxHttpSuiteNames,
  getCtxHttpSuiteTaskName,
  listCtxHttpIntegrationTests,
  validateCtxHttpSuites,
};
