use std::collections::HashMap;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use axum::body::{Body, Bytes};
use axum::extract::{Extension, MatchedPath, Path, Query, State};
use axum::http::header;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::Json;
use base64::Engine;
use opentelemetry::trace::SpanKind;
use opentelemetry::KeyValue;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio::process::Command;
use tower::util::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};
use url::Url;

mod artifacts;
mod auth;
mod errors;
mod execution;
mod extractors;
mod providers;
mod repo;
pub(crate) mod sessions;
mod settings;
mod shared;
mod tasks;
mod terminals;
mod workspaces;
mod ws;
use artifacts::*;
use execution::*;
use providers::*;
use repo::*;
use sessions::*;
use settings::*;
use tasks::*;
use terminals::*;
use workspaces::*;

use auth::{
    auth_middleware, generate_mobile_api_token, generate_pairing_token, hash_api_token,
    hash_pairing_token, MobileAuthContext,
};
use errors::ApiErrorResp;
use extractors::extract_workspace_edit_from_command;
use ws::{
    dictation_livekit_stream_ws, mobile_secure_workspace_stream_ws, terminal_stream_ws,
    web_session_signal, workspace_active_snapshot_stream_ws,
};

use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_store::store::MobileDeviceUpsert;

use crate::buffers::{
    BufferCloseReq, BufferConflictResp, BufferId, BufferOpenReq, BufferOpenResp, BufferUpdateReq,
    BufferUpdateResp,
};
use crate::daemon::AppState;
use crate::installer;
use crate::installs::InstallId;
use crate::logs;
use crate::merge_queue;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::resource_utilization;
use crate::title_generation_local;
use crate::updates;
use crate::web_sessions::{
    render_web_session_view, WebSessionCreateRequest, WebSessionInfo, WebSessionRunRequest,
    WebSessionRunResponse, WebSessionViewport,
};
use ctx_providers::adapters::ProviderStatus;

pub(super) fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("authorization")
        || (key.contains("api") && key.contains("key"))
}

pub(super) fn redact_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if is_sensitive_key(&k) {
                    out.insert(k, serde_json::Value::String("[REDACTED]".to_string()));
                    continue;
                }
                out.insert(k, redact_json_value(v));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(redact_json_value).collect())
        }
        serde_json::Value::String(s) => serde_json::Value::String(logs::redact_sensitive(&s)),
        other => other,
    }
}

fn header_first_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    Some(value.split(',').next()?.trim().to_string())
}

fn parse_forwarded_header(value: &str) -> (Option<String>, Option<String>) {
    let mut proto = None;
    let mut host = None;
    let first = value.split(',').next().unwrap_or(value);
    for part in first.split(';') {
        let part = part.trim();
        if let Some(raw) = part.strip_prefix("proto=") {
            let clean = raw.trim().trim_matches('"').trim_matches('\'');
            if !clean.is_empty() {
                proto = Some(clean.to_string());
            }
        } else if let Some(raw) = part.strip_prefix("host=") {
            let clean = raw.trim().trim_matches('"').trim_matches('\'');
            if !clean.is_empty() {
                host = Some(clean.to_string());
            }
        }
    }
    (proto, host)
}

fn resolve_request_base_url(headers: &HeaderMap, fallback: &str) -> String {
    let fallback = fallback.trim_end_matches('/');
    let fallback_url = Url::parse(fallback).ok();
    let fallback_scheme = fallback_url
        .as_ref()
        .map(|url| url.scheme().to_string())
        .unwrap_or_else(|| "http".to_string());
    let fallback_host = fallback_url.as_ref().and_then(|url| {
        let host = url.host_str()?;
        Some(match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        })
    });

    let (forwarded_proto, forwarded_host) = headers
        .get(header::FORWARDED)
        .and_then(|value| value.to_str().ok())
        .map(parse_forwarded_header)
        .unwrap_or((None, None));

    let proto = forwarded_proto
        .or_else(|| header_first_value(headers, "x-forwarded-proto"))
        .unwrap_or(fallback_scheme);
    let host = forwarded_host
        .or_else(|| header_first_value(headers, "x-forwarded-host"))
        .or_else(|| header_first_value(headers, header::HOST.as_str()))
        .or(fallback_host);

    match host {
        Some(host) => format!("{}://{}", proto, host.trim_end_matches('/')),
        None => fallback.to_string(),
    }
}

pub fn router(state: Arc<AppState>) -> axum::Router {
    let auth_state = state.clone();
    let perf_state = state.clone();
    let api = axum::Router::new()
        .route("/api/health", get(health))
        .route("/api/settings", get(get_settings).post(update_settings))
        .route(
            "/api/execution/container_image/prefetch",
            post(prefetch_container_image),
        )
        .route(
            "/api/execution/container_image/status",
            get(container_image_status),
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
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/resource_utilization", get(resource_utilization))
        .route("/api/telemetry/summary", get(get_telemetry_summary))
        .route("/api/telemetry/export", get(export_telemetry))
        .route("/api/telemetry/client", post(post_client_telemetry))
        .route("/api/blobs", post(upload_blob))
        .route("/api/blobs/:id", get(get_blob))
        .route("/api/artifacts/:id", get(get_artifact))
        .route("/api/logs/open", post(open_logs_folder))
        .route("/api/desktop/log", post(append_desktop_log))
        .route("/api/updates/check", get(check_updates))
        .route(
            "/api/updates/appimage/download",
            post(download_appimage_update),
        )
        .route("/api/updates/appimage/apply", post(apply_appimage_update))
        .route("/api/providers", get(list_providers))
        .route("/api/providers/install_all", post(install_all_providers))
        .route("/api/providers/:id", get(get_provider))
        .route("/api/providers/:id/usage", get(get_provider_usage))
        .route("/api/providers/codex/accounts", get(list_codex_accounts))
        .route(
            "/api/providers/codex/accounts/usage",
            get(get_codex_accounts_usage),
        )
        .route(
            "/api/providers/codex/accounts/login/start",
            post(start_codex_login),
        )
        .route(
            "/api/providers/codex/accounts/login/:id",
            get(get_codex_login),
        )
        .route(
            "/api/providers/codex/active-account",
            put(set_codex_active_account),
        )
        .route(
            "/api/providers/codex/accounts/:id",
            delete(delete_codex_account),
        )
        .route("/api/providers/:id/install", post(install_provider))
        .route("/api/providers/install/:install_id", get(get_install))
        .route(
            "/api/providers/install/:install_id/events",
            get(list_install_events),
        )
        .route(
            "/api/providers/install/:install_id/stream",
            get(install_stream_sse),
        )
        .route("/api/dev/providers/restart", post(dev_restart_providers))
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
            "/api/merge-queue/entries/:id/logs",
            get(get_merge_queue_entry_logs),
        )
        .route(
            "/api/merge-queue/entries/:id/cancel",
            post(cancel_merge_queue_entry),
        )
        .route(
            "/api/merge-queue/entries/:id/retry",
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
        .route(
            "/api/workspaces/:id/providers/:provider_id/options",
            get(get_provider_options),
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
            "/api/subagent_invocations/:id",
            get(get_subagent_invocation),
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
        .route("/api/messages/:id", delete(delete_message))
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
        .route(
            "/api/dictation/livekit/stream",
            get(dictation_livekit_stream_ws),
        )
        .route_layer(middleware::from_fn_with_state(perf_state, perf_middleware))
        .layer(middleware::from_fn_with_state(auth_state, auth_middleware))
        .with_state(state);

    let dist_dir = std::env::var("CTX_WEB_DIST").unwrap_or_else(|_| "apps/web/dist".into());
    let index_path = format!("{}/index.html", dist_dir);
    api.fallback_service(ServeDir::new(dist_dir).not_found_service(ServeFile::new(index_path)))
}

async fn submit_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MergeQueueSubmitReq>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = match req.session_id {
        Some(id) => Some(SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid session_id".to_string(),
                }),
            )
        })?)),
        None => None,
    };
    let worktree_id = match req.worktree_id {
        Some(id) => Some(WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid worktree_id".to_string(),
                }),
            )
        })?)),
        None => None,
    };
    let worktree_root = req.worktree_root.and_then(|root| {
        let trimmed = root.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let params = merge_queue::MergeQueueSubmitParams {
        session_id,
        worktree_id,
        worktree_root,
        target_branch: req.target_branch,
        message: req.message,
    };
    let entry = merge_queue::submit_merge_queue_entry(&state, params)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: err.to_string(),
                }),
            )
        })?;
    Ok(Json(entry))
}

async fn list_merge_queue_entries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MergeQueueListParams>,
) -> Result<Json<Vec<MergeQueueEntry>>, StatusCode> {
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&params.workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let entries = store
        .list_merge_queue_entries(workspace_id, params.limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(entries))
}

async fn cancel_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let entry_id = MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid entry id".to_string(),
            }),
        )
    })?);
    let entry = merge_queue::cancel_merge_queue_entry(&state, entry_id)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: err.to_string(),
                }),
            )
        })?;
    Ok(Json(entry))
}

async fn retry_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let entry_id = MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid entry id".to_string(),
            }),
        )
    })?);
    let entry = merge_queue::retry_merge_queue_entry(&state, entry_id)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: err.to_string(),
                }),
            )
        })?;
    Ok(Json(entry))
}

