use std::sync::Arc;

use axum::routing::{delete, get, post, put};

use super::*;
use crate::daemon::AppState;

pub(super) fn api_routes() -> axum::Router<Arc<AppState>> {
    core_routes()
        .merge(provider_routes())
        .merge(lsp_routes())
        .merge(workspace_routes())
        .merge(mobile_routes())
        .merge(session_routes())
}

fn core_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/api/health", get(health))
        .route("/api/settings", get(get_settings).post(update_settings))
        .route("/api/execution/launch/start", post(launch_start))
        .route("/api/execution/launch/status", get(launch_status))
        .route("/api/execution/launch/stream", get(launch_stream_ws))
        .route(
            "/api/execution/linux_sandbox_runtime/status",
            get(linux_sandbox_runtime_status_api),
        )
        .route(
            "/api/execution/linux_sandbox_runtime/stage",
            post(linux_sandbox_runtime_stage),
        )
        .route(
            "/api/execution/linux_sandbox_runtime/prepare",
            post(linux_sandbox_runtime_prepare),
        )
        .route(
            "/api/title_generation/local/status",
            get(get_title_generation_local_status),
        )
        .route(
            "/api/title_generation/local/install",
            post(install_title_generation_local),
        )
        .route("/api/repo/clone", post(repo_clone))
        .route("/api/repo/init", post(repo_init))
        .route("/api/repo/status", post(repo_status))
        .route(
            "/api/repo/validate_destination",
            get(repo_validate_destination_get).post(repo_validate_destination),
        )
        .route("/api/repo/staging_path", get(repo_staging_path))
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/resource_utilization", get(resource_utilization))
        .route("/api/telemetry/summary", get(get_telemetry_summary))
        .route("/api/telemetry/export", get(export_telemetry))
        .route("/api/telemetry/client", post(post_client_telemetry))
        .route("/api/telemetry/events", post(post_semantic_telemetry))
        .route("/api/blobs", post(upload_blob))
        .route("/api/blobs/:id", get(get_blob))
        .route("/api/logs/open", post(open_logs_folder))
        .route("/api/desktop/log", post(append_desktop_log))
        .route("/api/updates/check", get(check_updates))
        .route("/api/updates/activity", get(update_activity))
        .route("/api/updates/drain/begin", post(begin_update_drain))
        .route("/api/updates/drain/release", post(release_update_drain))
        .route(
            "/api/updates/appimage/download",
            post(download_appimage_update),
        )
        .route("/api/updates/appimage/apply", post(apply_appimage_update))
        .route("/api/dev/providers/restart", post(dev_restart_providers))
        .route(
            "/api/dev/sessions/:id/seed_transcript",
            post(dev_seed_session_transcript),
        )
        .route(
            "/api/dictation/livekit/stream",
            get(dictation_livekit_stream_ws),
        )
}

