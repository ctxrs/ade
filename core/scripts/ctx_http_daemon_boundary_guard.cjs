#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const ctxHttpSrcRoot = path.join(coreRoot, "crates", "ctx-http", "src");
const ctxHttpTestsRoot = path.join(coreRoot, "crates", "ctx-http", "tests");
const ctxHttpTestSupportSrcRoot = path.join(coreRoot, "crates", "ctx-http-test-support", "src");
const apiRoot = path.join(coreRoot, "crates", "ctx-http", "src", "api");
const legacyHttpDaemonRoot = path.join(coreRoot, "crates", "ctx-http", "src", "daemon");
const legacyHttpDaemonRootPath = path.join(coreRoot, "crates", "ctx-http", "src", "daemon.rs");
const daemonRoot = path.join(coreRoot, "crates", "ctx-daemon", "src", "daemon");
const daemonRootPath = path.join(coreRoot, "crates", "ctx-daemon", "src", "daemon.rs");
const daemonHandlePath = path.join(coreRoot, "crates", "ctx-daemon", "src", "daemon", "handle.rs");
const rawStoreBlindApiRoots = [
  "core/crates/ctx-http/src/api/sessions/",
  "core/crates/ctx-http/src/api/tasks/",
  "core/crates/ctx-http/src/api/workspaces/",
];
const migratedRawDaemonTestRoots = [
  "core/crates/ctx-http/src/api/settings.rs",
  "core/crates/ctx-http/src/api/sessions/tests.rs",
  "core/crates/ctx-http/src/api/sessions/tests/",
  "core/crates/ctx-http/src/api/providers/login/codex/tests.rs",
  "core/crates/ctx-http/src/api/providers/tests/install_statuses.rs",
  "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change.rs",
  "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change/",
  "core/crates/ctx-http/src/api/providers/tests/restarts/harness_source.rs",
  "core/crates/ctx-http/src/api/workspaces/tests.rs",
  "core/crates/ctx-http/src/api/tasks.rs",
  "core/crates/ctx-http/src/api/tasks/cleanup_lifecycle_tests.rs",
  "core/crates/ctx-http/src/api/tasks/lifecycle_tests/",
  "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests.rs",
  "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/",
  "core/crates/ctx-http/src/lib_tests/auth_boundaries/",
  "core/crates/ctx-http/src/lib_tests/cors.rs",
  "core/crates/ctx-http/src/lib_tests/daemon_smoke.rs",
  "core/crates/ctx-http/src/lib_tests/daemon_smoke/",
  "core/crates/ctx-http/src/lib_tests/execution_launch/",
  "core/crates/ctx-http/src/lib_tests/health_diagnostics/",
  "core/crates/ctx-http/src/lib_tests/log_path_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/log_path_boundaries/",
  "core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_profile_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/",
  "core/crates/ctx-http/src/lib_tests/org_policy_routes.rs",
  "core/crates/ctx-http/src/lib_tests/provider_routes/",
  "core/crates/ctx-http/src/lib_tests/run_archive_routes.rs",
  "core/crates/ctx-http/src/lib_tests/session_artifacts.rs",
  "core/crates/ctx-http/src/lib_tests/session_artifacts/",
  "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http.rs",
  "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http/",
  "core/crates/ctx-http/src/lib_tests/session_head_large_http.rs",
  "core/crates/ctx-http/src/lib_tests/telemetry_export_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/update_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/web_session_routes/fixtures.rs",
  "core/crates/ctx-http/src/lib_tests/workspace_active_routes.rs",
  "core/crates/ctx-http/tests/acp_target_scoped_status.rs",
  "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
  "core/crates/ctx-http/tests/assistant_message_persistence_faults.rs",
  "core/crates/ctx-http/tests/attachments_demo_react.rs",
  "core/crates/ctx-http/tests/codex_host_import_api.rs",
  "core/crates/ctx-http/tests/codex_login_callback_api.rs",
  "core/crates/ctx-http/tests/cache_rehydration.rs",
  "core/crates/ctx-http/tests/demo_seed_transcript_http.rs",
  "core/crates/ctx-http/tests/common/updates_failure_safety.rs",
  "core/crates/ctx-http/tests/disk_isolated_sandbox_smoke.rs",
  "core/crates/ctx-http/tests/disk_isolated_vcs_integrity.rs",
  "core/crates/ctx-http/tests/fault_matrix.rs",
  "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
  "core/crates/ctx-http/tests/global_id_routing_http.rs",
  "core/crates/ctx-http/tests/harness_container_sandbox_e2e.rs",
  "core/crates/ctx-http/tests/hot_endpoints_no_db.rs",
  "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
  "core/crates/ctx-http/tests/install_start_contract.rs",
  "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
  "core/crates/ctx-http/tests/live_provider_canary.rs",
  "core/crates/ctx-http/tests/memory_leak_e2e.rs",
  "core/crates/ctx-http/tests/merge_queue_isolation.rs",
  "core/crates/ctx-http/tests/message_idempotency.rs",
  "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
  "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
  "core/crates/ctx-http/tests/provider_current_ctx_version_regressions.rs",
  "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
  "core/crates/ctx-http/tests/provider_target_scoped_installs.rs",
  "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
  "core/crates/ctx-http/tests/replay_properties.rs",
  "core/crates/ctx-http/tests/repo_clone_branch_and_safety.rs",
  "core/crates/ctx-http/tests/repo_init_initial_commit.rs",
  "core/crates/ctx-http/tests/repo_status_and_staging.rs",
  "core/crates/ctx-http/tests/repo_validate_destination.rs",
  "core/crates/ctx-http/tests/session_diff_unavailable.rs",
  "core/crates/ctx-http/tests/session_model_api.rs",
  "core/crates/ctx-http/tests/subagent_mcp_http.rs",
  "core/crates/ctx-http/tests/subscription_accounts_api.rs",
  "core/crates/ctx-http/tests/system_prompt_append_http.rs",
  "core/crates/ctx-http/tests/task_default_session_http.rs",
  "core/crates/ctx-http/tests/terminal_rest_route_contracts.rs",
  "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
  "core/crates/ctx-http/tests/terminal_ws_reconnect.rs",
  "core/crates/ctx-http/tests/title_generation_local_e2e.rs",
  "core/crates/ctx-http/tests/turn_lifecycle_events.rs",
  "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
  "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
  "core/crates/ctx-http/tests/workspace_attachments_local_canonical.rs",
  "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
  "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
  "core/crates/ctx-http/tests/workspace_active_snapshot_http.rs",
  "core/crates/ctx-http/tests/workspace_stream_context_window_metrics.rs",
  "core/crates/ctx-http/tests/workspace_stream_stress_active_heads_lag.rs",
  "core/crates/ctx-http/tests/workspace_stream_no_gaps_under_activity.rs",
  "core/crates/ctx-http/tests/worktree_archive_http.rs",
  "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
];
const mobileStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/auth_boundaries/mobile_tokens.rs",
  "core/crates/ctx-http/src/lib_tests/auth_boundaries/mobile_tokens/",
  "core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_profile_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_secure_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/",
];

const providerCacheFacadeTestRoots = [
  "core/crates/ctx-http/src/api/providers/tests/restarts.rs",
  "core/crates/ctx-http/src/api/providers/tests/restarts/",
  "core/crates/ctx-http/src/lib_tests/provider_routes.rs",
  "core/crates/ctx-http/src/lib_tests/provider_routes/",
];

const providerRouteSetupStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/provider_routes.rs",
  "core/crates/ctx-http/src/lib_tests/provider_routes/",
];

const authBoundaryStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/auth_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/auth_boundaries/",
];

const externalProviderRouteStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/codex_host_import_api.rs",
  "core/crates/ctx-http/tests/codex_login_callback_api.rs",
  "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
];

const geminiLiveModelCatalogStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
];

const mobileAccessStoreDtoApiRoots = [
  "core/crates/ctx-http/src/api/mod.rs",
  "core/crates/ctx-http/src/api/mobile_access.rs",
  "core/crates/ctx-http/src/api/mobile_access/",
];

const mobileProfileRouteApiRoots = [
  "core/crates/ctx-http/src/api/mobile_access/profiles.rs",
  "core/crates/ctx-http/src/api/mobile_access/profiles/",
];

const routeFileDownloadApiRoots = [
  "core/crates/ctx-http/src/api/artifacts/",
  "core/crates/ctx-http/src/api/merge_queue_api/logs.rs",
  "core/crates/ctx-http/src/api/workspaces/worktrees.rs",
];

const sessionArtifactApiRoots = [
  "core/crates/ctx-http/src/api/artifacts/session/list.rs",
  "core/crates/ctx-http/src/api/artifacts/session/set.rs",
  "core/crates/ctx-http/src/api/artifacts/download.rs",
];

const mergeQueueSubmitApiRoots = [
  "core/crates/ctx-http/src/api/merge_queue_api/submit.rs",
];

const mergeQueueEntryApiRoots = [
  "core/crates/ctx-http/src/api/merge_queue_api/actions.rs",
  "core/crates/ctx-http/src/api/merge_queue_api/logs.rs",
  "core/crates/ctx-http/src/api/merge_queue_api/submit.rs",
  "core/crates/ctx-http/src/api/merge_queue_api/request.rs",
];

const terminalRestRouteApiRoots = [
  "core/crates/ctx-http/src/api/terminals.rs",
  "core/crates/ctx-http/src/api/terminals/",
];

const runArchiveApiRoots = [
  "core/crates/ctx-http/src/api/run_archive.rs",
  "core/crates/ctx-http/src/api/run_archive/",
];

const webSessionRestRouteApiRoots = [
  "core/crates/ctx-http/src/api/web_sessions/creation.rs",
  "core/crates/ctx-http/src/api/web_sessions/actions.rs",
];

const sessionHeadApiRoots = [
  "core/crates/ctx-http/src/api/sessions/snapshot/head.rs",
  "core/crates/ctx-http/src/api/sessions/snapshot/head_metrics.rs",
];

const sessionReadModelRouteApiRoots = [
  "core/crates/ctx-http/src/api/sessions/snapshot.rs",
  "core/crates/ctx-http/src/api/sessions/snapshot/events.rs",
  "core/crates/ctx-http/src/api/sessions/snapshot/head.rs",
  "core/crates/ctx-http/src/api/sessions/snapshot/history.rs",
  "core/crates/ctx-http/src/api/sessions/snapshot/state.rs",
];

const demoSeedTranscriptApiRoots = [
  "core/crates/ctx-http/src/api/demo/seed_transcript.rs",
];

const sessionControlRouteApiRoots = [
  "core/crates/ctx-http/src/api/sessions/control/interrupts.rs",
  "core/crates/ctx-http/src/api/sessions/control/authenticate.rs",
  "core/crates/ctx-http/src/api/sessions/control/ask_user.rs",
  "core/crates/ctx-http/src/api/sessions/file_completions.rs",
];

const sessionMessageCommandRouteApiRoots = [
  "core/crates/ctx-http/src/api/sessions/messages.rs",
  "core/crates/ctx-http/src/api/sessions/messages/",
];

const sessionSubagentRouteApiRoots = [
  "core/crates/ctx-http/src/api/sessions/subagents.rs",
  "core/crates/ctx-http/src/api/sessions/subagents/",
];

const sessionVcsApiRoots = [
  "core/crates/ctx-http/src/api/sessions/snapshot/vcs.rs",
  "core/crates/ctx-http/src/api/sessions/snapshot/vcs/",
];

const sessionModelSwitchApiRoots = [
  "core/crates/ctx-http/src/api/sessions/titles_and_modes.rs",
  "core/crates/ctx-http/src/api/sessions/titles_and_modes/",
];

const providerBootstrapApiRoots = [
  "core/crates/ctx-http/src/api/providers/bootstrap.rs",
  "core/crates/ctx-http/src/api/providers/bootstrap/",
];

const providerHarnessEndpointApiRoots = [
  "core/crates/ctx-http/src/api/providers/harness_config/endpoints.rs",
];

const providerHarnessConfigApiRoots = [
  "core/crates/ctx-http/src/api/providers/harness_config.rs",
  "core/crates/ctx-http/src/api/providers/types/harness.rs",
];

const providerInstallApiRoots = [
  "core/crates/ctx-http/src/api/provider_launch.rs",
  "core/crates/ctx-http/src/api/provider_launch/handlers/installs.rs",
  "core/crates/ctx-http/src/api/provider_launch/handlers/installs/",
];

const providerAdminApiRoots = [
  "core/crates/ctx-http/src/api/providers/install.rs",
  "core/crates/ctx-http/src/api/providers/types/dev.rs",
  "core/crates/ctx-http/src/api/providers/types/harness.rs",
];

const providerLaunchAuthApiRoots = [
  "core/crates/ctx-http/src/api/provider_launch/handlers/auth.rs",
  "core/crates/ctx-http/src/api/provider_launch/handlers/auth/",
];

const providerLaunchOptionsApiRoots = [
  "core/crates/ctx-http/src/api/provider_launch/handlers/options.rs",
  "core/crates/ctx-http/src/api/provider_launch/handlers/options/",
];

const providerAuthImportApiRoots = [
  "core/crates/ctx-http/src/api/providers/imports.rs",
  "core/crates/ctx-http/src/api/providers/types/auth_import.rs",
];

const providerStatusApiRoots = [
  "core/crates/ctx-http/src/api/providers/status.rs",
  "core/crates/ctx-http/src/api/providers/status/",
  "core/crates/ctx-http/src/api/providers/types/queries.rs",
];

const providerAccountsApiRoots = [
  "core/crates/ctx-http/src/api/providers/accounts.rs",
  "core/crates/ctx-http/src/api/providers/accounts/",
  "core/crates/ctx-http/src/api/providers/types/accounts.rs",
  "core/crates/ctx-http/src/api/providers/types/accounts/",
];

const providerUsageApiRoots = [
  "core/crates/ctx-http/src/api/providers/status/usage.rs",
  "core/crates/ctx-http/src/api/providers/accounts/codex/usage.rs",
  "core/crates/ctx-http/src/api/providers/types/queries.rs",
  "core/crates/ctx-http/src/api/providers/types/accounts/responses.rs",
];

const managedBrowserLoginApiRoots = [
  "core/crates/ctx-http/src/api/providers/login/browser/gemini.rs",
  "core/crates/ctx-http/src/api/providers/login/browser/gemini/",
  "core/crates/ctx-http/src/api/providers/login/browser/qwen.rs",
  "core/crates/ctx-http/src/api/providers/login/browser/qwen/",
  "core/crates/ctx-http/src/api/providers/login/browser/amp.rs",
  "core/crates/ctx-http/src/api/providers/login/browser/amp/",
  "core/crates/ctx-http/src/api/providers/login/mistral.rs",
  "core/crates/ctx-http/src/api/providers/login/mistral/",
  "core/crates/ctx-http/src/api/providers/login/kimi.rs",
  "core/crates/ctx-http/src/api/providers/login/kimi/",
];

const cursorProcessLoginApiRoots = [
  "core/crates/ctx-http/src/api/providers/cursor_login.rs",
  "core/crates/ctx-http/src/api/providers/cursor_login/",
];

const codexAppServerLoginApiRoots = [
  "core/crates/ctx-http/src/api/providers/login/codex.rs",
  "core/crates/ctx-http/src/api/providers/login/codex/",
];

const claudeSetupTokenLoginApiRoots = [
  "core/crates/ctx-http/src/api/providers/login/claude.rs",
  "core/crates/ctx-http/src/api/providers/login/claude/",
];

const providerLoginStatusApiRoots = [
  ...managedBrowserLoginApiRoots,
  ...cursorProcessLoginApiRoots,
  ...codexAppServerLoginApiRoots,
  ...claudeSetupTokenLoginApiRoots,
];

const executionApiRoots = [
  "core/crates/ctx-http/src/api/execution.rs",
  "core/crates/ctx-http/src/api/execution/",
];

const healthDiagnosticsApiRoots = [
  "core/crates/ctx-http/src/api/health.rs",
  "core/crates/ctx-http/src/api/diagnostics.rs",
];

const resourceUtilizationApiRoots = [
  "core/crates/ctx-http/src/api/resource_utilization.rs",
];

const settingsApiRoots = [
  "core/crates/ctx-http/src/api/settings.rs",
];

const telemetryApiRoots = [
  "core/crates/ctx-http/src/api/telemetry.rs",
];

const blobApiRoots = [
  "core/crates/ctx-http/src/api/artifacts/blob.rs",
  "core/crates/ctx-http/src/api/artifacts/blob/",
];

const logsApiRoots = [
  "core/crates/ctx-http/src/api/logs_api.rs",
];

const updateApiRoots = [
  "core/crates/ctx-http/src/api/updates/check.rs",
  "core/crates/ctx-http/src/api/updates/activity.rs",
  "core/crates/ctx-http/src/api/updates/appimage.rs",
  "core/crates/ctx-http/src/api/updates/appimage/",
];

const updateDrainApiRoots = [
  "core/crates/ctx-http/src/api/updates/drain.rs",
  "core/crates/ctx-http/src/api/updates/drain/",
];

const workspaceRegistrationConfigApiRoots = [
  "core/crates/ctx-http/src/api/workspaces/registry/create.rs",
  "core/crates/ctx-http/src/api/workspaces/management.rs",
  "core/crates/ctx-http/src/api/workspaces/management/config_ops/primary_branch.rs",
];

const workspaceExecutionConfigApiRoots = [
  "core/crates/ctx-http/src/api/workspaces/management.rs",
  "core/crates/ctx-http/src/api/workspaces/management/config_ops/execution.rs",
  "core/crates/ctx-http/src/api/workspaces/management/config_ops/execution/",
];

const workspaceManagementConfigApiRoots = [
  "core/crates/ctx-http/src/api/workspaces.rs",
  "core/crates/ctx-http/src/api/workspaces/context.rs",
  "core/crates/ctx-http/src/api/workspaces/management.rs",
  "core/crates/ctx-http/src/api/workspaces/management/config_ops.rs",
  "core/crates/ctx-http/src/api/workspaces/management/config_ops/",
  "core/crates/ctx-http/src/api/workspaces/management/prompt_config.rs",
  "core/crates/ctx-http/src/api/workspaces/management/prompt_config/",
  "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
  "core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs",
];

const workspacePromptAndModelConfigApiPaths = [
  "core/crates/ctx-http/src/api/workspaces/management/prompt_config/agent.rs",
  "core/crates/ctx-http/src/api/workspaces/management/prompt_config/subagent.rs",
  "core/crates/ctx-http/src/api/workspaces/management/provider_model_preferences.rs",
];

const workspaceRouteContractApiRoots = [
  "core/crates/ctx-http/src/api/workspaces.rs",
  "core/crates/ctx-http/src/api/workspaces/active.rs",
  "core/crates/ctx-http/src/api/workspaces/attachments.rs",
  "core/crates/ctx-http/src/api/workspaces/context.rs",
  "core/crates/ctx-http/src/api/workspaces/harness_container.rs",
  "core/crates/ctx-http/src/api/workspaces/management.rs",
  "core/crates/ctx-http/src/api/workspaces/registry.rs",
  "core/crates/ctx-http/src/api/workspaces/registry/",
  "core/crates/ctx-http/src/api/workspaces/worktrees.rs",
  "core/crates/ctx-http/src/api/workspaces/management/attachment_routes.rs",
  "core/crates/ctx-http/src/api/workspaces/management/attachment_ops.rs",
  "core/crates/ctx-http/src/api/workspaces/management/file_completions.rs",
  "core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs",
];

const workspaceRestRouteContractApiPaths = [
  "core/crates/ctx-http/src/api/workspaces.rs",
  "core/crates/ctx-http/src/api/workspaces/active.rs",
  "core/crates/ctx-http/src/api/workspaces/attachments.rs",
  "core/crates/ctx-http/src/api/workspaces/context.rs",
  "core/crates/ctx-http/src/api/workspaces/harness_container.rs",
  "core/crates/ctx-http/src/api/workspaces/management.rs",
  "core/crates/ctx-http/src/api/workspaces/registry.rs",
  "core/crates/ctx-http/src/api/workspaces/registry/delete.rs",
  "core/crates/ctx-http/src/api/workspaces/worktrees.rs",
  "core/crates/ctx-http/src/api/workspaces/management/attachment_routes.rs",
  "core/crates/ctx-http/src/api/workspaces/management/file_completions.rs",
  "core/crates/ctx-http/src/api/workspaces/management/worktree_bootstrap.rs",
];

const workspaceHarnessContainerApiPaths = [
  "core/crates/ctx-http/src/api/workspaces/harness_container.rs",
];

const workspaceFileCompletionApiPaths = [
  "core/crates/ctx-http/src/api/workspaces/management/file_completions.rs",
];

const taskSessionCreationApiRoots = [
  "core/crates/ctx-http/src/api/tasks/creation_session.rs",
  "core/crates/ctx-http/src/api/tasks/creation_session/",
];

const taskRouteContractApiRoots = [
  "core/crates/ctx-http/src/api/tasks.rs",
  "core/crates/ctx-http/src/api/tasks/creation.rs",
  "core/crates/ctx-http/src/api/tasks/creation_task.rs",
  "core/crates/ctx-http/src/api/tasks/creation_session.rs",
  "core/crates/ctx-http/src/api/tasks/creation_session/",
  "core/crates/ctx-http/src/api/tasks/handlers.rs",
  "core/crates/ctx-http/src/api/tasks/handlers/",
  "core/crates/ctx-http/src/api/tasks/task_deletion.rs",
  "core/crates/ctx-http/src/api/tasks/task_title.rs",
];

const workspaceStreamReadModelApiRoots = [
  "core/crates/ctx-http/src/api/ws/replay.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/lifecycle.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
];

const workspaceStreamSubscriptionPlanApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/",
];

const workspaceStreamSubscriptionTransactionApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
  "core/crates/ctx-http/src/api/ws/common/pins.rs",
];

const workspaceStreamReplayCursorApiRoots = [
  "core/crates/ctx-http/src/api/ws/common/cursor.rs",
  "core/crates/ctx-http/src/api/ws/queue/buffers/head/state.rs",
  "core/crates/ctx-http/src/api/ws/queue/buffers/summary.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/",
];

const workspaceStreamReplayProgramApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/",
];

const workspaceStreamEventRoutingApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_stream.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/events/",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/live_events.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/session.rs",
  "core/crates/ctx-http/src/api/ws/queue/partials.rs",
  "core/crates/ctx-http/src/api/ws/common/rev.rs",
];

const workspaceStreamEventRoutePlanApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/events/route.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/subscription/replay/session.rs",
];

const workspaceStreamLiveEventApplicationApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
  "core/crates/ctx-http/src/api/ws/workspace_stream/events/route.rs",
];

const workspaceStreamSubscriptionEventApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_stream/events.rs",
];

const workspaceVcsDemandApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/client.rs",
  "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/runtime.rs",
];

const workspaceVcsLiveRoutingApiRoots = [
  "core/crates/ctx-http/src/api/ws/workspace_vcs/socket.rs",
  "core/crates/ctx-http/src/api/ws/workspace_vcs/subscription/snapshots.rs",
];

const terminalStreamRuntimeApiRoots = [
  "core/crates/ctx-http/src/api/ws/terminal.rs",
  "core/crates/ctx-http/src/api/ws/terminal/",
  "core/crates/ctx-http/src/api/ws/tests/ws_queue_tests.rs",
];

const dictationWsConfigApiRoots = [
  "core/crates/ctx-http/src/api/ws/dictation_livekit.rs",
  "core/crates/ctx-http/src/api/ws/dictation_livekit/",
];

const workspaceWsAdmissionApiRoots = [
  "core/crates/ctx-http/src/api/ws/secure_mobile.rs",
  "core/crates/ctx-http/src/api/ws/terminal.rs",
  "core/crates/ctx-http/src/api/ws/workspace_active.rs",
  "core/crates/ctx-http/src/api/ws/workspace_vcs.rs",
];

const orgPolicyApiRoots = [
  "core/crates/ctx-http/src/api/org_policy.rs",
  "core/crates/ctx-http/src/api/org_policy/",
];

const repoOnboardingApiRoots = [
  "core/crates/ctx-http/src/api/repo.rs",
  "core/crates/ctx-http/src/api/repo/",
];

const smallApiUnitStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/api/settings.rs",
  "core/crates/ctx-http/src/api/providers/login/codex/tests.rs",
  "core/crates/ctx-http/src/api/providers/tests/mod.rs",
  "core/crates/ctx-http/src/api/providers/tests/install_statuses.rs",
  "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change/fixtures.rs",
  "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change/",
  "core/crates/ctx-http/src/api/providers/tests/restarts/harness_source.rs",
  "core/crates/ctx-http/src/api/sessions/tests.rs",
  "core/crates/ctx-http/src/api/sessions/tests/",
  "core/crates/ctx-http/src/api/workspaces/tests.rs",
];

const smallExternalStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
  "core/crates/ctx-http/tests/repo_clone_branch_and_safety.rs",
  "core/crates/ctx-http/tests/repo_init_initial_commit.rs",
  "core/crates/ctx-http/tests/repo_status_and_staging.rs",
  "core/crates/ctx-http/tests/repo_validate_destination.rs",
  "core/crates/ctx-http/tests/system_prompt_append_http.rs",
  "core/crates/ctx-http/tests/title_generation_local_e2e.rs",
  "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
  "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
  "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
];

const libTestDataRootFixtureRoots = [
  "core/crates/ctx-http/src/lib_tests.rs",
  "core/crates/ctx-http/src/lib_tests/",
];

const fakeDaemonExternalStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/acp_target_scoped_status.rs",
  "core/crates/ctx-http/tests/assistant_message_persistence_faults.rs",
  "core/crates/ctx-http/tests/cache_rehydration.rs",
  "core/crates/ctx-http/tests/demo_seed_transcript_http.rs",
  "core/crates/ctx-http/tests/fault_matrix.rs",
  "core/crates/ctx-http/tests/global_id_routing_http.rs",
  "core/crates/ctx-http/tests/hot_endpoints_no_db.rs",
  "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
  "core/crates/ctx-http/tests/install_start_contract.rs",
  "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
  "core/crates/ctx-http/tests/merge_queue_isolation.rs",
  "core/crates/ctx-http/tests/memory_leak_e2e.rs",
  "core/crates/ctx-http/tests/message_idempotency.rs",
  "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
  "core/crates/ctx-http/tests/provider_current_ctx_version_regressions.rs",
  "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
  "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
  "core/crates/ctx-http/tests/session_model_api.rs",
  "core/crates/ctx-http/tests/subscription_accounts_api.rs",
  "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
  "core/crates/ctx-http/tests/terminal_ws_reconnect.rs",
  "core/crates/ctx-http/tests/turn_lifecycle_events.rs",
  "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
  "core/crates/ctx-http/tests/workspace_attachments_local_canonical.rs",
  "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
  "core/crates/ctx-http/tests/workspace_stream_context_window_metrics.rs",
  "core/crates/ctx-http/tests/workspace_stream_no_gaps_under_activity.rs",
  "core/crates/ctx-http/tests/workspace_stream_stress_active_heads_lag.rs",
  "core/crates/ctx-http/tests/worktree_archive_http.rs",
  "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
];

const acpCrpBridgeTokenStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/acp_crp_bridge_tokens_e2e.rs",
];

const cacheRehydrationStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/cache_rehydration.rs",
];

const subscriptionAccountsApiStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/subscription_accounts_api.rs",
];

const sessionModelApiStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/session_model_api.rs",
];

const imageAttachmentsStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
];

const workspaceAttachmentsDemoStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/attachments_demo_react.rs",
];

const providerTargetScopedInstallsStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/provider_target_scoped_installs.rs",
];

const subagentMcpStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/subagent_mcp_http.rs",
];

const replayPropertiesStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/replay_properties.rs",
];

const worktreeVcsSnapshotStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
];

const providerProbeRuntimeEnvStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
];

const defaultSessionAndDiffFakeDaemonFixtureTestRoots = [
  "core/crates/ctx-http/tests/session_diff_unavailable.rs",
  "core/crates/ctx-http/tests/task_default_session_http.rs",
];

const workspaceVcsSetupFixtureTestRoots = [
  "core/crates/ctx-http/tests/disk_isolated_sandbox_smoke.rs",
  "core/crates/ctx-http/tests/disk_isolated_vcs_integrity.rs",
  "core/crates/ctx-http/tests/workspace_active_snapshot_http.rs",
];

const mcpDaemonFacadeTestRoots = [
  "core/crates/ctx-http-test-support/src/mcp_daemon.rs",
  "core/crates/ctx-http-test-support/src/mcp_daemon/",
];

const sessionFixtureStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/daemon_smoke.rs",
  "core/crates/ctx-http/src/lib_tests/daemon_smoke/",
  "core/crates/ctx-http/src/lib_tests/log_path_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/log_path_boundaries/",
  "core/crates/ctx-http/src/lib_tests/session_artifacts.rs",
  "core/crates/ctx-http/src/lib_tests/session_artifacts/",
  "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http.rs",
  "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http/",
  "core/crates/ctx-http/src/lib_tests/session_head_large_http.rs",
  "core/crates/ctx-http/src/lib_tests/session_head_large_http/",
  "core/crates/ctx-http/tests/task_default_session_http.rs",
];

const smallBoundaryStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/cors.rs",
  "core/crates/ctx-http/src/lib_tests/health_diagnostics/",
  "core/crates/ctx-http/src/lib_tests/org_policy_routes.rs",
  "core/crates/ctx-http/src/lib_tests/telemetry_export_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/update_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/workspace_active_routes.rs",
  "core/crates/ctx-http/src/lib_tests/run_archive_routes.rs",
  "core/crates/ctx-http/src/api/sessions/tests.rs",
  "core/crates/ctx-http/src/api/sessions/tests/title_generation.rs",
  "core/crates/ctx-http/src/api/workspaces/tests.rs",
];

const executionLaunchStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/execution_launch.rs",
  "core/crates/ctx-http/src/lib_tests/execution_launch/",
];

const providerlessLibRouteStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/cors.rs",
  "core/crates/ctx-http/src/lib_tests/health_diagnostics/",
  "core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_profile_routes.rs",
  "core/crates/ctx-http/src/lib_tests/org_policy_routes.rs",
  "core/crates/ctx-http/src/lib_tests/run_archive_routes.rs",
  "core/crates/ctx-http/src/lib_tests/telemetry_export_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/update_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/web_session_routes/fixtures.rs",
  "core/crates/ctx-http/src/lib_tests/workspace_active_routes.rs",
];

const globalIdRoutingStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/global_id_routing_http.rs",
];

const terminalWorkspaceStreamStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
  "core/crates/ctx-http/tests/terminal_ws_reconnect.rs",
];

const workspaceRuntimeSettingsStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/session_diff_unavailable.rs",
  "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
];

const workspaceMergeQueueConfigStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
];

const jjMergeQueueBasicsStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
];

const mergeQueueIsolationStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/merge_queue_isolation.rs",
];

const providerWorkerReapingStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
];

const providerScenariosOfflineStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
];

const harnessContainerSandboxStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/harness_container_sandbox_e2e.rs",
];

const liveProviderCanaryStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/live_provider_canary.rs",
];

const worktreeArchiveStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/worktree_archive_http.rs",
];

const faultInjectionStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/fault_matrix.rs",
  "core/crates/ctx-http/tests/hot_endpoints_no_db.rs",
];

const streamRuntimeStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
  "core/crates/ctx-http/tests/workspace_stream_context_window_metrics.rs",
  "core/crates/ctx-http/tests/workspace_stream_no_gaps_under_activity.rs",
  "core/crates/ctx-http/tests/workspace_stream_stress_active_heads_lag.rs",
];

const schedulerRuntimeStoreFacadeTestRoots = [
  "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
  "core/crates/ctx-http/tests/assistant_message_persistence_faults.rs",
  "core/crates/ctx-http/tests/turn_lifecycle_events.rs",
  "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
];

const smallRouteFixtureTestRoots = [
  "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
  "core/crates/ctx-http/tests/repo_clone_branch_and_safety.rs",
  "core/crates/ctx-http/tests/repo_init_initial_commit.rs",
  "core/crates/ctx-http/tests/repo_status_and_staging.rs",
  "core/crates/ctx-http/tests/repo_validate_destination.rs",
  "core/crates/ctx-http/tests/system_prompt_append_http.rs",
  "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
  "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
  "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
];

const providerAuthGlobalIdFixtureTestRoots = [
  "core/crates/ctx-http/tests/codex_host_import_api.rs",
  "core/crates/ctx-http/tests/codex_login_callback_api.rs",
  "core/crates/ctx-http/tests/global_id_routing_http.rs",
];

const updateRouteFixtureTestRoots = [
  "core/crates/ctx-http/tests/common/updates_failure_safety.rs",
  "core/crates/ctx-http/tests/updates_appimage_apply_safety.rs",
  "core/crates/ctx-http/tests/updates_failure_safety_checksum_mismatch.rs",
  "core/crates/ctx-http/tests/updates_failure_safety_interrupted_transfer.rs",
  "core/crates/ctx-http/tests/updates_failure_safety_manifest_parse.rs",
  "core/crates/ctx-http/tests/updates_failure_safety_manifest_signature.rs",
  "core/crates/ctx-http/tests/updates_failure_safety_missing_artifact.rs",
];

const taskLifecycleStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/api/tasks/cleanup_lifecycle_tests.rs",
  "core/crates/ctx-http/src/api/tasks/lifecycle_tests.rs",
  "core/crates/ctx-http/src/api/tasks/lifecycle_tests/",
  "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests.rs",
  "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/",
];

const storageAdmissionFixtureTestRoots = [
  "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests.rs",
  "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/",
];

const API_RAW_DAEMON_PATTERNS = [
  {
    name: "raw DaemonState type",
    regex: /\bDaemonState\b/,
  },
  {
    name: "raw daemon state extractor",
    regex: /State\s*<\s*Arc\s*<\s*DaemonState\s*>\s*>/,
  },
  {
    name: "raw daemon state arc",
    regex: /Arc\s*<\s*DaemonState\s*>/,
  },
  {
    name: "broad daemon handle extractor",
    regex: /State\s*<\s*DaemonHandle\s*>/,
  },
  {
    name: "router accepts broad daemon handle",
    regex: /a^/,
    contentRegex: /\bfn\s+router\s*\([^)]*\bDaemonHandle\b[^)]*\)/gm,
  },
  {
    name: "daemon handle escalation call",
    regex: /\.daemon_handle\s*\(/,
  },
  {
    name: "global store accessor",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "daemon store accessor",
    regex: /\.(?:(?:route_)?(?:load_)?(?:store_for_(?:workspace|worktree|task|session)|existing_(?:workspace|session)_store(?:_allow_archived|_for_write)?)|(?:route_|load_)(?:workspace|worktree|task|session|existing_workspace|existing_session(?:_allow_archived|_for_write)?)_store|route_(?:existing_)?(?:workspace|session)_store(?:_allow_archived|_for_write)?)\s*\(/,
  },
  {
    name: "broad daemon handle field",
    regex: /^\s*\w+\s*:\s*DaemonHandle\b/,
  },
];

const API_DOMAIN_RAW_STORE_PATTERNS = [
  {
    name: "raw ctx_store Store in daemon-blind API family",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
];

const HANDLE_BACKDOOR_PATTERNS = [
  {
    name: "raw daemon FromRef backdoor",
    regex: /FromRef\s*<\s*DaemonHandle\s*>\s*for\s*Arc\s*<\s*DaemonState\s*>/,
  },
  {
    name: "raw daemon state accessor",
    regex: /\bfn\s+state\s*\(\s*&self\s*\)\s*->\s*&\s*Arc\s*<\s*DaemonState\s*>/,
  },
  {
    name: "daemon handle escalation accessor",
    regex: /\bfn\s+daemon_handle\s*\(\s*&self\s*\)\s*->\s*DaemonHandle/,
  },
  {
    name: "secure proxy full-router backdoor",
    regex: /router\s*\(\s*handle\.clone\s*\(\s*\)\s*\)|Arc\s*<\s*axum::Router\s*>/,
  },
];

const DAEMON_EXTRACTION_BLOCKER_PATTERNS = [
  {
    name: "daemon depends on ctx-http",
    regex: /\bctx_http::/,
  },
  {
    name: "daemon imports API module",
    regex: /\bcrate::api\b|\bapi::router\b/,
    contentRegex: /\buse\s+crate::\s*\{[^;]*\bapi\b[^;]*\}\s*;/gm,
  },
  {
    name: "daemon depends on Axum",
    regex: /\buse\s+axum\b|\baxum::/,
  },
  {
    name: "daemon owns Axum extractor glue",
    regex: /\bFromRef\s*</,
  },
];

const TEST_RAW_DAEMON_BUCKET_PATTERNS = [
  {
    name: "raw daemon runtime bucket field access",
    regex: /\b(?:state|app_state|daemon_state)\s*\.\s*(?:core|sessions|workspaces|providers|telemetry|transport|execution)\s*\./,
  },
];

const MIGRATED_TEST_RAW_DAEMON_PATTERNS = [
  {
    name: "raw daemon state constructor in migrated test surface",
    regex: /\bDaemonState::new(?:_with_(?:public_base_url|runtime_flags))?\s*\(/,
  },
  {
    name: "raw daemon state arc in migrated test surface",
    regex: /Arc\s*<\s*DaemonState\s*>/,
  },
  {
    name: "raw daemon router wiring in migrated test surface",
    regex: /\bapi::router\s*\(\s*state(?:\.clone\s*\(\s*\))?\s*\)/,
  },
  {
    name: "raw common daemon state helper in migrated test surface",
    regex: /(?:\bcommon::|(?<![\w:.])\b)build_state\s*\(/,
  },
  {
    name: "raw common router helper in migrated test surface",
    regex: /(?:\bcommon::|(?<![\w:.])\b)router\s*\(/,
  },
  {
    name: "raw provider-session token helper in migrated test surface",
    regex: /(?:\bctx_daemon::daemon::|(?<!\.)\b)(?:issue_provider_session_mcp_token(?:_with_capabilities)?|revoke_provider_session_mcp_token)\s*\(/,
  },
  {
    name: "raw daemon scheduler helper in migrated test surface",
    regex: /\bctx_daemon::daemon::\s*scheduler\b|(?<![\w:.])daemon::scheduler::/,
    contentRegex: /\bctx_daemon::daemon::\s*\{(?=[^}]*\bscheduler\b)[^}]*\}/g,
  },
  {
    name: "raw daemon module alias in migrated test surface",
    regex: /\bctx_daemon::daemon\s+as\s+\w+/,
  },
  {
    name: "ctx-daemon crate alias in migrated test surface",
    regex: /\b(?:use|extern\s+crate)\s+ctx_daemon\s+as\s+\w+/,
  },
  {
    name: "raw ctx-daemon outer grouped daemon import in migrated test surface",
    regex: /\bctx_daemon::\s*\{(?=[^}\n]*\b(?:daemon|self\s+as)\b)[^}\n]*\}/,
    contentRegex: /\bctx_daemon::\s*\{(?=[^}]*\n)(?=[^}]*\b(?:daemon|self\s+as)\b)[^}]*\}/g,
  },
];

