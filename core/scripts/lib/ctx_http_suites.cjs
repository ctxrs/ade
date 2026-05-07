const fs = require("node:fs");
const path = require("node:path");

const CTX_HTTP_SUITE_PREFIX = "rust:ctx-http:test:";
const CTX_HTTP_BAZEL_PACKAGE = "//core/crates/ctx-http";
const CTX_HTTP_SUITE_CONCURRENCY_CLASSES = Object.freeze({
  SAFE: "safe",
  SERIALIZED: "serialized",
});
const CTX_HTTP_SUITE_SCRIPT_INPUTS = [
  "crates/ctx-http/BUILD.bazel",
  "crates/ctx-http/ctx_http_bazel_tests.bzl",
  "scripts/ctx_http_suite_task.cjs",
  "scripts/lib/ctx_http_suites.cjs",
];
const MANUAL_ONLY_CTX_HTTP_TEST_FILES = new Set([
  "attachments_demo_react",
]);
const CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS = [`${CTX_HTTP_BAZEL_PACKAGE}:manual-only`];
const CTX_HTTP_BASE_CHILD_SUITE_NAMES = [
  "unit-tests-api",
  "unit-tests-lib",
  "unit-tests-lib-session-head-large",
  "unit-tests-workspace-runtime",
  "unit-tests-daemon-and-scheduler",
  "unit-tests-provider-and-settings",
  "unit-tests-merge-queue",
  "bin-tests",
  "doc-tests",
];
const CTX_HTTP_CHECKIN_FANOUT_TARGETS_BY_SUITE = Object.freeze({
  "unit-tests-api": [
    "unit_tests_api",
    "unit_tests_api_cleanup_lifecycle",
    "unit_tests_api_task_lifecycle",
    "unit_tests_api_storage_admission",
    "unit_tests_api_workspaces",
  ],
  "unit-tests-lib": [
    "unit_tests_lib",
    "unit_tests_lib_mobile_secure_routes",
    "unit_tests_lib_provider_routes",
    "unit_tests_lib_session_artifacts",
    "unit_tests_lib_telemetry_export",
    "unit_tests_lib_update_boundaries",
    "unit_tests_lib_web_session_routes",
    "unit_tests_lib_workspace_active_routes",
    "unit_tests_lib_execution_launch_startup_prewarm_kind_supported",
  ],
  "unit-tests-lib-session-head-large": [
    "unit_tests_lib_session_head_large",
  ],
  "unit-tests-workspace-runtime": [
    "unit_tests_workspace_runtime",
    "unit_tests_workspace_runtime_reclaim_idle_runtime_with_parked_containers",
    "unit_tests_workspace_runtime_reuses_running_container",
    "unit_tests_workspace_runtime_prepare_starts_cached_container",
    "unit_tests_workspace_runtime_reclaim_idle_machine",
    "unit_tests_workspace_runtime_reclaim_idle_runtime_with_containers",
    "unit_tests_workspace_runtime_reclaim_ctx_harness_container",
    "unit_tests_workspace_runtime_container_status_avf",
    "unit_tests_workspace_runtime_starts_avf_workspace_vm",
    "unit_tests_workspace_runtime_keeps_avf_workspace_container_ready",
    "unit_tests_workspace_runtime_unknown_machine_state_engine_unreachable",
    "unit_tests_workspace_runtime_running_unreachable_machine_reconfiguration",
    "unit_tests_workspace_runtime_recreates_machine_for_memory_profile_change",
  ],
  "unit-tests-daemon-and-scheduler": [
    "unit_tests_daemon",
    "unit_tests_scheduler",
    "unit_tests_daemon_golden_path_with_fake_provider",
    "unit_tests_daemon_http_and_ws_streaming",
  ],
  "unit-tests-provider-and-settings": [
    "unit_tests_installer",
    "unit_tests_provider_launch",
    "//core/crates/ctx-managed-installs:unit_tests",
    "//core/crates/ctx-provider-matrix:unit_tests",
    "//core/crates/ctx-settings-model:unit_tests",
    "//core/crates/ctx-settings-service:unit_tests",
  ],
  "unit-tests-merge-queue": [
    "unit_tests_merge_queue",
    "unit_tests_merge_queue_enabled_workspace_resume_after_open",
  ],
  "bin-tests": [
    "bin_tests_root_help",
    "bin_tests_serve_help",
    "bin_tests_init_help",
    "bin_tests_self_update_help",
  ],
});
const CTX_HTTP_CHECKIN_FANOUT_TARGET_BATCHES_BY_SUITE = Object.freeze({
  "bin-tests": [
    [
      "bin_tests_root_help",
      "bin_tests_serve_help",
      "bin_tests_init_help",
      "bin_tests_self_update_help",
    ],
  ],
  "unit-tests-api": [
    [
      "unit_tests_api_cleanup_lifecycle",
      "unit_tests_api_task_lifecycle",
    ],
    [
      "unit_tests_api_storage_admission",
      "unit_tests_api_workspaces",
    ],
  ],
  "unit-tests-lib": [
    [
      "unit_tests_lib_provider_routes",
      "unit_tests_lib_session_artifacts",
      "unit_tests_lib_telemetry_export",
    ],
    [
      "unit_tests_lib_update_boundaries",
      "unit_tests_lib_web_session_routes",
      "unit_tests_lib_workspace_active_routes",
    ],
  ],
  "unit-tests-workspace-runtime": [
    [
      "unit_tests_workspace_runtime_reclaim_idle_runtime_with_parked_containers",
      "unit_tests_workspace_runtime_reuses_running_container",
      "unit_tests_workspace_runtime_prepare_starts_cached_container",
    ],
    [
      "unit_tests_workspace_runtime_reclaim_idle_machine",
      "unit_tests_workspace_runtime_reclaim_idle_runtime_with_containers",
      "unit_tests_workspace_runtime_reclaim_ctx_harness_container",
    ],
    [
      "unit_tests_workspace_runtime_container_status_avf",
      "unit_tests_workspace_runtime_starts_avf_workspace_vm",
      "unit_tests_workspace_runtime_keeps_avf_workspace_container_ready",
    ],
    [
      "unit_tests_workspace_runtime_unknown_machine_state_engine_unreachable",
      "unit_tests_workspace_runtime_running_unreachable_machine_reconfiguration",
      "unit_tests_workspace_runtime_recreates_machine_for_memory_profile_change",
    ],
  ],
  "unit-tests-daemon-and-scheduler": [
    [
      "unit_tests_daemon_golden_path_with_fake_provider",
      "unit_tests_daemon_http_and_ws_streaming",
    ],
  ],
  "unit-tests-provider-and-settings": [
    [
      "unit_tests_installer",
      "unit_tests_provider_launch",
    ],
    [
      "//core/crates/ctx-managed-installs:unit_tests",
      "//core/crates/ctx-provider-matrix:unit_tests",
      "//core/crates/ctx-settings-model:unit_tests",
      "//core/crates/ctx-settings-service:unit_tests",
    ],
  ],
  "unit-tests-merge-queue": [
    [
      "unit_tests_merge_queue",
      "unit_tests_merge_queue_enabled_workspace_resume_after_open",
    ],
  ],
  "workspace-stream": [
    [
      "cache_rehydration",
      "hot_endpoints_no_db",
      "replay_properties",
    ],
    [
      "fault_matrix",
      "task_default_session_http",
    ],
    [
      "workspace_active_snapshot_http",
      "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state",
      "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs",
    ],
    [
      "workspace_active_snapshot_http_workspace_stream_subscribe_does_not_reemit_when_worktree_vcs_is_already_computing",
      "workspace_active_snapshot_http_worktree_vcs_summary_refresh_reloads_live_inventory_before_ready_publish",
    ],
    [
      "workspace_stream_context_window_metrics",
      "workspace_stream_no_gaps_under_activity",
    ],
  ],
  "provider-auth": [
    [
      "acp_target_scoped_status",
      "codex_host_import_api",
      "codex_login_callback_api",
    ],
    [
      "install_start_contract",
      "provider_current_ctx_version_regressions",
      "provider_target_scoped_installs",
      "subscription_accounts_api",
    ],
  ],
  "provider-runtime-simulated": [
    [
      "provider_probe_runtime_env",
      "provider_worker_reaping_offline",
    ],
    [
      "provider_scenarios_offline_crp_fixtures",
      "provider_scenarios_offline_interleaved_assistant_tools_do_not_fragment_messages",
      "provider_scenarios_offline_crp_fixtures_persist_context_window_metrics",
    ],
    [
      "session_model_api",
      "workspace_provider_model_preferences_http",
    ],
  ],
  "repo-vcs": [
    [
      "jj_merge_queue_basics",
      "merge_queue_isolation",
    ],
    [
      "repo_clone_branch_and_safety",
      "repo_init_initial_commit",
      "repo_validate_destination",
    ],
    [
      "session_diff_unavailable",
      "workspace_merge_queue_config_http",
      "worktree_archive_http",
      "worktree_vcs_snapshot",
    ],
  ],
  "scheduler-runtime": [
    [
      "assistant_chunk_stream_only",
      "assistant_message_persistence_faults",
      "noisy_output_backpressure",
    ],
    [
      "turn_lifecycle_events",
      "turn_terminal_reconciliation",
    ],
  ],
  "turns-terminal": [
    [
      "demo_seed_transcript_http",
      "message_idempotency_post_message_idempotent_same_payload",
      "message_idempotency_post_message_idempotent_conflict_on_change",
    ],
    [
      "terminal_workspace_stream_separation",
      "terminal_ws_reconnect",
    ],
  ],
  "subagents-control": [
    [
      "subagent_mcp_http_archive_agent_reclaims_dedicated_child_worktree",
      "system_prompt_append_http",
      "title_generation_local",
    ],
  ],
  "updates-release": [
    [
      "openai_responses_sse_stub",
      "release_manifest_corpus",
    ],
    [
      "updates_appimage_apply_safety",
      "updates_failure_safety_checksum_mismatch",
    ],
    [
      "updates_failure_safety_interrupted_transfer",
      "updates_failure_safety_manifest_parse",
      "updates_failure_safety_manifest_signature",
      "updates_failure_safety_missing_artifact",
    ],
  ],
});
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