fn provider_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/api/providers", get(list_providers))
        .route(
            "/api/providers/matrix/refresh",
            post(refresh_provider_matrix),
        )
        .route(
            "/api/providers/install_all",
            post(provider_launch::install_all_providers),
        )
        .route("/api/providers/:id", get(get_provider))
        .route("/api/providers/:id/usage", get(get_provider_usage))
        .route(
            "/api/providers/:id/harness_config",
            get(get_provider_harness_config),
        )
        .route(
            "/api/providers/:id/harness_config/select",
            post(select_provider_harness_source),
        )
        .route(
            "/api/providers/:id/harness_config/endpoints",
            post(upsert_provider_harness_endpoint),
        )
        .route(
            "/api/providers/:id/harness_config/endpoints/:endpoint_id",
            delete(delete_provider_harness_endpoint),
        )
        .route(
            "/api/providers/:id/harness_config/endpoints/:endpoint_id/models/refresh",
            post(refresh_provider_harness_endpoint_models),
        )
        .route(
            "/api/providers/:id/harness_config/endpoints/:endpoint_id/models/manual",
            put(set_provider_harness_endpoint_manual_models),
        )
        .route(
            "/api/providers/auth/import/candidates",
            get(list_provider_auth_import_candidates),
        )
        .route(
            "/api/providers/auth/import/profiles",
            get(list_provider_auth_import_profiles),
        )
        .route(
            "/api/providers/auth/import",
            post(import_provider_auth_candidates),
        )
        .route(
            "/api/providers/codex-crp/accounts",
            get(list_codex_accounts),
        )
        .route("/api/providers/codex/accounts", get(list_codex_accounts))
        .route(
            "/api/providers/codex-crp/import/host",
            get(probe_host_codex_import).post(import_host_codex_auth),
        )
        .route(
            "/api/providers/codex/import/host",
            get(probe_host_codex_import).post(import_host_codex_auth),
        )
        .route(
            "/api/providers/codex-crp/accounts/usage",
            get(get_codex_accounts_usage),
        )
        .route(
            "/api/providers/codex/accounts/usage",
            get(get_codex_accounts_usage),
        )
        .route(
            "/api/providers/codex-crp/accounts/login/start",
            post(start_codex_login),
        )
        .route(
            "/api/providers/codex/accounts/login/start",
            post(start_codex_login),
        )
        .route(
            "/api/providers/codex-crp/accounts/login/:id",
            get(get_codex_login).post(complete_codex_login),
        )
        .route(
            "/api/providers/codex/accounts/login/:id",
            get(get_codex_login).post(complete_codex_login),
        )
        .route(
            "/api/providers/codex-crp/active-account",
            put(set_codex_active_account),
        )
        .route(
            "/api/providers/codex/active-account",
            put(set_codex_active_account),
        )
        .route(
            "/api/providers/codex-crp/accounts/:id",
            delete(delete_codex_account),
        )
        .route(
            "/api/providers/codex/accounts/:id",
            delete(delete_codex_account),
        )
        .route(
            "/api/providers/claude-crp/accounts",
            get(list_claude_accounts).post(upsert_claude_account),
        )
        .route(
            "/api/providers/claude-crp/accounts/login/start",
            post(start_claude_login),
        )
        .route(
            "/api/providers/claude-crp/accounts/login/:id",
            get(get_claude_login),
        )
        .route(
            "/api/providers/claude-crp/active-account",
            put(set_claude_active_account),
        )
        .route(
            "/api/providers/claude-crp/accounts/:id",
            delete(delete_claude_account),
        )
        .route(
            "/api/providers/gemini/accounts",
            get(list_gemini_accounts).post(upsert_gemini_account),
        )
        .route(
            "/api/providers/gemini/accounts/login/start",
            post(start_gemini_login),
        )
        .route(
            "/api/providers/gemini/accounts/login/:id",
            get(get_gemini_login),
        )
        .route(
            "/api/providers/gemini/active-account",
            put(set_gemini_active_account),
        )
        .route(
            "/api/providers/gemini/accounts/:id",
            delete(delete_gemini_account),
        )
        .route(
            "/api/providers/qwen/accounts",
            get(list_qwen_accounts).post(upsert_qwen_account),
        )
        .route(
            "/api/providers/qwen/accounts/login/start",
            post(start_qwen_login),
        )
        .route(
            "/api/providers/qwen/accounts/login/:id",
            get(get_qwen_login),
        )
        .route(
            "/api/providers/qwen/active-account",
            put(set_qwen_active_account),
        )
        .route(
            "/api/providers/qwen/accounts/:id",
            delete(delete_qwen_account),
        )
        .route(
            "/api/providers/kimi/accounts/login/start",
            post(start_kimi_login),
        )
        .route(
            "/api/providers/kimi/accounts/login/:id",
            get(get_kimi_login),
        )
        .route(
            "/api/providers/kimi/accounts",
            get(list_kimi_accounts).post(upsert_kimi_account),
        )
        .route(
            "/api/providers/kimi/active-account",
            put(set_kimi_active_account),
        )
        .route(
            "/api/providers/kimi/accounts/:id",
            delete(delete_kimi_account),
        )
        .route(
            "/api/providers/amp/accounts/login/start",
            post(start_amp_login),
        )
        .route("/api/providers/amp/accounts/login/:id", get(get_amp_login))
        .route(
            "/api/providers/amp/accounts",
            get(list_amp_accounts).post(upsert_amp_account),
        )
        .route(
            "/api/providers/amp/active-account",
            put(set_amp_active_account),
        )
        .route(
            "/api/providers/amp/accounts/:id",
            delete(delete_amp_account),
        )
        .route(
            "/api/providers/mistral/accounts",
            get(list_mistral_accounts).post(upsert_mistral_account),
        )
        .route(
            "/api/providers/mistral/accounts/login/start",
            post(start_mistral_login),
        )
        .route(
            "/api/providers/mistral/accounts/login/:id",
            get(get_mistral_login),
        )
        .route(
            "/api/providers/mistral/active-account",
            put(set_mistral_active_account),
        )
        .route(
            "/api/providers/mistral/accounts/:id",
            delete(delete_mistral_account),
        )
        .route(
            "/api/providers/copilot/accounts",
            get(list_copilot_accounts).post(upsert_copilot_account),
        )
        .route(
            "/api/providers/copilot/active-account",
            put(set_copilot_active_account),
        )
        .route(
            "/api/providers/copilot/accounts/:id",
            delete(delete_copilot_account),
        )
        .route(
            "/api/providers/cursor/accounts",
            get(list_cursor_accounts).post(upsert_cursor_account),
        )
        .route(
            "/api/providers/cursor/accounts/login/start",
            post(start_cursor_login),
        )
        .route(
            "/api/providers/cursor/accounts/login/:id",
            get(get_cursor_login),
        )
        .route(
            "/api/providers/cursor/active-account",
            put(set_cursor_active_account),
        )
        .route(
            "/api/providers/cursor/accounts/:id",
            delete(delete_cursor_account),
        )
        .route(
            "/api/providers/:id/install",
            post(provider_launch::install_provider),
        )
        .route(
            "/api/providers/install/:install_id",
            get(provider_launch::get_install),
        )
        .route(
            "/api/providers/install/statuses",
            post(provider_launch::get_install_statuses),
        )
        .route(
            "/api/providers/install/:install_id/cancel",
            post(provider_launch::cancel_install),
        )
        .route(
            "/api/providers/install/:install_id/events",
            get(provider_launch::list_install_events),
        )
        .route(
            "/api/providers/install/:install_id/stream",
            get(provider_launch::install_stream_sse),
        )
}