const TEST_ROUTER_COMPOSITION_PATTERNS = [
  {
    name: "direct API router composition outside test router helper",
    regex: /\b(?:ctx_http::)?api::router\s*\(|\bcrate::api::router\s*\(/,
  },
];

const MOBILE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct mobile test global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "legacy mobile test daemon construction",
    regex: /\btest_daemon\s*\(/,
  },
  {
    name: "raw mobile test StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const PROVIDER_TEST_CACHE_ACCESS_PATTERNS = [
  {
    name: "direct provider options cache closure access",
    regex: /\.test_with_provider_options_cache\s*\(/,
  },
  {
    name: "direct provider verify cache closure access",
    regex: /\.test_with_provider_verify_cache\s*\(/,
  },
  {
    name: "direct provider usage cache closure access",
    regex: /\.test_with_provider_usage_cache\s*\(/,
  },
];

const PROVIDER_ROUTE_SETUP_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct provider-route global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct provider-route session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct provider-route workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct provider-route uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct provider-route task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct provider-route worktree store access",
    regex: /\.store_for_worktree\s*\(/,
  },
  {
    name: "direct provider-route StoreManager access",
    regex: /\.stores\s*\(|\bStoreManager\b/,
  },
  {
    name: "direct provider-route StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct provider-route StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace(?:_uncached)?\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace(?:_uncached)?\s*\(/gm,
  },
  {
    name: "direct provider-route handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*(?:providers|sessions|workspaces|tasks)\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.(?:providers|sessions|workspaces|tasks)\s*\()/gm,
  },
  {
    name: "direct provider-route TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct provider-route lib-test daemon helper",
    regex: /\btest_daemon_(?:with_fake_provider_)?for_test\s*\(/,
  },
  {
    name: "direct provider-route router composition",
    regex: /\btest_router\s*\(|\b(?:crate::)?api::router\s*\(|\bRouteHandles::from_daemon_handle\s*\(/,
  },
  {
    name: "raw provider-route ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
];

const AUTH_BOUNDARY_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct auth-boundary global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct auth-boundary session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct auth-boundary workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct auth-boundary uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct auth-boundary task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct auth-boundary worktree store access",
    regex: /\.store_for_worktree\s*\(/,
  },
  {
    name: "direct auth-boundary StoreManager access",
    regex: /\.stores\s*\(|\bStoreManager\b/,
  },
  {
    name: "direct auth-boundary StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct auth-boundary StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace(?:_uncached)?\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace(?:_uncached)?\s*\(/gm,
  },
  {
    name: "direct auth-boundary handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*(?:providers|sessions|workspaces|tasks)\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.(?:providers|sessions|workspaces|tasks)\s*\()/gm,
  },
  {
    name: "direct auth-boundary TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct auth-boundary lib-test daemon helper",
    regex: /\btest_daemon_(?:with_fake_provider_)?for_test\s*\(/,
  },
  {
    name: "direct auth-boundary router composition",
    regex: /\btest_router\s*\(|\b(?:crate::)?api::router\s*\(|\bRouteHandles::from_daemon_handle\s*\(/,
  },
  {
    name: "raw auth-boundary ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
];

const EXTERNAL_PROVIDER_ROUTE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct external provider-route global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct external provider-route session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct external provider-route workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct external provider-route StoreManager access",
    regex: /\.stores\s*\(|\bStoreManager\b/,
  },
  {
    name: "direct external provider-route provider handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*providers\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.providers\s*\()/gm,
  },
  {
    name: "direct external provider-route provider facade import",
    regex: /\bProvidersHandle\b|\bctx_daemon::daemon::providers\b|\bctx_daemon::daemon::\{[^}]*\bproviders\b[^}]*\}/,
    contentRegex: /\bctx_daemon::daemon::\{[^}]*\bproviders\b[^}]*\}/gm,
  },
  {
    name: "direct external provider-route codex login session API access",
    regex: /\b(?:start_codex_login_session|codex_login_status|codex_login_statuses|claim_codex_login_callback|restore_codex_login_completion_token|remove_codex_login_session)\b/,
  },
  {
    name: "raw external provider-route ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
];

const GEMINI_LIVE_MODEL_CATALOG_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct Gemini live catalog common setup helper",
    regex: /\b(?:crate\s*::\s*)?common\s*::\s*(?:setup_store|build_daemon|router_for_daemon)\s*\(|\b(?:setup_store|build_daemon|router_for_daemon)\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\b(?:setup_store|build_daemon|router_for_daemon)\b[\s\S]*?;/gm,
  },
  {
    name: "direct Gemini live catalog providerless daemon helper",
    regex: /\b(?:crate\s*::\s*)?common\s*::\s*provider_route_providerless_daemon\s*\(|\bprovider_route_providerless_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bprovider_route_providerless_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct Gemini live catalog TestDaemon construction",
    regex: /\bTestDaemon\s*::\s*new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "raw Gemini live catalog store setup",
    regex: /\bStoreManager\b|\bctx_store\s*::\s*Store\b|\b[A-Za-z_][A-Za-z0-9_]*\s*::\s*open_sqlite\s*\(/,
    contentRegex: /\buse\s+ctx_store\s*::\s*\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
];

const MOBILE_ACCESS_STORE_DTO_API_PATTERNS = [
  {
    name: "mobile access API imports storage DTOs",
    regex: /\buse\s+ctx_store\s*::\s*store\s*::[^;]*(?:\bMobileAccessConfig\b|\bMobileDeviceUpsert\b|\bMobileDeviceSeqAdvance\b)/,
    contentRegex: /\buse\s+ctx_store\s*::\s*store\s*::\s*\{(?=[^}]*\n)[\s\S]*?(?:\bMobileAccessConfig\b|\bMobileDeviceUpsert\b|\bMobileDeviceSeqAdvance\b)[\s\S]*?\}/gm,
  },
  {
    name: "mobile access API references storage DTO path",
    regex: /\bctx_store\s*::\s*store\s*::\s*(?:MobileAccessConfig|MobileDeviceUpsert|MobileDeviceSeqAdvance)\b/,
  },
  {
    name: "mobile access API references storage DTO type",
    regex: /\b(?:MobileAccessConfig|MobileDeviceUpsert|MobileDeviceSeqAdvance)\b/,
  },
];

const MOBILE_ACCESS_ORCHESTRATION_API_PATTERNS = [
  {
    name: "mobile access API calls control plane directly",
    regex: /\breqwest\s*::\s*Client\b|\buse\s+reqwest\b|\bCTX_TUNNEL_CONTROL_PLANE_URL\b|\bresolve_control_plane_url\b/,
  },
  {
    name: "mobile access API owns mobile token helpers",
    regex: /\b(?:generate_mobile_api_token|generate_pairing_token|hash_api_token|hash_pairing_token)\s*\(/,
  },
  {
    name: "mobile access API owns mobile E2EE orchestration",
    regex: /\bctx_transport_runtime\s*::\s*mobile_e2ee\b|\bmobile_e2ee\s*::\s*(?:derive_key|decrypt|decrypt_pairing_request|encrypt|generate_keypair)\s*\(/,
  },
  {
    name: "mobile access API calls raw mobile access config facade",
    regex: /\.(?:get_mobile_access_config|upsert_mobile_access_config)\s*\(/,
  },
  {
    name: "mobile access API calls raw mobile profile facade",
    regex: /\.(?:create_mobile_connection_profile|list_mobile_connection_profiles|get_mobile_connection_profile|update_mobile_connection_profile_scopes|delete_mobile_connection_profile)\s*\(/,
  },
  {
    name: "mobile access API calls raw mobile pairing facade",
    regex: /\.(?:insert_mobile_pairing_token|consume_mobile_pairing_token)\s*\(/,
  },
  {
    name: "mobile access API calls raw mobile device facade",
    regex: /\.(?:list_mobile_devices|get_mobile_device|upsert_mobile_device|advance_mobile_device_seq)\s*\(/,
  },
  {
    name: "mobile access API calls raw mobile auth context facade",
    regex: /\.load_mobile_auth_context_for_profile\s*\(/,
  },
  {
    name: "mobile access API owns mobile scope parsing or defaults",
    regex: /\b(?:default_mobile_profile_scopes|mobile_scope_set_from_strings)\s*\(/,
  },
  {
    name: "mobile access API references raw mobile route DTOs",
    regex: /\b(?:MobileAccessConfigUpsert|MobileDeviceRegistrationUpdate|MobileDeviceSequenceAdvance)\b/,
  },
  {
    name: "mobile access API owns secure proxy router state",
    regex: /\bSecureProxyRouterState\b|\bdispatch_scoped_secure_proxy_request\s*\(/,
  },
  {
    name: "mobile access API owns secure proxy transport admission",
    regex: /\b(?:mobile_secure_proxy_allows_request|secure_proxy_path_is_unnormalized)\s*\(/,
  },
  {
    name: "mobile access API owns mobile secure proxy scope checks",
    regex: /\bMobileScope\s*::\s*WorkspaceRead\b|\.allows\s*\(\s*MobileScope\s*::\s*WorkspaceRead\s*\)/,
  },
  {
    name: "mobile access API builds secure proxy denial responses",
    regex: /\b(?:desktop_auth_required_secure_response|mobile_scope_required_secure_response)\s*\(/,
  },
  {
    name: "mobile access API owns secure proxy path dispatch",
    regex:
      /\bpath\s*==\s*"\/api\/(?:health|workspaces)"|\.strip_prefix\s*\(\s*"\/api\/workspaces\/"\s*\)/,
  },
  {
    name: "mobile access API re-enters proxied route handlers",
    regex: /\b(?:health|list_workspaces|get_workspace)\s*\(\s*State\s*\(/,
  },
  {
    name: "mobile access API owns secure proxy response reassembly",
    regex: /\bto_bytes\s*\(\s*resp\s*\.\s*into_body|\.into_body\s*\(/,
  },
  {
    name: "mobile access API owns secure proxy request header filtering",
    regex:
      /\bHeaderMap\s*::\s*new\s*\(|\bHeaderName\s*::\s*from_bytes\s*\(|\bHeaderValue\s*::\s*from_str\s*\(/,
  },
];

const MOBILE_PROFILE_ROUTE_PARAM_API_PATTERNS = [
  {
    name: "mobile profile route constructs ConnectionProfileId locally",
    regex: /\bConnectionProfileId\s*\(/,
  },
  {
    name: "mobile profile route parses profile UUID locally",
    regex: /\b(?:uuid\s*::\s*)?Uuid\s*::\s*parse_str\s*\(/,
  },
];

const ROUTE_FILE_DOWNLOAD_API_PATTERNS = [
  {
    name: "route file API reads or canonicalizes files directly",
    regex: /\btokio\s*::\s*fs\s*::\s*(?:read|metadata|canonicalize)\s*\(|\btokio\s*::\s*fs\s*::\s*File\s*::\s*open\s*\(/,
    contentRegex: /\buse\s+tokio\s*::\s*fs\s*::\s*\{(?=[^}]*\n)[\s\S]*?\b(?:read|metadata|canonicalize)\b[\s\S]*?\}/gm,
  },
  {
    name: "route file API owns symlink-safe open policy",
    regex: /\bstd\s*::\s*fs\s*::\s*OpenOptions\b|\bOpenOptionsExt\b|\bO_NOFOLLOW\b|\bspawn_blocking\s*\(/,
  },
  {
    name: "route file API calls local path root guard",
    regex: /\bpath_resolves_within_root\b/,
  },
  {
    name: "route file API reconstructs merge queue log root",
    regex: /\.join\s*\(\s*"merge-queue"\s*\)|\.join\s*\(\s*"logs"\s*\)/,
  },
  {
    name: "route file API calls worktree bootstrap log root facade",
    regex: /\.worktree_bootstrap_logs_root\s*\(/,
  },
  {
    name: "route file API calls session artifact path facades directly",
    regex: /\.(?:get_session_worktree|session_tool_output_spool_dir)\s*\(/,
  },
  {
    name: "route file API owns session artifact path authorization helpers",
    regex: /\b(?:resolve_session_artifact_accessible_path|validate_session_artifact_write_path|open_canonical_session_artifact_file|session_artifact_path_is_accessible|canonicalize_existing_or_raw)\b/,
  },
  {
    name: "route file API owns session artifact metadata derivation",
    regex: /\b(?:normalize_session_artifact_name|infer_session_artifact_mime_type|build_session_artifact_etag|build_session_artifact_last_modified)\b/,
  },
];

const SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS = [
  {
    name: "session artifact API owns local route id parsing",
    regex:
      /\b(?:SessionId|ArtifactId)\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "session artifact API owns local set request DTOs",
    regex: /\b(?:ArtifactInput|SetSessionArtifactsReq)\b/,
  },
  {
    name: "session artifact API constructs raw artifact inputs",
    regex: /\bSessionArtifactInput\b/,
  },
  {
    name: "session artifact API owns scoped MCP admission",
    regex: /\bvalidate_scoped_mcp_session_context\s*\(/,
  },
  {
    name: "session artifact API calls raw artifact facades",
    regex:
      /\.(?:list_session_artifacts_with_missing_for_route|set_session_artifacts_for_route|open_session_artifact_for_route)\s*\(/,
  },
];

const MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS = [
  {
    name: "merge queue submit API imports sessions handle",
    regex: /\bSessionsHandle\b/,
  },
  {
    name: "merge queue submit API parses session or worktree ids locally",
    regex: /\b(?:SessionId|WorktreeId)\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "merge queue submit API constructs low-level submit params",
    regex: /\bMergeQueueSubmitParams\b/,
  },
  {
    name: "merge queue submit API validates scoped MCP session context",
    regex: /\bvalidate_scoped_mcp_session_context\s*\(/,
  },
  {
    name: "merge queue submit API checks scoped MCP submit capability",
    regex: /\.allows_merge_queue_submit\s*\(/,
  },
  {
    name: "merge queue submit API reads scoped MCP ids directly",
    regex: /\bmcp_auth\s*\.\s*(?:session_id|worktree_id)\b/,
  },
  {
    name: "merge queue submit API calls low-level submit facade",
    regex: /\.submit_merge_queue_entry\s*\(/,
  },
];

const MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS = [
  {
    name: "merge queue entry API returns raw entry DTOs",
    regex:
      /\bctx_core::models::MergeQueueEntry\b|\bmodels::MergeQueueEntry\b|\bMergeQueueEntry\b/,
  },
  {
    name: "merge queue entry API calls raw action facades",
    regex:
      /\.(?:list_merge_queue_entries_for_route|cancel_merge_queue_entry|retry_merge_queue_entry)\s*\(/,
  },
  {
    name: "merge queue entry API owns local list request DTOs",
    regex: /\bMergeQueueListParams\b/,
  },
  {
    name: "merge queue entry API imports low-level merge queue crate",
    regex: /\bctx_merge_queue\b/,
  },
  {
    name: "merge queue entry API owns local route id parsing",
    regex:
      /\b(?:WorkspaceId|MergeQueueEntryId)\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "merge queue entry API calls raw log-download facade",
    regex: /\.download_merge_queue_entry_logs_for_route\s*\(/,
  },
  {
    name: "merge queue entry API maps low-level route-file errors",
    regex: /\bRouteFileDownloadError\b|\bmap_route_file_error\b/,
  },
];

const TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS = [
  {
    name: "terminal REST API exposes raw terminal route DTOs",
    regex:
      /\bctx_core::models::TerminalSession\b|\bmodels::TerminalSession\b|\bTerminalSession\b|\bctx_core::models::TerminalStatus\b|\bmodels::TerminalStatus\b|\bTerminalStatus\b/,
  },
  {
    name: "terminal REST API owns terminal ids or local id parsing",
    regex:
      /\b(?:WorkspaceId|TerminalId|TaskId|SessionId|WorktreeId)\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "terminal REST API owns local create or stream DTOs",
    regex: /\b(?:CreateTerminalReq|TerminalStreamConnectInfo)\b/,
  },
  {
    name: "terminal REST API imports low-level launch types",
    regex: /\b(?:CreateTerminalLaunchRequest|TerminalLaunchError|TerminalLaunchErrorKind)\b/,
  },
  {
    name: "terminal REST API calls raw terminal facades",
    regex:
      /\.(?:list_workspace_terminals|create_workspace_terminal|delete_terminal|mint_terminal_stream_token)\s*\(/,
  },
];

const RUN_ARCHIVE_API_ORCHESTRATION_PATTERNS = [
  {
    name: "run archive API owns route id parsing",
    regex: /\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(|\b(?:WorkspaceId|RunId)\s*\(/,
  },
  {
    name: "run archive API owns local validation/query helpers",
    regex:
      /\bstruct\s+RunArchiveBatchQuery\b|\bparse_archive_(?:workspace|run)_id\s*\(|\bmod\s+validation\s*;|\buse\s+validation\s*::/,
  },
  {
    name: "run archive API exposes raw archive body or response models",
    regex: /\bRunArchiveIngest(?:Batch|Cursor)\b/,
  },
  {
    name: "run archive API references low-level ingest errors",
    regex: /\bRunArchiveIngestError\b/,
  },
  {
    name: "run archive API validates acknowledgement batch fields",
    regex:
      /\bbatch\s*\.\s*run\s*\.\s*(?:workspace_id|id|org_id)\b|\bbatch\s*\.\s*scope\s*\.\s*is_cloud_visible\s*\(/,
  },
  {
    name: "run archive API owns batch item limit policy",
    regex:
      /\b(?:DEFAULT_RUN_ARCHIVE_BATCH_ITEMS|MAX_RUN_ARCHIVE_BATCH_ITEMS|requested_batch_item_limit)\b/,
  },
  {
    name: "run archive API calls low-level archive ingest methods",
    regex:
      /\.(?:build_run_archive_ingest_batch|acknowledge_run_archive_ingest_batch)\s*\(/,
  },
  {
    name: "run archive API owns ingest error mapping",
    regex: /\brun_archive_ingest_api_error\s*\(/,
  },
];

const WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS = [
  {
    name: "web-session REST API owns session/worktree ids or local id parsing",
    regex: /\buuid::Uuid::parse_str\s*\(|\b(?:SessionId|WorktreeId)\s*\(/,
  },
  {
    name: "web-session REST API exposes old local route DTOs",
    regex: /\b(?:WebSessionCreatePayload|WebSessionListQuery)\b/,
  },
  {
    name: "web-session REST API references low-level route errors",
    regex:
      /\b(?:WebSessionLaunchError|WebSessionLaunchErrorKind|WebSessionLaunchRequest|WebSessionActionError)\b/,
  },
  {
    name: "web-session REST API owns run/eval request defaults",
    regex:
      /\bWebSessionRunRequest\b|\btimeout_ms\b|\b5\s*\*\s*60\s*\*\s*1000\b|\b300000\b/,
  },
  {
    name: "web-session REST API calls low-level transport facades directly",
    regex:
      /\.(?:create_web_session|list_web_sessions|get_web_session|run_web_session|eval_web_session|close_web_session)\s*\(/,
  },
];

const SESSION_HEAD_API_ORCHESTRATION_PATTERNS = [
  {
    name: "session head API owns recovery timing",
    regex: /\bInstant\s*::\s*now\s*\(/,
  },
  {
    name: "session head API owns workspace lookup policy",
    regex: /\.(?:workspace_id_for_session|is_workspace_deleting)\s*\(/,
  },
  {
    name: "session head API owns read-model cache policy",
    regex: /\.(?:cached_session_head_for_request|update_session_head_cache)\s*\(/,
  },
  {
    name: "session head API owns store rebuild policy",
    regex: /\.load_session_head_snapshot_from_store\s*\(/,
  },
  {
    name: "session head API owns cache recovery telemetry",
    regex: /\.(?:emit_cache_miss|emit_cache_rehydrate|record_session_head_recovery_metrics)\s*\(|\brecord_session_head_recovery_metrics\s*\(/,
  },
  {
    name: "session head API owns stale min_event_seq policy",
    regex: /\blast_event_seq\s*<\s*min_event_seq\b/,
  },
];

const SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS = [
  {
    name: "session read-model API exposes raw read-model DTOs",
    regex:
      /\b(?:ctx_core::models::|models::)?(?:SessionSnapshot|SessionHeadSnapshot|SessionHistoryPage|SessionEventsPage|SessionState|SessionTurnTool)\b|\bJson\s*<\s*(?:Vec\s*<\s*)?(?:SessionSnapshot|SessionHeadSnapshot|SessionHistoryPage|SessionEventsPage|SessionState|SessionTurnTool)\b/,
  },
  {
    name: "session read-model API owns local route query DTOs",
    regex:
      /\b(?:SessionSnapshotQuery|SessionHeadQuery|SessionHistoryQuery|SessionEventsQuery)\b/,
  },
  {
    name: "session read-model API owns session or turn id parsing",
    regex: /\b(?:SessionId|TurnId)\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "session read-model API owns boolish flag parsing",
    regex: /\bparse_boolish_flag\s*\(|\binclude_(?:events|transient)\s*\.\s*as_deref\s*\(/,
  },
  {
    name: "session read-model API calls raw read-model facades",
    regex:
      /\.(?:load_session_snapshot|load_session_history_page|list_session_events_page|list_session_turn_tools_for_request|load_session_state)\s*\(/,
  },
];

const DEMO_SEED_TRANSCRIPT_API_ROUTE_CONTRACT_PATTERNS = [
  {
    name: "demo seed transcript API owns session id parsing",
    regex: /\bSessionId\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "demo seed transcript API owns local route DTOs",
    regex: /\b(?:SeedTranscriptReq|SeedTranscriptResp|SeedTranscriptTurnReq)\b/,
  },
  {
    name: "demo seed transcript API constructs raw demo seed domain objects",
    regex: /\b(?:DemoSeedTranscript|DemoSeedTranscriptTurn|DemoSeedTranscriptError)\b/,
  },
  {
    name: "demo seed transcript API owns empty-turn validation",
    regex: /\.turns\s*\.\s*is_empty\s*\(/,
  },
  {
    name: "demo seed transcript API calls raw seed transcript facade",
    regex: /\.seed_demo_transcript\s*\(/,
  },
];

const SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS = [
  {
    name: "session control API owns session id parsing",
    regex: /\bSessionId\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "session control API owns local route DTOs",
    regex:
      /\b(?:AuthenticateSessionReq|SubmitAskUserQuestionReq|SubmitAskUserQuestionResp|FileCompletionsQuery)\b/,
  },
  {
    name: "session control API imports low-level daemon control errors",
    regex:
      /\b(?:SessionSchedulerCommandError|SessionAuthError|SubmitAskUserAnswerError|AskUserQuestionOutcome|FileCompletionsError|FileCompletionsErrorKind)\b/,
  },
  {
    name: "session control API calls raw control facades",
    regex:
      /\.(?:cancel_session|interrupt_session|authenticate_session_for_request|submit_ask_user_answer|complete_files_for_session)\s*\(/,
  },
];

const SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS = [
  {
    name: "session message command API owns id parsing",
    regex: /\b(?:SessionId|MessageId|TurnId)\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "session message command API owns local route DTOs",
    regex: /\b(?:PostMessageReq|PostMessageParts)\b/,
  },
  {
    name: "session message command API owns delivery or attachment contracts",
    regex: /\b(?:MessageAttachment|MessageDelivery|MessageClientIdResolutionError)\b|\bresolve_message_client_ids\s*\(/,
  },
  {
    name: "session message command API owns attachment blob normalization",
    regex: /\bSessionImageBlobStoreError\b|\.(?:store_inline_image_blob|get_blob)\s*\(|\bbase64\s*::|\b(?:decode_inline_image_attachment|ensure_image_attachment_mime_type|ensure_image_attachment_size|image_attachment_too_large_error|load_image_blob_metadata|normalize_message_attachments)\s*\(/,
  },
  {
    name: "session message command API owns queued-message env policy",
    regex: /\bCTX_QUEUED_MESSAGES_ENABLED\b|\b(?:queued_messages_enabled|env_bool)\s*\(/,
  },
  {
    name: "session message command API imports low-level message errors",
    regex: /\b(?:PostUserMessageInput|PostUserMessageError|SessionSchedulerCommandError)\b/,
  },
  {
    name: "session message command API calls raw message facades",
    regex: /\.(?:post_user_message_for_request|delete_queued_session_message)\s*\(/,
  },
];

const SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS = [
  {
    name: "session subagent API owns id parsing",
    regex: /\b(?:SessionId|TurnId)\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "session subagent API owns local route DTOs",
    regex: /\bSessionSubagentInvocationsQuery\b/,
  },
  {
    name: "session subagent API imports low-level subagent errors",
    regex: /\b(?:SubagentError|SubagentErrorKind|ScopedMcpSessionAccessError)\b/,
  },
  {
    name: "session subagent API imports raw subagent wire DTOs",
    regex:
      /\b(?:SpawnAgentReq|SendInputReq|ArchiveAgentReq|GetAgentReq|InterruptAgentReq|WaitAgentReq|SpawnAgentResp|SendInputResp|ArchiveAgentResp|GetAgentResp|InterruptAgentResp|WaitAgentResp)\b/,
  },
  {
    name: "session subagent API exposes raw subagent models",
    regex:
      /\b(?:SessionSummary|SubagentInvocation|AgentSummary)\b|\bJson\s*<\s*(?:Vec\s*<\s*)?(?:SessionSummary|SubagentInvocation|AgentSummary)\b/,
  },
  {
    name: "session subagent API redacts scoped errors",
    regex: /\blogs\s*::\s*redact_sensitive\s*\(|\bredact_sensitive\s*\(/,
  },
  {
    name: "session subagent API validates scoped MCP directly",
    regex: /\b(?:require_scoped_mcp_session_context|resolve_scoped_parent_session_id)\s*\(/,
  },
  {
    name: "session subagent API calls raw subagent facades",
    regex:
      /(?:\.|\bSessionsHandle::)(?:spawn_agent|send_input|archive_agent|list_agents|get_agent|interrupt_agent|wait_agent|list_session_subagents_for_request|list_session_subagent_invocations_for_request|get_session_subagent_invocation_for_request)\s*\(/,
  },
];

const SESSION_VCS_API_ORCHESTRATION_PATTERNS = [
  {
    name: "session VCS API owns session id parsing",
    regex: /\bSessionId\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "session VCS API owns local route DTOs",
    regex:
      /\b(?:SessionDiffApplyReq|SessionDiffRouteQuery|SessionDiffResponse|SessionDiffSummaryResponse|SessionGitStatusResponse|SessionGitStatusEntryResponse)\b/,
  },
  {
    name: "session VCS API imports low-level route contracts",
    regex:
      /\b(?:SessionVcsApplyAction|SessionVcsDiff|SessionVcsDiffQuery|SessionVcsDiffSummary|SessionVcsError|SessionVcsGitStatus|SessionVcsGitStatusEntry)\b|\bctx_daemon::daemon::sessions::vcs\b/,
  },
  {
    name: "session VCS API redacts low-level route errors",
    regex: /\blogs\s*::\s*redact_sensitive\s*\(|\bredact_sensitive\s*\(/,
  },
  {
    name: "session VCS API calls raw VCS facades",
    regex:
      /(?:\.|\bSessionsHandle::)(?:get_session_vcs_diff_for_request|get_session_vcs_diff_summary_for_request|apply_session_vcs_diff_patch_for_request|get_session_vcs_git_status_for_request)\s*\(/,
  },
  {
    name: "session VCS API imports workspace VCS service",
    regex: /\buse\s+ctx_workspace_services\s*::\s*worktree_vcs\b|\bctx_workspace_services\s*::\s*worktree_vcs\s*::/,
    contentRegex: /\buse\s+ctx_workspace_services\s*::\s*worktree_vcs\s*::\s*\{(?=[^}]*\n)[\s\S]*?\}/gm,
  },
  {
    name: "session VCS API aliases workspace VCS service",
    regex: /\buse\s+ctx_workspace_services\s*::\s*worktree_vcs\s+as\s+\w+\b/,
  },
  {
    name: "session VCS API calls workspace VCS service helper",
    regex: /\b(?:apply_worktree_vcs_session_patch|is_no_vcs_repo_error|session_git_status_summary_from_snapshot|worktree_vcs_diff_summary_mismatch|worktree_vcs_session_diff_available|worktree_vcs_session_diff_unavailable|worktree_vcs_session_diff_summary_available|worktree_vcs_session_diff_summary_no_repo|worktree_vcs_session_diff_summary_unavailable|resolve_worktree_diff_base_from_source)\s*\(/,
  },
  {
    name: "session VCS API references workspace VCS service type",
    regex: /\b(?:WorktreeDiffBaseResolution|WorktreeVcsDiffBaseQuery|WorktreeVcsSessionDiffOutcome|WorktreeVcsSessionDiffSummaryOutcome|WorktreeVcsDiffSummaryCounts|GitStatusSnapshot|GitStatusEntry)\b/,
  },
];

const SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS = [
  {
    name: "session title/model/mode API owns session id parsing",
    regex: /\bSessionId\b|\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(/,
  },
  {
    name: "session title/model/mode API exposes raw Session success shape",
    regex:
      /\bctx_core::models::Session\b|\buse\s+ctx_core::models::[^;]*\bSession\b|\bJson\s*<\s*Session\s*>/,
  },
  {
    name: "session title/model/mode API owns local route DTOs",
    regex: /\b(?:GenerateSessionTitleReq|SetSessionModelReq|SetSessionModeReq)\b/,
  },
  {
    name: "session title/model/mode API imports low-level route errors",
    regex:
      /\b(?:GenerateSessionTitleError|SetSessionModeError|SetSessionModelRequest|SetSessionModelError|SetSessionModelErrorKind)\b/,
  },
  {
    name: "session title/model/mode API redacts low-level route errors",
    regex: /\blogs\s*::\s*redact_sensitive\s*\(|\bredact_sensitive\s*\(/,
  },
  {
    name: "session title/model/mode API calls raw title/model/mode facades",
    regex:
      /\.(?:generate_session_title_for_request|set_session_model_for_request|set_session_mode_for_request)\s*\(/,
  },
  {
    name: "session model API imports model-resolution helpers directly",
    regex:
      /\bctx_session_tools::model_resolution\b|\b(?:compose_model_id|normalize_effort_id|resolve_model_id)\b/,
  },
  {
    name: "session model API imports provider install target directly",
    regex: /\bctx_provider_install::install_state::InstallTarget\b|\bInstallTarget\b/,
  },
  {
    name: "session model API imports provider adapter directly",
    regex: /\bctx_providers::adapters::ProviderAdapter\b|\bProviderAdapter\b/,
  },
  {
    name: "session model API loads target parts directly",
    regex: /(?:\.|\bSessionsHandle::)load_session_model_target_parts\s*\(/,
  },
  {
    name: "session model API ensures provider adapter directly",
    regex: /(?:\.|\bSessionsHandle::)ensure_provider_adapter_for_target\s*\(/,
  },
  {
    name: "session model API loads provider model catalog directly",
    regex: /(?:\.|\bSessionsHandle::)load_provider_model_catalog_for_execution_environment\s*\(/,
  },
  {
    name: "session model API persists model update directly",
    regex: /(?:\.|\bSessionsHandle::)persist_session_model_update_for_request\s*\(/,
  },
  {
    name: "session model API defines old orchestration helpers",
    regex:
      /\b(?:load_session_model_target|resolve_session_model_update|switch_live_session_model|persist_session_model_update)\s*\(/,
  },
];

const PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS = [
  {
    name: "provider bootstrap API parses workspace ids directly",
    regex: /\bWorkspaceId\b|\buuid::Uuid::parse_str\s*\(/,
  },
  {
    name: "provider bootstrap API matches bootstrap errors directly",
    regex: /\bProvidersBootstrapError(?:Kind)?\b/,
  },
  {
    name: "provider bootstrap API owns bootstrap error JSON",
    regex: /\bserde_json::json!\s*\(/,
  },
  {
    name: "provider bootstrap API calls broad bootstrap facade",
    regex: /(?:\.|\bProvidersHandle::|\b)workspace_providers_bootstrap\s*\(/,
  },
  {
    name: "provider bootstrap API checks workspace existence directly",
    regex: /(?:\.|\bProvidersHandle::)workspace_exists\s*\(/,
  },
  {
    name: "provider bootstrap API resolves install target directly",
    regex: /(?:\.|\bProvidersHandle::)install_target_for_workspace\s*\(/,
  },
  {
    name: "provider bootstrap API loads preferred models directly",
    regex: /(?:\.|\bProvidersHandle::)load_preferred_new_session_models\s*\(|\bload_preferred_model_by_provider\s*\(/,
  },
  {
    name: "provider bootstrap API loads provider statuses directly",
    regex: /(?:\.|\bProvidersHandle::)providers_statuses_response\s*\(/,
  },
  {
    name: "provider bootstrap API builds provider options directly",
    regex: /(?:\.|\bProvidersHandle::)build_bootstrap_options\s*\(/,
  },
  {
    name: "provider bootstrap API owns provider visibility filtering",
    regex: /\bvisible_provider_count_hint\s*\(|\.detail_flag\s*\(\s*"ui_hidden"/,
  },
  {
    name: "provider bootstrap API owns bootstrap workspace helper",
    regex: /\bload_bootstrap_workspace\s*\(/,
  },
  {
    name: "provider bootstrap API loads bootstrap accounts directly",
    regex: /\bload_bootstrap_accounts\s*\(|\baccounts::(?:codex|claude|gemini|qwen|kimi|mistral|copilot|cursor|amp)_accounts_response\s*\(/,
  },
];

const PROVIDER_HARNESS_ENDPOINT_API_PATTERNS = [
  {
    name: "provider harness endpoint API imports harness source domain",
    regex: /\bctx_harness_sources\b|\bharness_sources\b/,
  },
  {
    name: "provider harness endpoint API constructs low-level endpoint upsert",
    regex: /\bHarnessEndpointUpsert\b/,
  },
  {
    name: "provider harness endpoint API owns delete error taxonomy",
    regex: /\bprovider_harness_delete_error\b|\bunknown endpoint\b/,
  },
  {
    name: "provider harness endpoint API redacts lower-level errors",
    regex: /\blogs\s*::\s*redact_sensitive\s*\(/,
  },
  {
    name: "provider harness endpoint API calls low-level endpoint facades",
    regex:
      /\.(?:upsert_provider_harness_endpoint|refresh_provider_harness_endpoint_models|set_provider_harness_endpoint_manual_models|delete_provider_harness_endpoint)\s*\(/,
  },
];

const PROVIDER_HARNESS_CONFIG_API_PATTERNS = [
  {
    name: "provider harness config API imports harness source domain",
    regex: /\bctx_harness_sources\b|\bharness_sources\b/,
  },
  {
    name: "provider harness config API owns select request DTO",
    regex: /\bSelectHarnessSourceReq\b/,
  },
  {
    name: "provider harness config API references source kind directly",
    regex: /\bHarnessSourceKind\b/,
  },
  {
    name: "provider harness config API calls broad get/select facades",
    regex: /\.(?:get_provider_harness_config|select_provider_harness_source)\s*\(/,
  },
  {
    name: "provider harness config API owns bad-request mapping",
    regex: /\bprovider_harness_bad_request_error\b/,
  },
  {
    name: "provider harness config API redacts lower-level errors",
    regex: /\blogs\s*::\s*redact_sensitive\s*\(/,
  },
];

const PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS = [
  {
    name: "provider install API imports install domain directly",
    regex: /\bctx_provider_install::install_state\b/,
  },
  {
    name: "provider install API parses install target directly",
    regex: /\bparse_provider_install_target\b/,
  },
  {
    name: "provider install API owns old install route DTOs",
    regex:
      /\b(?:InstallTargetQuery|InstallStartResponse|GetInstallStatusesReq|InstallStatusBatchItem|GetInstallStatusesResp)\b/,
  },
  {
    name: "provider install API references low-level install domain types",
    regex: /\b(?:InstallId|InstallInfo|InstallProgressEvent|InstallTarget)\b/,
  },
  {
    name: "provider install API calls low-level install facades",
    regex:
      /\.(?:start_provider_install|start_all_provider_installs|get_provider_install_info|cancel_provider_install|list_provider_install_events|provider_install_event_sender)\s*\(/,
  },
  {
    name: "provider install API parses install ids directly",
    regex: /\buuid::Uuid::parse_str\s*\(/,
  },
];

const PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS = [
  {
    name: "provider admin API owns matrix refresh DTOs",
    regex: /\bMatrixRefreshResponse\b/,
  },
  {
    name: "provider admin API owns dev restart DTOs",
    regex: /\bDevRestartProviders(?:Req|Resp|Result)\b/,
  },
  {
    name: "provider admin API reads dev-mode env directly",
    regex: /\bCTX_DEV_MODE\b|\bdev_tools_enabled\b|\bparse_boolish\b|\bstd::env::var\s*\(/,
  },
  {
    name: "provider admin API parses restart modes directly",
    regex: /\bProviderRestartMode\b|\bparse_restart_mode\b/,
  },
  {
    name: "provider admin API calls low-level admin facades",
    regex: /\.(?:refresh_provider_inventory|restart_all_provider_adapters)\s*\(/,
  },
  {
    name: "provider admin API owns matrix refresh error mapping",
    regex: /failed to refresh provider statuses/,
  },
  {
    name: "provider admin API maps restart results locally",
    regex: /\bresult\.(?:provider_id|status|message)\b/,
  },
];

const PROVIDER_LAUNCH_AUTH_API_PATTERNS = [
  {
    name: "provider launch auth API owns auth response DTO",
    regex: /\bProviderAuthCheckResp\b/,
  },
  {
    name: "provider launch auth API references auth check snapshot directly",
    regex: /\bProviderAuthCheckSnapshot\b/,
  },
  {
    name: "provider launch auth API matches auth check errors directly",
    regex: /\bProviderAuthCheckError\b/,
  },
  {
    name: "provider launch auth API owns auth request DTO",
    regex: /\bAuthenticateProviderReq\b/,
  },
  {
    name: "provider launch auth API calls broad auth/verify facades",
    regex: /\.(?:authenticate_provider_for_workspace|verify_provider_for_workspace)\s*\(/,
  },
  {
    name: "provider launch auth API owns auth error mapping",
    regex: /\bprovider_auth_check_error_json\b/,
  },
  {
    name: "provider launch auth API parses workspace id directly",
    regex: /\bparse_workspace_id\s*\(/,
  },
  {
    name: "provider launch auth API uses shared route error helpers directly",
    regex: /\b(?:workspace_execution_settings_error_json|provider_launch_config_error_response)\s*\(/,
  },
];

const PROVIDER_LAUNCH_OPTIONS_API_PATTERNS = [
  {
    name: "provider launch options API matches options errors directly",
    regex: /\bProviderOptionsResponseError\b/,
  },
  {
    name: "provider launch options API calls broad options method facade",
    regex: /\.get_provider_options_response\s*\(/,
  },
  {
    name: "provider launch options API calls options free function directly",
    regex: /(?:^|[^.\w])get_provider_options_response\s*\(/,
  },
  {
    name: "provider launch options API parses workspace id directly",
    regex: /\bparse_workspace_id\s*\(/,
  },
  {
    name: "provider launch options API parses UUIDs directly",
    regex: /\buuid::Uuid::parse_str\s*\(/,
  },
  {
    name: "provider launch options API owns options error mapping",
    regex: /\bprovider_options_response_error_json\b/,
  },
  {
    name: "provider launch options API uses shared route error helpers directly",
    regex: /\b(?:workspace_execution_settings_error_json|provider_launch_config_error_response)\s*\(/,
  },
  {
    name: "provider launch options API redacts errors directly",
    regex: /\blogs::redact_sensitive\s*\(/,
  },
];

const PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS = [
  {
    name: "provider auth import API imports auth-import domain directly",
    regex: /\bctx_provider_auth_import\b/,
  },
  {
    name: "provider auth import API owns old route DTOs",
    regex:
      /\b(?:ProviderAuthImportCandidatesResponse|ProviderAuthImportProfilesResponse|ProviderAuthImportReq|ProviderAuthImportResponse)\b/,
  },
  {
    name: "provider auth import API calls candidates free function directly",
    regex:
      /\bctx_daemon::daemon::providers::list_provider_auth_import_candidates\s*\(/,
  },
  {
    name: "provider auth import API calls broad auth-import facades",
    regex:
      /\.(?:list_provider_auth_import_profiles|import_provider_auth_candidates)\s*\(/,
  },
  {
    name: "provider auth import API redacts errors locally",
    regex: /\blogs\s*::\s*redact_sensitive\s*\(/,
  },
  {
    name: "provider auth import API stringifies lower-level errors locally",
    regex: /\b(?:e|err)\s*\.to_string\s*\(/,
  },
];

const PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS = [
  {
    name: "provider status API parses install target directly",
    regex: /\bparse_provider_install_target\b/,
  },
  {
    name: "provider status API owns install target query DTO",
    regex: /\bInstallTargetQuery\b/,
  },
  {
    name: "provider status API matches provider status errors directly",
    regex: /\bProviderStatusResponseError\b/,
  },
  {
    name: "provider status API calls broad status facades",
    regex: /\.(?:providers_statuses_response|provider_status_response)\s*\(/,
  },
  {
    name: "provider status API owns provider status error mapping",
    regex: /\bprovider_status_response_error\b/,
  },
];

const PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS = [
  {
    name: "provider usage API imports provider-runtime usage DTOs",
    regex:
      /\bctx_provider_runtime::provider_usage\b|\bprovider_usage::ProviderUsageSnapshot\b/,
  },
  {
    name: "provider usage API owns route query DTO",
    regex: /\bProviderUsageQuery\b/,
  },
  {
    name: "provider usage API owns Codex account usage DTOs",
    regex: /\b(?:CodexAccountsUsageResponse|CodexAccountUsageEntry)\b/,
  },
  {
    name: "provider usage API calls low-level usage facades",
    regex:
      /(?:\.|\bProvidersHandle::|\b)(?:load_provider_usage|load_codex_accounts_usage)\s*\(/,
  },
  {
    name: "provider usage API owns usage internal-error mapping",
    regex: /\bprovider_usage_internal_error\b/,
  },
  {
    name: "provider usage API maps account usage records locally",
    regex:
      /\bCodexAccountUsageRecord\b|\baccount_id\s*:\s*entry\.account_id\b|\busage\s*:\s*entry\.usage\b/,
  },
];

const PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS = [
  {
    name: "provider account API loads account registries directly",
    regex:
      /(?:\.|\bProvidersHandle::)(?:load_(?:codex|amp|claude|copilot|cursor|gemini|kimi|mistral|qwen)_account_registry|load_codex_accounts_snapshot|ensure_amp_account_registry_from_runtime_auth)\s*\(/,
  },
  {
    name: "provider account API mutates accounts directly",
    regex:
      /(?:\.|\bProvidersHandle::)(?:import_host_codex_auth|set_active_(?:codex|amp|claude|copilot|cursor|gemini|kimi|mistral|qwen)_account|remove_(?:codex|amp|claude|copilot|cursor|gemini|kimi|mistral|qwen)_account|add_(?:claude|copilot|cursor|gemini|kimi|qwen)_account|upsert_(?:amp|mistral)_account)(?!_response)\s*\(/,
  },
  {
    name: "provider account API matches account mutation errors directly",
    regex: /\bProviderAccountMutationError\b/,
  },
  {
    name: "provider account API defines local account response builders",
    regex:
      /\b(?:codex|amp|claude|copilot|cursor|gemini|kimi|mistral|qwen)_accounts_response_from_snapshot\s*\(|\b(?:pub\(crate\)\s+)?async\s+fn\s+(?:codex|amp|claude|copilot|cursor|gemini|kimi|mistral|qwen)_accounts_response\s*\(/,
  },
  {
    name: "provider account API owns unknown-account or mutation error mapping",
    regex:
      /\b(?:unknown_account|provider_account_mutation_error|provider_account_delete_error|codex_account_set_active_error)\s*\(/,
  },
  {
    name: "provider account API owns account request DTOs",
    regex:
      /\b(?:CodexActiveAccountReq|CodexHostImportReq|ClaudeAccountUpsertReq|ClaudeActiveAccountReq|GeminiAccountUpsertReq|GeminiActiveAccountReq|QwenAccountUpsertReq|QwenActiveAccountReq|KimiAccountUpsertReq|KimiActiveAccountReq|MistralAccountUpsertReq|MistralActiveAccountReq|CopilotAccountUpsertReq|CopilotActiveAccountReq|CursorAccountUpsertReq|CursorActiveAccountReq|AmpAccountUpsertReq|AmpActiveAccountReq)\b/,
  },
  {
    name: "provider account API calls broad account response facades",
    regex:
      /(?:\.|\bProvidersHandle::)(?:(?:codex|amp|claude|copilot|cursor|gemini|kimi|mistral|qwen)_accounts_response|import_host_codex_auth_response|set_active_(?:codex|amp|claude|copilot|cursor|gemini|kimi|mistral|qwen)_account_response|delete_(?:codex|amp|claude|copilot|cursor|gemini|kimi|mistral|qwen)_account_response|add_(?:claude|copilot|cursor|gemini|kimi|qwen)_account_response|upsert_(?:amp|mistral)_account_response)\s*\(/,
  },
  {
    name: "provider account API calls low-level Codex host import probe",
    regex:
      /\bprobe_host_codex_auth_candidate\s*\(|\bprovider_accounts::CodexHostImportProbe\b/,
  },
];

const MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS = [
  {
    name: "managed browser login API owns monitor task spawning",
    regex: /\btokio::spawn\s*\(/,
  },
  {
    name: "managed browser login API declares monitor module",
    regex: /(?:#\s*\[\s*path\s*=\s*"[^"]*monitor\.rs"\s*\]\s*)?mod\s+monitor\s*;/,
  },
  {
    name: "managed browser login API constructs provider auth request",
    regex:
      /\bProviderSessionAuthenticationRequest\b|\bauthenticate_provider_session\s*\(/,
  },
  {
    name: "managed browser login API owns login path or env preparation",
    regex:
      /\b(?:prepare_(?:gemini|qwen|amp|mistral)_login_paths|(?:gemini|qwen|amp|mistral)_login_provider_env)\s*\(/,
  },
  {
    name: "managed browser login API mutates login status directly",
    regex:
      /\b(?:set_(?:gemini|qwen|amp|mistral|kimi)_login_[a-z0-9_]+|finish_(?:gemini|qwen|amp|mistral|kimi)_login_session)\s*\(/,
  },
  {
    name: "managed browser login API finalizes provider accounts directly",
    regex:
      /\b(?:add_(?:gemini|qwen)_account_for_login|add_kimi_oauth_account_for_login|upsert_(?:amp|mistral)_account_for_login)\s*\(/,
  },
  {
    name: "managed browser login API owns login cleanup",
    regex: /\bremove_dir_all\s*\(/,
  },
  {
    name: "managed browser login API owns provider auth method constants",
    regex: /\b(?:QWEN_OAUTH_AUTH_METHOD_ID|AMP_BROWSER_AUTH_METHOD_ID)\b/,
  },
  {
    name: "managed browser login API owns Kimi OAuth client policy",
    regex:
      /\b(?:KIMI_CODE_CLIENT_ID|KIMI_CODE_OAUTH_HOST|KIMI_OAUTH_HOST|CTX_KIMI_LOGIN_TIMEOUT_SECS|KimiDeviceAuthorizationResp|KimiToken(?:Success|Error)Resp|request_kimi_device_authorization|poll_kimi_token|kimi_token_json|kimi_login_timeout|poll_interval_for_authorization|timeout_for_authorization)\b/,
  },
  {
    name: "managed browser login API owns Kimi OAuth protocol details",
    regex:
      /\b(?:device_authorization|authorization_pending|slow_down|expired_token|access_denied|urn:ietf:params:oauth:grant-type:device_code)\b/,
  },
  {
    name: "managed browser login API starts Kimi sessions directly",
    regex: /\bstart_kimi_login_session\s*\(/,
  },
  {
    name: "managed browser login API calls low-level start/status facades",
    regex:
      /\.(?:start_(?:amp|gemini|qwen|mistral)_browser_login|start_kimi_oauth_login|(?:amp|gemini|qwen|mistral|kimi)_login_status)\s*\(/,
  },
  {
    name: "managed browser login API owns login start DTOs",
    regex: /\b(?:Amp|Gemini|Qwen|Mistral|Kimi)LoginStart(?:Req|Resp)\b/,
  },
  {
    name: "managed browser login API owns login not-found mapping",
    regex: /"login not found"/,
  },
  {
    name: "managed browser login API maps Kimi start errors directly",
    regex: /\bKimiOAuthLoginStartError\b|\.route_safe_message\s*\(/,
  },
];

const CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS = [
  {
    name: "Cursor process login API owns route DTOs",
    regex: /\bCursorLoginStart(?:Req|Resp)\b/,
  },
  {
    name: "Cursor process login API calls low-level route facades",
    regex: /\b(?:start_cursor_process_login|cursor_login_status)\s*\(/,
  },
  {
    name: "Cursor process login API matches route errors directly",
    regex:
      /\b(?:CursorProcessLoginStartError|CursorProcessLoginStartErrorKind)\b|\.route_safe_message\s*\(/,
  },
  {
    name: "Cursor process login API owns login not-found mapping",
    regex: /"login not found"/,
  },
  {
    name: "Cursor process login API owns monitor task spawning",
    regex: /\btokio::spawn\s*\(/,
  },
  {
    name: "Cursor process login API owns process spawning",
    regex: /\b(?:tokio::process::Command|std::process::Command|Command::new|Stdio::)\b/,
  },
  {
    name: "Cursor process login API resolves runtime directly",
    regex: /\bresolve_cursor_login_runtime(?:_from_config)?\s*\(/,
  },
  {
    name: "Cursor process login API mutates login sessions directly",
    regex:
      /\b(?:start_cursor_login_session|set_cursor_login_error|update_cursor_login_auth_url|finish_cursor_login_session)\s*\(/,
  },
  {
    name: "Cursor process login API finalizes Cursor accounts directly",
    regex: /\badd_cursor_oauth_account_for_login\s*\(/,
  },
  {
    name: "Cursor process login API owns private capture workspace",
    regex:
      /\b(?:cursor_login_home|ensure_private_dir|initialize_cursor_capture_file|write_cursor_capture_hook|prepare_cursor_login_workspace|CTX_CURSOR_CAPTURE_FILE|NODE_OPTIONS)\b/,
  },
  {
    name: "Cursor process login API owns process output parsing",
    regex:
      /\b(?:parse_cursor_captured_tokens|spawn_cursor_login_reader|collect_cursor_login_output|record_cursor_login_output|first_email_from_text|CursorLoginOutputLine)\b/,
  },
  {
    name: "Cursor process login API owns login timeout or env scrubbing",
    regex: /\b(?:CTX_CURSOR_LOGIN_TIMEOUT_SECS|cursor_login_timeout|DAEMON_AUTH_ENV_VARS)\b/,
  },
  {
    name: "Cursor process login API declares process monitor module",
    regex: /(?:#\s*\[\s*path\s*=\s*"[^"]*session\.rs"\s*\]\s*)?mod\s+session\s*;/,
  },
];

const CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS = [
  {
    name: "Codex app-server login API owns route DTOs",
    regex: /\bCodexLogin(?:Start|Complete)(?:Req|Resp)\b/,
  },
  {
    name: "Codex app-server login API calls low-level route facades",
    regex:
      /(?:\.|\bProvidersHandle::|\b)(?:start_codex_app_server_login|codex_login_status|complete_codex_app_server_login)\s*\(/,
  },
  {
    name: "Codex app-server login API matches route errors directly",
    regex:
      /\bCodexLogin(?:StartError|CompleteError|CompleteErrorKind)\b|\.route_safe_message\s*\(/,
  },
  {
    name: "Codex app-server login API owns login not-found mapping",
    regex: /"login not found"/,
  },
  {
    name: "Codex app-server login API owns monitor task spawning",
    regex: /\btokio::spawn\s*\(/,
  },
  {
    name: "Codex app-server login API declares app-server modules",
    regex: /(?:#\s*\[\s*path\s*=\s*"[^"]*"\s*\]\s*)?mod\s+(?:app_server|completion|process)\s*;/,
  },
  {
    name: "Codex app-server login API owns process spawning",
    regex: /\b(?:tokio::process::Command|std::process::Command|Command::new|Stdio::)\b/,
  },
  {
    name: "Codex app-server login API owns app-server runtime policy",
    regex: /\b(?:CODEX_APP_SERVER_ARGS|CODEX_LOGIN_RPC_TIMEOUT|DAEMON_AUTH_ENV_VARS)\b/,
  },
  {
    name: "Codex app-server login API owns app-server JSON-RPC",
    regex:
      /\b(?:send_codex_jsonrpc|wait_for_codex_response|spawn_codex_app_server|start_codex_login_process|monitor_codex_login|wait_for_codex_login_completion|fetch_codex_account_details)\b/,
  },
  {
    name: "Codex app-server login API mutates login sessions directly",
    regex:
      /\b(?:prepare_codex_login_start|start_codex_login_session|claim_codex_login_callback|restore_codex_login_completion_token|finish_codex_login_session)\s*\(/,
  },
  {
    name: "Codex app-server login API finalizes Codex accounts directly",
    regex: /\bpersist_successful_codex_login\s*\(/,
  },
  {
    name: "Codex app-server login API owns callback replay",
    regex:
      /\b(?:reqwest::Client::builder|callback_replay_client|replay_codex_callback|CallbackReplayError)\b/,
  },
  {
    name: "Codex app-server login API owns login cleanup",
    regex: /\bremove_dir_all\s*\(/,
  },
];

const CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS = [
  {
    name: "Claude setup-token login API owns route DTOs",
    regex: /\bClaudeLoginStart(?:Req|Resp)\b/,
  },
  {
    name: "Claude setup-token login API calls low-level route facades",
    regex: /\b(?:start_claude_setup_token_login|claude_login_status)\s*\(/,
  },
  {
    name: "Claude setup-token login API matches route errors directly",
    regex:
      /\b(?:ClaudeSetupTokenLoginStartError|ClaudeSetupTokenLoginStartErrorKind)\b|\.route_safe_message\s*\(/,
  },
  {
    name: "Claude setup-token login API owns login not-found mapping",
    regex: /"login not found"/,
  },
  {
    name: "Claude setup-token login API owns monitor task spawning",
    regex: /\btokio::spawn\s*\(/,
  },
  {
    name: "Claude setup-token login API declares setup-token implementation modules",
    regex:
      /(?:#\s*\[\s*path\s*=\s*"[^"]*"\s*\]\s*)?mod\s+(?:auth_url|process|setup_token|tests)\s*;/,
  },
  {
    name: "Claude setup-token login API owns process spawning",
    regex:
      /\b(?:portable_pty|NativePtySystem|PtySystem|PtySize|ChildKiller|CommandBuilder|tokio::process::Command|std::process::Command|Command::new|Stdio::)\b/,
  },
  {
    name: "Claude setup-token login API resolves runtime directly",
    regex: /\bresolve_claude_login_runtime\s*\(/,
  },
  {
    name: "Claude setup-token login API owns process lifecycle",
    regex:
      /\b(?:spawn_claude_setup_token_command|start_claude_login_process|monitor_claude_login|kill_claude_login_process|terminate_claude_login_after_error|wait_for_claude_login_observation|finalize_claude_login)\b/,
  },
  {
    name: "Claude setup-token login API owns auth-url parsing",
    regex:
      /\b(?:extract_claude_setup_token|claude_login_hit_unsupported_manual_fallback|claude_manual_fallback_is_terminal|refresh_claude_auth_url_from_capture_path|read_claude_browser_open_capture_url|ClaudeAuthUrlSource|CLAUDE_BROWSER_OPEN_MARKER|CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR|normalize_claude_login_line|read_trailing_claude_login_lines|auth_url_looks_complete|extract_auth_url)\b/,
  },
  {
    name: "Claude setup-token login API owns browser-open shim",
    regex:
      /\b(?:claude_browser_open_shim_script|create_claude_browser_open_shim|claude_login_should_skip_browser_open|CLAUDE_BROWSER_AUTH_TIER|CTX_CLAUDE_AUTH_URL_CAPTURE_PATH)\b/,
  },
  {
    name: "Claude setup-token login API owns auth runtime env",
    regex: /\b(?:CLAUDE_LOGIN_[A-Z0-9_]*|DAEMON_AUTH_ENV_VARS)\b/,
  },
  {
    name: "Claude setup-token login API mutates login sessions directly",
    regex:
      /\b(?:start_claude_login_session|set_claude_login_auth_url|finish_claude_login_session)\s*\(/,
  },
  {
    name: "Claude setup-token login API finalizes Claude accounts directly",
    regex: /\badd_claude_account_for_login\s*\(/,
  },
];

const PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS = [
  {
    name: "provider login API exposes provider-account login status DTOs",
    regex:
      /\b(?:provider_accounts|ctx_provider_accounts)::(?:Codex|Claude|Cursor|Amp|Gemini|Qwen|Mistral|Kimi)LoginStatus\b|\buse\s+ctx_provider_accounts(?:::|\s*::\s*\{)[^;]*(?:Codex|Claude|Cursor|Amp|Gemini|Qwen|Mistral|Kimi)LoginStatus\b|\b(?:Json\s*<\s*)?(?:Codex|Claude|Cursor|Amp|Gemini|Qwen|Mistral|Kimi)LoginStatus\b/,
  },
];

const PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS = [
  {
    name: "provider API prelude exposes provider-account module",
    regex: /\buse\s+ctx_provider_accounts\s+as\s+provider_accounts\s*;/,
  },
];

const EXECUTION_API_ORCHESTRATION_PATTERNS = [
  {
    name: "execution API imports Linux sandbox runtime directly",
    regex: /\bctx_linux_sandbox_runtime\b/,
  },
  {
    name: "execution API accesses daemon data root directly",
    regex: /\.data_root\s*\(/,
  },
  {
    name: "execution API owns workspace lookup",
    regex: /\bWorkspacesHandle\b|\.get_workspace\s*\(/,
  },
  {
    name: "execution API owns effective execution settings",
    regex:
      /\b(?:ExecutionMode|ExecutionSettings|effective_execution_settings_classified|map_effective_execution_settings_error)\b/,
  },
  {
    name: "execution API owns maintenance drain",
    regex:
      /\b(?:daemon_maintenance|MaintenanceDrainError|reject_new_execution_during_maintenance|acquire_linux_sandbox_prepare_drain)\b/,
  },
  {
    name: "execution API owns workspace launch input resolution",
    regex: /\bresolve_workspace_launch_inputs\b/,
  },
  {
    name: "execution API owns Linux sandbox user messages",
    regex: /\blinux_sandbox_user_message\b/,
  },
  {
    name: "execution API calls Linux sandbox runtime by fully-qualified path",
    regex:
      /\bctx_linux_sandbox_runtime::(?:linux_sandbox_runtime_status|stage_linux_sandbox_runtime_downloads|prepare_linux_sandbox_runtime)\b/,
  },
];

const HEALTH_DIAGNOSTICS_API_ORCHESTRATION_PATTERNS = [
  {
    name: "health/diagnostics API calls update service directly",
    regex: /\bctx_update_service\b/,
  },
  {
    name: "health/diagnostics API imports Linux sandbox runtime directly",
    regex: /\bctx_linux_sandbox_runtime\b/,
  },
  {
    name: "health/diagnostics API reads process limits directly",
    regex: /\bctx_resource_utilization::process_limits\b|\bcurrent_open_file_limit\s*\(/,
  },
  {
    name: "health/diagnostics API reads observability logs directly",
    regex: /\blogs::(?:logs_dir|list_log_files)\s*\(/,
  },
  {
    name: "health/diagnostics API accesses daemon data root directly",
    regex: /\.data_root\s*\(/,
  },
  {
    name: "health/diagnostics API reads storage guard directly",
    regex: /\bstorage_guard_snapshot\s*\(/,
  },
  {
    name: "health/diagnostics API reads execution startup status directly",
    regex: /\bstartup_status\s*\(/,
  },
  {
    name: "health/diagnostics API reads provider diagnostics directly",
    regex: /\bprovider_diagnostics_snapshot\s*\(/,
  },
  {
    name: "health/diagnostics API owns health response assembly",
    regex: /\b(?:build_health_response|HealthResp|DiagnosticsResp|HealthCompatibility)\b/,
  },
  {
    name: "diagnostics API imports health route internals",
    regex: /\bsuper::health\b/,
  },
];

const DAEMON_HEALTH_VERSION_PATTERNS = [
  {
    name: "daemon health uses daemon crate package version directly",
    regex: /env!\s*\(\s*"CARGO_PKG_VERSION"\s*\)/,
  },
];

const RESOURCE_UTILIZATION_API_ROUTE_CONTRACT_PATTERNS = [
  {
    name: "resource utilization API parses workspace ids locally",
    regex: /\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(|\bWorkspaceId\s*\(/,
  },
  {
    name: "resource utilization API exposes raw resource snapshot",
    regex: /\bctx_resource_utilization::ResourceUtilizationSnapshot\b|\bResourceUtilizationSnapshot\b/,
  },
  {
    name: "resource utilization API imports low-level resource errors",
    regex: /\bResourceUtilizationSnapshotError\b|\bctx_daemon::daemon::resource_utilization\b/,
  },
  {
    name: "resource utilization API calls typed resource facade directly",
    regex: /\.workspace_resource_utilization_snapshot\s*\(/,
  },
];

const SETTINGS_API_ORCHESTRATION_PATTERNS = [
  {
    name: "settings API imports settings service directly",
    regex: /\bctx_settings_service\b/,
  },
  {
    name: "settings API owns host execution policy checks",
    regex: /\bHostExecutionPolicy\b|\bvalidate_execution_environment\s*\(/,
  },
  {
    name: "settings API applies settings updates directly",
    regex: /\bapply_update\s*\(/,
  },
  {
    name: "settings API owns settings persistence sequencing",
    regex: /\.(?:load_settings|save_settings|apply_settings_side_effects|public_settings_for_response)\s*\(/,
  },
];

const TELEMETRY_API_ORCHESTRATION_PATTERNS = [
  {
    name: "telemetry API derives perf log path directly",
    regex: /\bperf_log_path_for_date\s*\(/,
  },
  {
    name: "telemetry API accesses daemon data root directly",
    regex: /\.data_root\s*\(/,
  },
];

const BLOB_API_ORCHESTRATION_PATTERNS = [
  {
    name: "blob API accesses daemon data root directly",
    regex: /\.data_root\s*\(/,
  },
  {
    name: "blob API owns blob filesystem operations",
    regex:
      /\btokio::fs::(?:create_dir_all|write|rename|copy|read|read_to_string|metadata|symlink_metadata|remove_file|remove_dir|remove_dir_all|canonicalize)\s*\(|\btokio::fs::(?:File::open|OpenOptions)\b|\bstd::fs::|\buse\s+std::fs\b|\buse\s+tokio::fs\s+as\s+\w+\b|\buse\s+tokio::fs::\s*\{[^}]*\bself\b[^}]*\}|\bfs::(?:create_dir_all|write|rename|copy|read|read_to_string|metadata|symlink_metadata|remove_file|remove_dir|remove_dir_all|canonicalize)\s*\(|\bFile::open\s*\(|\bOpenOptions::/,
  },
  {
    name: "blob API owns blob checksum generation",
    regex: /\bsha2::(?:Digest|Sha256)\b|\bSha256::new\s*\(|\bDigest\b/,
  },
  {
    name: "blob API owns blob id generation",
    regex: /\buuid::Uuid::new_v4\s*\(|\bUuid::new_v4\s*\(/,
  },
  {
    name: "blob API accesses blob store metadata directly",
    regex: /\.(?:insert_blob|get_blob)\s*\(/,
  },
];

const LOGS_API_ORCHESTRATION_PATTERNS = [
  {
    name: "logs API imports observability logs directly",
    regex: /\bctx_observability::logs\b/,
  },
  {
    name: "logs API calls log filesystem helpers directly",
    regex: /\blogs::(?:open_logs_folder|append_desktop_log_line)\s*\(/,
  },
  {
    name: "logs API assembles desktop log line locally",
    regex: /\bchrono::Utc::now\s*\(|\bSecondsFormat::Secs\b/,
  },
  {
    name: "logs API accesses daemon data root directly",
    regex: /\.data_root\s*\(/,
  },
];

const UPDATE_API_ORCHESTRATION_PATTERNS = [
  {
    name: "update API calls update service directly",
    regex: /\bctx_update_service\b(?!(?:::route_contract\b))/,
  },
  {
    name: "update API redacts errors locally",
    regex: /\blogs::redact_sensitive\s*\(/,
  },
  {
    name: "update API accesses daemon data root directly",
    regex: /\.data_root\s*\(/,
  },
  {
    name: "update API owns managed auto-update DTO",
    regex: /\bManagedDaemonAutoUpdateStatus\b/,
  },
  {
    name: "update API owns update response DTO assembly",
    regex:
      /\b(?:UpdateCheckResp|UpdateActivityResp|DownloadAppImageReq|DownloadAppImageResp|ApplyAppImageReq|ApplyAppImageResp)\b/,
  },
];

const UPDATE_DRAIN_API_ORCHESTRATION_PATTERNS = [
  {
    name: "update drain API imports daemon maintenance internals",
    regex: /\bctx_daemon\s*::\s*daemon\s*::\s*maintenance\b/,
  },
  {
    name: "update drain API references low-level maintenance errors",
    regex: /\b(?:BeginUpdateDrainError|DaemonShutdownError|MaintenanceDrainError)\b/,
  },
  {
    name: "update drain API calls low-level daemon maintenance methods",
    regex: /\.(?:begin_update_drain|release_update_drain|request_daemon_shutdown)\s*\(/,
  },
  {
    name: "update drain API authorizes shutdown token locally",
    regex: /\blocal_shutdown_token_authorized\s*\(|\.local_shutdown_token\b|\bCTX_LOCAL_DAEMON_SHUTDOWN_TOKEN\b/,
  },
  {
    name: "update drain API redacts maintenance errors locally",
    regex: /\blogs::redact_sensitive\s*\(/,
  },
  {
    name: "update drain API owns maintenance default values",
    regex: /\b(?:daemon_update|desktop_quit|unknown)\b/,
  },
  {
    name: "update drain API owns maintenance error mapping helpers",
    regex: /\b(?:begin_update_drain_error|daemon_shutdown_error|internal_error_response)\b/,
  },
];

const WORKSPACE_REGISTRATION_CONFIG_API_PATTERNS = [
  {
    name: "workspace registration API imports registration service directly",
    regex: /\bctx_workspace_services::workspace_registration\b/,
  },
  {
    name: "workspace registration API owns registration preparation",
    regex: /\bprepare_workspace_registration\s*\(/,
  },
  {
    name: "workspace primary branch API owns branch validation",
    regex: /\bvalidate_workspace_primary_branch\s*\(/,
  },
  {
    name: "workspace registration API owns registration error type",
    regex: /\bWorkspaceRegistrationError\b/,
  },
  {
    name: "workspace registration API owns registration telemetry sequencing",
    regex: /\brecord_workspace_registered\s*\(/,
  },
];

const WORKSPACE_CONFIG_ROUTE_CONTEXT_PATTERNS = [
  {
    name: "workspace config API requires workspace context in HTTP",
    regex: /\b\B/,
    paths: ["core/crates/ctx-http/src/api/workspaces/management.rs"],
    contentRegex:
      /\b(?:get_workspace_primary_branch|update_workspace_primary_branch|get_execution_config|update_execution_config)\s*\([\s\S]*?\)\s*->[\s\S]*?\{[\s\S]{0,300}?\brequire_workspace_ctx\s*\(/g,
  },
  {
    name: "workspace config API loads workspace in HTTP",
    regex: /\b\B/,
    paths: ["core/crates/ctx-http/src/api/workspaces/management.rs"],
    contentRegex:
      /\b(?:get_workspace_primary_branch|update_workspace_primary_branch|get_execution_config|update_execution_config)\s*\([\s\S]*?\)\s*->[\s\S]*?\{[\s\S]{0,300}?\brequire_workspace\s*\(/g,
  },
  {
    name: "workspace config API fetches workspace directly in HTTP",
    regex: /\b\B/,
    paths: ["core/crates/ctx-http/src/api/workspaces/management.rs"],
    contentRegex:
      /\b(?:get_workspace_primary_branch|update_workspace_primary_branch|get_execution_config|update_execution_config)\s*\([\s\S]*?\)\s*->[\s\S]*?\{[\s\S]{0,300}?\.get_workspace\s*\(/g,
  },
];

const WORKSPACE_EXECUTION_CONFIG_API_PATTERNS = [
  {
    name: "workspace execution config API imports settings service directly",
    regex: /\bctx_settings_service\b/,
  },
  {
    name: "workspace execution config API applies overrides directly",
    regex: /\bapply_workspace_execution_settings_override\s*\(/,
  },
  {
    name: "workspace execution config API validates overrides directly",
    regex: /\bvalidate_workspace_execution_settings_override\s*\(/,
  },
  {
    name: "workspace execution config API loads daemon settings directly",
    regex: /\.load_settings\s*\(/,
  },
  {
    name: "workspace execution config API loads workspace execution override directly",
    regex: /\.load_workspace_execution_override\s*\(/,
  },
  {
    name: "workspace execution config API checks sandbox runtime directly",
    regex: /\.shared_vm_container_runtime_available\s*\(/,
  },
  {
    name: "workspace execution config API builds execution override directly",
    regex: /\bbuild_workspace_execution_config_override\s*\(/,
  },
  {
    name: "workspace execution config API projects execution config directly",
    regex: /\bproject_workspace_execution_config\s*\(/,
  },
  {
    name: "workspace execution config API persists execution config directly",
    regex: /\.update_workspace_execution_config\s*\(/,
  },
];

const WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS = [
  {
    name: "workspace management config API owns workspace context lookup",
    regex: /\brequire_workspace(?:_ctx)?\s*\(/,
  },
  {
    name: "workspace management config API defines workspace context lookup",
    regex: /\b(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+require_workspace(?:_ctx)?\s*\(/,
  },
  {
    name: "workspace management config API exposes raw Workspace",
    regex:
      /\bctx_core::models::Workspace\b|\buse\s+ctx_core::models(?:::|\s*::\s*\{)[^;]*\bWorkspace\b|\bWorkspaceRequestContext\b/,
  },
  {
    name: "workspace management config API owns local route DTOs",
    regex:
      /\b(?:UpdateWorkspaceConfigResp|UpdateMergeQueueConfigReq|WorkspaceMergeQueueConfigResp|UpdateWorktreeBootstrapReq|WorkspaceWorktreeBootstrapConfigResp|UpdateWorkspaceProviderModelPreferenceReq|WorkspaceProviderModelPreferenceResp|AgentSystemPromptConfigResponse|UpdateAgentSystemPromptConfigReq|SubagentSystemPromptConfigResponse|UpdateSubagentSystemPromptConfigReq)\b/,
  },
  {
    name: "workspace management config API builds raw workspace config updates",
    regex:
      /\bworkspace_config::(?:MergeQueueConfigUpdate|WorktreeBootstrapConfigUpdate)\b|\b(?:MergeQueueConfigUpdate|WorktreeBootstrapConfigUpdate)\s*\{/,
  },
  {
    name: "workspace management config API calls raw config facade methods",
    regex:
      /\.(?:load_workspace_merge_queue_config|update_workspace_merge_queue_config|load_worktree_bootstrap_config|update_worktree_bootstrap_config)\s*\(/,
  },
  {
    name: "workspace management config API parses route ids directly",
    regex: /\buuid::Uuid::parse_str\s*\(|\bWorkspaceId\s*\(/,
    paths: workspacePromptAndModelConfigApiPaths,
  },
  {
    name: "workspace management config API exposes raw provider preference contracts",
    regex: /\b(?:WorkspaceProviderModelPreference|WorkspaceProviderModelPreferenceError)\b/,
  },
  {
    name: "workspace management config API exposes raw prompt config contracts",
    regex:
      /\b(?:AgentSystemPromptAppendConfig|SubagentSystemPromptAppendConfig|AgentSystemPromptAppendSource)\b/,
  },
  {
    name: "workspace management config API calls raw prompt and model config facades",
    regex:
      /(?:\.\s*|WorkspacesHandle\s*::\s*)(?:get_workspace_provider_model_preference|set_workspace_provider_model_preference|load_agent_system_prompt_append|update_agent_system_prompt_append|load_subagent_system_prompt_append|update_subagent_system_prompt_append)\s*\(/,
  },
  {
    name: "workspace management config API owns prompt response projection",
    regex: /\b(?:source_label|configured_append)\s*\(/,
    paths: workspacePromptAndModelConfigApiPaths,
  },
  {
    name: "workspace management config API redacts prompt/model errors locally",
    regex: /\blogs::redact_sensitive\s*\(/,
    paths: workspacePromptAndModelConfigApiPaths,
  },
  {
    name: "workspace management config API imports workspace config directly",
    regex: /\bctx_workspace_config\b/,
  },
];

const WORKSPACE_ROUTE_CONTRACT_API_PATTERNS = [
  {
    name: "workspace route API exposes raw workspace route DTOs",
    regex:
      /\b(?:ctx_core::models::|ctx_workspace_container::)?(?:Workspace|Worktree|WorkspaceAttachment|WorkspaceActiveSnapshot|WorkspaceActiveHeadBatch|WorkspaceContainerStatus)\b|\buse\s+ctx_core::models(?:::|\s*::\s*\{)[^;]*\b(?:Workspace|Worktree|WorkspaceAttachment|WorkspaceActiveSnapshot|WorkspaceActiveHeadBatch)\b|\buse\s+ctx_workspace_container(?:::|\s*::\s*\{)[^;]*\bWorkspaceContainerStatus\b/,
    contentRegex:
      /\buse\s+ctx_core::models\s*::\s*\{(?=[^}]*\n)[\s\S]*?\b(?:Workspace|Worktree|WorkspaceAttachment|WorkspaceActiveSnapshot|WorkspaceActiveHeadBatch)\b[\s\S]*?\}\s*;/gm,
  },
  {
    name: "workspace attachment API owns attachment config construction",
    regex: /\bAttachmentConfig\b|\bctx_workspace_attachments::AttachmentConfig\b/,
  },
  {
    name: "workspace attachment API loads workspace context in HTTP",
    regex: /\brequire_workspace_ctx\s*\(/,
  },
  {
    name: "workspace attachment API calls raw attachment facade methods",
    regex:
      /\.(?:get_workspace|upsert_workspace_attachment|delete_workspace_attachment|sync_workspace_attachments)\s*\(/,
  },
  {
    name: "workspace REST route API parses route ids directly",
    regex: /\buuid::Uuid::parse_str\s*\(|\b(?:WorkspaceId|WorktreeId)\s*\(/,
    paths: workspaceRestRouteContractApiPaths,
  },
  {
    name: "workspace REST route API uses local workspace context helper",
    regex: /\bparse_workspace_id\s*\(|\bmod\s+context\s*;|\buse\s+context\s*::\s*\*\s*;/,
    paths: workspaceRestRouteContractApiPaths,
  },
  {
    name: "workspace REST route API inspects low-level workspace errors",
    regex:
      /\b(?:WorkspaceHydrationError|WorkspaceHydrationErrorKind|WorkspaceDeleteError|WorkspaceHarnessContainerError|RouteFileDownloadError|FileCompletionsError|FileCompletionsErrorKind)\b/,
    paths: workspaceRestRouteContractApiPaths,
  },
  {
    name: "workspace REST route API calls low-level workspace facades directly",
    regex:
      /(?:\.\s*|WorkspacesHandle\s*::\s*)(?:delete_workspace|load_workspace_active_snapshot_for_route|load_workspace_active_heads_for_route|get_worktree_for_route|download_worktree_bootstrap_logs_for_route|workspace_harness_container_status_for_route|stop_workspace_harness_container|ensure_workspace_harness_container|complete_files_for_workspace|list_workspace_attachments_for_route|sync_workspace_attachments_for_route|create_and_sync_workspace_attachment_for_route|delete_and_sync_workspace_attachment_for_route|workspace_merge_queue_config_for_route|update_workspace_merge_queue_config_for_route|workspace_primary_branch_for_request|update_workspace_primary_branch_for_request|workspace_execution_config_for_request|update_workspace_execution_config_for_request|worktree_bootstrap_config_for_route|update_worktree_bootstrap_config_for_route)\s*\(/,
    paths: workspaceRestRouteContractApiPaths,
  },
  {
    name: "workspace file-completion API maps low-level completion errors locally",
    regex: /\bmap_file_completions_error\b/,
    paths: workspaceFileCompletionApiPaths,
  },
  {
    name: "workspace harness-container API maps execution settings locally",
    regex: /\bmap_effective_execution_settings_error\b/,
    paths: workspaceHarnessContainerApiPaths,
  },
  {
    name: "workspace harness-container API redacts low-level errors locally",
    regex: /\blogs::redact_sensitive\s*\(/,
    paths: workspaceHarnessContainerApiPaths,
  },
];

const DAEMON_UPDATES_VERSION_PATTERNS = [
  {
    name: "daemon updates uses daemon crate package version directly",
    regex: /env!\s*\(\s*"CARGO_PKG_VERSION"\s*\)/,
  },
];

const TASK_SESSION_CREATION_API_ADMISSION_PATTERNS = [
  {
    name: "task session API owns sessions handle",
    regex: /\bSessionsHandle\b/,
  },
  {
    name: "task session API owns session creation lock",
    regex: /\btask_session_creation_lock\s*\(/,
  },
  {
    name: "task session API bypasses locked create-session entrypoint",
    regex: /\bcreate_session_for_loaded_task\s*\(/,
  },
];

const TASK_ROUTE_API_CONTRACT_PATTERNS = [
  {
    name: "task route API imports raw task/session/archive models",
    regex:
      /ctx_core::models::\{[^}]*\b(?:Task|Session|WorkspaceArchivedPage|WorkspaceTaskSummary|WorkspaceIndexCursor)\b|ctx_core::models::(?:Task|Session|WorkspaceArchivedPage|WorkspaceTaskSummary|WorkspaceIndexCursor)\b/,
  },
  {
    name: "task route API returns raw task/session DTOs",
    regex:
      /\bJson\s*<\s*(?:Vec\s*<\s*)?(?:Task|Session|WorkspaceArchivedPage|WorkspaceTaskSummary|WorkspaceIndexCursor)\b/,
  },
  {
    name: "task route API owns task/workspace id or cursor parsing",
    regex:
      /\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(|\bDateTime\s*::\s*parse_from_rfc3339\s*\(/,
  },
  {
    name: "task route API owns local task route DTOs",
    regex:
      /\b(?:CreateTaskReq|CreateSessionReq|CreateTaskDefaultSessionReq|WorkspaceArchivedQuery|ArchiveTaskResponse|UpdateTaskTitleReq)\b/,
  },
  {
    name: "task route API imports low-level task inputs or errors",
    regex:
      /\b(?:CreateTaskInput|CreateTaskSessionInput|TaskCreateError|TaskSessionCreateError|TaskLifecycleError)\b/,
  },
  {
    name: "task route API calls raw task handle facades",
    regex:
      /\.(?:list_workspace_tasks|list_workspace_archived_page|list_task_sessions|mark_task_read|mark_task_unread|update_task_title|archive_task|unarchive_task|delete_task|create_task_for_workspace|create_session_for_task)\s*\(/,
  },
];

const WORKSPACE_STREAM_READ_MODEL_API_PATTERNS = [
  {
    name: "workspace stream API owns read-model preparation",
    regex: /\b(?:ensure_workspace_active_snapshot_hydrated|activate_workspace_merge_queue|workspace_active_snapshot|workspace_active_heads|load_workspace_active_snapshot_state)\s*\(/,
  },
];

const WORKSPACE_STREAM_SUBSCRIPTION_PLAN_API_PATTERNS = [
  {
    name: "workspace stream API references raw subscription-resolution type",
    regex: /\b(?:ResolvedWorkspaceActiveSessionSubscription|ResolvedWorkspaceActiveSubscriptions|ResolvedWorkspaceActiveSessionReplay)\b/,
  },
];

const WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS = [
  {
    name: "workspace stream API calls raw subscription resolution directly",
    regex: /\.resolve_workspace_active_snapshot_subscriptions\s*\(/,
  },
  {
    name: "workspace stream API merges replayed subscription cursors locally",
    regex:
      /\b(?:merge_replayed_and_live_subscriptions|merge_replayed_and_live_subscription_cursors)\s*\(/,
  },
  {
    name: "workspace stream API defines local replay merge helper",
    regex: /\bfn\s+merge_replayed_and_live_subscriptions\s*\(/,
  },
  {
    name: "workspace stream API computes subscription pin diffs locally",
    regex:
      /\b(?:sync_workspace_stream_session_pins|fn\s+sync_workspace_stream_session_pins)\b/,
  },
  {
    name: "workspace stream API computes subscription pin set differences locally",
    regex: /\b(?:current|next)\s*\.\s*difference\s*\(\s*&(?:current|next)\s*\)/,
  },
  {
    name: "workspace stream API mutates session pins directly",
    regex: /\.(?:attach_session_pin|detach_session_pin)\s*\(/,
  },
];

const WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS = [
  {
    name: "workspace stream API owns cursor acceptance helper",
    regex:
      /(?<!\.)\b(?:accept_session_delta|accept_session_head|accept_session_cursor)\s*\(/,
  },
  {
    name: "workspace stream API calls active-snapshot replay cursor planner",
    regex: /\breplay_cursor_after_live_progress\b/,
  },
  {
    name: "workspace stream API owns replay cursor helper",
    regex: /(?<!\.)\b(?:resume_replay_cursor|head_only_snapshot_cursor)\b/,
  },
  {
    name: "workspace stream API builds replay cursor from snapshot head",
    regex: /\bSessionReplayCursor\s*::\s*from_head\s*\(/,
  },
  {
    name: "workspace stream API builds replay cursor from session delta",
    regex: /\bSessionReplayCursor\s*::\s*from_delta\s*\(/,
  },
  {
    name: "workspace stream API merges replay cursors directly",
    regex: /\.cover\s*\(/,
  },
  {
    name: "workspace stream API reads raw session replay cursor",
    regex: /(?:\.\s*|\bWorkspaceStreamHandle\s*::\s*)session_replay_cursor\s*\(/,
  },
];

const WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS = [
  {
    name: "workspace stream API references raw replay intent policy",
    regex: /\bWorkspaceActiveSnapshotSessionIntent\b/,
  },
  {
    name: "workspace stream API references raw replay mode policy",
    regex: /\bWorkspaceStreamSessionReplay\b/,
  },
  {
    name: "workspace stream API matches raw replay policy variant",
    regex:
      /(?:\b(?:WorkspaceActiveSnapshotSessionIntent|WorkspaceStreamSessionReplay)::(?:Head|Replay|Resume|Reset)\b|(?<!::)\b(?:Head|Replay|Resume|Reset)\s*\{)/,
  },
  {
    name: "workspace stream API interprets resolved replay subscription fields",
    regex: /\b(?:sub|subscription)\s*\.\s*(?:intent|replay)\b/,
  },
  {
    name: "workspace stream API calls replay cursor planner directly",
    regex:
      /(?:\.\s*|\bWorkspaceStreamHandle\s*::\s*|(?<!\.))\b(?:plan_resume_replay_cursor|head_only_snapshot_cursor)\s*\(/,
  },
  {
    name: "workspace stream API rebuilds pending replay blockers",
    regex: /a^/,
    contentRegex:
      /\bpending_replay_sessions\b[\s\S]{0,240}\bresolved_sessions\s*\.\s*iter\s*\(/gm,
  },
  {
    name: "workspace stream API reads active-task subscription map for replay deferral",
    regex: /\.active_task_sessions\b/,
  },
];

const WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS = [
  {
    name: "workspace stream API imports active-snapshot event predicate helper",
    regex: /\buse\s+(?:ctx_workspace_active_snapshot|ctx_daemon\s*::\s*daemon\s*::\s*workspaces\s*::\s*stream)\s*::\s*(?:\{[^}]*\b(?:primary_session_id_for_active_task|workspace_stream_event_blocks_pending_replay|should_stream_head_delta|filter_partial_delta_for_active_tasks|is_priority_control_event|is_foreground_session|event_snapshot_rev)\b[^}]*\}|(?:primary_session_id_for_active_task|workspace_stream_event_blocks_pending_replay|should_stream_head_delta|filter_partial_delta_for_active_tasks|is_priority_control_event|is_foreground_session|event_snapshot_rev)\b)/,
    contentRegex:
      /\buse\s+(?:ctx_workspace_active_snapshot|ctx_daemon\s*::\s*daemon\s*::\s*workspaces\s*::\s*stream)\s*::\s*\{(?=[^}]*\n)[\s\S]*?(?:\bprimary_session_id_for_active_task\b|\bworkspace_stream_event_blocks_pending_replay\b|\bshould_stream_head_delta\b|\bfilter_partial_delta_for_active_tasks\b|\bis_priority_control_event\b|\bis_foreground_session\b|\bevent_snapshot_rev\b)[\s\S]*?\}/gm,
  },
  {
    name: "workspace stream API owns event-routing predicate",
    regex:
      /(?<!\.)\b(?:primary_session_id_for_active_task|workspace_stream_event_blocks_pending_replay|should_stream_head_delta|filter_partial_delta_for_active_tasks|is_priority_control_event|is_foreground_session|event_snapshot_rev)\s*\(/,
  },
  {
    name: "workspace stream API defines event-routing predicate",
    regex:
      /\bfn\s+(?:should_stream_head_delta|filter_partial_delta_for_active_tasks|is_priority_control_event|is_foreground_session|event_snapshot_rev)\s*\(/,
  },
];

const WORKSPACE_STREAM_LIVE_EVENT_APPLICATION_API_PATTERNS = [
  {
    name: "workspace stream API calls live event route planning directly",
    regex: /\.\s*plan_workspace_stream_event_route\s*\(/,
  },
  {
    name: "workspace stream API calls live event cursor acceptance directly",
    regex: /\.\s*(?:accept_session_delta_cursor|accept_session_head_cursor)\s*\(/,
  },
  {
    name: "workspace stream API calls subscription event application directly",
    regex: /\.\s*apply_workspace_stream_subscription_event\s*\(/,
  },
];

const WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS = [
  {
    name: "workspace stream API matches event-routing domain event directly",
    regex:
      /\b(?:WorkspaceActiveSnapshotEvent::)?(?:SessionHeadDelta|SessionSummaryDelta|SessionGap|SessionHeadSeed|SessionRemoved)\s*\{/,
  },
  {
    name: "workspace stream API owns event route helper",
    regex: /\bfn\s+(?:route_head_delta|route_summary_delta|route_control_event)\s*\(/,
  },
];

const WORKSPACE_STREAM_SUBSCRIPTION_EVENT_API_PATTERNS = [
  {
    name: "workspace stream API mutates subscription state domain set",
    regex:
      /\bsubscription_state\s*\.\s*(?:active_task_sessions|explicit_sessions|foreground_session_ids|replay_sessions)\s*(?:[.=]|\s*\()/,
  },
  {
    name: "workspace stream API matches active subscription event domain",
    regex:
      /\bWorkspaceActiveSnapshotEvent::(?:ActiveTaskUpsert|ActiveTaskDelete|TaskDelta)\b/,
  },
  {
    name: "workspace stream API owns active subscription mutation helper",
    regex:
      /\bfn\s+(?:remove_active_task_subscription_if_unused|remove_runtime_subscription)\s*\(/,
  },
];

const WORKSPACE_VCS_DEMAND_API_PATTERNS = [
  {
    name: "workspace VCS API calls raw demand filter directly",
    regex: /\.\s*filter_workspace_worktree_ids\s*\(/,
  },
  {
    name: "workspace VCS API mutates demand refs directly",
    regex: /\.\s*(?:update_worktree_vcs_activity|update_worktree_vcs_open_panes)\s*\(/,
  },
  {
    name: "workspace VCS API computes demand set differences locally",
    regex: /\.\s*difference\s*\(/,
  },
  {
    name: "workspace VCS API owns active demand helper",
    regex: /\bfn\s+active_worktree_ids\s*\(/,
  },
];

const WORKSPACE_VCS_LIVE_ROUTING_API_PATTERNS = [
  {
    name: "workspace VCS API routes snapshots from raw demand sets",
    regex:
      /\b(?:runtime|demand)\s*\.\s*(?:detail_worktree_ids|summary_worktree_ids)\s*\.\s*contains\s*\(/,
  },
  {
    name: "workspace VCS API plans snapshot seeds from raw demand sets",
    regex:
      /(?:^|[^.\w])(?:summary_worktree_ids|detail_worktree_ids)\s*\.\s*(?:union|contains)\s*\(/,
  },
];

const TERMINAL_STREAM_RUNTIME_API_PATTERNS = [
  {
    name: "terminal WS API imports raw terminal session handle",
    regex: /\bTerminalSessionHandle\b/,
  },
  {
    name: "terminal WS API imports raw terminal status event",
    regex: /\bTerminalStatusEvent\b/,
  },
  {
    name: "terminal WS API uses raw terminal broadcast receiver",
    regex:
      /\b(?:broadcast\s*::\s*)?Receiver\s*<\s*(?:Vec\s*<\s*u8\s*>|TerminalStatusEvent)\s*>/,
  },
  {
    name: "terminal WS API calls raw terminal lifecycle or command methods",
    regex:
      /\.(?:mark_client_connected|mark_client_disconnected|send_input|resize)\s*\(/,
  },
  {
    name: "terminal WS API opens raw terminal receivers",
    regex: /\.(?:output_receiver|status_receiver)\s*\(/,
  },
  {
    name: "terminal WS API reads raw terminal snapshots",
    regex: /\.(?:output_snapshot(?:_tail)?|snapshot)\s*\(/,
  },
];

const DICTATION_WS_CONFIG_API_PATTERNS = [
  {
    name: "dictation WS API loads settings directly",
    regex: /\b(?:[A-Za-z_][\w]*\s*(?:\.|::)\s*)?load_settings\s*\(/,
  },
  {
    name: "dictation WS API interprets dictation provider policy",
    regex: /\bDictationProvider\b/,
  },
  {
    name: "dictation WS API normalizes LiveKit config directly",
    regex: /\bnormalize_livekit_dictation_config\s*\(/,
  },
  {
    name: "dictation WS API imports LiveKit config input",
    regex: /\bLiveKitDictationConfigInput\b/,
  },
];

const WORKSPACE_WS_ADMISSION_API_PATTERNS = [
  {
    name: "workspace WS API parses stream route ids directly",
    regex: /\buuid::Uuid::parse_str\s*\(|\b(?:WorkspaceId|TerminalId)\s*\(/,
  },
  {
    name: "workspace WS API performs direct workspace existence admission",
    regex: /(?:\.|\b(?:WorkspaceStreamHandle|WorkspacesHandle)::)workspace_exists\s*\(/,
  },
  {
    name: "workspace WS API calls raw stream admission helpers",
    regex:
      /\.(?:require_workspace_active_stream_access|require_workspace_vcs_stream_access|require_mobile_secure_stream_access|load_mobile_secure_stream_context|require_terminal_stream_access)\s*\(/,
  },
  {
    name: "workspace WS API defines local stream admission helper",
    regex:
      /\bfn\s+(?:require_workspace_(?:active|vcs)_stream_access|mobile_secure_stream_access_status|terminal_stream_access_status|terminal_stream_tail_bytes)\s*\(/,
  },
  {
    name: "workspace WS API trims secure mobile query fields directly",
    regex: /\bquery\.(?:device_id|token)\.trim\s*\(/,
  },
];

const ORG_POLICY_API_ORCHESTRATION_PATTERNS = [
  {
    name: "org policy API verifies policy snapshot signatures directly",
    regex: /\bverify_policy_snapshot_signature\s*\(/,
  },
  {
    name: "org policy API parses route ids directly",
    regex: /\buuid\s*::\s*Uuid\s*::\s*parse_str\s*\(|\b(?:OrgId|WorkspaceId)\b/,
  },
  {
    name: "org policy API uses raw policy model contracts directly",
    regex: /\b(?:DaemonEnrollment|OrgPolicySnapshot|WorkspacePolicyOverlay)\b/,
  },
  {
    name: "org policy API defines local route DTOs",
    regex: /\bDaemonEnrollmentResponse\b|\bparse_org_id\s*\(|\bparse_workspace_id\s*\(/,
  },
  {
    name: "org policy API uses low-level policy errors directly",
    regex:
      /\b(?:CacheOrgPolicySnapshotError|WorkspacePolicyOverlayError|UpsertWorkspacePolicyOverlayError|UpsertDaemonEnrollmentError)\b/,
  },
  {
    name: "org policy API calls low-level policy facades directly",
    regex:
      /(?:\.\s*|\b(?:CoreHandle|WorkspacesHandle)\s*::\s*)(?:list_daemon_enrollments|upsert_daemon_enrollment_checked|cache_and_activate_org_policy_snapshot|get_workspace_policy_overlay|upsert_workspace_policy_overlay_checked)\s*\(/,
  },
  {
    name: "org policy API loads daemon enrollment directly for snapshot or overlay orchestration",
    regex: /(?:\.|\bCoreHandle::)get_daemon_enrollment_by_org_id\s*\(/,
    paths: [
      "core/crates/ctx-http/src/api/org_policy/snapshots.rs",
      "core/crates/ctx-http/src/api/org_policy/workspace_overlay.rs",
    ],
  },
  {
    name: "org policy snapshot API stores snapshots directly",
    regex: /(?:\.|\bCoreHandle::)upsert_org_policy_snapshot\s*\(/,
    paths: ["core/crates/ctx-http/src/api/org_policy/snapshots.rs"],
  },
  {
    name: "org policy snapshot API mutates daemon enrollment directly",
    regex: /(?:\.|\bCoreHandle::)upsert_daemon_enrollment\s*\(/,
    paths: ["core/crates/ctx-http/src/api/org_policy/snapshots.rs"],
  },
  {
    name: "org policy workspace overlay API writes overlays without daemon admission",
    regex: /(?:\.|\bWorkspacesHandle::)upsert_workspace_policy_overlay\s*\(/,
    paths: ["core/crates/ctx-http/src/api/org_policy/workspace_overlay.rs"],
  },
  {
    name: "org policy snapshot API mutates active snapshot id directly",
    regex: /\bactive_policy_snapshot_id\s*=/,
    paths: ["core/crates/ctx-http/src/api/org_policy/snapshots.rs"],
  },
  {
    name: "org policy snapshot API refreshes enrollment timestamp directly",
    regex: /\bupdated_at\s*=\s*(?:chrono\s*::\s*)?Utc\s*::\s*now\s*\(/,
    paths: ["core/crates/ctx-http/src/api/org_policy/snapshots.rs"],
  },
  {
    name: "org policy enrollment API validates plan eligibility directly",
    regex: /a^/,
    contentRegex:
      /\b(?:matches!\s*\([^)]*\bPlanType::[A-Za-z0-9_]+\b[^)]*\)|(?:if|let\s+[A-Za-z_][A-Za-z0-9_]*\s*=)[\s\S]{0,240}(?:\.plan_type|PlanType::[A-Za-z0-9_]+)[\s\S]{0,240}(?:\.plan_type|PlanType::[A-Za-z0-9_]+)[\s\S]{0,80}(?:\{|;))/gm,
    paths: ["core/crates/ctx-http/src/api/org_policy/enrollments.rs"],
  },
  {
    name: "org policy enrollment API validates signing key directly",
    regex: /a^/,
    contentRegex:
      /\b(?:if\s+|let\s+[A-Za-z_][A-Za-z0-9_]*\s*=)[^;\n]*policy_signing_key\s*\.\s*trim\s*\(\s*\)\s*\.\s*is_empty\s*\(/gm,
    paths: ["core/crates/ctx-http/src/api/org_policy/enrollments.rs"],
  },
  {
    name: "org policy enrollment API refreshes enrollment timestamp directly",
    regex: /\bupdated_at\s*=\s*(?:chrono\s*::\s*)?Utc\s*::\s*now\s*\(/,
    contentRegex:
      /\blet\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:chrono\s*::\s*)?Utc\s*::\s*now\s*\(\s*\)\s*;[\s\S]{0,160}\bupdated_at\s*=\s*\1\b/gm,
    paths: ["core/crates/ctx-http/src/api/org_policy/enrollments.rs"],
  },
  {
    name: "org policy enrollment API persists enrollment without checked daemon validation",
    regex: /(?:\.|\bCoreHandle::)upsert_daemon_enrollment\s*\(/,
    paths: ["core/crates/ctx-http/src/api/org_policy/enrollments.rs"],
  },
];

const REPO_ONBOARDING_API_ORCHESTRATION_PATTERNS = [
  {
    name: "repo onboarding API calls workspace-service onboarding directly",
    regex:
      /\bctx_workspace_services::repo_onboarding\b|\b[A-Za-z_][A-Za-z0-9_]*::repo_onboarding::(?:initialize_repo|clone_repo|validate_repo_destination|create_repo_staging_path|inspect_repo_status)\b|\brepo_onboarding::(?:initialize_repo|clone_repo|validate_repo_destination|create_repo_staging_path|inspect_repo_status)\b|\bservice::(?:initialize_repo|clone_repo|validate_repo_destination|create_repo_staging_path|inspect_repo_status)\b/,
  },
  {
    name: "repo onboarding API uses workspace-service onboarding DTOs directly",
    regex: /\b(?:RepoInitRequest|RepoCloneRequest|RepoValidateDestinationRequest)\b/,
  },
  {
    name: "repo onboarding API uses low-level daemon onboarding DTOs directly",
    regex:
      /\b(?:DaemonRepoInitRequest|DaemonRepoCloneRequest|DaemonRepoValidateDestinationRequest|DaemonRepoStatusCheck)\b/,
  },
  {
    name: "repo onboarding API defines local route DTOs",
    regex:
      /\b(?:RepoInitReq|RepoInitResp|RepoCloneReq|RepoCloneResp|RepoValidateDestinationReq|RepoValidateDestinationResp|RepoStagingPathResp|RepoStatusReq|RepoStatusResp)\b/,
  },
  {
    name: "repo onboarding API inspects low-level daemon onboarding errors directly",
    regex: /\b(?:RepoOnboardingError|RepoOnboardingErrorKind)\b/,
  },
  {
    name: "repo onboarding API calls low-level daemon onboarding facade directly",
    regex:
      /(?:\.\s*|WorkspacesHandle\s*::\s*)(?:initialize_repo|clone_repo|validate_repo_destination|create_repo_staging_path|inspect_repo_status)\s*\(/,
  },
  {
    name: "repo onboarding API stringifies paths directly",
    regex: /\.to_string_lossy\s*\(/,
  },
  {
    name: "repo onboarding API reads daemon data root directly",
    regex: /(?:\.|\bCoreHandle::)data_root\s*\(/,
  },
  {
    name: "repo onboarding API redacts workflow errors directly",
    regex: /\blogs::redact_sensitive\s*\(/,
  },
  {
    name: "repo onboarding API inspects workspace-service git errors directly",
    regex: /\.(?:spawn_message|failed_message)\s*\(/,
  },
  {
    name: "repo onboarding API inspects workspace-service path errors directly",
    regex: /a^/,
    contentRegex:
      /\bRepoOnboardingPathError\b[\s\S]{0,1200}\.message\s*\(\s*\)\s*\.to_string\s*\(/gm,
  },
];

const SMALL_API_UNIT_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct small API unit StoreManager access",
    regex: /\.stores\s*\(|\bStoreManager\b/,
  },
  {
    name: "raw small API unit ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct small API unit provider handle reach-through",
    regex: /a^/,
    contentRegex: /(?:\b[a-zA-Z_][a-zA-Z0-9_]*\s*\.\s*handle\s*\(\s*\)\s*\.\s*providers\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;]*?\.handle\s*\(\s*\)\s*;|\b[a-zA-Z_][a-zA-Z0-9_]*\s*\.\s*providers\s*\()/gm,
    allowContentRegex: /\b(?:fixture|self\.fixture)\s*\.\s*providers\s*\(/gm,
    allowSmallApiUnitProviderHandlerState: true,
  },
  {
    name: "direct small API unit core handle reach-through",
    regex: /a^/,
    contentRegex: /(?:\b[a-zA-Z_][a-zA-Z0-9_]*\s*\.\s*handle\s*\(\s*\)\s*\.\s*core\s*\(|\b[a-zA-Z_][a-zA-Z0-9_]*\s*\.\s*core\s*\()/gm,
    allowContentRegex: /\b(?:fixture|self\.fixture)\s*\.\s*core\s*\(/gm,
  },
  {
    name: "direct small API unit raw TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct small API unit router composition",
    regex: /\b(?:crate::)?api::router\s*\(|\bRouteHandles::from_daemon_handle\s*\(/,
  },
  {
    name: "direct small API unit global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct small API unit settings persistence",
    regex: /\bctx_settings_service::save_settings\s*\(|(?<![\w.])\bsave_settings\s*\(/,
    contentRegex: /^\s*use\s+ctx_settings_service::(?:[^;\n]*\bsave_settings\b[^;\n]*|\{(?=[^}]*\bsave_settings\b)[^}]*\})\s*;/gm,
  },
];

const SMALL_EXTERNAL_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct small external StoreManager access",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw small external ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct small external store manager helper",
    regex: /\b(?:crate::)?common::setup_store\s*\(|\buse\s+[^;]*\bsetup_store\b|\bsetup_store\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bsetup_store\b[\s\S]*?;/gm,
  },
  {
    name: "direct small external daemon construction helper",
    regex: /\b(?:crate::)?common::build_daemon\s*\(|\buse\s+[^;]*\bbuild_daemon\b|\bbuild_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bbuild_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct small external TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
    contentRegex: /\buse\s+ctx_daemon::test_support::(?:TestDaemon\s+as\s+[A-Za-z_][A-Za-z0-9_]*|\{[^}]*\bTestDaemon\s+as\s+[A-Za-z_][A-Za-z0-9_]*[^}]*\})\s*;[\s\S]*?\b[A-Za-z_][A-Za-z0-9_]*::new[A-Za-z0-9_]*\s*\(|\btype\s+[A-Za-z_][A-Za-z0-9_]*\s*=\s*(?:ctx_daemon::test_support::)?TestDaemon\s*;[\s\S]*?\b[A-Za-z_][A-Za-z0-9_]*::new[A-Za-z0-9_]*\s*\(/gm,
  },
  {
    name: "direct small external TestDaemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
];

const FAKE_DAEMON_EXTERNAL_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct fake-daemon external StoreManager access",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw fake-daemon external ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct fake-daemon external store manager helper",
    regex: /\b(?:crate::)?common::setup_store\s*\(|\buse\s+[^;]*\bsetup_store\b|\bsetup_store\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bsetup_store\b[\s\S]*?;/gm,
  },
  {
    name: "direct fake-daemon external daemon construction helper",
    regex: /\b(?:crate::)?common::build_daemon\s*\(|\buse\s+[^;]*\bbuild_daemon\b|\bbuild_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bbuild_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct fake-daemon external TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct fake-daemon external TestDaemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
];

const ACP_CRP_BRIDGE_TOKEN_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct ACP CRP bridge token StoreManager access",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw ACP CRP bridge token ctx_store Store",
    regex: /\bctx_store\s*::\s*Store\b|\buse\s+ctx_store\s*::[^;]*\bStore\b|\b[A-Za-z_][A-Za-z0-9_]*\s*::\s*open_sqlite\s*\(/,
    contentRegex: /\buse\s+ctx_store\s*::\s*\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct ACP CRP bridge token TestDaemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "direct ACP CRP bridge token settings store load",
    regex: /\bctx_settings_service\s*::\s*load_settings\s*\(|(?<![\w.])\bload_settings\s*\(/,
    contentRegex: /\buse\s+ctx_settings_service\s*::\s*\{(?=[^}]*\n)[\s\S]*?\bload_settings\b[\s\S]*?\}|^\s*use\s+ctx_settings_service\s*::[^;\n]*\bload_settings\b[^;\n]*;/gm,
  },
  {
    name: "direct ACP CRP bridge token settings DB path",
    regex: /\bdb\.sqlite\b|\.join\s*\(\s*["']db["']\s*\)/,
  },
  {
    name: "direct ACP CRP bridge token settings store close",
    regex: /\b(?:store|settings_store|global_store|raw_store)\s*\.close\s*\(\s*\)\s*\.await\b/,
  },
];

const IMAGE_ATTACHMENTS_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct image attachments daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct image attachments generic store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "direct image attachments blob store access",
    regex: /\.(?:insert_blob|get_blob)\s*\(/,
  },
  {
    name: "direct image attachments session message query",
    regex: /\.(?:count_user_messages_for_session|list_messages_for_session|get_message)\s*\(/,
  },
];

const WORKSPACE_ATTACHMENTS_DEMO_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct workspace attachments demo StoreManager access",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw workspace attachments demo ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct workspace attachments demo raw TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct workspace attachments demo store setup helper",
    regex: /\b(?:crate::)?common::setup_store\s*\(|\buse\s+[^;]*\bsetup_store\b|\bsetup_store\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bsetup_store\b[\s\S]*?;/gm,
  },
  {
    name: "direct workspace attachments demo daemon construction helper",
    regex: /\b(?:crate::)?common::build_daemon\s*\(|\buse\s+[^;]*\bbuild_daemon\b|\bbuild_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bbuild_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct workspace attachments demo daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct workspace attachments demo generic store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "direct workspace attachments demo manual worktree setup",
    regex: /\bWorktree\s*\{|\binsert_worktree\s*\(|\bcreate_worktree\s*\(/,
  },
  {
    name: "direct workspace attachments demo generic seed helper",
    regex: /\bseed_(?:workspace|task_lifecycle_workspace|task_lifecycle_task)_for_test\s*\(/,
  },
];

const PROVIDER_TARGET_SCOPED_INSTALLS_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct provider target scoped installs StoreManager access",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw provider target scoped installs ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct provider target scoped installs store setup helper",
    regex: /\b(?:crate::)?common::setup_store\s*\(|\buse\s+[^;]*\bsetup_store\b|\bsetup_store\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bsetup_store\b[\s\S]*?;/gm,
  },
  {
    name: "direct provider target scoped installs daemon construction helper",
    regex: /\b(?:crate::)?common::build_daemon\s*\(|\buse\s+[^;]*\bbuild_daemon\b|\bbuild_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bbuild_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct provider target scoped installs daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct provider target scoped installs generic store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "direct provider target scoped installs broad install/cache observation",
    regex: /\.(?:get_install_info|find_running_install|tracked_install_ids|has_target_provider_adapter|target_provider_adapter_cache_keys|start_install|install_provider_with_progress)\s*\(/,
  },
];

const WORKTREE_VCS_SNAPSHOT_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct worktree-vcs-snapshot daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
];

const PROVIDER_PROBE_RUNTIME_ENV_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct provider-probe daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
];

const DEFAULT_SESSION_AND_DIFF_FAKE_DAEMON_FIXTURE_PATTERNS = [
  {
    name: "direct default-session/diff store manager helper",
    regex: /\b(?:crate::)?common::setup_store\s*\(|\buse\s+[^;]*\bsetup_store\b|\bsetup_store\s*\(/,
    contentRegex: /^\s*use\s+[^\n;]*\bsetup_store\b[^\n;]*;/gm,
  },
  {
    name: "direct default-session/diff daemon construction helper",
    regex: /\b(?:crate::)?common::build_daemon\s*\(|\buse\s+[^;]*\bbuild_daemon\b|\bbuild_daemon\s*\(/,
    contentRegex: /^\s*use\s+[^\n;]*\bbuild_daemon\b[^\n;]*;/gm,
  },
  {
    name: "direct default-session/diff daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /^\s*use\s+[^\n;]*\brouter_for_daemon\b[^\n;]*;/gm,
  },
  {
    name: "direct default-session/diff TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct default-session/diff TestDaemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "raw default-session/diff StoreManager",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw default-session/diff ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
];

const WORKSPACE_VCS_SETUP_FIXTURE_PATTERNS = [
  {
    name: "direct workspace/VCS setup store manager helper",
    regex: /\b(?:crate::)?common::setup_store\s*\(|\buse\s+[^;]*\bsetup_store\b|\bsetup_store\s*\(/,
    contentRegex: /^\s*use\s+[^\n;]*\bsetup_store\b[^\n;]*;/gm,
  },
  {
    name: "direct workspace/VCS setup daemon construction helper",
    regex: /\b(?:crate::)?common::build_daemon\s*\(|\buse\s+[^;]*\bbuild_daemon\b|\bbuild_daemon\s*\(/,
    contentRegex: /^\s*use\s+[^\n;]*\bbuild_daemon\b[^\n;]*;/gm,
  },
  {
    name: "direct workspace/VCS setup daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /^\s*use\s+[^\n;]*\brouter_for_daemon\b[^\n;]*;/gm,
  },
  {
    name: "direct workspace/VCS setup TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct workspace/VCS setup TestDaemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "raw workspace/VCS setup StoreManager",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw workspace/VCS setup ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
];

const SMALL_ROUTE_FIXTURE_PATTERNS = [
  {
    name: "direct small-route provider daemon fixture helper",
    regex: /\b(?:crate::)?common::provider_route_fake_daemon\s*\(|\buse\s+[^;]*\bprovider_route_fake_daemon\b|\bprovider_route_fake_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bprovider_route_fake_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct small-route daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct small-route TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct small-route TestDaemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "raw small-route StoreManager",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw small-route ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
];

const PROVIDER_AUTH_GLOBAL_ID_FIXTURE_PATTERNS = [
  {
    name: "direct provider-auth/global-id provider daemon fixture helper",
    regex: /\b(?:crate::)?common::provider_route_fake_daemon\s*\(|\buse\s+[^;]*\bprovider_route_fake_daemon\b|\bprovider_route_fake_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bprovider_route_fake_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct provider-auth/global-id daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct provider-auth/global-id TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct provider-auth/global-id TestDaemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "raw provider-auth/global-id StoreManager",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw provider-auth/global-id ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
];

const UPDATE_ROUTE_FIXTURE_PATTERNS = [
  {
    name: "direct update-route setup store manager helper",
    regex: /\b(?:crate::)?common::setup_store\s*\(|\buse\s+[^;]*\bsetup_store\b|\bsetup_store\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bsetup_store\b[\s\S]*?;/gm,
  },
  {
    name: "direct update-route setup daemon construction helper",
    regex: /\b(?:crate::)?common::build_daemon\s*\(|\buse\s+[^;]*\bbuild_daemon\b|\bbuild_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bbuild_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct update-route daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct update-route TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct update-route TestDaemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "raw update-route StoreManager",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw update-route ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
];

const CACHE_REHYDRATION_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct cache rehydration store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "direct cache rehydration store creation",
    regex: /\.(?:create_workspace|create_worktree|create_task|create_session|upsert_workspace_task_index|upsert_workspace_session_index)\s*\(/,
  },
  {
    name: "direct cache rehydration session event write",
    regex: /\.(?:insert_session_turn|append_session_event|update_session_turn_status|insert_message)\s*\(/,
  },
  {
    name: "direct cache rehydration projection read",
    regex: /\.(?:get_session_head_snapshot|get_active_snapshot_head|get_workspace_active_task_summary|get_session_projection_rev)\s*\(/,
  },
  {
    name: "direct cache rehydration workspace delete",
    regex: /\.(?:delete_workspace_indexes|delete_workspace)\s*\(/,
  },
  {
    name: "direct cache rehydration head delta publication",
    regex: /\.publish_session_head_delta\s*\(|\bSessionHeadDelta\b/,
  },
  {
    name: "raw cache rehydration event/status model",
    regex: /\bSessionEventType\b|\bSessionTurnStatus\b/,
  },
];

const SUBSCRIPTION_ACCOUNTS_API_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct subscription accounts daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\brouter_for_daemon\s*\(/,
  },
];

const SESSION_MODEL_API_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct session model daemon router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\brouter_for_daemon\s*\(/,
  },
  {
    name: "direct session model provider options cache mutation",
    regex: /\.test_with_provider_options_cache\s*\(|\bCachedProviderOptions\b/,
  },
  {
    name: "direct session model store seeding",
    regex: /\.(?:create_workspace|create_worktree|create_task|create_session|create_session_with_reasoning_effort|upsert_workspace_worktree_index|upsert_workspace_task_index|upsert_workspace_session_index|set_task_primary_session)\s*\(/,
  },
];

const MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct MCP daemon global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct MCP daemon session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct MCP daemon workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct MCP daemon StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct MCP daemon StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
];

const SUBAGENT_MCP_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct subagent MCP raw Store type",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::Store\b|\buse\s+ctx_store::\{[^}]*\bStore(?:\s+as\s+\w+)?\b[^}]*\}/s,
  },
  {
    name: "direct subagent MCP StoreManager access",
    regex: /\bStoreManager\b|\bStore::open_sqlite\s*\(/,
  },
  {
    name: "direct subagent MCP common setup helper",
    regex: /\bcommon::(?:setup_store|build_daemon|router_for_daemon)\s*\(/,
  },
  {
    name: "direct subagent MCP daemon store access",
    regex: /\.(?:global_store|stores|store_for_workspace|store_for_session|store_for_task)\s*\(/,
  },
  {
    name: "direct subagent MCP raw session seed/probe",
    regex: /\.(?:create_session|update_session_title|get_subagent_session_by_label|list_session_turns_page_by_seq|get_message|list_session_events_for_turn|is_archived_subagent_session|count_active_subagent_sessions)\s*\(/,
  },
  {
    name: "direct subagent MCP raw turn/tool write",
    regex: /\.(?:insert_session_turn|upsert_session_turn_tool)\s*\(/,
  },
  {
    name: "direct subagent MCP raw sandbox/worktree probe",
    regex: /\.(?:upsert_sandbox_binding|get_worktree|get_sandbox_binding)\s*\(/,
  },
  {
    name: "direct subagent MCP bootstrap config write",
    regex: /\bctx_workspace_config::update_worktree_bootstrap_config\s*\(/,
  },
  {
    name: "direct subagent MCP head delta publication",
    regex: /\.publish_session_head_delta\s*\(|\bSessionHeadDelta\b/,
  },
];

const REPLAY_PROPERTIES_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct replay properties raw Store type",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::Store\b|\buse\s+ctx_store::\{[^}]*\bStore(?:\s+as\s+\w+)?\b[^}]*\}/s,
  },
  {
    name: "direct replay properties StoreManager access",
    regex: /\bStoreManager\b|\bStore::open_sqlite\s*\(/,
  },
  {
    name: "direct replay properties common setup helper",
    regex: /\bcommon::(?:setup_store|build_daemon|router_for_daemon)\s*\(/,
  },
  {
    name: "direct replay properties daemon store access",
    regex: /\.(?:global_store|stores|store_for_workspace|store_for_session|store_for_task)\s*\(/,
  },
  {
    name: "direct replay properties raw row mutation",
    regex: /\.(?:insert_session_turn|insert_message|append_session_event|update_session_turn_status|upsert_session_turn_tool)\s*\(/,
  },
  {
    name: "direct replay properties raw replay projection helper",
    regex: /\.(?:publish_replay_fixture_event_for_test|refresh_replay_projection_fixture_for_test|remove_replay_session_head_for_test)\s*\(/,
  },
  {
    name: "direct replay properties raw projection seed model",
    regex: /\b(?:SessionTurn|SessionTurnStatus|SessionTurnTool|MessageRole|MessageDelivery|SessionEventType)\b/,
  },
];

const SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct session fixture global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct session fixture session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct session fixture workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct session fixture uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct session fixture task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct task session creation lock access",
    regex: /\.task_session_creation_lock\s*\(/,
  },
  {
    name: "direct session fixture StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct session fixture StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct session fixture StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "legacy session fixture provider daemon construction",
    regex: /\btest_daemon_with_providers\s*\(/,
  },
  {
    name: "raw session fixture ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw session fixture StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const SMALL_BOUNDARY_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct small-boundary global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct small-boundary session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct small-boundary workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct small-boundary uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct small-boundary task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct small-boundary StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct small-boundary StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct small-boundary StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "direct sessions handle access in migrated test",
    regex: /a^/,
    contentRegex: /\.handle\s*\(\s*\)\s*\.\s*sessions\s*\(/gm,
  },
  {
    name: "direct workspaces handle access in migrated test",
    regex: /a^/,
    contentRegex: /\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(/gm,
  },
  {
    name: "raw small-boundary ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
];

const PROVIDERLESS_LIB_ROUTE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "raw providerless lib-route StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const EXECUTION_LAUNCH_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct execution-launch global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct execution-launch session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct execution-launch workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct execution-launch uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct execution-launch task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct execution-launch StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct execution-launch StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct execution-launch StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "legacy execution-launch provider daemon construction",
    regex: /\btest_daemon_with_providers\s*\(/,
  },
  {
    name: "raw execution-launch ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw execution-launch StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const GLOBAL_ID_ROUTING_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct global-id-routing global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct global-id-routing session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct global-id-routing workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct global-id-routing uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct global-id-routing task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct global-id-routing StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct global-id-routing StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct global-id-routing StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "raw global-id-routing ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw global-id-routing StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const TERMINAL_WORKSPACE_STREAM_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct terminal-workspace-stream global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct terminal-workspace-stream session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct terminal-workspace-stream workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct terminal-workspace-stream uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct terminal-workspace-stream task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct terminal-workspace-stream StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct terminal-workspace-stream StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct terminal-workspace-stream StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "raw terminal-workspace-stream ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw terminal-workspace-stream StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const WORKSPACE_RUNTIME_SETTINGS_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct workspace-runtime-settings global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct workspace-runtime-settings session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct workspace-runtime-settings workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct workspace-runtime-settings uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct workspace-runtime-settings task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct workspace-runtime-settings StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct workspace-runtime-settings StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct workspace-runtime-settings StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "raw workspace-runtime-settings ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw workspace-runtime-settings StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const WORKSPACE_MERGE_QUEUE_CONFIG_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct workspace-merge-queue-config global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct workspace-merge-queue-config session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct workspace-merge-queue-config workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct workspace-merge-queue-config uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct workspace-merge-queue-config task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct workspace-merge-queue-config StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct workspace-merge-queue-config StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct workspace-merge-queue-config StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace\s*\(/gm,
  },
  {
    name: "direct workspace-merge-queue-config workspaces handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.workspaces\s*\()/gm,
  },
  {
    name: "direct workspace-merge-queue-config daemon merge-queue module access",
    regex: /\bctx_daemon::daemon::merge_queue\b|\bdaemon::merge_queue\b|\bmerge_queue::/,
  },
  {
    name: "raw workspace-merge-queue-config ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw workspace-merge-queue-config StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const JJ_MERGE_QUEUE_BASICS_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct jj-merge-queue-basics global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct jj-merge-queue-basics session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct jj-merge-queue-basics workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct jj-merge-queue-basics uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct jj-merge-queue-basics task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct jj-merge-queue-basics StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct jj-merge-queue-basics StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct jj-merge-queue-basics StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace\s*\(/gm,
  },
  {
    name: "direct jj-merge-queue-basics workspaces handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.workspaces\s*\()/gm,
  },
  {
    name: "direct jj-merge-queue-basics sessions handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*sessions\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.sessions\s*\()/gm,
  },
  {
    name: "direct jj-merge-queue-basics daemon merge-queue module access",
    regex: /\bctx_daemon::daemon::merge_queue\b|\bdaemon::merge_queue\b|\bmerge_queue::/,
  },
  {
    name: "direct jj-merge-queue-basics worktree row load",
    regex: /\.get_worktree\s*\(|\.load_worktree_for_test\s*\(/,
  },
  {
    name: "raw jj-merge-queue-basics ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw jj-merge-queue-basics StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const MERGE_QUEUE_ISOLATION_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct merge-queue-isolation global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct merge-queue-isolation session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct merge-queue-isolation workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct merge-queue-isolation uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct merge-queue-isolation task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct merge-queue-isolation worktree store access",
    regex: /\.store_for_worktree\s*\(/,
  },
  {
    name: "direct merge-queue-isolation StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct merge-queue-isolation StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct merge-queue-isolation StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace(?:_uncached)?\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace(?:_uncached)?\s*\(/gm,
  },
  {
    name: "direct merge-queue-isolation workspaces handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.workspaces\s*\()/gm,
  },
  {
    name: "direct merge-queue-isolation sessions handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*sessions\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.sessions\s*\()/gm,
  },
  {
    name: "direct merge-queue-isolation workspace config write",
    regex: /\bctx_workspace_config::|\bupdate_merge_queue_config\s*\(|\bMergeQueueConfigUpdate\b|\bMergeQueueCanonicalSync\b/,
  },
  {
    name: "direct merge-queue-isolation daemon merge-queue module access",
    regex: /\bctx_daemon::daemon::merge_queue\b|\bdaemon::merge_queue\b|\bmerge_queue::/,
  },
  {
    name: "direct merge-queue-isolation worktree row access",
    regex: /\.(?:create_worktree|insert_worktree|get_worktree|load_worktree_for_test|upsert_workspace_worktree_index)\s*\(/,
  },
  {
    name: "direct merge-queue-isolation entry or run row access",
    regex: /\.(?:get_merge_queue_entry|list_merge_queue_entries|get_latest_merge_queue_run|create_merge_queue_entry|create_merge_queue_run)\s*\(/,
  },
  {
    name: "raw merge-queue-isolation ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw merge-queue-isolation StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const PROVIDER_WORKER_REAPING_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct provider-worker-reaping global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct provider-worker-reaping session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct provider-worker-reaping workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct provider-worker-reaping uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct provider-worker-reaping task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct provider-worker-reaping worktree store access",
    regex: /\.store_for_worktree\s*\(/,
  },
  {
    name: "direct provider-worker-reaping StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct provider-worker-reaping StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct provider-worker-reaping StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace(?:_uncached)?\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace(?:_uncached)?\s*\(/gm,
  },
  {
    name: "direct provider-worker-reaping session event query",
    regex: /\.list_session_events\s*\(/,
  },
  {
    name: "direct provider-worker-reaping session row query",
    regex: /\.get_session\s*\(/,
  },
  {
    name: "direct provider-worker-reaping sessions handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*sessions\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.sessions\s*\()/gm,
  },
  {
    name: "direct provider-worker-reaping workspaces handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.workspaces\s*\()/gm,
  },
  {
    name: "direct provider-worker-reaping tasks handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*tasks\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.tasks\s*\()/gm,
  },
  {
    name: "direct provider-worker-reaping SessionEventType",
    regex: /\bSessionEventType\b/,
  },
  {
    name: "raw provider-worker-reaping ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw provider-worker-reaping StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const PROVIDER_SCENARIOS_OFFLINE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct provider-scenarios offline StoreManager access",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw provider-scenarios offline ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct provider-scenarios offline store manager helper",
    regex: /\b(?:crate::)?common::setup_store\s*\(|\buse\s+[^;]*\bsetup_store\b|\bsetup_store\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bsetup_store\b[\s\S]*?;/gm,
  },
  {
    name: "direct provider-scenarios offline daemon construction helper",
    regex: /\b(?:crate::)?common::build_daemon\s*\(|\buse\s+[^;]*\bbuild_daemon\b|\bbuild_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\bbuild_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct provider-scenarios offline router composition",
    regex: /\b(?:crate::)?common::router_for_daemon\s*\(|\buse\s+[^;]*\brouter_for_daemon\b|\brouter_for_daemon\s*\(/,
    contentRegex: /\buse\s+[\s\S]*?\brouter_for_daemon\b[\s\S]*?;/gm,
  },
  {
    name: "direct provider-scenarios offline TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct provider-scenarios offline daemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "direct provider-scenarios offline session event query",
    regex: /\.list_session_events\s*\(/,
  },
  {
    name: "direct provider-scenarios offline session turn query",
    regex: /\.list_session_turns_page_by_seq\s*\(/,
  },
  {
    name: "direct provider-scenarios offline session message query",
    regex: /\.list_messages_for_session\s*\(/,
  },
  {
    name: "direct provider-scenarios offline turn-status polling",
    regex: /\bSessionTurnStatus\b/,
  },
];

const HARNESS_CONTAINER_SANDBOX_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct harness-container sandbox StoreManager access",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw harness-container sandbox ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::open_sqlite\s*\(/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct harness-container sandbox settings service",
    regex: /\bctx_settings_service::(?:load_settings|save_settings)\s*\(|\buse\s+ctx_settings_service::[^;]*(?:load_settings|save_settings)\b|\b(?:load_settings|save_settings)\s*\(/,
    contentRegex: /\buse\s+ctx_settings_service::\{(?=[^}]*\n)[\s\S]*?\b(?:load_settings|save_settings)\b[\s\S]*?\}/gm,
  },
  {
    name: "direct harness-container sandbox common setup helper",
    regex: /\b(?:crate::)?common::(?:setup_store|build_daemon|router_for_daemon)\s*\(|\buse\s+[^;]*\b(?:setup_store|build_daemon|router_for_daemon)\b/,
    contentRegex: /\buse\s+[\s\S]*?\b(?:setup_store|build_daemon|router_for_daemon)\b[\s\S]*?;/gm,
  },
  {
    name: "direct harness-container sandbox TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct harness-container sandbox daemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "direct harness-container sandbox raw session event query",
    regex: /\.list_session_events\s*\(|\bSessionEventType\b/,
  },
  {
    name: "direct harness-container sandbox raw workspace/worktree query",
    regex: /\.(?:get_workspace|get_worktree)\s*\(/,
  },
  {
    name: "direct harness-container sandbox daemon harness prep",
    regex: /\.(?:prepare_workspace_harness_for_test|workspace_harness_egress_guard_for_test)\s*\(/,
  },
];

const LIVE_PROVIDER_CANARY_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct live-provider canary StoreManager access",
    regex: /\bStoreManager\b/,
  },
  {
    name: "raw live-provider canary ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore::open_sqlite\s*\(/,
    contentRegex: /\buse\s+ctx_store::\{(?=[^}]*\n)[\s\S]*?\bStore\b[\s\S]*?\}/gm,
  },
  {
    name: "direct live-provider canary common setup helper",
    regex: /\b(?:crate::)?common::(?:setup_store|build_daemon|router_for_daemon)\s*\(|\buse\s+[^;]*\b(?:setup_store|build_daemon|router_for_daemon)\b/,
    contentRegex: /\buse\s+[\s\S]*?\b(?:setup_store|build_daemon|router_for_daemon)\b[\s\S]*?;/gm,
  },
  {
    name: "direct live-provider canary TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct live-provider canary daemon store access",
    regex: /\.(?:stores|global_store|store_for_session|store_for_workspace|uncached_store_for_workspace|store_for_task|store_for_worktree)\s*\(/,
  },
  {
    name: "direct live-provider canary raw session event query",
    regex: /\.list_session_events\s*\(|\bSessionEventType\b/,
  },
  {
    name: "direct live-provider canary event-message extraction helper",
    regex: /\bassistant_messages_from_events\b/,
  },
];

const WORKTREE_ARCHIVE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct worktree-archive global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct worktree-archive session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct worktree-archive workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct worktree-archive uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct worktree-archive task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct worktree-archive worktree store access",
    regex: /\.store_for_worktree\s*\(/,
  },
  {
    name: "direct worktree-archive StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct worktree-archive StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct worktree-archive StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace(?:_uncached)?\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace(?:_uncached)?\s*\(/gm,
  },
  {
    name: "direct worktree-archive task row query",
    regex: /\.get_task\s*\(/,
  },
  {
    name: "direct worktree-archive task-session query",
    regex: /\.list_sessions_for_task\s*\(/,
  },
  {
    name: "direct worktree-archive worktree row query",
    regex: /\.get_worktree\s*\(|\.load_worktree_for_test\s*\(/,
  },
  {
    name: "direct worktree-archive sessions handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*sessions\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.sessions\s*\()/gm,
  },
  {
    name: "direct worktree-archive workspaces handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.workspaces\s*\()/gm,
  },
  {
    name: "direct worktree-archive tasks handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*tasks\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.tasks\s*\()/gm,
  },
  {
    name: "direct worktree-archive managed worktree path reconstruction",
    regex: /\bctx_fs::worktrees::managed_worktree_path\b|\bmanaged_worktree_path\b|\bmatching_managed_worktree_path\b|\bctx_workspace_services::worktree_vcs\b|\bctx_daemon::daemon::workspaces::managed_worktree_root\b|\bdaemon::workspaces::managed_worktree_root\b|\bworkspaces::managed_worktree_root\b|\bmanaged_worktree_root\b/,
  },
  {
    name: "raw worktree-archive ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw worktree-archive StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const FAULT_INJECTION_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct fault-injection global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct fault-injection session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct fault-injection workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct fault-injection uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct fault-injection task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct fault-injection worktree store access",
    regex: /\.store_for_worktree\s*\(/,
  },
  {
    name: "direct fault-injection StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct fault-injection StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct fault-injection StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace(?:_uncached)?\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace(?:_uncached)?\s*\(/gm,
  },
  {
    name: "direct fault-injection task-session query",
    regex: /\.list_sessions_for_task\s*\(/,
  },
  {
    name: "direct fault-injection session event append",
    regex: /\.append_session_event\s*\(/,
  },
  {
    name: "direct fault-injection session head store query",
    regex: /\.get_session_head_snapshot\s*\(/,
  },
  {
    name: "direct fault-injection sessions handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*sessions\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.sessions\s*\()/gm,
  },
  {
    name: "direct fault-injection workspaces handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.workspaces\s*\()/gm,
  },
  {
    name: "direct fault-injection tasks handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*tasks\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.tasks\s*\()/gm,
  },
  {
    name: "direct fault-injection SessionEventType",
    regex: /\bSessionEventType\b/,
  },
  {
    name: "raw fault-injection ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw fault-injection StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const STREAM_RUNTIME_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct stream-runtime global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct stream-runtime session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct stream-runtime workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct stream-runtime uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct stream-runtime task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct stream-runtime StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct stream-runtime StoreManager global access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.global\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.global\s*\(/gm,
  },
  {
    name: "direct stream-runtime StoreManager workspace access",
    regex: /\b[a-zA-Z_][a-zA-Z0-9_]*\.workspace(?:_uncached)?\s*\(/,
    contentRegex: /\b[a-zA-Z_][a-zA-Z0-9_]*\s*\n\s*\.workspace(?:_uncached)?\s*\(/gm,
  },
  {
    name: "direct stream-runtime sessions handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*sessions\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.sessions\s*\()/gm,
  },
  {
    name: "direct stream-runtime workspaces handle access",
    regex: /a^/,
    contentRegex: /(?:\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(|\blet\s+[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*[^;\n]*\.handle\s*\(\s*\)\s*;|[a-zA-Z_][a-zA-Z0-9_]*\.workspaces\s*\()/gm,
  },
  {
    name: "raw stream-runtime ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw stream-runtime StoreManager",
    regex: /\bStoreManager\b/,
  },
];

const SCHEDULER_RUNTIME_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct scheduler-runtime global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct scheduler-runtime session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct scheduler-runtime workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct scheduler-runtime uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct scheduler-runtime task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct scheduler-runtime StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct scheduler-runtime StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct scheduler-runtime StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "direct scheduler-runtime sessions handle access",
    regex: /a^/,
    contentRegex: /\.handle\s*\(\s*\)\s*\.\s*sessions\s*\(/gm,
  },
  {
    name: "raw scheduler-runtime ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
];

const TASK_LIFECYCLE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct task-lifecycle global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct task-lifecycle session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct task-lifecycle workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct task-lifecycle uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct task-lifecycle task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct task-lifecycle StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct task-lifecycle StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct task-lifecycle StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "direct task-lifecycle workspaces handle access",
    regex: /a^/,
    contentRegex: /\.handle\s*\(\s*\)\s*\.\s*workspaces\s*\(/gm,
  },
  {
    name: "raw task-lifecycle ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
  {
    name: "raw task-lifecycle StoreManager",
    regex: /\bStoreManager\b/,
  },
  {
    name: "direct task-lifecycle raw TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct task-lifecycle settings persistence",
    regex: /\bctx_settings_service::save_settings\s*\(|(?<![\w.])\bsave_settings\s*\(/,
    contentRegex: /^\s*use\s+ctx_settings_service::(?:[^;\n]*\bsave_settings\b[^;\n]*|\{(?=[^}]*\bsave_settings\b)[^}]*\})\s*;/gm,
  },
];

const STORAGE_ADMISSION_FIXTURE_PATTERNS = [
  {
    name: "direct storage-admission raw TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct storage-admission router helper",
    regex: /\btest_router\s*\(/,
  },
  {
    name: "direct storage-admission router composition",
    regex: /\b(?:crate::)?api::router\s*\(|\bRouteHandles::from_daemon_handle\s*\(/,
  },
];

const LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS = [
  {
    name: "direct lib-test raw TestDaemon construction",
    regex: /\bTestDaemon::new[A-Za-z0-9_]*\s*\(/,
  },
  {
    name: "direct lib-test raw TestDaemon construction",
    regex: /a^/,
    contentRegex: /\buse\s+ctx_daemon::test_support::TestDaemon\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\s*;[\s\S]*?\b\1::new[A-Za-z0-9_]*\s*\(/gm,
  },
  {
    name: "direct lib-test raw TestDaemon construction",
    regex: /a^/,
    contentRegex: /\buse\s+ctx_daemon::test_support::\{[^}]*\bTestDaemon\s+as\s+([A-Za-z_][A-Za-z0-9_]*)[^}]*\}\s*;[\s\S]*?\b\1::new[A-Za-z0-9_]*\s*\(/gm,
  },
  {
    name: "direct lib-test raw TestDaemon construction",
    regex: /a^/,
    contentRegex: /\btype\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:ctx_daemon::test_support::)?TestDaemon\s*;[\s\S]*?\b\1::new[A-Za-z0-9_]*\s*\(/gm,
  },
  {
    name: "direct lib-test legacy daemon helper",
    regex: /\btest_daemon_(?:with_fake_provider_)?for_test\s*\(/,
  },
  {
    name: "direct lib-test router helper",
    regex: /\btest_router\s*\(/,
  },
  {
    name: "direct lib-test router composition",
    regex: /\b(?:crate::)?api::router\s*\(|\bRouteHandles::from_daemon_handle\s*\(/,
  },
];

function isRustFile(filePath) {
  return filePath.endsWith(".rs");
}

function isTestRustPath(filePath) {
  const normalized = filePath.split(path.sep).join("/");
  const base = path.basename(filePath);
  return normalized.includes("/tests/")
    || normalized.includes("/lib_tests/")
    || normalized.includes("/test_support/")
    || base === "test_support.rs"
    || normalized.includes("/lifecycle_tests/")
    || normalized.includes("/storage_admission_http_tests/")
    || normalized.includes("/cleanup_lifecycle_tests")
    || base === "tests.rs"
    || base.endsWith("_tests.rs");
}

function testSurfaceRustFiles() {
  const files = [];
  if (fs.existsSync(ctxHttpSrcRoot)) {
    files.push(
      ...listRustFiles(ctxHttpSrcRoot).filter((filePath) => {
        const relativePath = repoRelative(filePath);
        return isTestRustPath(filePath) || migratedTestPatternsForPath(relativePath).length > 0;
      }),
    );
  }
  for (const root of [ctxHttpTestsRoot, ctxHttpTestSupportSrcRoot]) {
    if (fs.existsSync(root)) {
      files.push(...listRustFiles(root));
    }
  }
  return files;
}

function listRustFiles(root) {
  const out = [];
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const fullPath = path.join(root, entry.name);
    if (entry.isDirectory()) {
      out.push(...listRustFiles(fullPath));
    } else if (entry.isFile() && isRustFile(fullPath)) {
      out.push(fullPath);
    }
  }
  return out;
}

function stripCfgTestItems(contents) {
  const lines = contents.split(/\r?\n/);
  const kept = [];
  let skipCfgItem = false;
  let braceDepth = 0;
  let sawCfgItemBody = false;

  for (const line of lines) {
    if (!skipCfgItem && /^\s*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]/.test(line)) {
      skipCfgItem = true;
      braceDepth = 0;
      sawCfgItemBody = false;
      continue;
    }

    if (skipCfgItem) {
      for (const char of line) {
        if (char === "{") {
          braceDepth += 1;
          sawCfgItemBody = true;
        }
        if (char === "}") braceDepth -= 1;
      }
      if (!sawCfgItemBody && line.trim().endsWith(";")) {
        skipCfgItem = false;
      } else if (sawCfgItemBody && braceDepth <= 0) {
        skipCfgItem = false;
      }
      continue;
    }

    kept.push(line);
  }

  return kept.join("\n");
}

function isAllowedSmallApiUnitProviderHandlerState(lines, lineIndex, matchColumn, matchText) {
  if (!/^[a-zA-Z_][a-zA-Z0-9_]*\s*\.\s*handle\s*\(\s*\)\s*\.\s*providers\s*\(/.test(matchText)) {
    return false;
  }
  const line = lines[lineIndex] ?? "";
  const prefix = line.slice(0, matchColumn);
  if (!/^\s*State\s*\(\s*$/.test(prefix)) {
    return false;
  }
  const previous = lines[lineIndex - 1]?.trim() ?? "";
  return /^(?:let\s+(?:Json\([^)]*\)|err)\s*=\s*)?(?:get_install_statuses|select_provider_harness_source|set_codex_active_account)\s*\($/.test(
    previous,
  );
}

function scanText({ filePath, contents, patterns }) {
  const violations = [];
  const lines = contents.split(/\r?\n/);
  const lineStartOffsets = [];
  let offset = 0;
  for (const line of lines) {
    lineStartOffsets.push(offset);
    offset += line.length + 1;
  }
  const lineForOffset = (matchOffset) => {
    let lineIndex = 0;
    for (let index = 0; index < lineStartOffsets.length; index += 1) {
      if (lineStartOffsets[index] > matchOffset) {
        break;
      }
      lineIndex = index;
    }
    return lineIndex;
  };
  for (const pattern of patterns) {
    if (pattern.paths && !pattern.paths.includes(filePath)) {
      continue;
    }
    for (let index = 0; index < lines.length; index += 1) {
      if (pattern.regex.test(lines[index])) {
        violations.push({
          filePath,
          line: index + 1,
          name: pattern.name,
          text: lines[index].trim(),
        });
      }
    }
    if (pattern.contentRegex) {
      pattern.contentRegex.lastIndex = 0;
      const allowedRanges = [];
      if (pattern.allowContentRegex) {
        pattern.allowContentRegex.lastIndex = 0;
        for (
          let allowed = pattern.allowContentRegex.exec(contents);
          allowed;
          allowed = pattern.allowContentRegex.exec(contents)
        ) {
          allowedRanges.push([allowed.index, allowed.index + allowed[0].length]);
        }
      }
      for (let match = pattern.contentRegex.exec(contents); match; match = pattern.contentRegex.exec(contents)) {
        const lineIndex = lineForOffset(match.index);
        if (
          pattern.allowSmallApiUnitProviderHandlerState &&
          isAllowedSmallApiUnitProviderHandlerState(
            lines,
            lineIndex,
            match.index - lineStartOffsets[lineIndex],
            match[0],
          )
        ) {
          continue;
        }
        if (allowedRanges.some(([start, end]) => match.index >= start && match.index < end)) {
          continue;
        }
        violations.push({
          filePath,
          line: lineIndex + 1,
          name: pattern.name,
          text: match[0].trim().replace(/\s+/g, " "),
        });
      }
    }
  }
  return violations;
}

function repoRelative(filePath) {
  return path.relative(repoRoot, filePath).split(path.sep).join("/");
}

function apiPatternsForPath(relativePath) {
  const patterns = [...API_RAW_DAEMON_PATTERNS];
  if (rawStoreBlindApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...API_DOMAIN_RAW_STORE_PATTERNS);
  }
  if (sessionSubagentRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS);
  }
  if (sessionVcsApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SESSION_VCS_API_ORCHESTRATION_PATTERNS);
  }
  if (sessionModelSwitchApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS);
  }
  if (providerBootstrapApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS);
  }
  if (providerHarnessEndpointApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_HARNESS_ENDPOINT_API_PATTERNS);
  }
  if (providerHarnessConfigApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_HARNESS_CONFIG_API_PATTERNS);
  }
  if (providerInstallApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS);
  }
  if (providerAdminApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS);
  }
  if (providerLaunchAuthApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_LAUNCH_AUTH_API_PATTERNS);
  }
  if (providerLaunchOptionsApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_LAUNCH_OPTIONS_API_PATTERNS);
  }
  if (providerAuthImportApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS);
  }
  if (providerStatusApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS);
  }
  if (providerUsageApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS);
  }
  if (providerAccountsApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS);
  }
  if (managedBrowserLoginApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS);
  }
  if (cursorProcessLoginApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS);
  }
  if (codexAppServerLoginApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS);
  }
  if (claudeSetupTokenLoginApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS);
  }
  if (providerLoginStatusApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS);
  }
  if (relativePath === "core/crates/ctx-http/src/api/providers.rs") {
    patterns.push(...PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS);
  }
  if (executionApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...EXECUTION_API_ORCHESTRATION_PATTERNS);
  }
  if (healthDiagnosticsApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...HEALTH_DIAGNOSTICS_API_ORCHESTRATION_PATTERNS);
  }
  if (resourceUtilizationApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...RESOURCE_UTILIZATION_API_ROUTE_CONTRACT_PATTERNS);
  }
  if (settingsApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SETTINGS_API_ORCHESTRATION_PATTERNS);
  }
  if (telemetryApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...TELEMETRY_API_ORCHESTRATION_PATTERNS);
  }
  if (blobApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...BLOB_API_ORCHESTRATION_PATTERNS);
  }
  if (logsApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...LOGS_API_ORCHESTRATION_PATTERNS);
  }
  if (updateApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...UPDATE_API_ORCHESTRATION_PATTERNS);
  }
  if (updateDrainApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...UPDATE_DRAIN_API_ORCHESTRATION_PATTERNS);
  }
  if (routeFileDownloadApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...ROUTE_FILE_DOWNLOAD_API_PATTERNS);
  }
  if (sessionArtifactApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS);
  }
  if (mergeQueueSubmitApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS);
  }
  if (mergeQueueEntryApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS);
  }
  if (terminalRestRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS);
  }
  if (runArchiveApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...RUN_ARCHIVE_API_ORCHESTRATION_PATTERNS);
  }
  if (webSessionRestRouteApiRoots.some((root) => relativePath === root)) {
    patterns.push(...WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS);
  }
  if (sessionHeadApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SESSION_HEAD_API_ORCHESTRATION_PATTERNS);
  }
  if (sessionReadModelRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS);
  }
  if (demoSeedTranscriptApiRoots.some((root) => relativePath === root)) {
    patterns.push(...DEMO_SEED_TRANSCRIPT_API_ROUTE_CONTRACT_PATTERNS);
  }
  if (sessionControlRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS);
  }
  if (sessionMessageCommandRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS);
  }
  if (workspaceRegistrationConfigApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_REGISTRATION_CONFIG_API_PATTERNS);
  }
  if (relativePath === "core/crates/ctx-http/src/api/workspaces/management.rs") {
    patterns.push(...WORKSPACE_CONFIG_ROUTE_CONTEXT_PATTERNS);
  }
  if (workspaceExecutionConfigApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_EXECUTION_CONFIG_API_PATTERNS);
  }
  if (workspaceManagementConfigApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS);
  }
  if (
    workspaceRouteContractApiRoots.some((root) =>
      root.endsWith("/") ? relativePath.startsWith(root) : relativePath === root
    )
  ) {
    patterns.push(...WORKSPACE_ROUTE_CONTRACT_API_PATTERNS);
  }
  if (taskSessionCreationApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...TASK_SESSION_CREATION_API_ADMISSION_PATTERNS);
  }
  if (taskRouteContractApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...TASK_ROUTE_API_CONTRACT_PATTERNS);
  }
  if (workspaceStreamReadModelApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_STREAM_READ_MODEL_API_PATTERNS);
  }
  if (workspaceStreamSubscriptionPlanApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_STREAM_SUBSCRIPTION_PLAN_API_PATTERNS);
  }
  if (workspaceStreamSubscriptionTransactionApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS);
  }
  if (workspaceStreamReplayCursorApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS);
  }
  if (workspaceStreamReplayProgramApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS);
  }
  if (workspaceStreamEventRoutingApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS);
  }
  if (workspaceStreamEventRoutePlanApiRoots.some((root) => relativePath === root)) {
    patterns.push(...WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS);
  }
  if (workspaceStreamLiveEventApplicationApiRoots.some((root) => relativePath === root)) {
    patterns.push(...WORKSPACE_STREAM_LIVE_EVENT_APPLICATION_API_PATTERNS);
  }
  if (workspaceStreamSubscriptionEventApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...WORKSPACE_STREAM_SUBSCRIPTION_EVENT_API_PATTERNS);
  }
  if (workspaceVcsDemandApiRoots.some((root) => relativePath === root)) {
    patterns.push(...WORKSPACE_VCS_DEMAND_API_PATTERNS);
  }
  if (workspaceVcsLiveRoutingApiRoots.some((root) => relativePath === root)) {
    patterns.push(...WORKSPACE_VCS_LIVE_ROUTING_API_PATTERNS);
  }
  if (terminalStreamRuntimeApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...TERMINAL_STREAM_RUNTIME_API_PATTERNS);
  }
  if (dictationWsConfigApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...DICTATION_WS_CONFIG_API_PATTERNS);
  }
  if (workspaceWsAdmissionApiRoots.some((root) => relativePath === root)) {
    patterns.push(...WORKSPACE_WS_ADMISSION_API_PATTERNS);
  }
  if (orgPolicyApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...ORG_POLICY_API_ORCHESTRATION_PATTERNS);
  }
  if (repoOnboardingApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...REPO_ONBOARDING_API_ORCHESTRATION_PATTERNS);
  }
  if (mobileProfileRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...MOBILE_PROFILE_ROUTE_PARAM_API_PATTERNS);
  }
  return patterns;
}

function migratedTestPatternsForPath(relativePath) {
  if (migratedRawDaemonTestRoots.some((root) => relativePath.startsWith(root))) {
    return MIGRATED_TEST_RAW_DAEMON_PATTERNS;
  }
  return [];
}

function mobileStorePatternsForPath(relativePath) {
  if (mobileStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return MOBILE_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function providerCachePatternsForPath(relativePath) {
  if (providerCacheFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_TEST_CACHE_ACCESS_PATTERNS;
  }
  return [];
}

function providerRouteSetupStorePatternsForPath(relativePath) {
  if (providerRouteSetupStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_ROUTE_SETUP_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function authBoundaryStorePatternsForPath(relativePath) {
  if (authBoundaryStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return AUTH_BOUNDARY_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function externalProviderRouteStorePatternsForPath(relativePath) {
  if (externalProviderRouteStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return EXTERNAL_PROVIDER_ROUTE_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function geminiLiveModelCatalogStorePatternsForPath(relativePath) {
  if (geminiLiveModelCatalogStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return GEMINI_LIVE_MODEL_CATALOG_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function mobileAccessStoreDtoApiPatternsForPath(relativePath) {
  if (mobileAccessStoreDtoApiRoots.some((root) => relativePath.startsWith(root))) {
    return [
      ...MOBILE_ACCESS_STORE_DTO_API_PATTERNS,
      ...MOBILE_ACCESS_ORCHESTRATION_API_PATTERNS,
    ];
  }
  return [];
}

function mobileProfileRouteApiPatternsForPath(relativePath) {
  if (mobileProfileRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    return MOBILE_PROFILE_ROUTE_PARAM_API_PATTERNS;
  }
  return [];
}

function routeFileDownloadApiPatternsForPath(relativePath) {
  if (routeFileDownloadApiRoots.some((root) => relativePath.startsWith(root))) {
    return ROUTE_FILE_DOWNLOAD_API_PATTERNS;
  }
  return [];
}

function sessionArtifactApiPatternsForPath(relativePath) {
  if (sessionArtifactApiRoots.some((root) => relativePath.startsWith(root))) {
    return SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS;
  }
  return [];
}

function mergeQueueSubmitApiPatternsForPath(relativePath) {
  if (mergeQueueSubmitApiRoots.some((root) => relativePath.startsWith(root))) {
    return MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function mergeQueueEntryApiPatternsForPath(relativePath) {
  if (mergeQueueEntryApiRoots.some((root) => relativePath.startsWith(root))) {
    return MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS;
  }
  return [];
}

function terminalRestRouteApiPatternsForPath(relativePath) {
  if (terminalRestRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    return TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS;
  }
  return [];
}

function taskRouteApiPatternsForPath(relativePath) {
  if (taskRouteContractApiRoots.some((root) => relativePath.startsWith(root))) {
    return TASK_ROUTE_API_CONTRACT_PATTERNS;
  }
  return [];
}

function runArchiveApiPatternsForPath(relativePath) {
  if (runArchiveApiRoots.some((root) => relativePath.startsWith(root))) {
    return RUN_ARCHIVE_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function webSessionRestRouteApiPatternsForPath(relativePath) {
  if (webSessionRestRouteApiRoots.some((root) => relativePath === root)) {
    return WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS;
  }
  return [];
}

function providerHarnessEndpointApiPatternsForPath(relativePath) {
  if (providerHarnessEndpointApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_HARNESS_ENDPOINT_API_PATTERNS;
  }
  return [];
}

function providerHarnessConfigApiPatternsForPath(relativePath) {
  if (providerHarnessConfigApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_HARNESS_CONFIG_API_PATTERNS;
  }
  return [];
}

function providerInstallApiPatternsForPath(relativePath) {
  if (providerInstallApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function providerAdminApiPatternsForPath(relativePath) {
  if (providerAdminApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function providerLaunchAuthApiPatternsForPath(relativePath) {
  if (providerLaunchAuthApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_LAUNCH_AUTH_API_PATTERNS;
  }
  return [];
}

function providerLaunchOptionsApiPatternsForPath(relativePath) {
  if (providerLaunchOptionsApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_LAUNCH_OPTIONS_API_PATTERNS;
  }
  return [];
}

function providerAuthImportApiPatternsForPath(relativePath) {
  if (providerAuthImportApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function providerStatusApiPatternsForPath(relativePath) {
  if (providerStatusApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function providerUsageApiPatternsForPath(relativePath) {
  if (providerUsageApiRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function sessionHeadApiPatternsForPath(relativePath) {
  if (sessionHeadApiRoots.some((root) => relativePath.startsWith(root))) {
    return SESSION_HEAD_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function sessionReadModelRouteApiPatternsForPath(relativePath) {
  if (sessionReadModelRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    return SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS;
  }
  return [];
}

function demoSeedTranscriptApiPatternsForPath(relativePath) {
  if (demoSeedTranscriptApiRoots.some((root) => relativePath === root)) {
    return DEMO_SEED_TRANSCRIPT_API_ROUTE_CONTRACT_PATTERNS;
  }
  return [];
}

function sessionControlRouteApiPatternsForPath(relativePath) {
  if (sessionControlRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    return SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS;
  }
  return [];
}

function sessionMessageCommandRouteApiPatternsForPath(relativePath) {
  if (sessionMessageCommandRouteApiRoots.some((root) => relativePath.startsWith(root))) {
    return SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS;
  }
  return [];
}

function updateDrainApiPatternsForPath(relativePath) {
  if (updateDrainApiRoots.some((root) => relativePath.startsWith(root))) {
    return UPDATE_DRAIN_API_ORCHESTRATION_PATTERNS;
  }
  return [];
}

function smallApiUnitStorePatternsForPath(relativePath) {
  if (smallApiUnitStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return SMALL_API_UNIT_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function smallExternalStorePatternsForPath(relativePath) {
  if (smallExternalStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return SMALL_EXTERNAL_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function fakeDaemonExternalStorePatternsForPath(relativePath) {
  if (fakeDaemonExternalStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return FAKE_DAEMON_EXTERNAL_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function acpCrpBridgeTokenStorePatternsForPath(relativePath) {
  if (acpCrpBridgeTokenStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return ACP_CRP_BRIDGE_TOKEN_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function cacheRehydrationStorePatternsForPath(relativePath) {
  if (cacheRehydrationStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return CACHE_REHYDRATION_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function subscriptionAccountsApiStorePatternsForPath(relativePath) {
  if (subscriptionAccountsApiStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return SUBSCRIPTION_ACCOUNTS_API_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function sessionModelApiStorePatternsForPath(relativePath) {
  if (sessionModelApiStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return SESSION_MODEL_API_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function imageAttachmentsStorePatternsForPath(relativePath) {
  if (imageAttachmentsStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return IMAGE_ATTACHMENTS_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function workspaceAttachmentsDemoStorePatternsForPath(relativePath) {
  if (workspaceAttachmentsDemoStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return WORKSPACE_ATTACHMENTS_DEMO_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function providerTargetScopedInstallsStorePatternsForPath(relativePath) {
  if (
    providerTargetScopedInstallsStoreFacadeTestRoots.some((root) =>
      relativePath.startsWith(root),
    )
  ) {
    return PROVIDER_TARGET_SCOPED_INSTALLS_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function subagentMcpStorePatternsForPath(relativePath) {
  if (subagentMcpStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return SUBAGENT_MCP_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function replayPropertiesStorePatternsForPath(relativePath) {
  if (replayPropertiesStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return REPLAY_PROPERTIES_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function worktreeVcsSnapshotStorePatternsForPath(relativePath) {
  if (worktreeVcsSnapshotStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return WORKTREE_VCS_SNAPSHOT_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function providerProbeRuntimeEnvStorePatternsForPath(relativePath) {
  if (providerProbeRuntimeEnvStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_PROBE_RUNTIME_ENV_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function defaultSessionAndDiffFakeDaemonFixturePatternsForPath(relativePath) {
  if (defaultSessionAndDiffFakeDaemonFixtureTestRoots.some((root) => relativePath.startsWith(root))) {
    return DEFAULT_SESSION_AND_DIFF_FAKE_DAEMON_FIXTURE_PATTERNS;
  }
  return [];
}

function workspaceVcsSetupFixturePatternsForPath(relativePath) {
  if (workspaceVcsSetupFixtureTestRoots.some((root) => relativePath.startsWith(root))) {
    return WORKSPACE_VCS_SETUP_FIXTURE_PATTERNS;
  }
  return [];
}

function smallRouteFixturePatternsForPath(relativePath) {
  if (smallRouteFixtureTestRoots.some((root) => relativePath.startsWith(root))) {
    return SMALL_ROUTE_FIXTURE_PATTERNS;
  }
  return [];
}

function providerAuthGlobalIdFixturePatternsForPath(relativePath) {
  if (providerAuthGlobalIdFixtureTestRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_AUTH_GLOBAL_ID_FIXTURE_PATTERNS;
  }
  return [];
}

function updateRouteFixturePatternsForPath(relativePath) {
  if (updateRouteFixtureTestRoots.some((root) => relativePath.startsWith(root))) {
    return UPDATE_ROUTE_FIXTURE_PATTERNS;
  }
  return [];
}

function mcpDaemonPatternsForPath(relativePath) {
  if (mcpDaemonFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function sessionFixtureStorePatternsForPath(relativePath) {
  if (sessionFixtureStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function smallBoundaryStorePatternsForPath(relativePath) {
  const patterns = [];
  if (smallBoundaryStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...SMALL_BOUNDARY_TEST_STORE_ACCESS_PATTERNS);
  }
  if (providerlessLibRouteStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...PROVIDERLESS_LIB_ROUTE_TEST_STORE_ACCESS_PATTERNS);
  }
  return patterns;
}

function executionLaunchStorePatternsForPath(relativePath) {
  if (executionLaunchStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return EXECUTION_LAUNCH_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function globalIdRoutingStorePatternsForPath(relativePath) {
  if (globalIdRoutingStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return GLOBAL_ID_ROUTING_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function terminalWorkspaceStreamStorePatternsForPath(relativePath) {
  if (terminalWorkspaceStreamStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return TERMINAL_WORKSPACE_STREAM_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function workspaceRuntimeSettingsStorePatternsForPath(relativePath) {
  if (workspaceRuntimeSettingsStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return WORKSPACE_RUNTIME_SETTINGS_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function workspaceMergeQueueConfigStorePatternsForPath(relativePath) {
  if (workspaceMergeQueueConfigStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return WORKSPACE_MERGE_QUEUE_CONFIG_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function jjMergeQueueBasicsStorePatternsForPath(relativePath) {
  if (jjMergeQueueBasicsStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return JJ_MERGE_QUEUE_BASICS_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function mergeQueueIsolationStorePatternsForPath(relativePath) {
  if (mergeQueueIsolationStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return MERGE_QUEUE_ISOLATION_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function providerWorkerReapingStorePatternsForPath(relativePath) {
  if (providerWorkerReapingStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_WORKER_REAPING_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function providerScenariosOfflineStorePatternsForPath(relativePath) {
  if (providerScenariosOfflineStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_SCENARIOS_OFFLINE_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function harnessContainerSandboxStorePatternsForPath(relativePath) {
  if (harnessContainerSandboxStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return HARNESS_CONTAINER_SANDBOX_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function liveProviderCanaryStorePatternsForPath(relativePath) {
  if (liveProviderCanaryStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return LIVE_PROVIDER_CANARY_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function worktreeArchiveStorePatternsForPath(relativePath) {
  if (worktreeArchiveStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return WORKTREE_ARCHIVE_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function faultInjectionStorePatternsForPath(relativePath) {
  if (faultInjectionStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return FAULT_INJECTION_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function streamRuntimeStorePatternsForPath(relativePath) {
  if (streamRuntimeStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return STREAM_RUNTIME_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function schedulerRuntimeStorePatternsForPath(relativePath) {
  if (schedulerRuntimeStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return SCHEDULER_RUNTIME_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function taskLifecycleStorePatternsForPath(relativePath) {
  if (taskLifecycleStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return TASK_LIFECYCLE_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function storageAdmissionFixturePatternsForPath(relativePath) {
  if (storageAdmissionFixtureTestRoots.some((root) => relativePath.startsWith(root))) {
    return STORAGE_ADMISSION_FIXTURE_PATTERNS;
  }
  return [];
}

function libTestDataRootFixturePatternsForPath(relativePath) {
  if (libTestDataRootFixtureRoots.some((root) => relativePath.startsWith(root))) {
    return LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS;
  }
  return [];
}

function routerCompositionPatternsForPath(relativePath) {
  const isLibTestsRoot = relativePath === "core/crates/ctx-http/src/lib_tests.rs";
  if (
    !relativePath.startsWith("core/crates/ctx-http/tests/")
    && !isLibTestsRoot
    && !relativePath.startsWith("core/crates/ctx-http/src/lib_tests/")
    && !relativePath.startsWith("core/crates/ctx-http/src/api/")
    && !relativePath.startsWith("core/crates/ctx-http/src/test_support")
    && !relativePath.startsWith("core/crates/ctx-http-test-support/src/")
  ) {
    return [];
  }
  if (
    relativePath === "core/crates/ctx-http/src/api/router.rs"
  ) {
    return [];
  }
  return TEST_ROUTER_COMPOSITION_PATTERNS;
}

function countRustBlockDelta(line) {
  let delta = 0;
  for (const char of line) {
    if (char === "{") {
      delta += 1;
    } else if (char === "}") {
      delta -= 1;
    }
  }
  return delta;
}

function functionSpanForDeclaration(lines, declarationIndex) {
  let sawBody = false;
  let depth = 0;
  for (let index = declarationIndex; index < lines.length; index += 1) {
    const line = lines[index];
    if (line.includes("{")) {
      sawBody = true;
    }
    depth += countRustBlockDelta(line);
    if (sawBody && depth <= 0) {
      return { start: declarationIndex, end: index };
    }
  }
  return null;
}

function isInsideDeclaredFunction(lines, index, declarationRegex) {
  for (let declarationIndex = index; declarationIndex >= 0; declarationIndex -= 1) {
    if (!declarationRegex.test(lines[declarationIndex])) {
      continue;
    }
    const span = functionSpanForDeclaration(lines, declarationIndex);
    return span !== null && index >= span.start && index <= span.end;
  }
  return false;
}

function isAllowedRouterHelperComposition({ filePath, lines, index, line }) {
  if (
    filePath === "core/crates/ctx-http/tests/common/mod.rs"
    && /api::router\s*\(\s*api::RouteHandles::from_daemon_handle\s*\(\s*daemon\.handle\s*\(\s*\)\s*\)\s*\)/.test(line)
    && isInsideDeclaredFunction(lines, index, /\bpub\s+fn\s+router_for_daemon\s*\(/)
  ) {
    return true;
  }
  if (
    filePath === "core/crates/ctx-http/src/api/workspaces/tests.rs"
    && /crate::api::router\s*\(\s*crate::api::RouteHandles::from_daemon_handle\s*\(\s*daemon\.handle\s*\(\s*\)\s*,?\s*\)\s*\)/.test(lines.slice(index, index + 4).join(" "))
    && isInsideDeclaredFunction(lines, index, /\bfn\s+test_router\s*\(/)
  ) {
    return true;
  }
  if (
    filePath === "core/crates/ctx-http/src/test_support.rs"
    && /crate::api::router\s*\(\s*crate::api::RouteHandles::from_daemon_handle\s*\(\s*self\.daemon\.handle\s*\(\s*\)\s*,?\s*\)\s*\)/.test(lines.slice(index, index + 4).join(" "))
    && isInsideDeclaredFunction(lines, index, /\bpub\s*\(\s*crate\s*\)\s+fn\s+router\s*\(/)
  ) {
    return true;
  }
  if (
    filePath === "core/crates/ctx-http-test-support/src/mcp_daemon/router.rs"
    && /ctx_http::api::router\s*\(\s*ctx_http::api::RouteHandles::from_daemon_handle\s*\(\s*daemon\.handle\s*\(\s*\)\s*,?\s*\)\s*\)/.test(lines.slice(index, index + 4).join(" "))
    && isInsideDeclaredFunction(lines, index, /\bpub\s*\(\s*crate\s*\)\s+fn\s+spawn_router_for_daemon\s*\(/)
  ) {
    return true;
  }
  return false;
}

function scanRouterComposition({ filePath, contents, patterns }) {
  const violations = [];
  const lines = contents.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    for (const pattern of patterns) {
      if (!pattern.regex.test(line)) {
        continue;
      }
      if (isAllowedRouterHelperComposition({ filePath, lines, index, line })) {
        continue;
      }
      violations.push({
        filePath,
        line: index + 1,
        name: pattern.name,
        text: line.trim(),
      });
    }
  }
  return violations;
}

function scanRepo() {
  const violations = [];
  if (fs.existsSync(legacyHttpDaemonRootPath)) {
    violations.push({
      filePath: repoRelative(legacyHttpDaemonRootPath),
      line: 1,
      name: "legacy ctx-http daemon root",
      text: "ctx-http/src/daemon.rs must stay physically extracted into ctx-daemon",
    });
  }
  if (fs.existsSync(legacyHttpDaemonRoot)) {
    violations.push({
      filePath: repoRelative(legacyHttpDaemonRoot),
      line: 1,
      name: "legacy ctx-http daemon directory",
      text: "ctx-http/src/daemon must stay physically extracted into ctx-daemon",
    });
  }

  for (const filePath of listRustFiles(apiRoot)) {
    if (isTestRustPath(filePath)) {
      continue;
    }
    const relativePath = repoRelative(filePath);
    const contents = stripCfgTestItems(fs.readFileSync(filePath, "utf8"));
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: apiPatternsForPath(relativePath),
      }),
    );
  }

  if (fs.existsSync(daemonHandlePath)) {
    violations.push(
      ...scanText({
        filePath: repoRelative(daemonHandlePath),
        contents: fs.readFileSync(daemonHandlePath, "utf8"),
        patterns: HANDLE_BACKDOOR_PATTERNS,
      }),
    );
  }

  const daemonFiles = [];
  if (fs.existsSync(daemonRootPath)) {
    daemonFiles.push(daemonRootPath);
  }
  if (fs.existsSync(daemonRoot)) {
    daemonFiles.push(...listRustFiles(daemonRoot));
  }
  for (const filePath of daemonFiles) {
    if (isTestRustPath(filePath)) {
      continue;
    }
    const relativePath = repoRelative(filePath);
    const contents = stripCfgTestItems(fs.readFileSync(filePath, "utf8"));
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: DAEMON_EXTRACTION_BLOCKER_PATTERNS,
      }),
    );
    if (relativePath === "core/crates/ctx-daemon/src/daemon/health.rs") {
      violations.push(
        ...scanText({
          filePath: relativePath,
          contents,
          patterns: DAEMON_HEALTH_VERSION_PATTERNS,
        }),
      );
    }
    if (relativePath === "core/crates/ctx-daemon/src/daemon/updates.rs") {
      violations.push(
        ...scanText({
          filePath: relativePath,
          contents,
          patterns: DAEMON_UPDATES_VERSION_PATTERNS,
        }),
      );
    }
  }

  for (const filePath of testSurfaceRustFiles()) {
    const relativePath = repoRelative(filePath);
    const contents = fs.readFileSync(filePath, "utf8");
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: TEST_RAW_DAEMON_BUCKET_PATTERNS,
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: migratedTestPatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: mobileStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: providerCachePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: providerRouteSetupStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: authBoundaryStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: externalProviderRouteStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: geminiLiveModelCatalogStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: mobileAccessStoreDtoApiPatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: smallApiUnitStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: smallExternalStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: fakeDaemonExternalStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: acpCrpBridgeTokenStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: cacheRehydrationStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: subscriptionAccountsApiStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: sessionModelApiStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: imageAttachmentsStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: workspaceAttachmentsDemoStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: providerTargetScopedInstallsStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: subagentMcpStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: replayPropertiesStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: worktreeVcsSnapshotStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: providerProbeRuntimeEnvStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: defaultSessionAndDiffFakeDaemonFixturePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: workspaceVcsSetupFixturePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: smallRouteFixturePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: providerAuthGlobalIdFixturePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: updateRouteFixturePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: mcpDaemonPatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: sessionFixtureStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: smallBoundaryStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: executionLaunchStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: globalIdRoutingStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: terminalWorkspaceStreamStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: workspaceRuntimeSettingsStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: workspaceMergeQueueConfigStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: jjMergeQueueBasicsStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: mergeQueueIsolationStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: providerWorkerReapingStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: providerScenariosOfflineStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: harnessContainerSandboxStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: liveProviderCanaryStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: worktreeArchiveStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: faultInjectionStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: streamRuntimeStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: schedulerRuntimeStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: taskLifecycleStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: storageAdmissionFixturePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: libTestDataRootFixturePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanRouterComposition({
        filePath: relativePath,
        contents,
        patterns: routerCompositionPatternsForPath(relativePath),
      }),
    );
  }

  return violations;
}

function main() {
  const violations = scanRepo();
  if (violations.length === 0) {
    console.log("ctx-http daemon boundary guard: OK");
    return;
  }

  for (const violation of violations) {
    console.error(
      `${violation.filePath}:${violation.line}: ${violation.name}: ${violation.text}`,
    );
  }
  process.exitCode = 1;
}

if (require.main === module) {
  main();
}

module.exports = {
  ACP_CRP_BRIDGE_TOKEN_TEST_STORE_ACCESS_PATTERNS,
  AUTH_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
  CACHE_REHYDRATION_TEST_STORE_ACCESS_PATTERNS,
  DAEMON_EXTRACTION_BLOCKER_PATTERNS,
  API_RAW_DAEMON_PATTERNS,
  API_DOMAIN_RAW_STORE_PATTERNS,
  DEFAULT_SESSION_AND_DIFF_FAKE_DAEMON_FIXTURE_PATTERNS,
  EXECUTION_LAUNCH_TEST_STORE_ACCESS_PATTERNS,
  EXTERNAL_PROVIDER_ROUTE_TEST_STORE_ACCESS_PATTERNS,
  FAKE_DAEMON_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
  FAULT_INJECTION_TEST_STORE_ACCESS_PATTERNS,
  GEMINI_LIVE_MODEL_CATALOG_TEST_STORE_ACCESS_PATTERNS,
  GLOBAL_ID_ROUTING_TEST_STORE_ACCESS_PATTERNS,
  HARNESS_CONTAINER_SANDBOX_TEST_STORE_ACCESS_PATTERNS,
  HANDLE_BACKDOOR_PATTERNS,
  DAEMON_HEALTH_VERSION_PATTERNS,
  DAEMON_UPDATES_VERSION_PATTERNS,
  BLOB_API_ORCHESTRATION_PATTERNS,
  HEALTH_DIAGNOSTICS_API_ORCHESTRATION_PATTERNS,
  RESOURCE_UTILIZATION_API_ROUTE_CONTRACT_PATTERNS,
  SETTINGS_API_ORCHESTRATION_PATTERNS,
  TELEMETRY_API_ORCHESTRATION_PATTERNS,
  LOGS_API_ORCHESTRATION_PATTERNS,
  UPDATE_API_ORCHESTRATION_PATTERNS,
  UPDATE_DRAIN_API_ORCHESTRATION_PATTERNS,
  ROUTE_FILE_DOWNLOAD_API_PATTERNS,
  SESSION_ARTIFACT_API_ROUTE_CONTRACT_PATTERNS,
  RUN_ARCHIVE_API_ORCHESTRATION_PATTERNS,
  WEB_SESSION_REST_ROUTE_API_CONTRACT_PATTERNS,
  SESSION_HEAD_API_ORCHESTRATION_PATTERNS,
  DEMO_SEED_TRANSCRIPT_API_ROUTE_CONTRACT_PATTERNS,
  SESSION_CONTROL_ROUTE_API_CONTRACT_PATTERNS,
  SESSION_MESSAGE_COMMAND_ROUTE_API_CONTRACT_PATTERNS,
  SESSION_READ_MODEL_ROUTE_API_CONTRACT_PATTERNS,
  WORKSPACE_CONFIG_ROUTE_CONTEXT_PATTERNS,
  WORKSPACE_EXECUTION_CONFIG_API_PATTERNS,
  WORKSPACE_MANAGEMENT_CONFIG_API_PATTERNS,
  WORKSPACE_ROUTE_CONTRACT_API_PATTERNS,
  WORKSPACE_REGISTRATION_CONFIG_API_PATTERNS,
  IMAGE_ATTACHMENTS_TEST_STORE_ACCESS_PATTERNS,
  JJ_MERGE_QUEUE_BASICS_TEST_STORE_ACCESS_PATTERNS,
  LIB_TEST_DATA_ROOT_FIXTURE_PATTERNS,
  LIVE_PROVIDER_CANARY_TEST_STORE_ACCESS_PATTERNS,
  MERGE_QUEUE_ISOLATION_TEST_STORE_ACCESS_PATTERNS,
  MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS,
  MOBILE_ACCESS_STORE_DTO_API_PATTERNS,
  MOBILE_ACCESS_ORCHESTRATION_API_PATTERNS,
  MOBILE_PROFILE_ROUTE_PARAM_API_PATTERNS,
  MOBILE_TEST_STORE_ACCESS_PATTERNS,
  MERGE_QUEUE_ENTRY_API_ROUTE_CONTRACT_PATTERNS,
  MERGE_QUEUE_SUBMIT_API_ORCHESTRATION_PATTERNS,
  TERMINAL_REST_ROUTE_API_CONTRACT_PATTERNS,
  TASK_ROUTE_API_CONTRACT_PATTERNS,
  MANAGED_BROWSER_LOGIN_API_ORCHESTRATION_PATTERNS,
  CURSOR_PROCESS_LOGIN_API_ORCHESTRATION_PATTERNS,
  CODEX_APP_SERVER_LOGIN_API_ORCHESTRATION_PATTERNS,
  CLAUDE_SETUP_TOKEN_LOGIN_API_ORCHESTRATION_PATTERNS,
  PROVIDER_LOGIN_STATUS_ROUTE_DTO_PATTERNS,
  PROVIDER_PRELUDE_ROUTE_DTO_PATTERNS,
  PROVIDER_BOOTSTRAP_API_ORCHESTRATION_PATTERNS,
  PROVIDER_HARNESS_CONFIG_API_PATTERNS,
  PROVIDER_HARNESS_ENDPOINT_API_PATTERNS,
  PROVIDER_INSTALL_API_ORCHESTRATION_PATTERNS,
  PROVIDER_ADMIN_API_ORCHESTRATION_PATTERNS,
  PROVIDER_LAUNCH_AUTH_API_PATTERNS,
  PROVIDER_LAUNCH_OPTIONS_API_PATTERNS,
  PROVIDER_AUTH_IMPORT_API_ORCHESTRATION_PATTERNS,
  PROVIDER_STATUS_API_ORCHESTRATION_PATTERNS,
  PROVIDER_USAGE_API_ORCHESTRATION_PATTERNS,
  PROVIDER_ACCOUNT_API_ORCHESTRATION_PATTERNS,
  SESSION_MODEL_SWITCH_API_ORCHESTRATION_PATTERNS,
  SESSION_SUBAGENT_ROUTE_API_CONTRACT_PATTERNS,
  SESSION_VCS_API_ORCHESTRATION_PATTERNS,
  TASK_SESSION_CREATION_API_ADMISSION_PATTERNS,
  WORKSPACE_STREAM_READ_MODEL_API_PATTERNS,
  WORKSPACE_STREAM_SUBSCRIPTION_PLAN_API_PATTERNS,
  WORKSPACE_STREAM_SUBSCRIPTION_TRANSACTION_API_PATTERNS,
  WORKSPACE_STREAM_REPLAY_CURSOR_API_PATTERNS,
  WORKSPACE_STREAM_REPLAY_PROGRAM_API_PATTERNS,
  WORKSPACE_STREAM_EVENT_ROUTING_API_PATTERNS,
  WORKSPACE_STREAM_EVENT_ROUTE_PLAN_API_PATTERNS,
  WORKSPACE_STREAM_LIVE_EVENT_APPLICATION_API_PATTERNS,
  WORKSPACE_STREAM_SUBSCRIPTION_EVENT_API_PATTERNS,
  WORKSPACE_VCS_DEMAND_API_PATTERNS,
  WORKSPACE_VCS_LIVE_ROUTING_API_PATTERNS,
  TERMINAL_STREAM_RUNTIME_API_PATTERNS,
  DICTATION_WS_CONFIG_API_PATTERNS,
  WORKSPACE_WS_ADMISSION_API_PATTERNS,
  ORG_POLICY_API_ORCHESTRATION_PATTERNS,
  REPO_ONBOARDING_API_ORCHESTRATION_PATTERNS,
  PROVIDER_AUTH_GLOBAL_ID_FIXTURE_PATTERNS,
  PROVIDERLESS_LIB_ROUTE_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_PROBE_RUNTIME_ENV_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_ROUTE_SETUP_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_SCENARIOS_OFFLINE_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_TARGET_SCOPED_INSTALLS_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_WORKER_REAPING_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_TEST_CACHE_ACCESS_PATTERNS,
  REPLAY_PROPERTIES_TEST_STORE_ACCESS_PATTERNS,
  SCHEDULER_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
  SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS,
  SESSION_MODEL_API_TEST_STORE_ACCESS_PATTERNS,
  SMALL_API_UNIT_TEST_STORE_ACCESS_PATTERNS,
  SMALL_EXTERNAL_TEST_STORE_ACCESS_PATTERNS,
  SMALL_BOUNDARY_TEST_STORE_ACCESS_PATTERNS,
  SMALL_ROUTE_FIXTURE_PATTERNS,
  STORAGE_ADMISSION_FIXTURE_PATTERNS,
  STREAM_RUNTIME_TEST_STORE_ACCESS_PATTERNS,
  SUBAGENT_MCP_TEST_STORE_ACCESS_PATTERNS,
  SUBSCRIPTION_ACCOUNTS_API_TEST_STORE_ACCESS_PATTERNS,
  TASK_LIFECYCLE_TEST_STORE_ACCESS_PATTERNS,
  TERMINAL_WORKSPACE_STREAM_TEST_STORE_ACCESS_PATTERNS,
  TEST_ROUTER_COMPOSITION_PATTERNS,
  TEST_RAW_DAEMON_BUCKET_PATTERNS,
  UPDATE_ROUTE_FIXTURE_PATTERNS,
  WORKSPACE_MERGE_QUEUE_CONFIG_TEST_STORE_ACCESS_PATTERNS,
  WORKSPACE_RUNTIME_SETTINGS_TEST_STORE_ACCESS_PATTERNS,
  WORKSPACE_ATTACHMENTS_DEMO_TEST_STORE_ACCESS_PATTERNS,
  WORKSPACE_VCS_SETUP_FIXTURE_PATTERNS,
  WORKTREE_ARCHIVE_TEST_STORE_ACCESS_PATTERNS,
  WORKTREE_VCS_SNAPSHOT_TEST_STORE_ACCESS_PATTERNS,
  apiPatternsForPath,
  acpCrpBridgeTokenStorePatternsForPath,
  authBoundaryStorePatternsForPath,
  cacheRehydrationStorePatternsForPath,
  defaultSessionAndDiffFakeDaemonFixturePatternsForPath,
  externalProviderRouteStorePatternsForPath,
  executionLaunchStorePatternsForPath,
  fakeDaemonExternalStorePatternsForPath,
  faultInjectionStorePatternsForPath,
  geminiLiveModelCatalogStorePatternsForPath,
  globalIdRoutingStorePatternsForPath,
  harnessContainerSandboxStorePatternsForPath,
  imageAttachmentsStorePatternsForPath,
  isTestRustPath,
  jjMergeQueueBasicsStorePatternsForPath,
  liveProviderCanaryStorePatternsForPath,
  libTestDataRootFixturePatternsForPath,
  mergeQueueIsolationStorePatternsForPath,
  mergeQueueEntryApiPatternsForPath,
  mergeQueueSubmitApiPatternsForPath,
  terminalRestRouteApiPatternsForPath,
  webSessionRestRouteApiPatternsForPath,
  taskRouteApiPatternsForPath,
  mcpDaemonPatternsForPath,
  migratedTestPatternsForPath,
  mobileAccessStoreDtoApiPatternsForPath,
  mobileProfileRouteApiPatternsForPath,
  mobileStorePatternsForPath,
  providerAuthImportApiPatternsForPath,
  providerAuthGlobalIdFixturePatternsForPath,
  providerHarnessConfigApiPatternsForPath,
  providerHarnessEndpointApiPatternsForPath,
  providerInstallApiPatternsForPath,
  providerAdminApiPatternsForPath,
  providerLaunchAuthApiPatternsForPath,
  providerLaunchOptionsApiPatternsForPath,
  providerStatusApiPatternsForPath,
  providerUsageApiPatternsForPath,
  providerScenariosOfflineStorePatternsForPath,
  providerWorkerReapingStorePatternsForPath,
  providerCachePatternsForPath,
  providerProbeRuntimeEnvStorePatternsForPath,
  providerRouteSetupStorePatternsForPath,
  providerTargetScopedInstallsStorePatternsForPath,
  replayPropertiesStorePatternsForPath,
  routeFileDownloadApiPatternsForPath,
  sessionArtifactApiPatternsForPath,
  runArchiveApiPatternsForPath,
  sessionHeadApiPatternsForPath,
  sessionControlRouteApiPatternsForPath,
  sessionMessageCommandRouteApiPatternsForPath,
  sessionReadModelRouteApiPatternsForPath,
  demoSeedTranscriptApiPatternsForPath,
  routerCompositionPatternsForPath,
  scanRepo,
  scanRouterComposition,
  scanText,
  schedulerRuntimeStorePatternsForPath,
  sessionModelApiStorePatternsForPath,
  sessionFixtureStorePatternsForPath,
  smallApiUnitStorePatternsForPath,
  smallExternalStorePatternsForPath,
  smallBoundaryStorePatternsForPath,
  smallRouteFixturePatternsForPath,
  storageAdmissionFixturePatternsForPath,
  streamRuntimeStorePatternsForPath,
  subagentMcpStorePatternsForPath,
  subscriptionAccountsApiStorePatternsForPath,
  taskLifecycleStorePatternsForPath,
  terminalWorkspaceStreamStorePatternsForPath,
  updateDrainApiPatternsForPath,
  updateRouteFixturePatternsForPath,
  worktreeArchiveStorePatternsForPath,
  workspaceMergeQueueConfigStorePatternsForPath,
  workspaceAttachmentsDemoStorePatternsForPath,
  workspaceRuntimeSettingsStorePatternsForPath,
  workspaceVcsSetupFixturePatternsForPath,
  worktreeVcsSnapshotStorePatternsForPath,
  stripCfgTestItems,
};