const CTX_HTTP_UNIT_SUITES = [
  {
    dependencyCrates: ["ctx-session-service", "ctx-storage-admission"],
    family: "workspace-stream",
    name: "unit-tests-api",
    description: "ctx-http API unit test family",
    sourceGlobs: [
      "crates/ctx-http/src/api/**",
    ],
  },
  {
    family: "workspace-stream",
    name: "unit-tests-lib",
    description: "ctx-http lib route and shared helper unit family",
    sourceGlobs: [
      "crates/ctx-http/src/api/**",
      "crates/ctx-http/src/daemon/**",
      "crates/ctx-http/src/lib.rs",
      "crates/ctx-http/src/test_support.rs",
    ],
  },
  {
    family: "workspace-stream",
    name: "unit-tests-lib-session-head-large",
    description: "ctx-http large session-head response-boundary unit family",
    sourceGlobs: [
      "crates/ctx-http/src/api/sessions/**",
      "crates/ctx-http/src/api/workspaces.rs",
      "crates/ctx-http/src/daemon/workspaces/**",
      "crates/ctx-http/src/test_support.rs",
    ],
  },
  {
    family: "workspace-stream",
    name: "unit-tests-workspace-runtime",
    description: "ctx-http workspace runtime unit family",
    sourceGlobs: [
      "crates/ctx-http/src/container_builder.rs",
      "crates/ctx-http/src/container_fs.rs",
      "crates/ctx-http/src/disk_isolated.rs",
      "crates/ctx-http/src/disk_isolated_copy.rs",
      "crates/ctx-http/src/disk_isolated_sandbox.rs",
      "crates/ctx-http/src/workspace_runtime/**",
    ],
  },
  {
    family: "turns-terminal",
    name: "unit-tests-daemon-and-scheduler",
    description: "ctx-http daemon and scheduler unit family",
    sourceGlobs: [
      "crates/ctx-http/src/daemon.rs",
      "crates/ctx-http/src/daemon/**",
      "crates/ctx-http/src/scheduler.rs",
      "crates/ctx-http/src/scheduler/**",
    ],
  },
  {
    family: "provider-runtime",
    name: "unit-tests-provider-and-settings",
    description: "ctx-http provider launch, installer, provider matrix owner, and settings unit family",
    dependencyCrates: [
      "ctx-managed-installs",
      "ctx-provider-matrix",
      "ctx-settings-model",
      "ctx-settings-service",
    ],
    sourceGlobs: [
      "crates/ctx-http/src/api/providers/**",
      "crates/ctx-http/src/installer.rs",
      "crates/ctx-http/src/installer/**",
      "crates/ctx-http/src/provider_launch.rs",
      "crates/ctx-http/src/provider_launch/**",
      "crates/ctx-http/src/settings.rs",
      "crates/ctx-http/src/settings/**",
    ],
  },
  {
    family: "repo-vcs",
    name: "unit-tests-merge-queue",
    description: "ctx-http merge queue unit family",
    sourceGlobs: [
      "crates/ctx-http/src/api/merge_queue_api.rs",
      "crates/ctx-http/src/merge_queue.rs",
      "crates/ctx-http/src/merge_queue/**",
    ],
  },
].map((suite) => ({
  concurrencyClass: "serialized",
  dependencyCrates: [],
  execution: "bazel-rbe-preferred",
  oracle: "direct-assertion",
  requirements: ["linux", "buildbuddy-rbe"],
  stability: "stable",
  surface: "unit",
  testFiles: [],
  type: "unit",
  world: "hermetic",
  ...suite,
}));