async fn get_merge_queue_entry_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let entry_id =
        MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let entry = merge_queue::get_merge_queue_entry(&state, entry_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let store = state
        .store_for_workspace(entry.workspace_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let run = store
        .get_latest_merge_queue_run(entry_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(path) = run.log_path.as_deref() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let filename = format!("merge-queue-{}.log", entry_id.0);
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| header::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}

const MOBILE_API_MIN_VERSION: i64 = 1;
const MOBILE_API_MAX_VERSION: i64 = 1;

#[derive(Debug, Serialize)]
struct HealthCompatibility {
    desktop_exact_version: String,
    mobile_api_min: i64,
    mobile_api_max: i64,
}

#[derive(Debug, Serialize)]
struct HealthResp {
    version: String,
    daemon_version: String,
    pid: u32,
    data_root: String,
    daemon_url: String,
    auth_required: bool,
    compatibility: HealthCompatibility,
}

#[derive(Debug, Deserialize)]
struct MergeQueueSubmitReq {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    worktree_root: Option<String>,
    #[serde(default)]
    target_branch: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MergeQueueListParams {
    workspace_id: String,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CreateMobileConnectionProfileReq {
    label: String,
    base_url: String,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CreateMobileConnectionProfileResp {
    profile: MobileConnectionProfile,
    token: String,
    qr_payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct RegisterMobileDeviceReq {
    device_id: String,
    #[serde(default)]
    device_label: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    push_token: Option<String>,
    #[serde(default)]
    push_provider: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
    #[serde(default)]
    app_version: Option<String>,
}

async fn health(State(state): State<Arc<AppState>>) -> Result<Json<HealthResp>, StatusCode> {
    let version = env!("CARGO_PKG_VERSION").to_string();
    Ok(Json(HealthResp {
        version: version.clone(),
        daemon_version: version.clone(),
        pid: std::process::id(),
        data_root: state.core.data_root.to_string_lossy().to_string(),
        daemon_url: state.core.daemon_url.clone(),
        auth_required: state.core.auth_token.is_some(),
        compatibility: HealthCompatibility {
            desktop_exact_version: version,
            mobile_api_min: MOBILE_API_MIN_VERSION,
            mobile_api_max: MOBILE_API_MAX_VERSION,
        },
    }))
}

const TITLE_GENERATION_LOCAL_INSTALL_KEY: &str = "title_generation_local";

#[derive(Debug, Serialize)]
struct TitleGenerationLocalStatusResponse {
    pub ready: bool,
    pub runtime: title_generation_local::TitleGenerationLocalRuntimeStatus,
    pub model: title_generation_local::TitleGenerationLocalModelStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_id: Option<InstallId>,
    pub install_running: bool,
}

#[derive(Debug, Serialize)]
struct TitleGenerationLocalInstallResponse {
    pub install_id: InstallId,
}

async fn get_title_generation_local_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TitleGenerationLocalStatusResponse>, StatusCode> {
    let status = title_generation_local::local_status(&state.core.data_root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let install_id = state
        .find_running_install(TITLE_GENERATION_LOCAL_INSTALL_KEY)
        .await;
    Ok(Json(TitleGenerationLocalStatusResponse {
        ready: status.ready,
        runtime: status.runtime,
        model: status.model,
        install_id,
        install_running: install_id.is_some(),
    }))
}

async fn install_title_generation_local(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TitleGenerationLocalInstallResponse>, StatusCode> {
    let (install_id, started_new) = state
        .start_install(TITLE_GENERATION_LOCAL_INSTALL_KEY.to_string())
        .await;
    if started_new {
        let state2 = state.clone();
        tokio::spawn(async move {
            if let Err(e) =
                installer::install_title_generation_local_with_progress(state2.clone(), install_id)
                    .await
            {
                tracing::error!("local title generation install failed: {e:#}");
            }
        });
    }

    Ok(Json(TitleGenerationLocalInstallResponse { install_id }))
}

#[derive(Debug, Serialize)]
struct DiagnosticsResp {
    daemon: HealthResp,
    platform: serde_json::Value,
    logs: serde_json::Value,
    providers: Vec<ProviderStatus>,
    managed_installs: serde_json::Value,
}

async fn diagnostics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DiagnosticsResp>, StatusCode> {
    let providers = {
        let map = state.providers.statuses.lock().await;
        map.values()
            .cloned()
            .map(|mut s| {
                s.diagnostics = s
                    .diagnostics
                    .into_iter()
                    .map(|d| logs::redact_sensitive(&d))
                    .collect();
                s.details = s
                    .details
                    .into_iter()
                    .filter(|(k, _)| !is_sensitive_key(k))
                    .map(|(k, v)| (k, logs::redact_sensitive(&v)))
                    .collect();
                s
            })
            .collect::<Vec<_>>()
    };

    let log_files = logs::list_log_files(&state.core.data_root).await;
    let managed_installs = installer::load_agent_server_config(&state.core.data_root)
        .await
        .map(|cfg| serde_json::to_value(cfg).unwrap_or_else(|_| serde_json::json!({})))
        .unwrap_or_else(|e| serde_json::json!({"error": logs::redact_sensitive(&e.to_string())}));
    let managed_installs = redact_json_value(managed_installs);

    let version = env!("CARGO_PKG_VERSION").to_string();
    Ok(Json(DiagnosticsResp {
        daemon: HealthResp {
            version: version.clone(),
            daemon_version: version.clone(),
            pid: std::process::id(),
            data_root: state.core.data_root.to_string_lossy().to_string(),
            daemon_url: state.core.daemon_url.clone(),
            auth_required: state.core.auth_token.is_some(),
            compatibility: HealthCompatibility {
                desktop_exact_version: version,
                mobile_api_min: MOBILE_API_MIN_VERSION,
                mobile_api_max: MOBILE_API_MAX_VERSION,
            },
        },
        platform: serde_json::json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        }),
        logs: serde_json::json!({
            "dir": logs::logs_dir(&state.core.data_root).to_string_lossy(),
            "files": log_files,
        }),
        providers,
        managed_installs,
    }))
}

#[derive(Debug, Deserialize)]
struct ResourceUtilizationQuery {
    workspace_id: String,
}

async fn resource_utilization(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ResourceUtilizationQuery>,
) -> Result<Json<resource_utilization::ResourceUtilizationSnapshot>, StatusCode> {
    if resource_utilization_disabled() {
        return Err(StatusCode::NOT_FOUND);
    }
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&query.workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let worktrees = store
        .list_worktrees(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let provider_adapters = {
        let providers = state.providers.adapters.lock().await;
        providers.values().cloned().collect::<Vec<_>>()
    };
    let mut provider_processes = Vec::new();
    for adapter in provider_adapters {
        provider_processes.extend(adapter.list_processes().await);
    }

    let (system, disks, cache_age_ms, processes, disk_cache) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        let (system, disks, cache_age_ms) = sampler.system_snapshot();
        let processes = sampler.processes_snapshot_light(std::process::id(), &provider_processes);
        let disk_cache = sampler.disk_cache_entry(workspace_id);
        (system, disks, cache_age_ms, processes, disk_cache)
    };

    let disk = resource_utilization::disk_for_path(StdPath::new(&workspace.root_path), &disks);

    let now = Instant::now();
    let refresh_disk = resource_utilization::should_refresh_disk_cache(now, disk_cache.as_ref());
    let (mut workspace_snapshot, size_cache_age_ms) = if refresh_disk {
        let workspace_clone = workspace.clone();
        let worktrees_clone = worktrees.clone();
        let disk_clone = disk.clone();
        let snapshot = tokio::task::spawn_blocking(move || {
            resource_utilization::compute_workspace_disk_snapshot(
                workspace_clone,
                worktrees_clone,
                disk_clone,
                0,
            )
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        sampler.update_disk_cache(workspace_id, now, snapshot.clone());
        (snapshot, 0)
    } else {
        let age_ms = resource_utilization::disk_cache_age_ms(now, disk_cache.as_ref());
        let snapshot = disk_cache
            .as_ref()
            .map(|c| c.snapshot.clone())
            .unwrap_or_else(|| {
                resource_utilization::compute_workspace_disk_snapshot(
                    workspace.clone(),
                    worktrees.clone(),
                    disk.clone(),
                    age_ms,
                )
            });
        (snapshot, age_ms)
    };

    workspace_snapshot.disk = disk;
    workspace_snapshot.size_cache_age_ms = size_cache_age_ms;

    Ok(Json(resource_utilization::ResourceUtilizationSnapshot {
        collected_at: chrono::Utc::now().to_rfc3339(),
        cache_age_ms,
        system,
        processes,
        workspace: workspace_snapshot,
    }))
}

fn resource_utilization_disabled() -> bool {
    env_bool("CTX_RESOURCE_UTILIZATION_DISABLED").unwrap_or(true)
}

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().and_then(|v| match v.trim() {
        "1" | "true" | "TRUE" | "yes" | "YES" => Some(true),
        "0" | "false" | "FALSE" | "no" | "NO" => Some(false),
        _ => None,
    })
}

#[derive(Debug, Deserialize)]
struct LspFileReq {
    /// Optional session scope; when present, `path` is resolved within the session worktree.
    session_id: Option<String>,
    /// Optional explicit root path; used only when `session_id` is absent.
    root_path: Option<String>,
    /// File path to analyze (absolute or relative to resolved root).
    path: String,
}

#[derive(Debug, Serialize)]
struct LspServerStatus {
    language: String,
    command: String,
    args: Vec<String>,
    found: bool,
    resolved_path: Option<String>,
    version: Option<String>,
    install_hints: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LspStatusResp {
    enabled: bool,
    edit_plans_enabled: bool,
    servers: Vec<LspServerStatus>,
}

async fn lsp_status(State(state): State<Arc<AppState>>) -> Result<Json<LspStatusResp>, StatusCode> {
    let cfg = &state.core.lsp_cfg;
    let enabled = cfg.enabled;
    let edit_plans_enabled = state.core.lsp_edit_plans_enabled;

    let servers = vec![
        ("rust", cfg.rust_command.clone(), cfg.rust_args.clone()),
        ("typescript", cfg.ts_command.clone(), cfg.ts_args.clone()),
        ("python", cfg.py_command.clone(), cfg.py_args.clone()),
        ("go", cfg.go_command.clone(), cfg.go_args.clone()),
        ("html", cfg.html_command.clone(), cfg.html_args.clone()),
        ("css", cfg.css_command.clone(), cfg.css_args.clone()),
        ("json", cfg.json_command.clone(), cfg.json_args.clone()),
        ("yaml", cfg.yaml_command.clone(), cfg.yaml_args.clone()),
        ("bash", cfg.bash_command.clone(), cfg.bash_args.clone()),
        (
            "dockerfile",
            cfg.dockerfile_command.clone(),
            cfg.dockerfile_args.clone(),
        ),
        ("cpp", cfg.clangd_command.clone(), cfg.clangd_args.clone()),
        ("lua", cfg.lua_command.clone(), cfg.lua_args.clone()),
        ("toml", cfg.toml_command.clone(), cfg.toml_args.clone()),
        (
            "markdown",
            cfg.markdown_command.clone(),
            cfg.markdown_args.clone(),
        ),
    ];

    let mut out = Vec::new();
    for (language, command, args) in servers {
        let (found, resolved_path) = resolve_command(&command);
        let version = if found {
            get_command_version(&command, &resolved_path, &args).await
        } else {
            None
        };
        out.push(LspServerStatus {
            language: language.to_string(),
            command,
            args,
            found,
            resolved_path: resolved_path.map(|p| p.to_string_lossy().to_string()),
            version,
            install_hints: install_hints_for(language),
        });
    }

    // Add BYO servers not already represented by built-ins.
    for (language, (command, args)) in cfg.custom_servers.iter() {
        if out.iter().any(|s| s.language == *language) {
            continue;
        }
        let (found, resolved_path) = resolve_command(command);
        let version = if found {
            get_command_version(command, &resolved_path, args).await
        } else {
            None
        };
        out.push(LspServerStatus {
            language: language.clone(),
            command: command.clone(),
            args: args.clone(),
            found,
            resolved_path: resolved_path.map(|p| p.to_string_lossy().to_string()),
            version,
            install_hints: vec![
                "Configured via data_root/lsp/user_servers.json (restart daemon after edits)."
                    .to_string(),
            ],
        });
    }

    Ok(Json(LspStatusResp {
        enabled,
        edit_plans_enabled,
        servers: out,
    }))
}

#[derive(Debug, Serialize)]
struct LspCatalogEntryStatusResp {
    id: String,
    title: String,
    language_id: String,
    install_kind: String,
    installed: bool,
    installed_version: Option<String>,
    enabled: bool,
    enabled_command: Option<String>,
}

async fn lsp_catalog_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<LspCatalogEntryStatusResp>>, StatusCode> {
    let catalog = crate::lsp_catalog::load_catalog(&state.core.data_root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let installed = installer::load_lsp_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();

    let mut out = Vec::new();
    for entry in catalog.servers {
        let (install_kind, installed_key) = match &entry.install {
            crate::lsp_catalog::LspCatalogInstall::ManagedNode { server_id } => {
                ("managed_node".to_string(), server_id.clone())
            }
            crate::lsp_catalog::LspCatalogInstall::UrlBinary { .. } => {
                ("url_binary".to_string(), entry.id.clone())
            }
            crate::lsp_catalog::LspCatalogInstall::GoInstall { .. } => {
                ("go_install".to_string(), entry.id.clone())
            }
            crate::lsp_catalog::LspCatalogInstall::System { .. } => {
                ("system".to_string(), entry.id.clone())
            }
        };

        let meta = installed.managed_installs.get(&installed_key);
        let installed_version = meta.and_then(|m| m.version.clone());
        let enabled_cmd = installed
            .servers
            .get(&entry.language_id)
            .map(|c| c.command.clone());
        out.push(LspCatalogEntryStatusResp {
            id: entry.id,
            title: entry.title,
            language_id: entry.language_id,
            install_kind,
            installed: meta.is_some(),
            installed_version,
            enabled: enabled_cmd.is_some(),
            enabled_command: enabled_cmd,
        });
    }

    Ok(Json(out))
}

#[derive(Debug, Serialize)]
struct LspCatalogInstallStartResponse {
    catalog_id: String,
    install_id: InstallId,
}

async fn install_lsp_catalog_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LspCatalogInstallStartResponse>, StatusCode> {
    // Validate id exists.
    if crate::lsp_catalog::get_entry(&state.core.data_root, &id)
        .await
        .is_err()
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let install_key = format!("lsp:{id}");
    let (install_id, started_new) = state.start_install(install_key).await;
    if started_new {
        let state2 = state.clone();
        let catalog_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_lsp_catalog_server_with_progress(
                state2.clone(),
                install_id,
                catalog_id.clone(),
            )
            .await
            {
                tracing::error!("lsp catalog install failed ({catalog_id}): {e:#}");
            }
        });
    }

    Ok(Json(LspCatalogInstallStartResponse {
        catalog_id: id,
        install_id,
    }))
}

fn resolve_command(command: &str) -> (bool, Option<PathBuf>) {
    if command.trim().is_empty() {
        return (false, None);
    }
    let path = PathBuf::from(command);
    if path.components().count() > 1 {
        return (path.exists(), Some(path));
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return (false, None);
    };
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(command);
        if candidate.exists() {
            return (true, Some(candidate));
        }
    }
    (false, None)
}

async fn get_command_version(
    command: &str,
    resolved: &Option<PathBuf>,
    _args: &[String],
) -> Option<String> {
    let exe = resolved
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| command.to_string());
    let base = StdPath::new(&exe)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let candidates: Vec<Vec<&'static str>> = if base.contains("gopls") {
        vec![vec!["version"], vec!["--version"]]
    } else {
        vec![vec!["--version"], vec!["version"]]
    };

    for args in candidates {
        let fut = Command::new(&exe).args(args.iter()).output();
        if let Ok(Ok(output)) = tokio::time::timeout(std::time::Duration::from_secs(2), fut).await {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let s = if s.is_empty() {
                    String::from_utf8_lossy(&output.stderr).trim().to_string()
                } else {
                    s
                };
                if !s.is_empty() {
                    return Some(s.lines().next().unwrap_or("").trim().to_string());
                }
            }
        }
    }
    None
}

fn install_hints_for(language: &str) -> Vec<String> {
    let os = std::env::consts::OS;
    match (language, os) {
        ("rust", "darwin") => vec![
            "brew install rust-analyzer".to_string(),
            "or: rustup component add rust-analyzer (if available)".to_string(),
        ],
        ("rust", "linux") => vec![
            "rustup component add rust-analyzer (if available)".to_string(),
            "or: install rust-analyzer from your distro/package manager".to_string(),
        ],
        ("typescript", _) => vec![
            "managed: POST /api/lsp/servers/typescript/install (restart daemon after install)".to_string(),
            "or: npm i -g typescript typescript-language-server".to_string(),
        ],
        ("python", _) => vec![
            "managed: POST /api/lsp/servers/python/install (restart daemon after install)".to_string(),
            "or: npm i -g pyright".to_string(),
        ],
        ("go", _) => vec!["go install golang.org/x/tools/gopls@latest".to_string()],
        ("html" | "css" | "json", _) => vec![
            "managed: POST /api/lsp/servers/html/install (installs html+css+json; restart daemon after install)".to_string(),
            "or: npm i -g vscode-langservers-extracted".to_string(),
        ],
        ("yaml", _) => vec![
            "managed: POST /api/lsp/servers/yaml/install (restart daemon after install)".to_string(),
            "or: npm i -g yaml-language-server".to_string(),
        ],
        ("bash", _) => vec![
            "managed: POST /api/lsp/servers/bash/install (restart daemon after install)".to_string(),
            "or: npm i -g bash-language-server".to_string(),
        ],
        ("dockerfile", _) => vec![
            "managed: POST /api/lsp/servers/dockerfile/install (restart daemon after install)".to_string(),
            "or: npm i -g dockerfile-language-server-nodejs".to_string(),
        ],
        ("cpp", "darwin") => vec!["brew install llvm (clangd)".to_string()],
        ("cpp", "linux") => vec!["sudo apt-get install clangd (or distro equivalent)".to_string()],
        ("lua", "darwin") => vec!["brew install lua-language-server".to_string()],
        ("lua", "linux") => vec!["install lua-language-server via your distro/package manager".to_string()],
        ("toml", "darwin") => vec!["brew install taplo".to_string()],
        ("toml", "linux") => vec!["cargo install taplo-cli --locked".to_string()],
        ("markdown", "darwin") => vec!["brew install marksman".to_string()],
        ("markdown", "linux") => vec!["install marksman via your distro/package manager".to_string()],
        _ => vec![],
    }
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

fn plan_paths_match(a_old: &str, a_new: &str, b_old: &str, b_new: &str) -> bool {
    (!a_new.is_empty() || a_old == b_old) && a_new == b_new
}

async fn lsp_diagnostics(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let (root, file) = resolve_lsp_target(&state, req).await?;
    let diags = state
        .core
        .lsp
        .diagnostics_for_file(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let out = diags
        .into_iter()
        .filter_map(|d| serde_json::to_value(d).ok())
        .collect::<Vec<_>>();
    Ok(Json(out))
}

async fn resolve_lsp_target(
    state: &Arc<AppState>,
    req: LspFileReq,
) -> Result<(PathBuf, PathBuf), StatusCode> {
    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid =
            SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        let store = state
            .store_for_session(sid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let session = store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        PathBuf::from(wt.root_path)
    } else if let Some(root_path) = req.root_path.as_deref() {
        PathBuf::from(root_path)
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let root = root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?;
    let candidate = if PathBuf::from(&req.path).is_absolute() {
        PathBuf::from(&req.path)
    } else {
        root.join(&req.path)
    };
    let file = candidate
        .canonicalize()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if !file.starts_with(&root) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((root, file))
}

async fn resolve_session_root_and_file(
    state: &Arc<AppState>,
    session_id: &str,
    path: &str,
) -> Result<(SessionId, WorktreeId, PathBuf, PathBuf), StatusCode> {
    let sid = SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(sid)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let root = PathBuf::from(wt.root_path)
        .canonicalize()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let file = crate::buffers::BufferStore::resolve_path(&root, path)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok((sid, session.worktree_id, root, file))
}

async fn open_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferOpenReq>,
) -> Result<Json<BufferOpenResp>, StatusCode> {
    let (sid, worktree_id, root, file) =
        resolve_session_root_and_file(&state, &req.session_id, &req.path).await?;
    let text = tokio::fs::read_to_string(&file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let disk_sha = sha256_hex(&text);
    let st = state
        .core
        .buffers
        .open_or_reuse(
            sid,
            worktree_id,
            root.clone(),
            file.clone(),
            text.clone(),
            disk_sha.clone(),
        )
        .await;
    if state.core.lsp.enabled() {
        if let Some(lang) = ctx_lsp::Language::detect(&file, &state.core.lsp_cfg) {
            state
                .ensure_lsp_diagnostics_forwarder(root.clone(), lang)
                .await;
        }
        let _ = state
            .core
            .lsp
            .sync_document_text(&root, &file, st.text.clone())
            .await;
    }
    Ok(Json(BufferOpenResp {
        buffer_id: st.id.0.to_string(),
        path: req.path,
        version: st.version,
        text: st.text,
        last_disk_sha256: st.last_disk_sha256,
    }))
}

async fn update_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferUpdateReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<BufferConflictResp>)> {
    let bid = BufferId(uuid::Uuid::parse_str(&req.buffer_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(BufferConflictResp {
                error: "invalid buffer_id".to_string(),
                disk_sha256: "".to_string(),
                disk_text: "".to_string(),
            }),
        )
    })?);
    let current = state.core.buffers.get(bid).await.ok_or((
        StatusCode::NOT_FOUND,
        Json(BufferConflictResp {
            error: "buffer not found".to_string(),
            disk_sha256: "".to_string(),
            disk_text: "".to_string(),
        }),
    ))?;

    let new_sha = if req.persist {
        // Detect external changes on disk.
        let disk_text = tokio::fs::read_to_string(&current.path)
            .await
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(BufferConflictResp {
                        error: "failed to read file".to_string(),
                        disk_sha256: "".to_string(),
                        disk_text: "".to_string(),
                    }),
                )
            })?;
        let disk_sha = sha256_hex(&disk_text);
        if !req.force && disk_sha != current.last_disk_sha256 {
            return Err((
                StatusCode::CONFLICT,
                Json(BufferConflictResp {
                    error: "file changed on disk while buffer was open".to_string(),
                    disk_sha256: disk_sha,
                    disk_text,
                }),
            ));
        }

        // Write to disk (autosave).
        tokio::fs::write(&current.path, req.text.as_bytes())
            .await
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(BufferConflictResp {
                        error: "failed to write file".to_string(),
                        disk_sha256: "".to_string(),
                        disk_text: "".to_string(),
                    }),
                )
            })?;
        Some(sha256_hex(&req.text))
    } else {
        None
    };
    let st = state
        .core
        .buffers
        .update(bid, req.version, req.text, new_sha.clone())
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(BufferConflictResp {
                    error: logs::redact_sensitive(&e.to_string()),
                    disk_sha256: "".to_string(),
                    disk_text: "".to_string(),
                }),
            )
        })?;

    if state.core.lsp.enabled() {
        if let Some(lang) = ctx_lsp::Language::detect(&st.path, &state.core.lsp_cfg) {
            state
                .ensure_lsp_diagnostics_forwarder(st.root.clone(), lang)
                .await;
        }
        let _ = state
            .core
            .lsp
            .sync_document_text(&st.root, &st.path, st.text.clone())
            .await;
    }

    Ok(Json(BufferUpdateResp {
        buffer_id: st.id.0.to_string(),
        version: st.version,
        last_disk_sha256: st.last_disk_sha256,
    }))
}

