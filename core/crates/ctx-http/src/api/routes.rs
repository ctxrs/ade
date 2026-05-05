use std::sync::Arc;

use axum::routing::{delete, get, post, put};

use super::*;
use crate::daemon::AppState;

mod provider_routes;

use provider_routes::provider_routes;

pub(super) fn api_routes() -> axum::Router<Arc<AppState>> {
    core_routes()
        .merge(provider_routes())
        .merge(workspace_routes())
        .merge(mobile_routes())
        .merge(session_routes())
}

fn core_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/api/health", get(health))
        .route("/api/settings", get(get_settings).post(update_settings))
        .route("/api/orgs/daemon_enrollments", get(list_daemon_enrollments))
        .route(
            "/api/orgs/:org_id/daemon_enrollment",
            put(upsert_daemon_enrollment),
        )
        .route(
            "/api/orgs/:org_id/policy_snapshots",
            post(cache_org_policy_snapshot),
        )
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
        .route("/api/daemon/shutdown", post(shutdown_daemon))
        .route("/api/updates/check", get(check_updates))
        .route("/api/updates/activity", get(update_activity))
        .route("/api/updates/drain/begin", post(begin_update_drain))
        .route("/api/updates/drain/release", post(release_update_drain))
        .route(
            "/api/updates/appimage/download",
            post(download_appimage_update),
        )
        .route("/api/updates/appimage/apply", post(apply_appimage_update))
        .route(
            "/api/merge-queue/entries",
            get(list_merge_queue_entries).post(submit_merge_queue_entry),
        )
        .route("/api/dev/providers/restart", post(dev_restart_providers))
        .route("/api/dev/clock", get(dev_clock))
        .route(
            "/api/dev/sessions/:id/seed_transcript",
            post(dev_seed_session_transcript),
        )
        .route(
            "/api/dictation/livekit/stream",
            get(dictation_livekit_stream_ws),
        )
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
            "/api/workspaces/:id/org_policy",
            get(get_workspace_org_policy).put(upsert_workspace_org_policy),
        )
        .route(
            "/api/workspaces/:workspace_id/runs/:run_id/archive/ingest_batch",
            get(build_workspace_run_archive_ingest_batch),
        )
        .route(
            "/api/workspaces/:workspace_id/runs/:run_id/archive/ingest_ack",
            post(acknowledge_workspace_run_archive_ingest_batch),
        )
        .route(
            "/api/workspaces/:workspace_id/merge_queue/entries/:id/cancel",
            post(cancel_merge_queue_entry),
        )
        .route(
            "/api/workspaces/:workspace_id/merge_queue/entries/:id/retry",
            post(retry_merge_queue_entry),
        )
        .route(
            "/api/workspaces/:workspace_id/merge_queue/entries/:id/logs",
            get(get_merge_queue_entry_logs),
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
        .route(
            "/api/terminals/:id/stream_token",
            post(mint_terminal_stream_token),
        )
        .route("/api/terminals/:id/stream", get(terminal_stream_ws))
        .route("/api/worktrees/:id", get(get_worktree))
        .route(
            "/api/worktrees/:id/bootstrap/logs",
            get(get_worktree_bootstrap_logs),
        )
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
        .route("/api/mcp/sessions/:id/spawn_agent", post(mcp_spawn_agent))
        .route("/api/mcp/sessions/:id/send_input", post(mcp_send_input))
        .route(
            "/api/mcp/sessions/:id/archive_agent",
            post(mcp_archive_agent),
        )
        .route(
            "/api/mcp/sessions/:id/interrupt_agent",
            post(mcp_interrupt_agent),
        )
        .route("/api/mcp/sessions/:id/list_agents", get(mcp_list_agents))
        .route("/api/mcp/sessions/:id/get_agent", post(mcp_get_agent))
        .route("/api/mcp/sessions/:id/wait_agent", post(mcp_wait_agent))
        .route(
            "/api/sessions/web",
            post(create_web_session).get(list_web_sessions),
        )
        .route("/api/sessions/web/:id", get(get_web_session))
        .route(
            "/api/sessions/web/:id/stream_token",
            post(mint_web_session_stream_token),
        )
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