const CTX_HTTP_SAFE_CONCURRENT_SUITES = new Set([
  "attachments-routing",
  "provider-auth",
  "provider-runtime-simulated",
  "updates-release",
]);

const CTX_HTTP_SUITES = [
  {
    concurrencyClass: "serialized",
    dependencyCrates: ["ctx-http"],
    includeInAll: false,
    name: "base",
    description: "ctx-http lib, bins, and doc tests",
    sourceGlobs: [],
    testFiles: [],
    type: "base",
  },
  {
    concurrencyClass: "serialized",
    dependencyCrates: [],
    execution: "bazel-rbe-preferred",
    family: "build-graph",
    name: "bin-tests",
    description: "ctx-http binary smoke tests",
    oracle: "direct-assertion",
    requirements: ["linux", "buildbuddy-rbe"],
    stability: "stable",
    surface: "unit",
    targetName: "bin_tests",
    sourceGlobs: [
      "crates/ctx-http/src/bin/**",
      "crates/ctx-http/tests/bin_smoke.sh",
    ],
    testFiles: [],
    type: "unit",
    world: "hermetic",
  },
  {
    concurrencyClass: "serialized",
    dependencyCrates: [],
    execution: "bazel-rbe-preferred",
    family: "build-graph",
    name: "doc-tests",
    description: "ctx-http rustdoc examples and docs",
    oracle: "compiler",
    requirements: ["linux", "buildbuddy-rbe"],
    stability: "stable",
    surface: "compile",
    targetName: "doc_tests",
    sourceGlobs: [
      "crates/ctx-http/src/lib.rs",
    ],
    testFiles: [],
    type: "unit",
    world: "hermetic",
  },
  ...CTX_HTTP_UNIT_SUITES,
  {
    dependencyCrates: [
      "ctx-core",
      "ctx-events",
      "ctx-store",
      "ctx-workspace-active-snapshot",
      "ctx-session-service",
      "ctx-workspace-services",
    ],
    name: "workspace-stream",
    description: "workspace snapshot, stream, cache, and replay behavior",
    expandTestFilesToTargets: true,
    directTargets: [
      "cache_rehydration",
      "fault_matrix",
      "hot_endpoints_no_db",
      "replay_properties",
      "task_default_session_http",
      "workspace_active_snapshot_http",
      "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state",
      "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs",
      "workspace_active_snapshot_http_workspace_stream_subscribe_does_not_reemit_when_worktree_vcs_is_already_computing",
      "workspace_active_snapshot_http_worktree_vcs_summary_refresh_reloads_live_inventory_before_ready_publish",
      "workspace_stream_context_window_metrics",
      "workspace_stream_no_gaps_under_activity",
      "workspace_stream_stress_active_heads_lag",
    ],
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
      "ctx-provider-matrix",
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
      "crates/ctx-http/src/api/providers/cursor_login/**",
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
      "ctx-provider-matrix",
      "ctx-providers",
      "ctx-provider-runtime",
      "ctx-store",
      "ctx-workspace-runtime",
    ],
    name: "provider-runtime-simulated",
    description: "provider runtime, model selection, and offline simulated scenarios",
    directTargets: [
      "provider_probe_runtime_env",
      "provider_worker_reaping_offline",
      "provider_scenarios_offline_crp_fixtures",
      "provider_scenarios_offline_interleaved_assistant_tools_do_not_fragment_messages",
      "provider_scenarios_offline_crp_fixtures_persist_context_window_metrics",
      "session_model_api",
      "workspace_provider_model_preferences_http",
    ],
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
      "ctx-provider-matrix",
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
    expandTestFilesToTargets: true,
    directTargets: [
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
    sourceGlobs: [
      "crates/ctx-http/src/api/merge_queue_api.rs",
      "crates/ctx-http/src/api/repo.rs",
      "crates/ctx-http/src/api/repo/**",
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
      "ctx-mcp-command",
      "ctx-storage-admission",
      "ctx-transport-runtime",
    ],
    name: "scheduler-runtime",
    description: "scheduler runtime, turn lifecycle, and stream backpressure flows",
    directTargets: [
      "assistant_chunk_stream_only",
      "assistant_message_persistence_faults",
      "noisy_output_backpressure",
      "turn_lifecycle_events",
      "turn_terminal_reconciliation",
      "unit_tests_scheduler",
    ],
    sourceGlobs: [
      "crates/ctx-http/src/api/execution.rs",
      "crates/ctx-http/src/api/sessions/control.rs",
      "crates/ctx-http/src/api/sessions/messages.rs",
      "crates/ctx-http/src/api/sessions/messages/**",
      "crates/ctx-http/src/api/sessions/mod.rs",
      "crates/ctx-http/src/api/terminals.rs",
      "crates/ctx-http/src/api/terminals/**",
      "crates/ctx-workspace-services/src/file_completions.rs",
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
    ],
    name: "turns-terminal",
    description: "turn lifecycle, terminal, streaming, and message durability flows",
    directTargets: [
      "demo_seed_transcript_http",
      "message_idempotency_post_message_idempotent_same_payload",
      "message_idempotency_post_message_idempotent_conflict_on_change",
      "terminal_workspace_stream_separation",
      "terminal_ws_reconnect",
    ],
    sourceGlobs: [
      "crates/ctx-http/src/api/execution.rs",
      "crates/ctx-http/src/api/sessions/control.rs",
      "crates/ctx-http/src/api/sessions/messages.rs",
      "crates/ctx-http/src/api/sessions/messages/**",
      "crates/ctx-http/src/api/sessions/mod.rs",
      "crates/ctx-http/src/api/sessions/titles_and_modes.rs",
      "crates/ctx-http/src/api/sessions/titles_and_modes/**",
      "crates/ctx-http/src/api/terminals.rs",
      "crates/ctx-http/src/api/terminals/**",
      "crates/ctx-http/src/api/web_sessions.rs",
      "crates/ctx-workspace-services/src/file_completions.rs",
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
      "ctx-workspace-attachments",
      "ctx-workspace-services",
    ],
    name: "attachments-routing",
    description: "artifact uploads, attachment materialization, and route-scoping coverage",
    expandTestFilesToTargets: true,
    directTargets: [
      "global_id_routing_http",
      "image_attachments_http_e2e",
      "workspace_attachments_local_canonical",
    ],
    sourceGlobs: [
      "crates/ctx-http/src/api/artifacts.rs",
      "crates/ctx-http/src/api/artifacts/**",
      "crates/ctx-http/src/api/demo.rs",
      "crates/ctx-http/src/daemon/workspaces/attachments.rs",
      "crates/ctx-http/src/storage_guard.rs",
      "crates/ctx-http/src/worktree_data_plane.rs",
      "crates/ctx-workspace-attachments/src/**",
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
      "ctx-mcp-command",
      "ctx-store",
      "ctx-transport-runtime",
      "ctx-workspace-services",
    ],
    name: "subagents-control",
    description: "subagent orchestration, MCP/oracle, and title-path control flows",
    directTargets: [
      "subagent_mcp_http",
      "subagent_mcp_http_archive_agent_reclaims_dedicated_child_worktree",
      "system_prompt_append_http",
      "title_generation_local",
    ],
    sourceGlobs: [
      "crates/ctx-http/src/api/mobile_access.rs",
      "crates/ctx-http/src/api/mobile_access/**",
      "crates/ctx-http/src/api/sessions/subagents.rs",
      "crates/ctx-http/src/api/sessions/subagents/**",
      "crates/ctx-http/src/daemon/sessions/subagents.rs",
      "crates/ctx-http/src/oracle.rs",
      "crates/ctx-http/src/title_generation.rs",
      "crates/ctx-managed-installs/src/title_generation_local.rs",
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
      "ctx-workspace-services",
    ],
    name: "subagents-local-runtime",
    description: "real local title-generation runtime flows",
    sourceGlobs: [
      "crates/ctx-http/src/title_generation.rs",
      "crates/ctx-managed-installs/src/title_generation_local.rs",
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
      "ctx-update-service",
      "ctx-workspace-services",
    ],
    name: "updates-release",
    description: "updates, manifests, release safety, and auxiliary response flows",
    sourceGlobs: [
      "crates/ctx-http/src/api/updates.rs",
      "crates/ctx-http/src/bundled_assets.rs",
      "crates/ctx-http/src/bundled_assets/**",
      "crates/ctx-update-service/src/**",
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
      "ctx-resource-utilization",
      "ctx-store",
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
      "crates/ctx-http/src/network_allowlist.rs",
      "crates/ctx-http/src/resource_governance.rs",
      "crates/ctx-http/src/resource_telemetry.rs",
      "crates/ctx-http/src/tool_cgroup.rs",
      "crates/ctx-http/src/workspace_runtime/**",
      "crates/ctx-resource-utilization/src/**",
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
      "ctx-resource-utilization",
      "ctx-store",
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
      "ctx-workspace-runtime",
    ],
    name: "sandbox-runtime-resource-governance",
    description: "resource governance and cgroup/systemd enforcement flows",
    sourceGlobs: [
      "crates/ctx-http/src/execution_effective.rs",
      "crates/ctx-http/src/resource_governance.rs",
      "crates/ctx-http/src/resource_telemetry.rs",
      "crates/ctx-http/src/tool_cgroup.rs",
      "crates/ctx-resource-utilization/src/**",
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
      "ctx-resource-utilization",
      "ctx-store",
      "ctx-workspace-runtime",
    ],
    name: "sandbox-runtime-memory-leak",
    description: "sandbox/runtime memory pressure and leak detection flows",
    sourceGlobs: [
      "crates/ctx-http/src/resource_telemetry.rs",
      "crates/ctx-http/src/workspace_runtime/**",
      "crates/ctx-resource-utilization/src/**",
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

function getCtxHttpSuiteNamesForAllTarget() {
  return CTX_HTTP_SUITES
    .filter((suite) => suite.includeInAll !== false)
    .map((suite) => suite.name);
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

function expandCtxHttpSuiteForPlanner(suiteName) {
  return suiteName === "base" ? [...CTX_HTTP_BASE_CHILD_SUITE_NAMES] : [suiteName];
}

function getCtxHttpSuiteConcurrencyClass(suiteName) {
  const suite = getCtxHttpSuiteByName(suiteName);
  if (!suite || suite.name === "all") {
    throw new Error(`unknown ctx-http suite for concurrency: ${suiteName}`);
  }
  if (suite.concurrencyClass) {
    return suite.concurrencyClass;
  }
  if (CTX_HTTP_SAFE_CONCURRENT_SUITES.has(suite.name)) {
    return CTX_HTTP_SUITE_CONCURRENCY_CLASSES.SAFE;
  }
  return CTX_HTTP_SUITE_CONCURRENCY_CLASSES.SERIALIZED;
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
  const targets = getCtxHttpSuiteExecutionTargets(suiteName);
  if (targets.length === 0) {
    return [];
  }
  return [
    {
      args: ["scripts/run_bazel_pilot.cjs", "test", ...targets],
      command: "node",
    },
  ];
}

function getCtxHttpSuiteTarget(suiteName) {
  const suite = getCtxHttpSuiteByName(suiteName);
  if (!suite) {
    throw new Error(`unknown ctx-http suite: ${suiteName}`);
  }
  return `${CTX_HTTP_BAZEL_PACKAGE}:${suite.targetName || suite.name}`;
}

function toCtxHttpBazelLabel(targetName) {
  return targetName.startsWith("//") ? targetName : `${CTX_HTTP_BAZEL_PACKAGE}:${targetName}`;
}

function getCtxHttpSuiteDirectTargets(suiteName) {
  const suite = getCtxHttpSuiteByName(suiteName);
  if (!suite) {
    throw new Error(`unknown ctx-http suite: ${suiteName}`);
  }
  if (suite.expandTestFilesToTargets) {
    return (suite.directTargets || suite.testFiles)
      .map((targetName) => toCtxHttpBazelLabel(targetName));
  }
  return [getCtxHttpSuiteTarget(suiteName)];
}

function getCtxHttpSuiteTargets(suiteName) {
  const suiteNames = normalizeCtxHttpSuiteSelection(suiteName);
  if (suiteNames.length === 1 && suiteNames[0] === "all") {
    return getCtxHttpSuiteNamesForAllTarget().flatMap((entry) => getCtxHttpSuiteDirectTargets(entry));
  }
  return suiteNames.flatMap((entry) => getCtxHttpSuiteDirectTargets(entry));
}

function getCtxHttpSuiteCheckinFanoutTargets(suiteName) {
  return getCtxHttpSuiteCheckinFanoutTargetBatches(suiteName).flat();
}

function getCtxHttpSuiteCheckinFanoutTargetBatches(suiteName) {
  const suite = getCtxHttpSuiteByName(suiteName);
  if (!suite) {
    throw new Error(`unknown ctx-http suite: ${suiteName}`);
  }
  if (suite.name === "base") {
    return dedupeTargetBatchesPreservingOrder(
      CTX_HTTP_BASE_CHILD_SUITE_NAMES.flatMap((childSuiteName) =>
        getCtxHttpSuiteCheckinFanoutTargetBatches(childSuiteName)
      ),
    );
  }
  const targetNames = CTX_HTTP_CHECKIN_FANOUT_TARGETS_BY_SUITE[suite.name]
    || (suite.type === "integration" && (suite.directTargets || suite.testFiles).length > 0
      ? (suite.directTargets || suite.testFiles)
      : null);
  if (!targetNames) {
    return [[getCtxHttpSuiteTarget(suite.name)]];
  }
  const configuredBatches = CTX_HTTP_CHECKIN_FANOUT_TARGET_BATCHES_BY_SUITE[suite.name] || [];
  const declaredTargetNames = new Set(targetNames);
  const configuredBatchByFirstTarget = new Map();
  const configuredBatchedTargetNames = new Set();
  for (const batch of configuredBatches) {
    const firstTargetName = batch[0];
    if (!firstTargetName) {
      throw new Error(`empty ctx-http checkin target batch for suite ${suite.name}`);
    }
    if (configuredBatchByFirstTarget.has(firstTargetName)) {
      throw new Error(`duplicate ctx-http checkin target batch starts with ${firstTargetName} for suite ${suite.name}`);
    }
    for (const targetName of batch) {
      if (!declaredTargetNames.has(targetName)) {
        throw new Error(`ctx-http checkin target batch for suite ${suite.name} references unknown target ${targetName}`);
      }
      if (configuredBatchedTargetNames.has(targetName)) {
        throw new Error(`ctx-http checkin target batch for suite ${suite.name} repeats target ${targetName}`);
      }
      configuredBatchedTargetNames.add(targetName);
    }
    configuredBatchByFirstTarget.set(firstTargetName, batch);
  }

  const consumedTargetNames = new Set();
  const batches = [];
  for (const targetName of targetNames) {
    if (consumedTargetNames.has(targetName)) {
      continue;
    }
    const configuredBatch = configuredBatchByFirstTarget.get(targetName);
    if (configuredBatch) {
      batches.push(configuredBatch.map((entry) => toCtxHttpBazelLabel(entry)));
      for (const entry of configuredBatch) {
        consumedTargetNames.add(entry);
      }
      continue;
    }
    batches.push([toCtxHttpBazelLabel(targetName)]);
    consumedTargetNames.add(targetName);
  }
  return batches;
}

function dedupePreservingOrder(values) {
  return [...new Set(values)];
}

function dedupeTargetBatchesPreservingOrder(batches) {
  const seen = new Set();
  const dedupedBatches = [];
  for (const batch of batches) {
    const dedupedBatch = [];
    for (const target of batch) {
      if (seen.has(target)) {
        continue;
      }
      seen.add(target);
      dedupedBatch.push(target);
    }
    if (dedupedBatch.length > 0) {
      dedupedBatches.push(dedupedBatch);
    }
  }
  return dedupedBatches;
}

function getCtxHttpSuiteExecutionTargets(suiteSelection) {
  const suiteNames = normalizeCtxHttpSuiteSelection(suiteSelection);
  const expandedSuiteNames = suiteNames.length === 1 && suiteNames[0] === "all"
    ? getCtxHttpSuiteNamesForAllTarget()
    : suiteNames;
  return dedupePreservingOrder(
    expandedSuiteNames.flatMap((suiteName) => getCtxHttpSuiteCheckinFanoutTargets(suiteName)),
  );
}

function getAllCtxHttpSuiteCheckinFanoutTargets() {
  return dedupePreservingOrder(
    getCtxHttpSuiteNamesForAllTarget()
      .flatMap((suiteName) => getCtxHttpSuiteCheckinFanoutTargets(suiteName)),
  ).sort();
}

function buildCtxHttpSuiteTaskArgs(suiteName) {
  return normalizeCtxHttpSuiteSelection(suiteName)
    .flatMap((entry) => ["--suite", entry]);
}

module.exports = {
  CTX_HTTP_BAZEL_PACKAGE,
  CTX_HTTP_BASE_CHILD_SUITE_NAMES,
  CTX_HTTP_CHECKIN_FANOUT_TARGET_BATCHES_BY_SUITE,
  CTX_HTTP_CHECKIN_FANOUT_TARGETS_BY_SUITE,
  CTX_HTTP_MANUAL_ONLY_BAZEL_TARGETS,
  CTX_HTTP_SAFE_CONCURRENT_SUITES,
  CTX_HTTP_SUITES,
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITE_PREFIX,
  CTX_HTTP_SUITE_CONCURRENCY_CLASSES,
  CTX_HTTP_SUITE_SCRIPT_INPUTS,
  MANUAL_ONLY_CTX_HTTP_TEST_FILES,
  buildCtxHttpSuiteCommands,
  buildCtxHttpSuiteTaskArgs,
  expandCtxHttpSuiteForPlanner,
  getAllCtxHttpSuiteCheckinFanoutTargets,
  getCtxHttpSuiteTarget,
  getCtxHttpSuiteTargets,
  getCtxHttpSuiteCheckinFanoutTargets,
  getCtxHttpSuiteCheckinFanoutTargetBatches,
  getCtxHttpSuiteExecutionTargets,
  getCtxHttpSuiteByName,
  getCtxHttpSuiteConcurrencyClass,
  getCtxHttpSuiteNames,
  getCtxHttpSuiteNamesForAllTarget,
  getCtxHttpSuiteTaskName,
  listCtxHttpIntegrationTests,
  validateCtxHttpSuites,
};