async fn close_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferCloseReq>,
) -> Result<StatusCode, StatusCode> {
    let bid = BufferId(uuid::Uuid::parse_str(&req.buffer_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let sid =
        SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state.core.buffers.close(bid, sid).await;
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
struct LspPosReq {
    #[serde(flatten)]
    file: LspFileReq,
    line: u32,
    character: u32,
}

#[derive(Debug, Deserialize)]
struct LspRefsReq {
    #[serde(flatten)]
    pos: LspPosReq,
    include_declaration: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LspWorkspaceSymbolsReq {
    session_id: Option<String>,
    root_path: Option<String>,
    query: String,
}

#[derive(Debug, Deserialize)]
struct LspCodeActionsReq {
    #[serde(flatten)]
    file: LspFileReq,
    start_line: u32,
    start_character: u32,
    end_line: u32,
    end_character: u32,
}

#[derive(Debug, Deserialize)]
struct LspRangeReq {
    #[serde(flatten)]
    file: LspFileReq,
    start_line: u32,
    start_character: u32,
    end_line: u32,
    end_character: u32,
}

#[derive(Debug, Deserialize)]
struct LspResolveReq {
    #[serde(flatten)]
    file: LspFileReq,
    item: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct LspLineChar {
    line: u32,
    character: u32,
}

#[derive(Debug, Deserialize)]
struct LspSelectionRangesReq {
    #[serde(flatten)]
    file: LspFileReq,
    positions: Vec<LspLineChar>,
}

#[derive(Debug, Deserialize)]
struct LspSemanticTokensDeltaReq {
    #[serde(flatten)]
    file: LspFileReq,
    previous_result_id: String,
}

#[derive(Debug, Deserialize)]
struct LspWorkspaceSymbolResolveReq {
    session_id: Option<String>,
    root_path: Option<String>,
    item: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct LspExecuteCommandReq {
    #[serde(flatten)]
    file: LspFileReq,
    command: String,
    arguments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct LspCodeActionsByDiagnosticPlanReq {
    session_id: String,
    path: String,
    diagnostic: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct LspRenamePlanReq {
    session_id: String,
    path: String,
    line: u32,
    character: u32,
    new_name: String,
}

#[derive(Debug, Deserialize)]
struct LspFormatPlanReq {
    session_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct LspOrganizeImportsPlanReq {
    session_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct LspCodeActionPlanReq {
    session_id: String,
    action: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct EditPlanApplyReq {
    action: String, // "accept" | "reject"
    patch: String,
}

async fn lsp_definition(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .definition(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_type_definition(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_definition(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_implementation(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .implementation(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_references(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRefsReq>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.pos.file).await?;
    let out = state
        .core
        .lsp
        .references(
            &root,
            &file,
            lsp_types::Position {
                line: req.pos.line,
                character: req.pos.character,
            },
            req.include_declaration.unwrap_or(true),
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .into_iter()
        .filter_map(|l| serde_json::to_value(l).ok())
        .collect::<Vec<_>>();
    Ok(Json(out))
}

async fn lsp_hover(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .hover(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_signature_help(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .signature_help(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_completion(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .completion(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_completion_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .completion_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_code_action_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_action_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_inlay_hints(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRangeReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .inlay_hints(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: req.start_line,
                    character: req.start_character,
                },
                end: lsp_types::Position {
                    line: req.end_line,
                    character: req.end_character,
                },
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_document_highlight(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .document_highlight(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_selection_ranges(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspSelectionRangesReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let positions = req
        .positions
        .into_iter()
        .map(|p| lsp_types::Position {
            line: p.line,
            character: p.character,
        })
        .collect::<Vec<_>>();
    let v = state
        .core
        .lsp
        .selection_ranges(&root, &file, positions)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_call_hierarchy_prepare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_prepare(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_call_hierarchy_incoming(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_incoming(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_call_hierarchy_outgoing(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .call_hierarchy_outgoing(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_code_lens(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .code_lens(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_code_lens_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_lens_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_prepare_rename(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .prepare_rename(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_document_links(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .document_links(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_document_link_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .document_link_resolve(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_semantic_tokens_full(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .semantic_tokens_full(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_semantic_tokens_delta(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspSemanticTokensDeltaReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .semantic_tokens_delta(&root, &file, req.previous_result_id)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_folding_ranges(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .folding_ranges(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_linked_editing_range(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .linked_editing_range(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_type_hierarchy_prepare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspPosReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_prepare(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_type_hierarchy_supertypes(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_supertypes(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_type_hierarchy_subtypes(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .type_hierarchy_subtypes(&root, &file, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_code_actions_by_diagnostic_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionsByDiagnosticPlanReq>,
) -> Result<Json<Vec<crate::edit_plans::EditPlanSummary>>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;
    let file = crate::buffers::BufferStore::resolve_path(&root, &req.path)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid path".to_string(),
                }),
            )
        })?;

    let diag: lsp_types::Diagnostic =
        serde_json::from_value(req.diagnostic.clone()).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let actions = state
        .core
        .lsp
        .code_actions_typed(
            &root,
            &file,
            diag.range,
            vec![diag.clone()],
            Some(vec![lsp_types::CodeActionKind::QUICKFIX]),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let mut plans: Vec<crate::edit_plans::EditPlanSummary> = Vec::new();
    for action in actions {
        let (title, edit, preferred) = match action {
            lsp_types::CodeActionOrCommand::CodeAction(ca) => {
                let edit = ca.edit.or_else(|| {
                    ca.command
                        .as_ref()
                        .and_then(extract_workspace_edit_from_command)
                });
                (ca.title, edit, ca.is_preferred.unwrap_or(false))
            }
            lsp_types::CodeActionOrCommand::Command(cmd) => {
                let edit = extract_workspace_edit_from_command(&cmd);
                (cmd.title, edit, false)
            }
        };
        let Some(edit) = edit else { continue };
        let plan =
            crate::edit_plans::workspace_edit_to_plan(&root, &root, sid, worktree_id, title, edit)
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
        let summary = plan.to_summary();
        state.persist_edit_plan(&plan);
        state
            .workspaces
            .edit_plans
            .lock()
            .await
            .insert(plan.id, plan);
        if preferred {
            plans.insert(0, summary);
        } else {
            plans.push(summary);
        }
    }

    Ok(Json(plans))
}
async fn lsp_execute_command(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspExecuteCommandReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let (result, edit) = state
        .core
        .lsp
        .execute_command_for_file(&root, &file, req.command, req.arguments.unwrap_or_default())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let edit = edit
        .and_then(|e| serde_json::to_value(e).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(Json(serde_json::json!({
        "result": result,
        "workspace_edit": edit
    })))
}

async fn lsp_execute_command_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspExecuteCommandReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let session_id = req.file.session_id.clone().ok_or((
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: "session_id required".to_string(),
        }),
    ))?;
    let sid = SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;

    let (root, file) = resolve_lsp_target(&state, req.file).await.map_err(|sc| {
        (
            sc,
            Json(ApiErrorResp {
                error: "invalid LSP target".to_string(),
            }),
        )
    })?;

    let (result, edit) = state
        .core
        .lsp
        .execute_command_for_file(
            &root,
            &file,
            req.command.clone(),
            req.arguments.unwrap_or_default(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!(
                    "execute_command returned no WorkspaceEdit (result={})",
                    result
                ),
            }),
        ));
    };

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        format!("Execute command: {}", req.command),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

async fn lsp_document_symbols(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
        .core
        .lsp
        .document_symbols(&root, &file)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_workspace_symbols(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspWorkspaceSymbolsReq>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid =
            SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        let store = state
            .store_for_session(sid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let session = store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        PathBuf::from(wt.root_path)
    } else if let Some(root_path) = req.root_path.as_deref() {
        PathBuf::from(root_path)
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let root = root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?;

    let out = state
        .core
        .lsp
        .workspace_symbols(&root, req.query)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .into_iter()
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect::<Vec<_>>();
    Ok(Json(out))
}

async fn lsp_workspace_symbol_resolve(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspWorkspaceSymbolResolveReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid =
            SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        let store = state
            .store_for_session(sid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let session = store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        PathBuf::from(wt.root_path)
    } else if let Some(root_path) = req.root_path.as_deref() {
        PathBuf::from(root_path)
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let root = root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?;

    let out = state
        .core
        .lsp
        .workspace_symbol_resolve(&root, req.item)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(out))
}

async fn lsp_code_actions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionsReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.core.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
        .core
        .lsp
        .code_actions(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: req.start_line,
                    character: req.start_character,
                },
                end: lsp_types::Position {
                    line: req.end_line,
                    character: req.end_character,
                },
            },
            vec![],
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(v))
}

async fn lsp_rename_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspRenamePlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;
    let file = root.join(&req.path);
    let edit = state
        .core
        .lsp
        .rename(
            &root,
            &file,
            lsp_types::Position {
                line: req.line,
                character: req.character,
            },
            req.new_name,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        "Rename".to_string(),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

async fn lsp_format_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFormatPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;
    let file = root.join(&req.path);
    let edits = state
        .core
        .lsp
        .format_document(&root, &file)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let rel = file
        .strip_prefix(&root)
        .unwrap_or(&file)
        .to_string_lossy()
        .to_string();
    let plan = crate::edit_plans::text_edits_to_plan(
        &root,
        sid,
        worktree_id,
        "Format document".to_string(),
        rel,
        edits,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

async fn lsp_code_actions_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;

    let action: lsp_types::CodeActionOrCommand = serde_json::from_value(req.action.clone())
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let (title, edit) = match action {
        lsp_types::CodeActionOrCommand::CodeAction(ca) => {
            let edit = ca.edit.or_else(|| {
                ca.command
                    .as_ref()
                    .and_then(extract_workspace_edit_from_command)
            });
            (ca.title, edit)
        }
        lsp_types::CodeActionOrCommand::Command(cmd) => {
            let edit = extract_workspace_edit_from_command(&cmd);
            (cmd.title, edit)
        }
    };
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "code action has no edit".to_string(),
            }),
        ));
    };

    let plan =
        crate::edit_plans::workspace_edit_to_plan(&root, &root, sid, worktree_id, title, edit)
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

async fn lsp_organize_imports_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspOrganizeImportsPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp.enabled() || !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }

    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session_id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(sid).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    let worktree_id = session.worktree_id;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
    let root = PathBuf::from(wt.root_path).canonicalize().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid worktree root".to_string(),
            }),
        )
    })?;
    let file = root.join(&req.path);

    let text = tokio::fs::read_to_string(&file).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let (end_line, end_character) = text
        .split('\n')
        .enumerate()
        .fold((0u32, 0u32), |(_l, _c), (i, line)| {
            (i as u32, line.chars().count() as u32)
        });

    let actions = state
        .core
        .lsp
        .code_actions_typed(
            &root,
            &file,
            lsp_types::Range {
                start: lsp_types::Position {
                    line: 0,
                    character: 0,
                },
                end: lsp_types::Position {
                    line: end_line,
                    character: end_character,
                },
            },
            vec![],
            Some(vec![lsp_types::CodeActionKind::SOURCE_ORGANIZE_IMPORTS]),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let edit = actions.into_iter().find_map(|a| match a {
        lsp_types::CodeActionOrCommand::CodeAction(ca) => ca.edit.or_else(|| {
            ca.command
                .as_ref()
                .and_then(extract_workspace_edit_from_command)
        }),
        lsp_types::CodeActionOrCommand::Command(cmd) => extract_workspace_edit_from_command(&cmd),
    });
    let Some(edit) = edit else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "no organize-imports edit returned".to_string(),
            }),
        ));
    };

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        worktree_id,
        "Organize imports".to_string(),
        edit,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let summary = plan.to_summary();
    state.persist_edit_plan(&plan);
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan);
    Ok(Json(summary))
}

async fn list_edit_plans_for_worktree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::edit_plans::EditPlanSummary>>, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let map = state.workspaces.edit_plans.lock().await;
    let mut out = map
        .values()
        .filter(|p| p.worktree_id == worktree_id)
        .map(|p| p.to_summary())
        .collect::<Vec<_>>();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Json(out))
}

async fn get_edit_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, StatusCode> {
    let pid = crate::edit_plans::EditPlanId(
        uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let map = state.workspaces.edit_plans.lock().await;
    let Some(plan) = map.get(&pid) else {
        return Err(StatusCode::NOT_FOUND);
    };
    Ok(Json(plan.to_summary()))
}

async fn apply_edit_plan_patch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<EditPlanApplyReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.core.lsp_edit_plans_enabled {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "LSP edit plans disabled".to_string(),
            }),
        ));
    }
    let pid = crate::edit_plans::EditPlanId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid edit plan id".to_string(),
            }),
        )
    })?);
    if req.patch.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "patch is empty".to_string(),
            }),
        ));
    }

    let action = req.action.trim().to_lowercase();
    let patch = req.patch;

    match action.as_str() {
        "accept" => {
            let parsed = crate::edit_plans::parse_unified_diff(&patch);

            let (worktree_root, plan_files) = {
                let map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                (plan.worktree_root.clone(), plan.files.clone())
            };

            // Stale-plan check: ensure files match the base used to create the plan.
            for pf in &parsed {
                let rel = if !pf.new_path.is_empty() {
                    &pf.new_path
                } else {
                    &pf.old_path
                };
                let Some(base) = plan_files
                    .iter()
                    .find(|f| {
                        plan_paths_match(&pf.old_path, &pf.new_path, &f.old_path, &f.new_path)
                    })
                    .map(|f| f.base_sha256.clone())
                else {
                    continue;
                };
                if base.trim().is_empty() {
                    continue;
                }
                let abs = worktree_root.join(rel);
                let current = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                let current_sha = sha256_hex(&current);
                if current_sha != base {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ApiErrorResp {
                            error: format!("edit plan is stale for {}; regenerate the plan", rel),
                        }),
                    ));
                }
            }

            let worktree_root = {
                let map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.worktree_root.clone()
            };

            ctx_fs::git::git_apply_patch(
                worktree_root.to_string_lossy().as_ref(),
                &patch,
                ctx_fs::git::ApplyPatchTarget::Worktree,
                false,
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;

            // After applying, update base hashes for affected files so subsequent partial applies don't always look stale.
            let mut updated_bases: Vec<(String, String)> = Vec::new();
            for pf in &parsed {
                let rel = if !pf.new_path.is_empty() {
                    pf.new_path.clone()
                } else {
                    pf.old_path.clone()
                };
                let abs = worktree_root.join(&rel);
                let current = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                updated_bases.push((rel, sha256_hex(&current)));
            }

            let (summary, to_persist, removed) = {
                let mut map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get_mut(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.remove_patch(&patch);
                for (rel, sha) in &updated_bases {
                    for f in &mut plan.files {
                        let f_rel = if !f.new_path.is_empty() {
                            &f.new_path
                        } else {
                            &f.old_path
                        };
                        if f_rel == rel {
                            f.base_sha256 = sha.clone();
                        }
                    }
                }
                let summary = plan.to_summary();
                let removed = plan.files.is_empty();
                let to_persist = if removed { None } else { Some(plan.clone()) };
                if removed {
                    map.remove(&pid);
                }
                (summary, to_persist, removed)
            };
            if let Some(plan) = to_persist {
                state.persist_edit_plan(&plan);
            } else if removed {
                state.delete_edit_plan_file(pid);
            }
            Ok(Json(summary))
        }
        "reject" => {
            let (summary, to_persist, removed) = {
                let mut map = state.workspaces.edit_plans.lock().await;
                let Some(plan) = map.get_mut(&pid) else {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResp {
                            error: "edit plan not found".to_string(),
                        }),
                    ));
                };
                plan.remove_patch(&patch);
                let summary = plan.to_summary();
                let removed = plan.files.is_empty();
                let to_persist = if removed { None } else { Some(plan.clone()) };
                if removed {
                    map.remove(&pid);
                }
                (summary, to_persist, removed)
            };
            if let Some(plan) = to_persist {
                state.persist_edit_plan(&plan);
            } else if removed {
                state.delete_edit_plan_file(pid);
            }
            Ok(Json(summary))
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "action must be accept or reject".to_string(),
            }),
        )),
    }
}

async fn discard_edit_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let pid = crate::edit_plans::EditPlanId(
        uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    state.workspaces.edit_plans.lock().await.remove(&pid);
    state.delete_edit_plan_file(pid);
    Ok(StatusCode::NO_CONTENT)
}

async fn open_logs_folder(
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    logs::open_logs_folder(&state.core.data_root)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct DesktopLogReq {
    level: Option<String>,
    message: String,
}

async fn append_desktop_log(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DesktopLogReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let level = req.level.unwrap_or_else(|| "info".to_string());
    let line = format!(
        "{} [{level}] {}",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        req.message
    );
    logs::append_desktop_log_line(&state.core.data_root, &line)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
struct UpdateCheckResp {
    channel: String,
    base_url: String,
    platform: Option<String>,
    current_version: String,
    latest_version: Option<String>,
    update_available: bool,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    manifest: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct UpdateCheckQuery {
    #[serde(default)]
    channel: Option<String>,
}

async fn check_updates(
    State(_state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<UpdateCheckQuery>,
) -> Result<Json<UpdateCheckResp>, (StatusCode, Json<ApiErrorResp>)> {
    let channel = q.channel.unwrap_or_else(|| "stable".to_string());
    let base_url = updates::default_download_base_url();
    let platform = updates::platform_key().map(|s| s.to_string());
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    let query = platform.as_ref().map(|p| {
        vec![
            ("current_version", current_version.clone()),
            ("platform", p.clone()),
        ]
    });

    let manifest =
        updates::fetch_latest_manifest_with_params(&base_url, &channel, query.as_deref())
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;

    let latest_version = manifest.latest_version.clone();
    let update_available = match (
        updates::normalize_version_str(&current_version),
        updates::normalize_version_str(&latest_version),
    ) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => false,
    };

    Ok(Json(UpdateCheckResp {
        channel,
        base_url,
        platform,
        current_version,
        latest_version: Some(latest_version),
        update_available,
        manifest: serde_json::to_value(manifest).unwrap_or(serde_json::Value::Null),
    }))
}

#[derive(Debug, Deserialize)]
struct DownloadAppImageReq {
    #[serde(default)]
    channel: Option<String>,
}

#[derive(Debug, Serialize)]
struct DownloadAppImageResp {
    downloaded_path: String,
    can_apply_in_place: bool,
}

async fn download_appimage_update(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DownloadAppImageReq>,
) -> Result<Json<DownloadAppImageResp>, (StatusCode, Json<ApiErrorResp>)> {
    let channel = req.channel.unwrap_or_else(|| "stable".to_string());
    let base_url = updates::default_download_base_url();
    let platform = updates::platform_key().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "unsupported platform".to_string(),
            }),
        )
    })?;

    let manifest = updates::fetch_latest_manifest(&base_url, &channel)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let platform_entry = manifest.platforms.get(platform).ok_or_else(|| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: format!("manifest missing platform {platform}"),
            }),
        )
    })?;
    let appimage = platform_entry.appimage.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: "manifest missing appimage artifact".to_string(),
            }),
        )
    })?;

    let url = updates::join_url(&base_url, &appimage.url_path);
    let dest = updates::updates_dir(&state.core.data_root).join("ctx.AppImage.new");
    updates::download_and_verify(&url, &appimage.sha256, &dest)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(md) = tokio::fs::metadata(&dest).await {
            let mut p = md.permissions();
            p.set_mode(0o755);
            let _ = tokio::fs::set_permissions(&dest, p).await;
        }
    }

    Ok(Json(DownloadAppImageResp {
        downloaded_path: dest.to_string_lossy().to_string(),
        can_apply_in_place: updates::appimage_path_env().is_some(),
    }))
}