fn lsp_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/api/lsp/status", get(lsp_status))
        .route("/api/lsp/catalog", get(lsp_catalog_list))
        .route(
            "/api/worktrees/:id/bootstrap/logs",
            get(get_worktree_bootstrap_logs),
        )
        .route(
            "/api/merge-queue/entries",
            get(list_merge_queue_entries).post(submit_merge_queue_entry),
        )
        .route(
            "/api/workspaces/:id/merge_queue/entries/:entry_id/logs",
            get(get_merge_queue_entry_logs),
        )
        .route(
            "/api/workspaces/:id/merge_queue/entries/:entry_id/cancel",
            post(cancel_merge_queue_entry),
        )
        .route(
            "/api/workspaces/:id/merge_queue/entries/:entry_id/retry",
            post(retry_merge_queue_entry),
        )
        .route(
            "/api/lsp/catalog/:id/install",
            post(install_lsp_catalog_server),
        )
        .route("/api/lsp/servers/:id/install", post(install_lsp_server))
        .route("/api/lsp/diagnostics", post(lsp_diagnostics))
        .route("/api/lsp/definition", post(lsp_definition))
        .route("/api/lsp/type_definition", post(lsp_type_definition))
        .route("/api/lsp/implementation", post(lsp_implementation))
        .route("/api/lsp/references", post(lsp_references))
        .route("/api/lsp/hover", post(lsp_hover))
        .route("/api/lsp/signature_help", post(lsp_signature_help))
        .route("/api/lsp/completion", post(lsp_completion))
        .route("/api/lsp/completion/resolve", post(lsp_completion_resolve))
        .route(
            "/api/lsp/code_action/resolve",
            post(lsp_code_action_resolve),
        )
        .route("/api/lsp/inlay_hints", post(lsp_inlay_hints))
        .route("/api/lsp/document_highlight", post(lsp_document_highlight))
        .route("/api/lsp/selection_ranges", post(lsp_selection_ranges))
        .route(
            "/api/lsp/call_hierarchy/prepare",
            post(lsp_call_hierarchy_prepare),
        )
        .route(
            "/api/lsp/call_hierarchy/incoming",
            post(lsp_call_hierarchy_incoming),
        )
        .route(
            "/api/lsp/call_hierarchy/outgoing",
            post(lsp_call_hierarchy_outgoing),
        )
        .route("/api/lsp/code_lens", post(lsp_code_lens))
        .route("/api/lsp/code_lens/resolve", post(lsp_code_lens_resolve))
        .route("/api/lsp/prepare_rename", post(lsp_prepare_rename))
        .route("/api/lsp/document_links", post(lsp_document_links))
        .route(
            "/api/lsp/document_links/resolve",
            post(lsp_document_link_resolve),
        )
        .route(
            "/api/lsp/semantic_tokens/full",
            post(lsp_semantic_tokens_full),
        )
        .route(
            "/api/lsp/semantic_tokens/delta",
            post(lsp_semantic_tokens_delta),
        )
        .route("/api/lsp/folding_ranges", post(lsp_folding_ranges))
        .route(
            "/api/lsp/linked_editing_range",
            post(lsp_linked_editing_range),
        )
        .route(
            "/api/lsp/type_hierarchy/prepare",
            post(lsp_type_hierarchy_prepare),
        )
        .route(
            "/api/lsp/type_hierarchy/supertypes",
            post(lsp_type_hierarchy_supertypes),
        )
        .route(
            "/api/lsp/type_hierarchy/subtypes",
            post(lsp_type_hierarchy_subtypes),
        )
        .route("/api/lsp/execute_command", post(lsp_execute_command))
        .route(
            "/api/lsp/execute_command/plan",
            post(lsp_execute_command_plan),
        )
        .route("/api/lsp/document_symbols", post(lsp_document_symbols))
        .route("/api/lsp/workspace_symbols", post(lsp_workspace_symbols))
        .route(
            "/api/lsp/workspace_symbols/resolve",
            post(lsp_workspace_symbol_resolve),
        )
        .route("/api/lsp/code_actions", post(lsp_code_actions))
        .route(
            "/api/lsp/code_actions/by_diagnostic/plan",
            post(lsp_code_actions_by_diagnostic_plan),
        )
        .route("/api/lsp/rename/plan", post(lsp_rename_plan))
        .route("/api/lsp/format/plan", post(lsp_format_plan))
        .route(
            "/api/lsp/organize_imports/plan",
            post(lsp_organize_imports_plan),
        )
        .route("/api/lsp/code_actions/plan", post(lsp_code_actions_plan))
        .route(
            "/api/worktrees/:id/edit_plans",
            get(list_edit_plans_for_worktree),
        )
        .route("/api/edit_plans/:id", get(get_edit_plan))
        .route("/api/edit_plans/:id/apply", post(apply_edit_plan_patch))
        .route("/api/edit_plans/:id/discard", post(discard_edit_plan))
        .route("/api/buffers/open", post(open_buffer))
        .route("/api/buffers/update", post(update_buffer))
        .route("/api/buffers/close", post(close_buffer))
}

