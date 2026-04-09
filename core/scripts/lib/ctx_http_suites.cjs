const fs = require("node:fs");
const path = require("node:path");

const CTX_HTTP_SUITE_PREFIX = "rust:ctx-http:test:";
const CTX_HTTP_SUITE_SCRIPT_INPUTS = [
  "scripts/ctx_http_suite_task.cjs",
  "scripts/lib/ctx_http_suites.cjs",
];

const CTX_HTTP_SUITES = [
  {
    name: "base",
    description: "ctx-http lib, bins, and doc tests",
    testFiles: [],
    type: "base",
  },
  {
    name: "workspace-stream",
    description: "workspace snapshot, stream, cache, and replay behavior",
    testFiles: [
      "cache_rehydration",
      "fault_matrix",
      "hot_endpoints_no_db",
      "replay_properties",
      "workspace_active_snapshot_http",
      "workspace_stream_context_window_metrics",
      "workspace_stream_no_gaps_under_activity",
      "workspace_stream_stress_active_heads_lag",
    ],
    type: "integration",
  },
  {
    name: "provider-auth",
    description: "provider install, auth callback, and account status flows",
    testFiles: [
      "acp_target_scoped_status",
      "codex_host_import_api",
      "codex_login_callback_api",
      "install_start_contract",
      "provider_target_scoped_installs",
      "subscription_accounts_api",
    ],
    type: "integration",
  },
  {
    name: "provider-runtime",
    description: "provider runtime, model selection, and offline scenarios",
    testFiles: [
      "acp_crp_bridge_tokens_e2e",
      "gemini_live_model_catalog",
      "live_provider_canary",
      "provider_probe_runtime_env",
      "provider_scenarios_offline",
      "session_model_api",
      "workspace_provider_model_preferences_http",
    ],
    type: "integration",
  },
  {
    name: "repo-vcs",
    description: "repo initialization, worktree state, merge queue, and VCS snapshots",
    testFiles: [
      "jj_merge_queue_basics",
      "merge_queue_isolation",
      "repo_clone_branch_and_safety",
      "repo_init_initial_commit",
      "repo_validate_destination",
      "session_diff_unavailable",
      "workspace_merge_queue_config_http",
      "worktree_archive_http",
      "worktree_vcs_snapshot",
    ],
    type: "integration",
  },
  {
    name: "lsp",
    description: "LSP, MCP-adjacent editing, and buffer flows",
    testFiles: [
      "buffers_http_e2e",
      "lsp_catalog_http_e2e",
      "lsp_edit_plans_http_e2e",
      "lsp_http_e2e",
    ],
    type: "integration",
  },
  {
    name: "turns-terminal",
    description: "turn lifecycle, terminal, streaming, and message durability flows",
    testFiles: [
      "assistant_chunk_stream_only",
      "assistant_message_persistence_faults",
      "demo_seed_transcript_http",
      "message_idempotency",
      "noisy_output_backpressure",
      "terminal_workspace_stream_separation",
      "terminal_ws_reconnect",
      "turn_lifecycle_events",
      "turn_terminal_reconciliation",
    ],
    type: "integration",
  },
  {
    name: "artifacts-updates",
    description: "artifacts, updates, MCP/control-plane, and auxiliary API flows",
    testFiles: [
      "attachments_demo_react",
      "global_id_routing_http",
      "image_attachments_http_e2e",
      "openai_responses_sse_stub",
      "oracle_mcp_http",
      "release_manifest_corpus",
      "storage_guard_api",
      "subagent_mcp_http",
      "system_prompt_append_http",
      "title_generation_local",
      "updates_failure_safety_checksum_mismatch",
      "updates_failure_safety_interrupted_transfer",
      "updates_failure_safety_manifest_parse",
      "updates_failure_safety_missing_artifact",
      "workspace_attachments_local_canonical",
    ],
    type: "integration",
  },
  {
    name: "sandbox-cloud",
    description: "sandbox, cloud, system, and external-runtime ctx-http coverage",
    testFiles: [
      "cloud_gateway_azure_e2e",
      "cloud_gateway_gcp_e2e",
      "disk_isolated_sandbox_smoke",
      "disk_isolated_vcs_integrity",
      "harness_container_sandbox_e2e",
      "memory_leak_e2e",
      "resource_governance_systemd_e2e",
      "title_generation_local_e2e",
      "workspace_runtime_crash_recovery",
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
  const missing = actualFiles.filter((testFile) => !assignedByFile.has(testFile));
  const unknown = [...assignedByFile.keys()].filter((testFile) => !actualFiles.includes(testFile));
  return {
    duplicates,
    missing,
    unknown,
  };
}

function buildCtxHttpSuiteCommands(suiteName) {
  if (suiteName === "all") {
    return CTX_HTTP_SUITES.flatMap((suite) => buildCtxHttpSuiteCommands(suite.name));
  }

  const suite = getCtxHttpSuiteByName(suiteName);
  if (!suite) {
    throw new Error(`unknown ctx-http suite: ${suiteName}`);
  }

  if (suite.type === "base") {
    return [
      {
        args: ["test", "-q", "-p", "ctx-http", "--lib", "--bins"],
        command: "cargo",
      },
      {
        args: ["test", "-q", "-p", "ctx-http", "--doc"],
        command: "cargo",
      },
    ];
  }

  return suite.testFiles.map((testFile) => ({
    args: ["test", "-q", "-p", "ctx-http", "--test", testFile],
    command: "cargo",
  }));
}

module.exports = {
  CTX_HTTP_SUITES,
  CTX_HTTP_SUITE_PREFIX,
  CTX_HTTP_SUITE_SCRIPT_INPUTS,
  buildCtxHttpSuiteCommands,
  getCtxHttpSuiteByName,
  getCtxHttpSuiteNames,
  getCtxHttpSuiteTaskName,
  listCtxHttpIntegrationTests,
  validateCtxHttpSuites,
};