#[derive(Debug, Deserialize)]
struct ApplyAppImageReq {
    confirm: bool,
}

#[derive(Debug, Serialize)]
struct ApplyAppImageResp {
    applied: bool,
    target_path: Option<String>,
    message: String,
}

async fn apply_appimage_update(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApplyAppImageReq>,
) -> Result<Json<ApplyAppImageResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !req.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "confirm required".to_string(),
            }),
        ));
    }

    let Some(target) = updates::appimage_path_env() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_APPIMAGE_PATH not set; cannot apply in place".to_string(),
            }),
        ));
    };
    let downloaded = updates::updates_dir(&state.core.data_root).join("ctx.AppImage.new");
    if !downloaded.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "no downloaded update found; call download first".to_string(),
            }),
        ));
    }

    updates::atomic_replace_file(&target, &downloaded)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    Ok(Json(ApplyAppImageResp {
        applied: true,
        target_path: Some(target.to_string_lossy().to_string()),
        message:
            "Update applied in place. Quit and relaunch the desktop app to run the new version."
                .to_string(),
    }))
}

async fn perf_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let run_id = req
        .headers()
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let endpoint = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let parent = state
        .telemetry
        .perf_telemetry
        .extract_trace_context(req.headers());
    let span = state.telemetry.perf_telemetry.start_span(
        "http_request",
        SpanKind::Server,
        Some(parent),
        vec![
            KeyValue::new("http.method", method.clone()),
            KeyValue::new("http.route", endpoint.clone()),
        ],
    );
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let duration_ms = start.elapsed().as_millis() as u64;
    let success = status < 500;
    let mut labels = HashMap::new();
    labels.insert("endpoint".to_string(), endpoint);
    labels.insert("method".to_string(), method);
    labels.insert("status".to_string(), status.to_string());
    labels.insert("success".to_string(), success.to_string());
    labels.insert("source".to_string(), "daemon".to_string());
    let (trace_id, span_id) = state.telemetry.perf_telemetry.finish_span(
        span,
        Some(status.to_string()),
        Some(success),
        vec![KeyValue::new("http.status_code", status as i64)],
    );
    let metric = PerfMetric {
        name: "http.request.duration_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: duration_ms as f64,
        labels,
    };
    state
        .telemetry
        .perf_telemetry
        .record_metric(metric, run_id, trace_id, span_id)
        .await;
    response
}