fn workspace_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/api/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route(
            "/api/workspaces/:id",
            delete(delete_workspace).get(get_workspace),
        )
        .route(
            "/api/workspaces/:id/harness_container",
            get(get_workspace_harness_container),
        )
        .route(
            "/api/workspaces/:id/harness_container/ensure",
            post(ensure_workspace_harness_container),
        )
        .route(
            "/api/workspaces/:id/harness_container/stop",
            post(stop_workspace_harness_container),
        )
        .route(
            "/api/workspaces/:id/active_snapshot",
            get(get_workspace_active_snapshot),
        )
        .route(
            "/api/workspaces/:id/active_heads",
            get(get_workspace_active_heads),
        )
        .route(
            "/api/workspaces/:id/terminals",
            get(list_workspace_terminals).post(create_workspace_terminal),
        )
        .route(
            "/api/workspaces/:id/active_snapshot/stream",
            get(workspace_active_snapshot_stream_ws),
        )
        .route(
            "/api/workspaces/:id/stream",
            get(workspace_active_snapshot_stream_ws),
        )
        .route(
            "/api/workspaces/:id/completions/files",
            get(workspace_file_completions),
        )
        .route(
            "/api/workspaces/:id/providers/:provider_id/options",
            get(provider_launch::get_provider_options),
        )
        .route(
            "/api/workspaces/:id/providers/bootstrap",
            get(get_workspace_providers_bootstrap),
        )
        .route(
            "/api/workspaces/:id/providers/:provider_id/authenticate",
            post(provider_launch::authenticate_provider_for_workspace),
        )
        .route(
            "/api/workspaces/:id/providers/:provider_id/verify",
            post(provider_launch::verify_provider_for_workspace),
        )
        .route(
            "/api/workspaces/:id/attachments",
            get(list_workspace_attachments)
                .post(create_workspace_attachment)
                .delete(delete_workspace_attachment),
        )
        .route(
            "/api/workspaces/:id/attachments/sync",
            post(sync_workspace_attachments),
        )
        .route(
            "/api/workspaces/:id/agent_system_prompt",
            get(get_agent_system_prompt).post(update_agent_system_prompt),
        )
        .route(
            "/api/workspaces/:id/subagent_system_prompt",
            get(get_subagent_system_prompt).post(update_subagent_system_prompt),
        )
        .route(
            "/api/workspaces/:id/provider_model_preferences/:provider_id",
            get(get_workspace_provider_model_preference)
                .post(update_workspace_provider_model_preference),
        )
        .route(
            "/api/workspaces/:id/primary_branch",
            get(get_workspace_primary_branch).post(update_workspace_primary_branch),
        )
        .route(
            "/api/workspaces/:id/merge_queue_config",
            get(get_merge_queue_config).post(update_merge_queue_config),
        )
        .route(
            "/api/workspaces/:id/execution_config",
            get(get_execution_config).post(update_execution_config),
        )
        .route(
            "/api/workspaces/:id/worktree_bootstrap_config",
            get(get_worktree_bootstrap_config).post(update_worktree_bootstrap_config),
        )
        .route(
            "/api/workspaces/:id/tasks",
            get(list_workspace_tasks).post(create_task),
        )
        .route(
            "/api/workspaces/:id/archived_task_summaries",
            get(list_workspace_archived_task_summaries),
        )
        .route("/api/tasks/:id", delete(delete_task))
        .route("/api/tasks/:id/title", post(update_task_title))
        .route("/api/tasks/:id/archive", post(archive_task))
        .route("/api/tasks/:id/unarchive", post(unarchive_task))
        .route("/api/tasks/:id/mark_read", post(mark_task_read))
        .route("/api/tasks/:id/mark_unread", post(mark_task_unread))
        .route("/api/terminals/:id", delete(delete_terminal))
        .route("/api/terminals/:id/stream", get(terminal_stream_ws))
        .route("/api/worktrees/:id", get(get_worktree))
}