async fn list_mobile_connection_profiles(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<Json<Vec<MobileConnectionProfile>>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let profiles = state
        .global_store()
        .list_mobile_connection_profiles()
        .await
        .map_err(|e| {
            tracing::error!("failed to list mobile profiles: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(profiles))
}

async fn create_mobile_connection_profile(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CreateMobileConnectionProfileReq>,
) -> Result<Json<CreateMobileConnectionProfileResp>, (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".into(),
            }),
        ));
    }
    let label = req.label.trim();
    if label.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "label is required".into(),
            }),
        ));
    }
    let base_url_raw = req.base_url.trim();
    if base_url_raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "base_url is required".into(),
            }),
        ));
    }
    let parsed = Url::parse(base_url_raw).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "base_url must be a valid URL".into(),
            }),
        )
    })?;
    if parsed.scheme() != "https" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "base_url must use https://".into(),
            }),
        ));
    }
    let normalized_base = parsed.as_str().trim_end_matches('/').to_string();
    let scopes: Vec<String> = req
        .scopes
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let token = generate_mobile_api_token();
    let token_hash = hash_api_token(&token);
    let token_prefix: String = token.chars().take(8).collect();
    let profile = state
        .global_store()
        .create_mobile_connection_profile(
            label.to_string(),
            normalized_base.clone(),
            token_hash,
            token_prefix,
            scopes,
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to create mobile profile: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to create profile".into(),
                }),
            )
        })?;
    let qr_payload = serde_json::json!({
        "connection_profile": {
            "label": profile.label,
            "connection": {
                "type": "direct_https",
                "base_url": normalized_base,
            },
            "auth": {
                "api_token": token,
            }
        },
        "label": profile.label,
        "baseUrl": normalized_base,
        "token": token,
    });
    Ok(Json(CreateMobileConnectionProfileResp {
        profile,
        token,
        qr_payload,
    }))
}

#[derive(Debug, Deserialize)]
struct EnableMobileAccessReq {
    supabase_token: String,
}

#[derive(Debug, Serialize)]
struct MobileAccessStatus {
    enabled: bool,
    tunnel_id: Option<String>,
    public_base_url: Option<String>,
    relay_base_url: Option<String>,
    daemon_public_key: Option<String>,
    tunnel_state: crate::mobile_tunnel::MobileTunnelState,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct EnableMobileAccessResp {
    status: MobileAccessStatus,
    qr_payload: serde_json::Value,
    pairing_expires_at: String,
}

#[derive(Debug, Deserialize)]
struct ControlPlaneEnableResp {
    tunnel_id: String,
    public_base_url: String,
    relay_base_url: String,
    tunnel_secret: String,
}

#[derive(Debug, Deserialize)]
struct PairMobileDeviceReq {
    pairing_token: String,
    device_id: String,
    device_label: Option<String>,
    platform: Option<String>,
    public_key: String,
    app_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct SecureEnvelope {
    device_id: String,
    seq: i64,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Deserialize)]
struct SecureRequestPayload {
    method: String,
    path: String,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    body_b64: String,
}

#[derive(Debug, Serialize)]
struct SecureResponsePayload {
    status: u16,
    headers: Vec<(String, String)>,
    body_b64: String,
}

#[derive(Debug, Deserialize)]
struct MobileSecureEnvelope {
    device_id: String,
    seq: i64,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Deserialize)]
struct MobileSecureStreamQuery {
    device_id: String,
}

const PAIRING_TOKEN_TTL_SECS: i64 = 10 * 60;
const DEFAULT_TUNNEL_CONTROL_PLANE_URL: &str = "https://tunnel.ctx.rs";

fn resolve_control_plane_url() -> String {
    std::env::var("CTX_TUNNEL_CONTROL_PLANE_URL")
        .unwrap_or_else(|_| DEFAULT_TUNNEL_CONTROL_PLANE_URL.to_string())
}