fn mobile_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/api/mobile/connection_profiles",
            get(list_mobile_connection_profiles).post(create_mobile_connection_profile),
        )
        .route("/api/mobile/access/status", get(get_mobile_access_status))
        .route("/api/mobile/access/enable", post(enable_mobile_access))
        .route("/api/mobile/access/disable", post(disable_mobile_access))
        .route("/api/mobile/pair", post(pair_mobile_device))
        .route("/api/mobile/secure", post(handle_mobile_secure))
        .route(
            "/api/mobile/secure/workspaces/:id/stream",
            get(mobile_secure_workspace_stream_ws),
        )
        .route(
            "/api/mobile/connection_profiles/:id",
            delete(delete_mobile_connection_profile),
        )
        .route(
            "/api/mobile/connection_profiles/:id/devices",
            get(list_mobile_devices_for_profile),
        )
        .route("/api/mobile/register", post(register_mobile_device))
}

fn session_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/api/sessions/:id/artifacts/:artifact_id",
            get(get_session_artifact),
        )
        .route(
            "/api/tasks/:id/sessions",
            get(list_task_sessions).post(create_session_for_task),
        )
        .route("/api/sessions/:id/messages", post(post_message))
        .route("/api/sessions/:id/subagents", get(list_session_subagents))
        .route(
            "/api/sessions/:id/subagent_invocations",
            get(list_session_subagent_invocations),
        )
        .route(
            "/api/sessions/:id/subagent_invocations/:invocation_id",
            get(get_session_subagent_invocation),
        )
        .route(
            "/api/sessions/:id/artifacts",
            get(list_session_artifacts).post(set_session_artifacts),
        )
        .route("/api/sessions/:id/model", post(set_session_model))
        .route("/api/sessions/:id/mode", post(set_session_mode))
        .route(
            "/api/sessions/:id/title/generate",
            post(generate_session_title),
        )
        .route("/api/sessions/:id/snapshot", get(get_session_snapshot))
        .route("/api/sessions/:id/head", get(get_session_head))
        .route("/api/sessions/:id/state", get(get_session_state))
        .route("/api/sessions/:id/diff", get(get_session_diff))
        .route(
            "/api/sessions/:id/diff/summary",
            get(get_session_diff_summary),
        )
        .route("/api/sessions/:id/git/status", get(get_session_git_status))
        .route(
            "/api/sessions/:id/diff/apply",
            post(apply_session_diff_patch),
        )
        .route("/api/sessions/:id/events", get(get_session_events))
        .route("/api/sessions/:id/history", get(get_session_history))
        .route(
            "/api/sessions/:id/turns/:turn_id/tools",
            get(list_session_turn_tools),
        )
        .route(
            "/api/sessions/:id/completions/files",
            get(session_file_completions),
        )
        .route(
            "/api/sessions/:id/messages/:message_id",
            delete(delete_session_message),
        )
        .route("/api/sessions/:id/cancel", post(cancel_session))
        .route("/api/sessions/:id/interrupt", post(interrupt_session))
        .route("/api/sessions/:id/authenticate", post(authenticate_session))
        .route("/api/mcp/sessions/:id/subagent_init", post(mcp_agent_init))
        .route(
            "/api/mcp/sessions/:id/subagent_reply",
            post(mcp_agent_reply),
        )
        .route(
            "/api/mcp/sessions/:id/subagent_interrupt",
            post(mcp_subagent_interrupt),
        )
        .route(
            "/api/mcp/sessions/:id/subagent_list",
            get(mcp_subagent_list),
        )
        .route("/api/mcp/sessions/:id/oracle", post(mcp_oracle))
        .route(
            "/api/mcp/sessions/:id/subagent_wait",
            post(mcp_subagent_wait),
        )
        .route(
            "/api/sessions/web",
            post(create_web_session).get(list_web_sessions),
        )
        .route("/api/sessions/web/:id", get(get_web_session))
        .route("/api/sessions/web/:id/run", post(run_web_session))
        .route("/api/sessions/web/:id/eval", post(eval_web_session))
        .route("/api/sessions/web/:id/close", post(close_web_session))
        .route("/sessions/web/:id/view", get(web_session_view))
        .route("/sessions/web/:id/signal", get(web_session_signal))
        .route(
            "/api/sessions/:id/ask_user_question",
            post(submit_ask_user_question),
        )
}