async fn get_mobile_access_status(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<Json<MobileAccessStatus>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let tunnel_status = state.transport.mobile_tunnel.status().await;
    let (enabled, tunnel_id, public_base_url, relay_base_url, daemon_public_key) = match cfg {
        Some(cfg) => (
            cfg.enabled,
            Some(cfg.tunnel_id),
            Some(cfg.public_base_url),
            Some(cfg.relay_base_url),
            Some(cfg.daemon_public_key),
        ),
        None => (false, None, None, None, None),
    };
    Ok(Json(MobileAccessStatus {
        enabled,
        tunnel_id,
        public_base_url,
        relay_base_url,
        daemon_public_key,
        tunnel_state: tunnel_status.state,
        last_error: tunnel_status.last_error,
    }))
}

async fn enable_mobile_access(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<EnableMobileAccessReq>,
) -> Result<Json<EnableMobileAccessResp>, (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".into(),
            }),
        ));
    }
    if state.core.auth_token.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "daemon auth token is not configured; refusing to expose daemon publicly"
                    .into(),
            }),
        ));
    }

    let control_plane_url = resolve_control_plane_url();
    if control_plane_url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_TUNNEL_CONTROL_PLANE_URL is not set".into(),
            }),
        ));
    }

    let enable_resp = reqwest::Client::new()
        .post(format!(
            "{}/v1/mobile/enable",
            control_plane_url.trim_end_matches('/')
        ))
        .bearer_auth(req.supabase_token.trim())
        .send()
        .await
        .map_err(|e| {
            tracing::error!("failed to call control plane: {e:?}");
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: "failed to reach control plane".into(),
                }),
            )
        })?;

    if !enable_resp.status().is_success() {
        let status = enable_resp.status();
        let body = enable_resp.text().await.unwrap_or_default();
        tracing::warn!("control plane denied enable: {status} {body}");
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorResp {
                error: "mobile access not entitled".into(),
            }),
        ));
    }

    let payload = enable_resp
        .json::<ControlPlaneEnableResp>()
        .await
        .map_err(|e| {
            tracing::error!("invalid control plane response: {e:?}");
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: "invalid control plane response".into(),
                }),
            )
        })?;

    let public_url = Url::parse(&payload.public_base_url).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "public_base_url must be a valid URL".into(),
            }),
        )
    })?;
    if public_url.scheme() != "https" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "public_base_url must use https://".into(),
            }),
        ));
    }

    let now = chrono::Utc::now();
    let (daemon_public_key, daemon_private_key, profile_id, created_at) = match state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to read mobile access config".into(),
                }),
            )
        })? {
        Some(cfg) => (
            cfg.daemon_public_key,
            cfg.daemon_private_key,
            cfg.profile_id,
            cfg.created_at,
        ),
        None => {
            let (public_key, private_key) = crate::mobile_e2ee::generate_keypair();
            let token = generate_mobile_api_token();
            let token_hash = hash_api_token(&token);
            let token_prefix: String = token.chars().take(8).collect();
            let profile = state
                .global_store()
                .create_mobile_connection_profile(
                    "Managed Mobile Access".to_string(),
                    public_url.as_str().trim_end_matches('/').to_string(),
                    token_hash,
                    token_prefix,
                    Vec::new(),
                )
                .await
                .map_err(|e| {
                    tracing::error!("failed to create managed mobile profile: {e:?}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "failed to create managed profile".into(),
                        }),
                    )
                })?;
            (public_key, private_key, profile.id, now)
        }
    };

    let config = ctx_store::store::MobileAccessConfig {
        id: "default".to_string(),
        profile_id,
        tunnel_id: payload.tunnel_id.clone(),
        public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
        relay_base_url: payload.relay_base_url.clone(),
        tunnel_secret: payload.tunnel_secret.clone(),
        daemon_public_key: daemon_public_key.clone(),
        daemon_private_key: daemon_private_key.clone(),
        enabled: true,
        created_at,
        updated_at: now,
    };

    state
        .global_store()
        .upsert_mobile_access_config(config)
        .await
        .map_err(|e| {
            tracing::error!("failed to persist mobile access config: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to persist mobile access config".into(),
                }),
            )
        })?;

    let pairing_token = generate_pairing_token();
    let pairing_hash = hash_pairing_token(&pairing_token);
    let expires_at = now + chrono::Duration::seconds(PAIRING_TOKEN_TTL_SECS);
    state
        .global_store()
        .insert_mobile_pairing_token(&uuid::Uuid::new_v4().to_string(), &pairing_hash, expires_at)
        .await
        .map_err(|e| {
            tracing::error!("failed to persist pairing token: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to persist pairing token".into(),
                }),
            )
        })?;

    let tunnel_cfg = crate::mobile_tunnel::StartMobileTunnelConfig {
        relay_base_url: payload.relay_base_url.clone(),
        tunnel_id: payload.tunnel_id.clone(),
        tunnel_secret: payload.tunnel_secret.clone(),
        public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
        local_daemon_url: state.core.daemon_url.trim_end_matches('/').to_string(),
    };
    if let Err(e) = state.transport.mobile_tunnel.start(tunnel_cfg).await {
        tracing::warn!("failed to start mobile tunnel: {e:#}");
    }

    let status = MobileAccessStatus {
        enabled: true,
        tunnel_id: Some(payload.tunnel_id.clone()),
        public_base_url: Some(public_url.as_str().trim_end_matches('/').to_string()),
        relay_base_url: Some(payload.relay_base_url.clone()),
        daemon_public_key: Some(daemon_public_key.clone()),
        tunnel_state: crate::mobile_tunnel::MobileTunnelState::Running,
        last_error: None,
    };

    let qr_payload = serde_json::json!({
        "type": "context_mobile_e2ee",
        "version": 1,
        "tunnel_id": payload.tunnel_id,
        "base_url": public_url.as_str().trim_end_matches('/'),
        "pairing_token": pairing_token,
        "daemon_public_key": daemon_public_key,
    });

    Ok(Json(EnableMobileAccessResp {
        status,
        qr_payload,
        pairing_expires_at: expires_at.to_rfc3339(),
    }))
}

async fn disable_mobile_access(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<EnableMobileAccessReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".into(),
            }),
        ));
    }

    let control_plane_url = resolve_control_plane_url();
    if !control_plane_url.trim().is_empty() {
        let _ = reqwest::Client::new()
            .post(format!(
                "{}/v1/mobile/revoke",
                control_plane_url.trim_end_matches('/')
            ))
            .bearer_auth(req.supabase_token.trim())
            .send()
            .await;
    }

    state.transport.mobile_tunnel.stop().await;
    let _ = state.global_store().set_mobile_access_enabled(false).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn pair_mobile_device(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: PairMobileDeviceReq = parse_json_body(body)?;
    let token_hash = hash_pairing_token(req.pairing_token.trim());
    let allowed = state
        .global_store()
        .consume_mobile_pairing_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to check pairing token: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to validate pairing token".into(),
                }),
            )
        })?;
    if !allowed {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "pairing token invalid or expired".into(),
            }),
        ));
    }

    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "mobile access not configured".into(),
                }),
            )
        })?;
    let Some(cfg) = cfg else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mobile access not enabled".into(),
            }),
        ));
    };

    let device_uuid = uuid::Uuid::parse_str(req.device_id.trim()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;

    let _device = state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(device_uuid),
            cfg.profile_id,
            MobileDeviceUpsert {
                device_label: req
                    .device_label
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                platform: req
                    .platform
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                push_token: None,
                push_provider: None,
                public_key: Some(req.public_key.trim().to_string()),
                app_version: req
                    .app_version
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            },
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to register device: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to register device".into(),
                }),
            )
        })?;

    let key = crate::mobile_e2ee::derive_key(
        &req.device_id,
        req.public_key.trim(),
        &cfg.daemon_private_key,
    )
    .map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "failed to derive pairing key".into(),
            }),
        )
    })?;

    let payload = serde_json::json!({
        "paired": true,
        "device_id": req.device_id,
        "daemon_public_key": cfg.daemon_public_key,
        "paired_at": chrono::Utc::now().to_rfc3339(),
    });
    let plaintext = serde_json::to_vec(&payload).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "failed to encode pairing response".into(),
            }),
        )
    })?;
    let envelope =
        crate::mobile_e2ee::encrypt(&key, &req.device_id, 0, &plaintext).map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to encrypt pairing response".into(),
                }),
            )
        })?;

    Ok(Json(SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    }))
}

async fn handle_mobile_secure(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: MobileSecureEnvelope = parse_json_body(body)?;
    let device_uuid = uuid::Uuid::parse_str(req.device_id.trim()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "mobile access not configured".into(),
                }),
            )
        })?;
    let Some(cfg) = cfg else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mobile access not enabled".into(),
            }),
        ));
    };
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile device: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to read device".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "unknown device".into(),
                }),
            )
        })?;
    if device.profile_id != cfg.profile_id {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "device not authorized for this tunnel".into(),
            }),
        ));
    }

    let Some(device_public_key) = device.public_key.as_ref() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device missing public key".into(),
            }),
        ));
    };
    let key =
        crate::mobile_e2ee::derive_key(&req.device_id, device_public_key, &cfg.daemon_private_key)
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "failed to derive device key".into(),
                    }),
                )
            })?;

    let plaintext =
        crate::mobile_e2ee::decrypt(&key, &req.device_id, req.seq, &req.nonce, &req.ciphertext)
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "failed to decrypt request".into(),
                    }),
                )
            })?;

    let payload: SecureRequestPayload = serde_json::from_slice(&plaintext).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid secure request payload".into(),
            }),
        )
    })?;

    if payload.path.starts_with("/api/mobile/secure")
        || payload.path.starts_with("/api/mobile/pair")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "secure proxy cannot target mobile secure endpoints".into(),
            }),
        ));
    }

    let last_seen = state
        .global_store()
        .update_mobile_device_seq(MobileDeviceId(device_uuid), req.seq)
        .await
        .map_err(|e| {
            tracing::error!("failed to update device seq: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to update device".into(),
                }),
            )
        })?;
    if let Some(last) = last_seen {
        if req.seq <= last {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "stale request sequence".into(),
                }),
            ));
        }
    }

    let response_payload = proxy_secure_request(&state, device.profile_id, payload)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ApiErrorResp { error: e })))?;

    let response_bytes = serde_json::to_vec(&response_payload).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "failed to encode secure response".into(),
            }),
        )
    })?;
    let envelope = crate::mobile_e2ee::encrypt(&key, &req.device_id, req.seq, &response_bytes)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to encrypt response".into(),
                }),
            )
        })?;

    Ok(Json(SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    }))
}

async fn proxy_secure_request(
    state: &Arc<AppState>,
    profile_id: ConnectionProfileId,
    mut payload: SecureRequestPayload,
) -> Result<SecureResponsePayload, String> {
    if let Some((path, query)) = payload.path.split_once('?') {
        let path = path.to_string();
        let query = query.to_string();
        payload.path = path;
        if payload.query.is_none() {
            payload.query = Some(query);
        }
    }
    let path = payload.path.trim().to_string();
    if !path.starts_with("/api/") {
        return Err("secure proxy only supports /api/* paths".to_string());
    }

    let method = axum::http::Method::from_bytes(payload.method.as_bytes())
        .map_err(|_| "invalid http method".to_string())?;
    let mut uri = path;
    if let Some(query) = payload
        .query
        .as_ref()
        .map(|q| q.trim())
        .filter(|q| !q.is_empty())
    {
        uri.push('?');
        uri.push_str(query.trim_start_matches('?'));
    }

    let body = decode_body_b64(&payload.body_b64)?;
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in payload.headers {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        let Ok(header_name) = header::HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(header_value) = header::HeaderValue::from_str(&value) else {
            continue;
        };
        builder = builder.header(header_name, header_value);
    }

    let mut req = builder
        .body(Body::from(body))
        .map_err(|_| "failed to build proxied request".to_string())?;
    req.extensions_mut()
        .insert(MobileAuthContext { profile_id });

    let app = router(state.clone());
    let resp = app
        .oneshot(req)
        .await
        .map_err(|_| "failed to proxy request".to_string())?;

    let status = resp.status().as_u16();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|_| "failed to read proxied response".to_string())?;
    let body_b64 = base64::engine::general_purpose::STANDARD.encode(body_bytes);
    Ok(SecureResponsePayload {
        status,
        headers,
        body_b64,
    })
}

fn parse_json_body<T: serde::de::DeserializeOwned>(
    body: Bytes,
) -> Result<T, (StatusCode, Json<ApiErrorResp>)> {
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "missing request body".into(),
            }),
        ));
    }
    serde_json::from_slice(&body).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid json body".into(),
            }),
        )
    })
}

fn decode_body_b64(value: &str) -> Result<Vec<u8>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut normalized = trimmed.replace('-', "+").replace('_', "/");
    while !normalized.len().is_multiple_of(4) {
        normalized.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .map_err(|_| "invalid base64 body".to_string())
}

async fn delete_mobile_connection_profile(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .global_store()
        .delete_mobile_connection_profile(ConnectionProfileId(uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to delete mobile profile: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_mobile_devices_for_profile(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<MobileDeviceRegistration>>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let devices = state
        .global_store()
        .list_mobile_devices(ConnectionProfileId(uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to list mobile devices: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(devices))
}

async fn register_mobile_device(
    State(state): State<Arc<AppState>>,
    auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<RegisterMobileDeviceReq>,
) -> Result<Json<MobileDeviceRegistration>, (StatusCode, Json<ApiErrorResp>)> {
    let Some(Extension(mobile_auth)) = auth else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "mobile token required".into(),
            }),
        ));
    };
    let device_uuid = uuid::Uuid::parse_str(req.device_id.trim()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;
    let sanitize = |input: Option<String>| -> Option<String> {
        input
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let device = state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(device_uuid),
            mobile_auth.profile_id,
            MobileDeviceUpsert {
                device_label: sanitize(req.device_label),
                platform: sanitize(req.platform),
                push_token: sanitize(req.push_token),
                push_provider: sanitize(req.push_provider),
                public_key: sanitize(req.public_key),
                app_version: sanitize(req.app_version),
            },
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to register mobile device: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to register device".into(),
                }),
            )
        })?;
    Ok(Json(device))
}

#[derive(Debug, Deserialize)]
struct WebSessionCreatePayload {
    session_id: Option<String>,
    worktree_id: Option<String>,
    url: String,
    viewport: Option<WebSessionViewport>,
    fps: Option<u32>,
}

async fn create_web_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<WebSessionCreatePayload>,
) -> Result<Json<WebSessionInfo>, (StatusCode, Json<ApiErrorResp>)> {
    if payload.url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "url is required".to_string(),
            }),
        ));
    }

    let session_id = payload.session_id.clone();
    let worktree_id = payload.worktree_id.clone();
    let work_dir = resolve_web_session_work_dir(&state, session_id.clone(), worktree_id.clone())
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;

    let node_runtime = crate::installer::ensure_node_runtime(
        state.as_ref(),
        None,
        "web_session_worker",
        &state.core.data_root,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!("failed to prepare node runtime: {e}"),
            }),
        )
    })?;

    let worker_bundle =
        crate::web_sessions::ensure_worker_bundle(&state.core.data_root, &node_runtime)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: format!("failed to prepare web session worker: {e}"),
                    }),
                )
            })?;

    let req = WebSessionCreateRequest {
        url: payload.url,
        viewport: payload.viewport,
        fps: payload.fps,
        work_dir,
        session_id,
        worktree_id,
        node_bin: node_runtime.node_bin,
        worker_path: worker_bundle.worker_path,
        node_modules_path: worker_bundle.node_modules_path,
    };

    let handle = state
        .transport
        .web_sessions
        .create(req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!("failed to create web session: {e}"),
                }),
            )
        })?;

    let mut info = handle.snapshot().await;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    info.stream_url = Some(format!("{}{}", base_url, info.stream_path));
    Ok(Json(info))
}

async fn list_web_sessions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<WebSessionInfo>>, StatusCode> {
    let mut sessions = state.transport.web_sessions.list().await;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    for session in sessions.iter_mut() {
        session.stream_url = Some(format!("{}{}", base_url, session.stream_path));
    }
    Ok(Json(sessions))
}

async fn get_web_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<WebSessionInfo>, StatusCode> {
    let handle = state
        .transport
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut info = handle.snapshot().await;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    info.stream_url = Some(format!("{}{}", base_url, info.stream_path));
    Ok(Json(info))
}

async fn run_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut payload): Json<WebSessionRunRequest>,
) -> Result<Json<WebSessionRunResponse>, StatusCode> {
    if payload.timeout_ms.is_none() {
        payload.timeout_ms = Some(5 * 60 * 1000);
    }
    let resp = state
        .transport
        .web_sessions
        .run(&id, payload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(resp))
}

async fn eval_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut payload): Json<WebSessionRunRequest>,
) -> Result<Json<WebSessionRunResponse>, StatusCode> {
    if payload.timeout_ms.is_none() {
        payload.timeout_ms = Some(5 * 60 * 1000);
    }
    let resp = state
        .transport
        .web_sessions
        .eval(&id, payload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(resp))
}

async fn close_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    state
        .transport
        .web_sessions
        .close(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn web_session_view(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let handle = state
        .transport
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let info = handle.snapshot().await;
    let body = render_web_session_view(&info);
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response())
}

async fn resolve_web_session_work_dir(
    state: &Arc<AppState>,
    session_id: Option<String>,
    worktree_id: Option<String>,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(worktree_id) = worktree_id {
        let worktree_id =
            WorktreeId(uuid::Uuid::parse_str(&worktree_id).context("invalid worktree id")?);
        let store = state.store_for_worktree(worktree_id).await?;
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(Some(PathBuf::from(worktree.root_path)));
    }
    if let Some(session_id) = session_id {
        let session_id =
            SessionId(uuid::Uuid::parse_str(&session_id).context("invalid session id")?);
        let store = state.store_for_session(session_id).await?;
        let session = store
            .get_session(session_id)
            .await?
            .context("session not found")?;
        let worktree = store
            .get_worktree(session.worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(Some(PathBuf::from(worktree.root_path)));
    }
    Ok(None)
}

#[derive(Debug, Deserialize)]
struct TelemetrySummaryQuery {
    metric: Option<String>,
    run_id: Option<String>,
    window_ms: Option<u64>,
    limit: Option<u32>,
}

async fn get_telemetry_summary(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TelemetrySummaryQuery>,
) -> Result<Json<crate::perf_telemetry::PerfSummary>, StatusCode> {
    let limit = q.limit.map(|v| v as usize);
    let summary = state.telemetry.perf_telemetry.summary(
        q.metric.as_deref(),
        q.run_id.as_deref(),
        q.window_ms,
        limit,
    );
    Ok(Json(summary))
}

#[derive(Debug, Deserialize)]
struct TelemetryExportQuery {
    date: Option<String>,
}

async fn export_telemetry(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TelemetryExportQuery>,
) -> Result<Response, StatusCode> {
    let date = q
        .date
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let path = crate::perf_telemetry::perf_log_path_for_date(&state.core.data_root, &date);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    Ok(resp)
}

#[derive(Debug, Deserialize)]
struct ClientTelemetryMetric {
    name: String,
    kind: PerfMetricKind,
    unit: String,
    value: f64,
    labels: Option<HashMap<String, String>>,
    run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClientTelemetryBatch {
    events: Vec<ClientTelemetryMetric>,
}

async fn post_client_telemetry(
    State(state): State<Arc<AppState>>,
    Json(batch): Json<ClientTelemetryBatch>,
) -> Result<StatusCode, StatusCode> {
    for event in batch.events {
        let mut labels = event.labels.unwrap_or_default();
        labels.insert("source".to_string(), "client".to_string());
        let metric = PerfMetric {
            name: event.name,
            kind: event.kind,
            unit: event.unit,
            value: event.value,
            labels,
        };
        state
            .telemetry
            .perf_telemetry
            .record_metric(metric, event.run_id, None, None)
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
