use std::collections::{HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context};
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, MatchedPath, Multipart, Path, Query, State};
use axum::http::header;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::Json;
use base64::Engine;
use futures::{SinkExt, Stream, StreamExt};
use opentelemetry::trace::SpanKind;
use opentelemetry::KeyValue;
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};
use tokio_util::io::ReaderStream;
use tower::util::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};
use url::Url;

use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_fs::git::{assert_git_repo, list_tracked_files, list_untracked_files, rev_parse_head};
use ctx_fs::worktrees::{create_worktree, managed_worktree_path};
use ctx_store::store::MobileDeviceUpsert;

use crate::attachments;
use crate::buffers::{
    BufferCloseReq, BufferConflictResp, BufferId, BufferOpenReq, BufferOpenResp, BufferUpdateReq,
    BufferUpdateResp,
};
use crate::completions;
use crate::daemon::AppState;
use crate::dictation_livekit;
use crate::installer;
use crate::installs::{InstallId, InstallInfo, InstallProgressEvent};
use crate::logs;
use crate::merge_queue;
use crate::ops_events::OpsEvent;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::provider_guard;
use crate::resource_governance;
use crate::resource_utilization;
use crate::scheduler::SchedulerCommand;
use crate::settings as user_settings;
use crate::telemetry::{TelemetryConfig, TelemetryEvent};
use crate::terminals::{TerminalClientMessage, TerminalCreateRequest, TerminalServerMessage};
use crate::title_generation;
use crate::updates;
use crate::web_sessions::{
    render_web_session_view, WebSessionCreateRequest, WebSessionInfo, WebSessionRunRequest,
    WebSessionRunResponse, WebSessionViewport,
};
use crate::workspace_config;
use crate::worktree_bootstrap;
use ctx_providers::adapters::ProviderStatus;
use ctx_providers::events::NormalizedEvent;
use ctx_providers::{
    acp::{
        authenticate_provider, probe_provider_options, verify_provider_connection, AcpAgentConfig,
        AcpClientConfig,
    },
    ask_user_question::{AskUserQuestionAnswer, AskUserQuestionOutcome},
};

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("authorization")
        || (key.contains("api") && key.contains("key"))
}

fn redact_json_value(value: serde_json::Value) -> serde_json::Value {
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
            "/api/workspaces/:id/terminals",
            get(list_workspace_terminals).post(create_workspace_terminal),
        )
        .route(
            "/api/workspaces/:id/active_snapshot",
            get(get_workspace_active_snapshot),
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
            "/api/workspaces/:id/providers/:provider_id/authenticate",
            post(authenticate_provider_for_workspace),
        )
        .route(
            "/api/workspaces/:id/providers/:provider_id/verify",
            post(verify_provider_for_workspace),
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
        .route("/api/sessions/:id/diff", get(get_session_diff))
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
        .route("/api/mcp/sessions/:id/agent_init", post(mcp_agent_init))
        .route("/api/mcp/sessions/:id/agent_reply", post(mcp_agent_reply))
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

async fn get_worktree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Worktree>, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match state
        .store
        .get_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(wt) => Ok(Json(wt)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_worktree_bootstrap_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let worktree = state
        .store
        .get_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(path) = worktree.bootstrap_log_path.as_deref() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let filename = format!("worktree-bootstrap-{}.log", worktree_id.0);
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

    let params = merge_queue::MergeQueueSubmitParams {
        session_id,
        worktree_id,
        target_branch: req.target_branch,
        message: req.message,
        patch: req.patch,
        base_commit_sha: req.base_commit_sha,
        head_commit_sha: req.head_commit_sha,
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
    let entries = state
        .store
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
    let run = state
        .store
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

#[derive(Debug, Serialize)]
struct HealthResp {
    version: String,
    pid: u32,
    data_root: String,
    daemon_url: String,
    auth_required: bool,
}

#[derive(Debug, Serialize)]
struct ApiErrorResp {
    error: String,
}

#[derive(Debug, Deserialize)]
struct MergeQueueSubmitReq {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    target_branch: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    base_commit_sha: Option<String>,
    #[serde(default)]
    head_commit_sha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MergeQueueListParams {
    workspace_id: String,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Clone, Copy)]
struct MobileAuthContext {
    profile_id: ConnectionProfileId,
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

#[derive(Debug, Deserialize)]
struct UpdateTaskTitleReq {
    title: String,
}

#[derive(Debug, Deserialize)]
struct CreateTerminalReq {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    shell: Option<String>,
}

#[derive(Debug, Serialize)]
struct BlobUploadResp {
    blob_id: String,
    sha256: String,
    bytes: i64,
    mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

async fn health(State(state): State<Arc<AppState>>) -> Result<Json<HealthResp>, StatusCode> {
    Ok(Json(HealthResp {
        version: env!("CARGO_PKG_VERSION").to_string(),
        pid: std::process::id(),
        data_root: state.data_root.to_string_lossy().to_string(),
        daemon_url: state.daemon_url.clone(),
        auth_required: state.auth_token.is_some(),
    }))
}

fn blobs_dir(data_root: &StdPath) -> PathBuf {
    data_root.join("blobs")
}

async fn persist_blob_bytes(
    state: &AppState,
    bytes: &[u8],
    mime_type: &str,
    name: Option<&str>,
) -> Result<BlobUploadResp, StatusCode> {
    const MAX_BLOB_BYTES: usize = 25 * 1024 * 1024;
    if bytes.len() > MAX_BLOB_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    if !mime_type.starts_with("image/") {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let sha256 = hex::encode(hasher.finalize());

    let blob_id = uuid::Uuid::new_v4().to_string();

    let dir = blobs_dir(&state.data_root);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let path = dir.join(&blob_id);
    let tmp = dir.join(format!("{blob_id}.tmp"));

    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    state
        .store
        .insert_blob(
            &blob_id,
            &sha256,
            bytes.len() as i64,
            mime_type,
            name,
            chrono::Utc::now(),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(BlobUploadResp {
        blob_id,
        sha256,
        bytes: bytes.len() as i64,
        mime_type: mime_type.to_string(),
        name: name.map(|s| s.to_string()),
    })
}

async fn upload_blob(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<BlobUploadResp>, StatusCode> {
    let mut file_name: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut bytes: Option<Bytes> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        if name != "file" {
            continue;
        }
        file_name = field.file_name().map(|s| s.to_string());
        mime_type = field.content_type().map(|s| s.to_string());
        let b = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
        bytes = Some(b);
        break;
    }

    let Some(bytes) = bytes else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let mime_type = mime_type.unwrap_or_else(|| "application/octet-stream".to_string());
    let resp = persist_blob_bytes(&state, &bytes, &mime_type, file_name.as_deref()).await?;
    Ok(Json(resp))
}

async fn get_blob(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let Some((_sha256, mime_type, _bytes, name, _created_at)) = state
        .store
        .get_blob(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    let path = blobs_dir(&state.data_root).join(&id);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let stream = ReaderStream::new(file);
    let mut resp = Response::new(Body::from_stream(stream));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        mime_type
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    if let Some(name) = name {
        let value = format!("inline; filename=\"{}\"", name.replace('"', ""));
        if let Ok(v) = value.parse() {
            resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(resp)
}

fn normalize_artifact_name(name: Option<String>, path: &StdPath) -> Option<String> {
    if let Some(name) = name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn infer_artifact_mime_type(path: &StdPath, override_value: Option<String>) -> String {
    if let Some(value) = override_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

fn artifact_is_quicktime(path: &StdPath, mime_type: &str) -> bool {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "mov" {
        return true;
    }
    mime_type.trim().eq_ignore_ascii_case("video/quicktime")
}

fn parse_range_header(range: Option<&HeaderValue>, size: u64) -> Option<(u64, u64)> {
    let header = range?.to_str().ok()?.trim().to_string();
    let range = header.strip_prefix("bytes=")?;
    let (start_raw, end_raw) = range.split_once('-')?;
    if start_raw.is_empty() {
        let suffix: u64 = end_raw.parse().ok()?;
        if suffix == 0 || size == 0 {
            return None;
        }
        let start = size.saturating_sub(suffix);
        let end = size.saturating_sub(1);
        return Some((start, end));
    }
    let start: u64 = start_raw.parse().ok()?;
    if start >= size {
        return None;
    }
    let end = if end_raw.is_empty() {
        size.saturating_sub(1)
    } else {
        end_raw.parse::<u64>().ok()?.min(size.saturating_sub(1))
    };
    if start > end {
        return None;
    }
    Some((start, end))
}

async fn get_artifact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let artifact_id = ArtifactId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let Some(artifact) = state
        .store
        .get_artifact(artifact_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    let path = PathBuf::from(&artifact.absolute_path);
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !meta.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }
    let size = meta.len();
    let maybe_range = parse_range_header(headers.get(header::RANGE), size);

    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let (status, body, content_length, content_range) = if let Some((start, end)) = maybe_range {
        file.seek(SeekFrom::Start(start))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let len = end.saturating_sub(start).saturating_add(1);
        let stream = ReaderStream::new(file.take(len));
        (
            StatusCode::PARTIAL_CONTENT,
            Body::from_stream(stream),
            len,
            Some(format!("bytes {}-{}/{}", start, end, size)),
        )
    } else {
        let stream = ReaderStream::new(file);
        (StatusCode::OK, Body::from_stream(stream), size, None)
    };

    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        artifact
            .mime_type
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    resp.headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        resp.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    if let Some(content_range) = content_range {
        if let Ok(value) = HeaderValue::from_str(&content_range) {
            resp.headers_mut().insert(header::CONTENT_RANGE, value);
        }
    }
    if let Some(name) = artifact.name {
        let value = format!("inline; filename=\"{}\"", name.replace('"', ""));
        if let Ok(v) = value.parse() {
            resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(resp)
}

async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let settings = user_settings::load_settings(&state.data_root).await;
    let mut public = user_settings::to_public(&settings);
    public.resource_governance =
        resource_governance::build_public_settings(&state, &settings).await;
    Ok(Json(public))
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<user_settings::UpdateSettingsReq>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let current = user_settings::load_settings(&state.data_root).await;
    let next = user_settings::apply_update(current, req);
    user_settings::save_settings(&state.data_root, &next)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut telemetry_cfg = TelemetryConfig::default();
    if let Some(telemetry) = next.telemetry.as_ref() {
        telemetry_cfg.enabled = telemetry.enabled;
        if !telemetry.endpoint.trim().is_empty() {
            telemetry_cfg.endpoint = telemetry.endpoint.clone();
        }
    }
    state.telemetry.update_config(telemetry_cfg).await;
    let perf_enabled = next.telemetry.as_ref().map(|t| t.enabled).unwrap_or(true);
    state
        .perf_telemetry
        .update_remote_enabled(perf_enabled)
        .await;
    if let Err(err) = resource_governance::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply resource governance settings: {err:#}");
    }
    if let Err(err) = provider_guard::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply provider guard settings: {err:#}");
    }
    let mut public = user_settings::to_public(&next);
    public.resource_governance = resource_governance::build_public_settings(&state, &next).await;
    Ok(Json(public))
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
        let map = state.provider_statuses.lock().await;
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

    let log_files = logs::list_log_files(&state.data_root).await;
    let managed_installs = installer::load_agent_server_config(&state.data_root)
        .await
        .map(|cfg| serde_json::to_value(cfg).unwrap_or_else(|_| serde_json::json!({})))
        .unwrap_or_else(|e| serde_json::json!({"error": logs::redact_sensitive(&e.to_string())}));
    let managed_installs = redact_json_value(managed_installs);

    Ok(Json(DiagnosticsResp {
        daemon: HealthResp {
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
            data_root: state.data_root.to_string_lossy().to_string(),
            daemon_url: state.daemon_url.clone(),
            auth_required: state.auth_token.is_some(),
        },
        platform: serde_json::json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        }),
        logs: serde_json::json!({
            "dir": logs::logs_dir(&state.data_root).to_string_lossy(),
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
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&query.workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let workspace = state
        .store
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktrees = state
        .store
        .list_worktrees(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let provider_adapters = {
        let providers = state.providers.lock().await;
        providers.values().cloned().collect::<Vec<_>>()
    };
    let mut provider_processes = Vec::new();
    for adapter in provider_adapters {
        provider_processes.extend(adapter.list_processes().await);
    }

    let (system, disks, cache_age_ms, processes, disk_cache) = {
        let mut sampler = state.resource_sampler.lock().await;
        let (system, disks, cache_age_ms) = sampler.system_snapshot();
        let processes = sampler.processes_snapshot(std::process::id(), &provider_processes);
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
        let mut sampler = state.resource_sampler.lock().await;
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
    let cfg = &state.lsp_cfg;
    let enabled = cfg.enabled;
    let edit_plans_enabled = state.lsp_edit_plans_enabled;

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
    let catalog = crate::lsp_catalog::load_catalog(&state.data_root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let installed = installer::load_lsp_server_config(&state.data_root)
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
    if crate::lsp_catalog::get_entry(&state.data_root, &id)
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let (root, file) = resolve_lsp_target(&state, req).await?;
    let diags = state
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
        let session = state
            .store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = state
            .store
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

    let session = state
        .store
        .get_session(sid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let wt = state
        .store
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
    if state.lsp.enabled() {
        if let Some(lang) = ctx_lsp::Language::detect(&file, &state.lsp_cfg) {
            state
                .ensure_lsp_diagnostics_forwarder(root.clone(), lang)
                .await;
        }
        let _ = state
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
    let current = state.buffers.get(bid).await.ok_or((
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

    if state.lsp.enabled() {
        if let Some(lang) = ctx_lsp::Language::detect(&st.path, &state.lsp_cfg) {
            state
                .ensure_lsp_diagnostics_forwarder(st.root.clone(), lang)
                .await;
        }
        let _ = state
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
    state.buffers.close(bid, sid).await;
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.pos.file).await?;
    let out = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() || !state.lsp_edit_plans_enabled {
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

    let session = state
        .store
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
    let wt = state
        .store
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
        state.edit_plans.lock().await.insert(plan.id, plan);
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let (result, edit) = state
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
    if !state.lsp.enabled() || !state.lsp_edit_plans_enabled {
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

    let session = state
        .store
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
    state.edit_plans.lock().await.insert(plan.id, plan);
    Ok(Json(summary))
}

async fn lsp_document_symbols(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFileReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req).await?;
    let v = state
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid =
            SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        let session = state
            .store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = state
            .store
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }

    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid =
            SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        let session = state
            .store
            .get_session(sid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let wt = state
            .store
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
    if !state.lsp.enabled() {
        return Err(StatusCode::CONFLICT);
    }
    let (root, file) = resolve_lsp_target(&state, req.file).await?;
    let v = state
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
    if !state.lsp.enabled() || !state.lsp_edit_plans_enabled {
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

    let session = state
        .store
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
    let wt = state
        .store
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
    state.edit_plans.lock().await.insert(plan.id, plan);
    Ok(Json(summary))
}

async fn lsp_format_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspFormatPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.lsp.enabled() || !state.lsp_edit_plans_enabled {
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

    let session = state
        .store
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
    let wt = state
        .store
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
    let edits = state.lsp.format_document(&root, &file).await.map_err(|e| {
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
    state.edit_plans.lock().await.insert(plan.id, plan);
    Ok(Json(summary))
}

fn extract_workspace_edit_from_command(
    cmd: &lsp_types::Command,
) -> Option<lsp_types::WorkspaceEdit> {
    let args = cmd.arguments.as_ref()?;
    for arg in args {
        if let Ok(edit) = serde_json::from_value::<lsp_types::WorkspaceEdit>(arg.clone()) {
            let has_edits = edit
                .changes
                .as_ref()
                .map(|m| !m.is_empty())
                .unwrap_or(false)
                || edit.document_changes.is_some();
            if has_edits {
                return Some(edit);
            }
        }
        if let Some(obj) = arg.as_object() {
            for key in ["edit", "workspaceEdit"] {
                if let Some(val) = obj.get(key) {
                    if let Ok(edit) =
                        serde_json::from_value::<lsp_types::WorkspaceEdit>(val.clone())
                    {
                        let has_edits = edit
                            .changes
                            .as_ref()
                            .map(|m| !m.is_empty())
                            .unwrap_or(false)
                            || edit.document_changes.is_some();
                        if !has_edits {
                            continue;
                        }
                        return Some(edit);
                    }
                }
            }
        }
    }
    None
}

async fn lsp_code_actions_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspCodeActionPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.lsp.enabled() || !state.lsp_edit_plans_enabled {
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

    let session = state
        .store
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
    let wt = state
        .store
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
    state.edit_plans.lock().await.insert(plan.id, plan);
    Ok(Json(summary))
}

async fn lsp_organize_imports_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspOrganizeImportsPlanReq>,
) -> Result<Json<crate::edit_plans::EditPlanSummary>, (StatusCode, Json<ApiErrorResp>)> {
    if !state.lsp.enabled() || !state.lsp_edit_plans_enabled {
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

    let session = state
        .store
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
    let wt = state
        .store
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
    state.edit_plans.lock().await.insert(plan.id, plan);
    Ok(Json(summary))
}

async fn list_edit_plans_for_worktree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::edit_plans::EditPlanSummary>>, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let map = state.edit_plans.lock().await;
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
    let map = state.edit_plans.lock().await;
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
    if !state.lsp_edit_plans_enabled {
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
                let map = state.edit_plans.lock().await;
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
                let map = state.edit_plans.lock().await;
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
                let mut map = state.edit_plans.lock().await;
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
                let mut map = state.edit_plans.lock().await;
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
    state.edit_plans.lock().await.remove(&pid);
    state.delete_edit_plan_file(pid);
    Ok(StatusCode::NO_CONTENT)
}

async fn open_logs_folder(
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    logs::open_logs_folder(&state.data_root)
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
    logs::append_desktop_log_line(&state.data_root, &line)
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
    let dest = updates::updates_dir(&state.data_root).join("ctx.AppImage.new");
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
    let downloaded = updates::updates_dir(&state.data_root).join("ctx.AppImage.new");
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
    let parent = state.perf_telemetry.extract_trace_context(req.headers());
    let span = state.perf_telemetry.start_span(
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
    let (trace_id, span_id) = state.perf_telemetry.finish_span(
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
        .perf_telemetry
        .record_metric(metric, run_id, trace_id, span_id)
        .await;
    response
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    let path = req.uri().path();
    if !path.starts_with("/api/") || path == "/api/health" {
        return Ok(next.run(req).await);
    }
    if path.starts_with("/api/mobile/secure") || path == "/api/mobile/pair" {
        return Ok(next.run(req).await);
    }
    if state.auth_token.is_none() {
        return Ok(next.run(req).await);
    }
    if req.extensions().get::<MobileAuthContext>().is_some() {
        return Ok(next.run(req).await);
    }

    let mut token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.to_string());

    if token.is_none() {
        token = req.uri().query().and_then(|q| {
            q.split('&').find_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                if k == "token" {
                    Some(v.to_string())
                } else {
                    None
                }
            })
        });
    }

    if token.as_deref() == state.auth_token.as_deref() {
        return Ok(next.run(req).await);
    }
    if let Some(token_value) = token {
        if let Some(profile_id) = verify_mobile_api_token(&state, &token_value).await? {
            req.extensions_mut()
                .insert(MobileAuthContext { profile_id });
            return Ok(next.run(req).await);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

async fn list_mobile_connection_profiles(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<Json<Vec<MobileConnectionProfile>>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let profiles = state
        .store
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
        .store
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
    let cfg = state.store.get_mobile_access_config().await.map_err(|e| {
        tracing::error!("failed to read mobile access config: {e:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let tunnel_status = state.mobile_tunnel.status().await;
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
    if state.auth_token.is_none() {
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
    let (daemon_public_key, daemon_private_key, profile_id, created_at) =
        match state.store.get_mobile_access_config().await.map_err(|e| {
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
                    .store
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
        .store
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
        .store
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
        local_daemon_url: state.daemon_url.trim_end_matches('/').to_string(),
    };
    if let Err(e) = state.mobile_tunnel.start(tunnel_cfg).await {
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

    state.mobile_tunnel.stop().await;
    let _ = state.store.set_mobile_access_enabled(false).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn pair_mobile_device(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: PairMobileDeviceReq = parse_json_body(body)?;
    let token_hash = hash_pairing_token(req.pairing_token.trim());
    let allowed = state
        .store
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

    let cfg = state.store.get_mobile_access_config().await.map_err(|e| {
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
        .store
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
    let cfg = state.store.get_mobile_access_config().await.map_err(|e| {
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
        .store
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
        .store
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

async fn mobile_secure_workspace_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<MobileSecureStreamQuery>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let device_id = query.device_id.trim().to_string();
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_mobile_secure_ws(socket, state, workspace_id, device_id).await {
            tracing::warn!("secure mobile ws ended: {err:#}");
        }
    })
}

async fn handle_mobile_secure_ws(
    mut socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
    device_id: String,
) -> Result<(), anyhow::Error> {
    let device_uuid = uuid::Uuid::parse_str(&device_id)?;
    let cfg = state.store.get_mobile_access_config().await?;
    let cfg = cfg.ok_or_else(|| anyhow::anyhow!("mobile access not configured"))?;
    let device = state
        .store
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not registered"))?;
    if device.profile_id != cfg.profile_id {
        return Err(anyhow::anyhow!("device not authorized for tunnel"));
    }
    let device_public_key = device
        .public_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("device missing public key"))?;
    let key =
        crate::mobile_e2ee::derive_key(&device_id, device_public_key, &cfg.daemon_private_key)?;

    let mut rx = state
        .workspace_active_snapshot
        .subscribe(workspace_id)
        .await;
    let mut subscriptions: HashMap<SessionId, SessionCursor> = HashMap::new();
    let mut outbound_seq: i64 = 0;

    let ready = WorkspaceActiveSnapshotEvent::Ready {
        workspace_id,
        snapshot_rev: state
            .workspace_active_snapshot
            .current_rev(workspace_id)
            .await,
    };
    outbound_seq += 1;
    send_secure_ws(&mut socket, &key, &device_id, outbound_seq, &ready).await?;

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        let frame: MobileSecureEnvelope = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let payload = crate::mobile_e2ee::decrypt(
                            &key,
                            &device_id,
                            frame.seq,
                            &frame.nonce,
                            &frame.ciphertext,
                        )?;
                        let message: WorkspaceActiveSnapshotClientMessage = serde_json::from_slice(&payload)?;
                        let (session_ids, sessions) = match message {
                            WorkspaceActiveSnapshotClientMessage::Subscribe { session_ids, sessions } => (session_ids, sessions),
                        };
                        let mut next: Vec<WorkspaceActiveSnapshotSessionSubscription> = if !sessions.is_empty() {
                            sessions
                        } else {
                            session_ids
                                .into_iter()
                                .map(|session_id| WorkspaceActiveSnapshotSessionSubscription { session_id, after_seq: None })
                                .collect()
                        };
                        next.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
                        let mut next_map = HashMap::new();
                        for sub in next {
                            let after_seq = sub.after_seq.unwrap_or(0);
                            let (last_sent, gap) = replay_session_events_secure(
                                &mut socket,
                                &state,
                                workspace_id,
                                sub.session_id,
                                after_seq,
                                &key,
                                &device_id,
                                &mut outbound_seq,
                            )
                            .await
                            .map_err(|_| anyhow::anyhow!("replay failed"))?;
                            if gap {
                                next_map.insert(sub.session_id, SessionCursor { last_sent });
                                continue;
                            }
                            next_map.insert(sub.session_id, SessionCursor { last_sent });
                        }
                        subscriptions = next_map;
                    }
                    Some(Ok(WsMessage::Close(_))) => break,
                    Some(Ok(_)) => {},
                    Some(Err(_)) => break,
                    None => break,
                }
            }
            event = rx.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let mut failed = false;
                        for (session_id, cursor) in &mut subscriptions {
                            let latest = state
                                .store
                                .get_session_last_event_seq(*session_id)
                                .await
                                .unwrap_or(cursor.last_sent);
                            cursor.last_sent = latest;
                            if !send_secure_workspace_gap(
                                &mut socket,
                                &state,
                                workspace_id,
                                *session_id,
                                latest,
                                "stream_lagged",
                                &key,
                                &device_id,
                                &mut outbound_seq,
                            )
                            .await
                            {
                                failed = true;
                                break;
                            }
                        }
                        if failed {
                            break;
                        }
                        break;
                    }
                    Err(_) => break,
                };

                if let WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } = &event {
                    let Some(cursor) = subscriptions.get_mut(&delta.session_id) else {
                        continue;
                    };
                    if let Some(ev) = &delta.event {
                        if ev.seq <= cursor.last_sent {
                            continue;
                        }
                        cursor.last_sent = ev.seq;
                    } else if delta.last_event_seq <= cursor.last_sent {
                        continue;
                    } else {
                        cursor.last_sent = delta.last_event_seq;
                    }
                }

                outbound_seq += 1;
                if send_secure_ws(&mut socket, &key, &device_id, outbound_seq, &event).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}

struct SessionCursor {
    last_sent: i64,
}

async fn send_secure_ws(
    socket: &mut WebSocket,
    key: &crate::mobile_e2ee::E2eeKey,
    device_id: &str,
    seq: i64,
    payload: &WorkspaceActiveSnapshotEvent,
) -> Result<(), anyhow::Error> {
    let plaintext = serde_json::to_vec(payload)?;
    let envelope = crate::mobile_e2ee::encrypt(key, device_id, seq, &plaintext)?;
    let frame = SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    };
    let text = serde_json::to_string(&frame)?;
    socket.send(WsMessage::Text(text)).await?;
    Ok(())
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
        .store
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
        .store
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
        .store
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

async fn list_providers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ProviderStatus>>, StatusCode> {
    let map = state.provider_statuses.lock().await;
    let mut out: Vec<ProviderStatus> = map.values().cloned().collect();
    drop(map);

    let managed = installer::load_agent_server_config(&state.data_root)
        .await
        .unwrap_or_default();
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;

    let show_fake = std::env::var("CTX_SHOW_FAKE_PROVIDER").ok().as_deref() == Some("1");
    for status in out.iter_mut() {
        installer::apply_managed_install_details(status, &managed);
        if status.provider_id == "fake" {
            status.details.insert(
                "ui_hidden".into(),
                if show_fake { "false" } else { "true" }.into(),
            );
        }
        status.details.insert(
            "install_supported".into(),
            if installer::is_supported_managed_provider(&matrix, &status.provider_id) {
                "true".into()
            } else {
                "false".into()
            },
        );
        if let Some(install_id) = state.find_running_install(&status.provider_id).await {
            status
                .details
                .insert("install_running".into(), "true".into());
            status
                .details
                .insert("install_id".into(), install_id.to_string());
        }
    }
    Ok(Json(out))
}

async fn get_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProviderStatus>, StatusCode> {
    let map = state.provider_statuses.lock().await;
    let mut status = map.get(&id).cloned().ok_or(StatusCode::NOT_FOUND)?;
    drop(map);

    let managed = installer::load_agent_server_config(&state.data_root)
        .await
        .unwrap_or_default();
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;
    installer::apply_managed_install_details(&mut status, &managed);
    status.details.insert(
        "install_supported".into(),
        if installer::is_supported_managed_provider(&matrix, &status.provider_id) {
            "true".into()
        } else {
            "false".into()
        },
    );
    if let Some(install_id) = state.find_running_install(&id).await {
        status
            .details
            .insert("install_running".into(), "true".into());
        status
            .details
            .insert("install_id".into(), install_id.to_string());
    }
    Ok(Json(status))
}

#[derive(Debug, Serialize)]
struct InstallStartResponse {
    provider_id: String,
    install_id: InstallId,
}

fn default_agent_server_command(
    matrix: &crate::provider_matrix::ProviderMatrix,
    data_root: &std::path::Path,
    provider_id: &str,
) -> Option<(String, Vec<String>)> {
    let entry = crate::provider_matrix::get_entry(matrix, provider_id)?;
    let mut cmd = entry.command.clone()?;
    if provider_id == "cagent" {
        cmd.args.push(
            crate::installer::cagent_config_path(data_root)
                .to_string_lossy()
                .to_string(),
        );
    }
    Some((cmd.command, cmd.args))
}

async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiErrorResp>)> {
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
    const VERIFY_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let cache_key = format!("{}/{}", ws_id.0, provider_id);
    let verify_entry: Option<(std::time::Instant, serde_json::Value)> = state
        .provider_verify_cache
        .lock()
        .await
        .get(&cache_key)
        .map(|c| (c.cached_at, c.value.clone()));
    let cached_entry: Option<(std::time::Instant, serde_json::Value)> = state
        .provider_options_cache
        .lock()
        .await
        .get(&cache_key)
        .map(|c| (c.cached_at, c.value.clone()));
    if let Some((cached_at, cached_value)) = cached_entry.as_ref() {
        if cached_at.elapsed() < CACHE_TTL {
            let mut out = cached_value.clone();
            if let Some((verify_at, verify)) = verify_entry.as_ref() {
                if verify_at.elapsed() < VERIFY_TTL {
                    if let Some(obj) = out.as_object_mut() {
                        obj.insert("verify".to_string(), verify.clone());
                    }
                }
            }
            return Ok(Json(out));
        }
    }
    let cached_models = cached_entry
        .as_ref()
        .and_then(|(_, value)| value.get("models"))
        .cloned()
        .filter(|v| !v.is_null());
    let cached_modes = cached_entry
        .as_ref()
        .and_then(|(_, value)| value.get("modes"))
        .cloned()
        .filter(|v| !v.is_null());

    let provider_status = state
        .provider_statuses
        .lock()
        .await
        .get(&provider_id)
        .cloned();

    if let Some(st) = provider_status.as_ref() {
        if !st.installed || !matches!(st.health, ctx_providers::adapters::ProviderHealth::Ok) {
            let base_resp = redact_json_value(serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": st.installed,
                "health": st.health,
                "diagnostics": st.diagnostics,
                "probe_ok": false,
                "probe_error": "provider not installed or unhealthy",
                "probed_at": chrono::Utc::now().to_rfc3339(),
            }));
            state.provider_options_cache.lock().await.insert(
                cache_key,
                crate::daemon::CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value: base_resp.clone(),
                },
            );
            let mut out = base_resp;
            if let Some((verify_at, verify)) = verify_entry.as_ref() {
                if verify_at.elapsed() < VERIFY_TTL {
                    if let Some(obj) = out.as_object_mut() {
                        obj.insert("verify".to_string(), verify.clone());
                    }
                }
            }
            return Ok(Json(out));
        }
    }

    let ws = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load workspace".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let cfg = installer::load_agent_server_config(&state.data_root)
        .await
        .unwrap_or_default();
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;

    let (command, args) = cfg
        .providers
        .get(&provider_id)
        .map(|c| (c.command.clone(), c.args.clone()))
        .or_else(|| default_agent_server_command(&matrix, &state.data_root, &provider_id))
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "unknown provider id".to_string(),
            }),
        ))?;

    let agent = AcpAgentConfig {
        provider_id: provider_id.clone(),
        command,
        args,
    };
    let client = AcpClientConfig {
        client_name: "ctx".to_string(),
        client_title: "ctx".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        client_capabilities: serde_json::json!({}),
        system_prompt_append: None,
        mcp_servers: vec![],
    };

    let mut env = std::collections::HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.as_ref() {
        env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
    }

    let probe = probe_provider_options(agent, client, PathBuf::from(&ws.root_path), env).await;

    let mut raw_resp = match probe {
        Ok(probe) => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.as_ref().map(|s| s.installed).unwrap_or(true),
            "probe_ok": true,
            "supports_load": probe.supports_load,
            "auth_required": probe.auth_required,
            "auth_methods": probe.auth_methods,
            "modes": probe.modes,
            "models": probe.models,
            "acp_error": probe.acp_error,
            "probed_at": chrono::Utc::now().to_rfc3339(),
        }),
        Err(e) => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.as_ref().map(|s| s.installed).unwrap_or(false),
            "probe_ok": false,
            "probe_error": logs::redact_sensitive(&e.to_string()),
            "probed_at": chrono::Utc::now().to_rfc3339(),
        }),
    };

    // If probing fails (or returns null lists), keep the last successfully probed models/modes so
    // the UI can stay populated.
    if raw_resp.get("models").is_none() || raw_resp.get("models").is_some_and(|v| v.is_null()) {
        if let Some(models) = cached_models {
            raw_resp["models"] = models;
        }
    }
    if raw_resp.get("modes").is_none() || raw_resp.get("modes").is_some_and(|v| v.is_null()) {
        if let Some(modes) = cached_modes {
            raw_resp["modes"] = modes;
        }
    }

    let resp = redact_json_value(raw_resp);

    state.provider_options_cache.lock().await.insert(
        cache_key,
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: resp.clone(),
        },
    );

    let mut out = resp;
    if let Some((verify_at, verify)) = verify_entry.as_ref() {
        if verify_at.elapsed() < VERIFY_TTL {
            if let Some(obj) = out.as_object_mut() {
                obj.insert("verify".to_string(), verify.clone());
            }
        }
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
struct AuthenticateProviderReq {
    #[serde(default)]
    method_id: Option<String>,
}

async fn authenticate_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
    Json(req): Json<AuthenticateProviderReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let ws = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load workspace".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let cfg = installer::load_agent_server_config(&state.data_root)
        .await
        .unwrap_or_default();
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;
    let (command, args) = cfg
        .providers
        .get(&provider_id)
        .map(|c| (c.command.clone(), c.args.clone()))
        .or_else(|| default_agent_server_command(&matrix, &state.data_root, &provider_id))
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "unknown provider id".to_string(),
            }),
        ))?;

    let agent = AcpAgentConfig {
        provider_id: provider_id.clone(),
        command,
        args,
    };
    let client = AcpClientConfig {
        client_name: "ctx".to_string(),
        client_title: "ctx".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        client_capabilities: serde_json::json!({}),
        system_prompt_append: None,
        mcp_servers: vec![],
    };

    let mut env = std::collections::HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.as_ref() {
        env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
    }
    env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());

    let probe = authenticate_provider(
        agent,
        client,
        PathBuf::from(&ws.root_path),
        env,
        req.method_id,
    )
    .await;
    let (status, auth_required, auth_methods, acp_error) = match probe {
        Ok(p) => (p.status, p.auth_required, p.auth_methods, p.acp_error),
        Err(e) => (
            "error".to_string(),
            false,
            None,
            Some(serde_json::json!({"message": logs::redact_sensitive(&e.to_string())})),
        ),
    };

    let resp = redact_json_value(serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": ws_id.0,
        "status": status,
        "auth_required": auth_required,
        "auth_methods": auth_methods,
        "acp_error": acp_error,
        "checked_at": chrono::Utc::now().to_rfc3339(),
    }));
    Ok(Json(resp))
}

async fn verify_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let ws = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load workspace".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let cfg = installer::load_agent_server_config(&state.data_root)
        .await
        .unwrap_or_default();
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;
    let (command, args) = cfg
        .providers
        .get(&provider_id)
        .map(|c| (c.command.clone(), c.args.clone()))
        .or_else(|| default_agent_server_command(&matrix, &state.data_root, &provider_id))
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "unknown provider id".to_string(),
            }),
        ))?;

    let agent = AcpAgentConfig {
        provider_id: provider_id.clone(),
        command,
        args,
    };
    let client = AcpClientConfig {
        client_name: "ctx".to_string(),
        client_title: "ctx".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        client_capabilities: serde_json::json!({}),
        system_prompt_append: None,
        mcp_servers: vec![],
    };

    let mut env = std::collections::HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.as_ref() {
        env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
    }
    env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());

    let probe = verify_provider_connection(agent, client, PathBuf::from(&ws.root_path), env).await;
    let (status, auth_required, auth_methods, acp_error) = match probe {
        Ok(p) => (p.status, p.auth_required, p.auth_methods, p.acp_error),
        Err(e) => (
            "error".to_string(),
            false,
            None,
            Some(serde_json::json!({"message": logs::redact_sensitive(&e.to_string())})),
        ),
    };

    let resp = redact_json_value(serde_json::json!({
        "provider_id": provider_id.clone(),
        "workspace_id": ws_id.0,
        "status": status,
        "auth_required": auth_required,
        "auth_methods": auth_methods,
        "acp_error": acp_error,
        "checked_at": chrono::Utc::now().to_rfc3339(),
    }));

    let cache_key = format!("{}/{}", ws_id.0, provider_id);
    state.provider_verify_cache.lock().await.insert(
        cache_key,
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: resp.clone(),
        },
    );

    Ok(Json(resp))
}

async fn install_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<InstallStartResponse>, StatusCode> {
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;
    if !installer::is_supported_managed_provider(&matrix, &id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let (install_id, started_new) = state.start_install(id.clone()).await;
    if started_new {
        let state2 = state.clone();
        let provider_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_provider_with_progress(
                state2.clone(),
                install_id,
                provider_id.clone(),
            )
            .await
            {
                tracing::error!("provider install failed ({provider_id}): {e:#}");
            }
        });
    }

    Ok(Json(InstallStartResponse {
        provider_id: id,
        install_id,
    }))
}

#[derive(Debug, Serialize)]
struct LspInstallStartResponse {
    server_id: String,
    install_id: InstallId,
}

async fn install_lsp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LspInstallStartResponse>, StatusCode> {
    if !installer::is_supported_managed_lsp_server(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let install_key = format!("lsp:{id}");
    let (install_id, started_new) = state.start_install(install_key).await;
    if started_new {
        let state2 = state.clone();
        let server_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_lsp_server_with_progress(
                state2.clone(),
                install_id,
                server_id.clone(),
            )
            .await
            {
                tracing::error!("lsp install failed ({server_id}): {e:#}");
            }
        });
    }

    Ok(Json(LspInstallStartResponse {
        server_id: id,
        install_id,
    }))
}

async fn install_all_providers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<InstallStartResponse>>, StatusCode> {
    let mut out = Vec::new();
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;
    for entry in matrix.providers.iter() {
        if !installer::is_supported_managed_provider(&matrix, &entry.id) {
            continue;
        }
        let id = entry.id.as_str();
        if let Some(install_id) = state.find_running_install(id).await {
            out.push(InstallStartResponse {
                provider_id: id.to_string(),
                install_id,
            });
            continue;
        }

        let status = state.provider_statuses.lock().await.get(id).cloned();
        if let Some(st) = status {
            if st.installed && matches!(st.health, ctx_providers::adapters::ProviderHealth::Ok) {
                continue;
            }
        }

        let (install_id, started_new) = state.start_install(id.to_string()).await;
        if started_new {
            let state2 = state.clone();
            let provider_id = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = installer::install_provider_with_progress(
                    state2.clone(),
                    install_id,
                    provider_id.clone(),
                )
                .await
                {
                    tracing::error!("provider install failed ({provider_id}): {e:#}");
                }
            });
        }
        out.push(InstallStartResponse {
            provider_id: id.to_string(),
            install_id,
        });
    }
    Ok(Json(out))
}

async fn get_install(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_info(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_install_events(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<Vec<InstallProgressEvent>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_events(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn install_stream_sse(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, axum::Error>>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let Some(sender) = state.get_install_sender(install_id).await else {
        return Err(StatusCode::NOT_FOUND);
    };

    let history = state
        .get_install_events(install_id)
        .await
        .unwrap_or_default();
    let initial = futures::stream::iter(history.into_iter().map(|ev| {
        let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
        Ok::<_, axum::Error>(SseEvent::default().event("progress").data(payload))
    }));

    let live = futures::stream::unfold(sender.subscribe(), move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    return Some((Ok(SseEvent::default().event("progress").data(payload)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let stream = initial.chain(live);

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15))))
}

#[derive(Debug, Deserialize)]
struct CreateWorkspaceReq {
    root_path: String,
    name: Option<String>,
}

async fn list_workspaces(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Workspace>>, StatusCode> {
    state
        .store
        .list_workspaces()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Workspace>, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match state
        .store
        .get_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(ws) => {
            state
                .telemetry
                .emit(TelemetryEvent::workspace_opened())
                .await;
            Ok(Json(ws))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn list_workspace_terminals(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TerminalSession>>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let terminals = state.terminals.list(workspace_id).await;
    Ok(Json(terminals))
}

fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

async fn create_workspace_terminal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateTerminalReq>,
) -> Result<Json<TerminalSession>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let workspace = state
        .store
        .get_workspace(workspace_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load workspace".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let task_id = match req.task_id {
        Some(raw) => Some(TaskId(uuid::Uuid::parse_str(raw.trim()).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid task_id".to_string(),
                }),
            )
        })?)),
        None => None,
    };
    let session_id = match req.session_id {
        Some(raw) => Some(SessionId(uuid::Uuid::parse_str(raw.trim()).map_err(
            |_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "invalid session_id".to_string(),
                    }),
                )
            },
        )?)),
        None => None,
    };
    let worktree_id = match req.worktree_id {
        Some(raw) => Some(WorktreeId(uuid::Uuid::parse_str(raw.trim()).map_err(
            |_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "invalid worktree_id".to_string(),
                    }),
                )
            },
        )?)),
        None => None,
    };

    let workspace_root = PathBuf::from(&workspace.root_path);
    let workspace_root = tokio::fs::canonicalize(&workspace_root)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "workspace root is unavailable".to_string(),
                }),
            )
        })?;

    let worktree_root = if let Some(wt_id) = worktree_id {
        let wt = state
            .store
            .get_worktree(wt_id)
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
        Some(tokio::fs::canonicalize(&wt.root_path).await.map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "worktree root is unavailable".to_string(),
                }),
            )
        })?)
    } else {
        None
    };

    let requested_cwd = req.cwd.as_ref().and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    });
    let fallback_cwd = worktree_root
        .clone()
        .unwrap_or_else(|| workspace_root.clone());
    let cwd = requested_cwd.unwrap_or(fallback_cwd);
    let cwd = tokio::fs::canonicalize(&cwd).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "cwd does not exist".to_string(),
            }),
        )
    })?;

    let allowed = worktree_root
        .as_ref()
        .map(|root| cwd.starts_with(root))
        .unwrap_or(false)
        || cwd.starts_with(&workspace_root);
    if !allowed {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "cwd must be within the workspace or worktree".to_string(),
            }),
        ));
    }

    let requested_shell = req.shell.as_deref().and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let shell = requested_shell
        .map(|value| value.to_string())
        .unwrap_or_else(default_shell);
    let session = state
        .terminals
        .create(TerminalCreateRequest {
            workspace_id,
            task_id,
            session_id,
            worktree_id,
            cwd,
            shell,
            cols: None,
            rows: None,
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!("failed to create terminal: {e}"),
                }),
            )
        })?;

    Ok(Json(session.snapshot()))
}

async fn delete_terminal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let terminal_id = TerminalId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let session = state.terminals.remove(terminal_id).await;
    if let Some(session) = session {
        let _ = session.kill();
        session.mark_exited(None);
        return Ok(StatusCode::NO_CONTENT);
    }
    Err(StatusCode::NOT_FOUND)
}

async fn terminal_stream_ws(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    let terminal_id = TerminalId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let session = state
        .terminals
        .get(terminal_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(ws.on_upgrade(move |socket| async move {
        handle_terminal_socket(socket, session).await;
    }))
}

async fn handle_terminal_socket(
    mut socket: WebSocket,
    session: Arc<crate::terminals::TerminalSessionHandle>,
) {
    let snapshot = session.snapshot();
    let status_payload = serde_json::to_string(&TerminalServerMessage::Status {
        status: snapshot.status.clone(),
        exit_code: snapshot.exit_code,
    })
    .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"running\"}".to_string());
    let _ = socket.send(WsMessage::Text(status_payload)).await;

    let buffer = session.output_snapshot();
    if !buffer.is_empty() {
        let _ = socket.send(WsMessage::Binary(buffer)).await;
    }

    let mut output_rx = session.output_receiver();
    let mut status_rx = session.status_receiver();

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();
    let event_tx_output = event_tx.clone();
    let event_tx_status = event_tx.clone();

    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Some(msg) = event_rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    tasks.spawn(async move {
        loop {
            match output_rx.recv().await {
                Ok(bytes) => {
                    let _ = event_tx_output.send(WsMessage::Binary(bytes));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    tasks.spawn(async move {
        loop {
            match status_rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&TerminalServerMessage::Status {
                        status: ev.status,
                        exit_code: ev.exit_code,
                    })
                    .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"exited\"}".to_string());
                    let _ = event_tx_status.send(WsMessage::Text(payload));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    tasks.spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                WsMessage::Binary(data) => {
                    session.send_input(data);
                }
                WsMessage::Text(text) => {
                    if let Ok(parsed) = serde_json::from_str::<TerminalClientMessage>(&text) {
                        match parsed {
                            TerminalClientMessage::Resize { cols, rows } => {
                                let _ = session.resize(cols, rows);
                            }
                            TerminalClientMessage::Input { data } => {
                                session.send_input(data.into_bytes());
                            }
                        }
                    } else {
                        session.send_input(text.into_bytes());
                    }
                }
                WsMessage::Close(_) => break,
                WsMessage::Ping(_) | WsMessage::Pong(_) => {}
            }
        }
    });

    let _ = tasks.join_next().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
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
        &state.data_root,
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

    let worker_bundle = crate::web_sessions::ensure_worker_bundle(&state.data_root, &node_runtime)
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

    let handle = state.web_sessions.create(req).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!("failed to create web session: {e}"),
            }),
        )
    })?;

    let mut info = handle.snapshot().await;
    let base_url = resolve_request_base_url(&headers, &state.daemon_url);
    info.stream_url = Some(format!("{}{}", base_url, info.stream_path));
    Ok(Json(info))
}

async fn list_web_sessions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<WebSessionInfo>>, StatusCode> {
    let mut sessions = state.web_sessions.list().await;
    let base_url = resolve_request_base_url(&headers, &state.daemon_url);
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
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut info = handle.snapshot().await;
    let base_url = resolve_request_base_url(&headers, &state.daemon_url);
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
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let info = handle.snapshot().await;
    let body = render_web_session_view(&info);
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response())
}

async fn web_session_signal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    state
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let manager = state.web_sessions.clone();
    let session_id = id.clone();
    Ok(ws.on_upgrade(move |socket| async move {
        handle_web_session_socket(socket, manager, session_id).await;
    }))
}

async fn handle_web_session_socket(
    socket: WebSocket,
    manager: Arc<crate::web_sessions::WebSessionManager>,
    session_id: String,
) {
    let handle = match manager.get(&session_id).await {
        Some(handle) => handle,
        None => {
            let _ = socket.close().await;
            return;
        }
    };
    let port = handle.worker_port().await;
    let url = format!("ws://127.0.0.1:{}/signal", port);
    let _ = manager.bump_viewers(&session_id, 1).await;

    let connect = connect_async(url).await;
    let upstream = match connect {
        Ok((stream, _)) => stream,
        Err(_) => {
            let _ = manager.bump_viewers(&session_id, -1).await;
            return;
        }
    };

    let (mut client_tx, mut client_rx) = socket.split();
    let (mut up_tx, mut up_rx) = upstream.split();

    let client_to_up = tokio::spawn(async move {
        while let Some(Ok(msg)) = client_rx.next().await {
            let out = match msg {
                WsMessage::Text(text) => TungsteniteMessage::Text(text.into()),
                WsMessage::Binary(bytes) => TungsteniteMessage::Binary(bytes.into()),
                WsMessage::Ping(bytes) => TungsteniteMessage::Ping(bytes.into()),
                WsMessage::Pong(bytes) => TungsteniteMessage::Pong(bytes.into()),
                WsMessage::Close(frame) => {
                    let frame =
                        frame.map(|f| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                            code: f.code.into(),
                            reason: f.reason.to_string().into(),
                        });
                    TungsteniteMessage::Close(frame)
                }
            };
            if up_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    let up_to_client = tokio::spawn(async move {
        while let Some(Ok(msg)) = up_rx.next().await {
            let out = match msg {
                TungsteniteMessage::Text(text) => WsMessage::Text(text.to_string()),
                TungsteniteMessage::Binary(bytes) => WsMessage::Binary(bytes.to_vec()),
                TungsteniteMessage::Ping(bytes) => WsMessage::Ping(bytes.to_vec()),
                TungsteniteMessage::Pong(bytes) => WsMessage::Pong(bytes.to_vec()),
                TungsteniteMessage::Close(frame) => {
                    let frame = frame.map(|f| axum::extract::ws::CloseFrame {
                        code: f.code.into(),
                        reason: f.reason.to_string().into(),
                    });
                    WsMessage::Close(frame)
                }
                TungsteniteMessage::Frame(_) => continue,
            };
            if client_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    tokio::select! {
        _ = client_to_up => {},
        _ = up_to_client => {},
    };

    let _ = manager.bump_viewers(&session_id, -1).await;
}

async fn resolve_web_session_work_dir(
    state: &Arc<AppState>,
    session_id: Option<String>,
    worktree_id: Option<String>,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(worktree_id) = worktree_id {
        let worktree_id =
            WorktreeId(uuid::Uuid::parse_str(&worktree_id).context("invalid worktree id")?);
        let worktree = state
            .store
            .get_worktree(worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(Some(PathBuf::from(worktree.root_path)));
    }
    if let Some(session_id) = session_id {
        let session_id =
            SessionId(uuid::Uuid::parse_str(&session_id).context("invalid session id")?);
        let session = state
            .store
            .get_session(session_id)
            .await?
            .context("session not found")?;
        let worktree = state
            .store
            .get_worktree(session.worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(Some(PathBuf::from(worktree.root_path)));
    }
    Ok(None)
}

async fn create_workspace(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateWorkspaceReq>,
) -> Result<Json<Workspace>, (StatusCode, Json<ApiErrorResp>)> {
    let raw = req.root_path.trim();
    if raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "root_path is required".to_string(),
            }),
        ));
    }

    let expanded = if raw == "~" || raw.starts_with("~/") {
        let base = directories::BaseDirs::new().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "could not resolve home directory to expand '~'".to_string(),
                }),
            )
        })?;
        let home = base.home_dir();
        if raw == "~" {
            home.to_path_buf()
        } else {
            home.join(raw.trim_start_matches("~/"))
        }
    } else {
        PathBuf::from(raw)
    };

    let root_path = tokio::fs::canonicalize(&expanded).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("invalid root_path '{}': {}", expanded.to_string_lossy(), e),
            }),
        )
    })?;

    assert_git_repo(&root_path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;

    let root_path_str = root_path.to_string_lossy().to_string();

    let name = req.name.unwrap_or_else(|| {
        root_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("workspace")
            .to_string()
    });
    let workspace = state
        .store
        .create_workspace(name, root_path_str)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    state
        .telemetry
        .emit(TelemetryEvent::workspace_registered())
        .await;
    Ok(Json(workspace))
}

async fn delete_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .delete_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct SyncWorkspaceAttachmentsReq {
    #[serde(default)]
    refresh: Option<bool>,
}

async fn list_workspace_attachments(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<WorkspaceAttachment>>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .list_workspace_attachments(ws_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn sync_workspace_attachments(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SyncWorkspaceAttachmentsReq>,
) -> Result<Json<Vec<WorkspaceAttachment>>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let refresh = req.refresh.unwrap_or(false);
    let attachments = attachments::sync_workspace_attachments(&state, &workspace, refresh)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let _ = attachments::ensure_workspace_attachments_for_worktrees_with_attachments(
        &state,
        &workspace,
        &attachments,
        false,
        false,
    )
    .await;
    Ok(Json(attachments))
}

#[derive(Debug, Serialize)]
struct AgentSystemPromptConfigResponse {
    config_path: String,
    default_append: String,
    configured_append: Option<String>,
    effective_append: Option<String>,
    source: String,
}

#[derive(Debug, Deserialize)]
struct UpdateAgentSystemPromptConfigReq {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

#[derive(Debug, Serialize)]
struct SubagentSystemPromptConfigResponse {
    config_path: String,
    default_append: String,
    configured_append: Option<String>,
    effective_append: Option<String>,
    source: String,
}

#[derive(Debug, Deserialize)]
struct UpdateSubagentSystemPromptConfigReq {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

async fn get_agent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AgentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let cfg = workspace_config::load_agent_system_prompt_append(StdPath::new(&workspace.root_path))
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let configured_append = cfg
        .configured_append
        .as_ref()
        .map(|value| value.trim().to_string());
    let effective_append = cfg.effective_append();
    let source = match cfg.source() {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    };
    let response = AgentSystemPromptConfigResponse {
        config_path: cfg.config_path.to_string_lossy().to_string(),
        default_append: cfg.default_append.clone(),
        configured_append,
        effective_append,
        source,
    };

    Ok(Json(response))
}

async fn update_agent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentSystemPromptConfigReq>,
) -> Result<Json<AgentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    workspace_config::update_agent_system_prompt_append(
        StdPath::new(&workspace.root_path),
        req.system_prompt_append,
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

    let cfg = workspace_config::load_agent_system_prompt_append(StdPath::new(&workspace.root_path))
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let configured_append = cfg
        .configured_append
        .as_ref()
        .map(|value| value.trim().to_string());
    let effective_append = cfg.effective_append();
    let source = match cfg.source() {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    };
    let response = AgentSystemPromptConfigResponse {
        config_path: cfg.config_path.to_string_lossy().to_string(),
        default_append: cfg.default_append.clone(),
        configured_append,
        effective_append,
        source,
    };

    Ok(Json(response))
}

async fn get_subagent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SubagentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let cfg =
        workspace_config::load_subagent_system_prompt_append(StdPath::new(&workspace.root_path))
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;

    let configured_append = cfg
        .configured_append
        .as_ref()
        .map(|value| value.trim().to_string());
    let effective_append = cfg.effective_append();
    let source = match cfg.source() {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    };
    let response = SubagentSystemPromptConfigResponse {
        config_path: cfg.config_path.to_string_lossy().to_string(),
        default_append: cfg.default_append.clone(),
        configured_append,
        effective_append,
        source,
    };

    Ok(Json(response))
}

async fn update_subagent_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSubagentSystemPromptConfigReq>,
) -> Result<Json<SubagentSystemPromptConfigResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    workspace_config::update_subagent_system_prompt_append(
        StdPath::new(&workspace.root_path),
        req.system_prompt_append,
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

    let cfg =
        workspace_config::load_subagent_system_prompt_append(StdPath::new(&workspace.root_path))
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;

    let configured_append = cfg
        .configured_append
        .as_ref()
        .map(|value| value.trim().to_string());
    let effective_append = cfg.effective_append();
    let source = match cfg.source() {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    };
    let response = SubagentSystemPromptConfigResponse {
        config_path: cfg.config_path.to_string_lossy().to_string(),
        default_append: cfg.default_append.clone(),
        configured_append,
        effective_append,
        source,
    };

    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
struct CreateWorkspaceAttachmentReq {
    kind: WorkspaceAttachmentKind,
    name: String,
    source: String,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    subpath: Option<String>,
    #[serde(default)]
    mount_relpath: Option<String>,
    #[serde(default)]
    mode: Option<AttachmentMode>,
    #[serde(default)]
    update_policy: Option<AttachmentUpdatePolicy>,
}

async fn create_workspace_attachment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateWorkspaceAttachmentReq>,
) -> Result<Json<Vec<WorkspaceAttachment>>, (StatusCode, Json<ApiErrorResp>)> {
    if req.name.trim().is_empty() || req.source.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "name and source are required".to_string(),
            }),
        ));
    }
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let cfg = attachments::AttachmentConfig {
        kind: req.kind,
        name: req.name,
        source: req.source,
        revision: req.revision,
        subpath: req.subpath,
        mount_relpath: req.mount_relpath,
        mode: req.mode,
        update_policy: req.update_policy,
    };
    attachments::upsert_attachment_config(StdPath::new(&workspace.root_path), cfg)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let attachments = attachments::sync_workspace_attachments(&state, &workspace, true)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let _ = attachments::ensure_workspace_attachments_for_worktrees_with_attachments(
        &state,
        &workspace,
        &attachments,
        false,
        false,
    )
    .await;
    Ok(Json(attachments))
}

#[derive(Debug, Deserialize)]
struct DeleteWorkspaceAttachmentReq {
    kind: WorkspaceAttachmentKind,
    name: String,
}

async fn delete_workspace_attachment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<DeleteWorkspaceAttachmentReq>,
) -> Result<Json<Vec<WorkspaceAttachment>>, (StatusCode, Json<ApiErrorResp>)> {
    if req.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "name is required".to_string(),
            }),
        ));
    }
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let removed = attachments::remove_attachment_config(
        StdPath::new(&workspace.root_path),
        req.kind,
        &req.name,
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
    if !removed {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "attachment not found".to_string(),
            }),
        ));
    }

    let attachments = attachments::sync_workspace_attachments(&state, &workspace, false)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(attachments))
}

#[derive(Debug, Deserialize)]
struct CreateTaskReq {
    title: String,
    description: Option<String>,
    #[serde(default = "default_true")]
    create_default_session: bool,
}

fn default_true() -> bool {
    true
}

async fn update_task_title(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTaskTitleReq>,
) -> Result<Json<Task>, (StatusCode, Json<ApiErrorResp>)> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid task id".to_string(),
            }),
        )
    })?);
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title is required".to_string(),
            }),
        ));
    }
    if title.len() > 120 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title is too long".to_string(),
            }),
        ));
    }

    let updated = state
        .store
        .update_task_title(task_id, title)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    if !updated {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "task not found".to_string(),
            }),
        ));
    }

    let task = match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })? {
        Some(task) => task,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            ))
        }
    };

    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    let sessions = match state.store.list_sessions_for_task(task_id).await {
        Ok(sessions) => sessions,
        Err(e) => {
            tracing::warn!(task_id = %task_id.0, "failed to list sessions for archived task: {e:?}");
            Vec::new()
        }
    };
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut worktree_id_strings = HashSet::new();
    for worktree_id in worktree_ids {
        match state.store.get_worktree(worktree_id).await {
            Ok(Some(worktree)) => {
                worktree_id_strings.insert(worktree.id.0.to_string());
            }
            Ok(None) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "worktree missing for archived task"
                );
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load worktree for archived task: {e:?}"
                );
            }
        }
    }
    let session_ids: HashSet<String> = sessions
        .iter()
        .map(|session| session.id.0.to_string())
        .collect();
    if let Err(e) = state
        .web_sessions
        .close_for_task(&session_ids, &worktree_id_strings)
        .await
    {
        tracing::warn!(task_id = %task_id.0, "failed to close web sessions for archived task: {e:?}");
    }
    Ok(Json(task))
}

async fn delete_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let task = state
        .store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let deleted = state
        .store
        .delete_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    state
        .emit_workspace_task_delete(task.workspace_id, task_id)
        .await;
    Ok(StatusCode::NO_CONTENT)
}

fn managed_worktree_root(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Option<PathBuf> {
    let root = PathBuf::from(&worktree.root_path);
    let expected = managed_worktree_path(&state.data_root, workspace.id, worktree.id);
    if root == expected {
        Some(root)
    } else {
        None
    }
}

async fn remove_worktree(
    workspace_root: impl AsRef<StdPath>,
    worktree_path: impl AsRef<StdPath>,
) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path.as_ref())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree remove")?;
    if !output.status.success() {
        bail!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if tokio::fs::metadata(worktree_path.as_ref()).await.is_ok() {
        tokio::fs::remove_dir_all(worktree_path.as_ref())
            .await
            .context("removing worktree dir")?;
    }
    Ok(())
}

async fn prune_worktrees(workspace_root: impl AsRef<StdPath>) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("prune")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree prune")?;
    if !output.status.success() {
        bail!(
            "git worktree prune failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn ensure_worktree_attached(
    workspace_root: impl AsRef<StdPath>,
    worktree_path: impl AsRef<StdPath>,
    base_commit_sha: &str,
    branch_name: &str,
) -> anyhow::Result<()> {
    let worktree_path = worktree_path.as_ref();
    if let Some(parent) = worktree_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating worktree parent dir")?;
    }

    if tokio::fs::metadata(worktree_path).await.is_ok() {
        if is_git_worktree(worktree_path).await.unwrap_or(false) {
            return Ok(());
        }
        tokio::fs::remove_dir_all(worktree_path)
            .await
            .context("removing stale worktree dir")?;
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("add")
        .arg(worktree_path);
    if branch_exists(workspace_root, branch_name).await? {
        cmd.arg(branch_name);
    } else {
        cmd.arg("-b").arg(branch_name).arg(base_commit_sha);
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree add")?;
    if !output.status.success() {
        bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn branch_exists(
    workspace_root: impl AsRef<StdPath>,
    branch_name: &str,
) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("show-ref")
        .arg("--verify")
        .arg("--quiet")
        .arg(format!("refs/heads/{branch_name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git show-ref --verify")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    bail!(
        "git show-ref failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )
}

async fn is_git_worktree(worktree_path: impl AsRef<StdPath>) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_path.as_ref())
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git rev-parse --is-inside-work-tree")?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

async fn archive_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let task = state
        .store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let workspace = state
        .store
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let sessions = state
        .store
        .list_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut seen = HashSet::new();
    let mut worktrees = Vec::new();
    for worktree_id in worktree_ids {
        if !seen.insert(worktree_id) {
            continue;
        }
        let worktree = state
            .store
            .get_worktree(worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        worktrees.push(worktree);
    }

    let mut errors: Vec<anyhow::Error> = Vec::new();
    let mut needs_prune = false;
    for worktree in &worktrees {
        let Some(root) = managed_worktree_root(&state, &workspace, worktree) else {
            continue;
        };
        if tokio::fs::metadata(&root).await.is_err() {
            continue;
        }
        let is_git = is_git_worktree(&root).await.unwrap_or(false);
        if is_git {
            needs_prune = true;
            if let Err(err) = remove_worktree(&workspace.root_path, &root).await {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove worktree: {err:#}"
                );
                errors.push(err);
                continue;
            }
            // Defensive: ensure the directory is actually gone even if `git worktree remove`
            // succeeds but leaves the directory behind.
            if tokio::fs::metadata(&root).await.is_ok() {
                if let Err(err) = tokio::fs::remove_dir_all(&root)
                    .await
                    .with_context(|| format!("removing worktree dir at {}", root.display()))
                {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        "failed to remove worktree dir: {err:#}"
                    );
                    errors.push(err);
                }
            }
        } else if let Err(err) = tokio::fs::remove_dir_all(&root)
            .await
            .with_context(|| format!("removing non-git worktree dir at {}", root.display()))
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to remove worktree dir: {err:#}"
            );
            errors.push(err);
        }
    }
    if needs_prune {
        if let Err(err) = prune_worktrees(&workspace.root_path).await {
            tracing::warn!(
                task_id = %task_id.0,
                "failed to prune worktrees: {err:#}"
            );
            errors.push(err);
        }
    }
    if !errors.is_empty() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let updated = state
        .store
        .archive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

async fn unarchive_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let task = state
        .store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let workspace = state
        .store
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut seen = HashSet::new();
    let mut managed_worktrees: Vec<(Worktree, PathBuf)> = Vec::new();
    let mut worktrees: Vec<Worktree> = Vec::new();
    let sessions = state
        .store
        .list_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary) = task.primary_worktree_id {
        worktree_ids.insert(primary);
    }
    for worktree_id in worktree_ids {
        let worktree = state
            .store
            .get_worktree(worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if let Some(root) = managed_worktree_root(&state, &workspace, &worktree) {
            if seen.insert(worktree.id) {
                managed_worktrees.push((worktree.clone(), root));
            }
        }
        worktrees.push(worktree);
    }

    for (worktree, root) in &managed_worktrees {
        let branch = worktree.git_branch.as_deref().unwrap_or_default();
        if let Err(err) = ensure_worktree_attached(
            &workspace.root_path,
            root,
            &worktree.base_commit_sha,
            branch,
        )
        .await
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to recreate worktree: {err:#}"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    for worktree in &worktrees {
        if let Err(e) =
            attachments::ensure_worktree_attachment_mounts(&state, &workspace, worktree, false)
                .await
        {
            tracing::warn!(task_id = %task_id.0, "attachment mounts failed: {e:?}");
        }
        if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
            Arc::clone(&state),
            workspace.clone(),
            worktree.clone(),
        )
        .await
        {
            tracing::warn!(task_id = %task_id.0, "worktree bootstrap failed: {e:?}");
        }
    }

    let updated = state
        .store
        .unarchive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

async fn mark_task_read(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let updated = state
        .store
        .mark_task_read(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

async fn mark_task_unread(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let updated = state
        .store
        .mark_task_unread(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

async fn list_workspace_tasks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Task>>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let tasks = state
        .store
        .list_tasks(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(tasks))
}

async fn list_task_sessions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Session>>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let sessions = state
        .store
        .list_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(sessions))
}

async fn create_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateTaskReq>,
) -> Result<Json<Task>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let ws = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let want_default_session = req.create_default_session;
    if want_default_session {
        assert_git_repo(&ws.root_path).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    }

    let task = state
        .store
        .create_task(ws_id, req.title, req.description)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    if !req.create_default_session {
        if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
            tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
        }
        return Ok(Json(task));
    }

    let base_commit_sha = rev_parse_head(&ws.root_path).await.map_err(|e| {
        let msg = e.to_string().to_lowercase();
        if msg.contains("ambiguous argument 'head'")
            || msg.contains("unknown revision or path not in the working tree")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "git repo has no commits; create an initial commit before creating a worktree".to_string(),
                }),
            );
        }
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let worktree_id = WorktreeId::new();
    let wt_path = managed_worktree_path(&state.data_root, ws_id, worktree_id);
    if let Some(parent) = wt_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    }
    let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
    create_worktree(&ws.root_path, &wt_path, &base_commit_sha, &branch_name)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: ws_id,
        root_path: wt_path.to_string_lossy().to_string(),
        base_commit_sha,
        git_branch: Some(branch_name),
        created_at: chrono::Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_config_path: None,
        bootstrap_config_key: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    };
    state
        .store
        .insert_worktree(worktree.clone())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
        Arc::clone(&state),
        ws.clone(),
        worktree.clone(),
    )
    .await
    {
        tracing::warn!(task_id = %task.id.0, "worktree bootstrap failed: {e:?}");
    }

    if let Err(e) = state
        .store
        .set_task_primary_worktree(task.id, worktree_id)
        .await
    {
        tracing::warn!(task_id = %task.id.0, "failed to set primary worktree: {e:?}");
    }

    if let Err(e) =
        attachments::ensure_worktree_attachment_mounts(&state, &ws, &worktree, false).await
    {
        tracing::warn!(task_id = %task.id.0, "attachment mounts failed: {e:?}");
    }

    let task = match state
        .store
        .get_task_with_activity(task.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })? {
        Some(task) => task,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            ))
        }
    };

    if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
        tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

#[derive(Debug, Serialize)]
struct SessionWithEnv {
    #[serde(flatten)]
    session: Session,
    env_target: String, // "worktree" | "local"
}

fn env_target_for_worktree(wt: Option<&Worktree>) -> String {
    match wt.and_then(|w| w.git_branch.as_ref()) {
        Some(_) => "worktree".to_string(),
        None => "local".to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct CreateSessionReq {
    provider_id: String,
    model_id: String,
    parent_session_id: Option<String>,
    relationship: Option<String>,
    initial_prompt: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    env_target: Option<String>, // "worktree" | "local"
}

async fn create_session_for_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<SessionWithEnv>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let task = state
        .store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let workspace = state
        .store
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    let parent_session_id = match req.parent_session_id {
        Some(id) => Some(SessionId(
            uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
        )),
        None => None,
    };
    let relationship = req
        .relationship
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    if parent_session_id.is_some() != relationship.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let worktree_id = if let Some(worktree_id) = req.worktree_id.as_deref() {
        WorktreeId(uuid::Uuid::parse_str(worktree_id).map_err(|_| StatusCode::BAD_REQUEST)?)
    } else if let Some(primary) = task.primary_worktree_id {
        primary
    } else {
        let env_target = req
            .env_target
            .as_deref()
            .unwrap_or("local")
            .trim()
            .to_lowercase();
        let base_commit_sha = rev_parse_head(&workspace.root_path).await.map_err(|e| {
            let msg = e.to_string().to_lowercase();
            if msg.contains("ambiguous argument 'head'")
                || msg.contains("unknown revision or path not in the working tree")
            {
                return StatusCode::BAD_REQUEST;
            }
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        match env_target.as_str() {
            "worktree" | "cloud" => {
                let worktree_id = WorktreeId::new();
                let wt_path =
                    managed_worktree_path(&state.data_root, task.workspace_id, worktree_id);
                if let Some(parent) = wt_path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                }
                let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
                create_worktree(
                    &workspace.root_path,
                    &wt_path,
                    &base_commit_sha,
                    &branch_name,
                )
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                let worktree = Worktree {
                    id: worktree_id,
                    workspace_id: task.workspace_id,
                    root_path: wt_path.to_string_lossy().to_string(),
                    base_commit_sha,
                    git_branch: Some(branch_name),
                    created_at: chrono::Utc::now(),
                    bootstrap_status: None,
                    bootstrap_started_at: None,
                    bootstrap_finished_at: None,
                    bootstrap_exit_code: None,
                    bootstrap_timeout_sec: None,
                    bootstrap_error: None,
                    bootstrap_log_path: None,
                    bootstrap_log_truncated: None,
                    bootstrap_config_path: None,
                    bootstrap_config_key: None,
                    bootstrap_command: None,
                    bootstrap_script_path: None,
                };
                state
                    .store
                    .insert_worktree(worktree.clone())
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
                    Arc::clone(&state),
                    workspace.clone(),
                    worktree.clone(),
                )
                .await
                {
                    tracing::warn!(task_id = %task.id.0, "worktree bootstrap failed: {e:?}");
                }
                worktree_id
            }
            "local" => {
                if let Some(existing) = state
                    .store
                    .get_local_worktree_for_root(task.workspace_id, &workspace.root_path)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                {
                    existing.id
                } else {
                    let worktree_id = WorktreeId::new();
                    let worktree = Worktree {
                        id: worktree_id,
                        workspace_id: task.workspace_id,
                        root_path: workspace.root_path.clone(),
                        base_commit_sha,
                        git_branch: None,
                        created_at: chrono::Utc::now(),
                        bootstrap_status: None,
                        bootstrap_started_at: None,
                        bootstrap_finished_at: None,
                        bootstrap_exit_code: None,
                        bootstrap_timeout_sec: None,
                        bootstrap_error: None,
                        bootstrap_log_path: None,
                        bootstrap_log_truncated: None,
                        bootstrap_config_path: None,
                        bootstrap_config_key: None,
                        bootstrap_command: None,
                        bootstrap_script_path: None,
                    };
                    state
                        .store
                        .insert_worktree(worktree)
                        .await
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                    worktree_id
                }
            }
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    };

    let session = state
        .store
        .create_session(
            task.id,
            task.workspace_id,
            worktree_id,
            req.provider_id,
            req.model_id,
            "implementer".into(),
            parent_session_id,
            relationship,
            None,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if session.parent_session_id.is_none() && session.relationship.is_none() {
        let _ = state
            .store
            .set_task_primary_session(task.id, session.id, worktree_id)
            .await;
    }

    if let Some(prompt) = req.initial_prompt {
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let msg = Message {
            id: MessageId::new(),
            session_id: session.id,
            task_id: session.task_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(0),
            role: MessageRole::User,
            content: prompt,
            attachments: vec![],
            delivery: MessageDelivery::Immediate,
            delivered_at: None,
            created_at: chrono::Utc::now(),
        };
        let saved = state
            .store
            .insert_message(msg)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let event = state
            .store
            .append_session_event(
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::UserMessage,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "content": saved.content.clone(),
                    "delivery": saved.delivery.clone(),
                    "attachments": saved.attachments,
                }),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let start_seq = event.seq;

        let turn = SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: Some(saved.id),
            status: SessionTurnStatus::Running,
            start_seq: Some(start_seq),
            end_seq: None,
            started_at: saved.created_at,
            updated_at: saved.created_at,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        };
        let _ = state.store.insert_session_turn(turn).await;
        state.publish_event(event).await;

        let prompt = saved.content.clone();
        let tx = state.ensure_scheduler(session.clone()).await;
        let queued = crate::scheduler::QueuedMessage {
            message: saved,
            enqueued_at: Instant::now(),
            run_id: run_id_header.clone(),
        };
        let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

        let _ =
            schedule_session_title_generation(state.clone(), session.clone(), prompt, false).await;
    }

    let worktree = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .ok()
        .flatten();
    let env_target = env_target_for_worktree(worktree.as_ref());
    state
        .telemetry
        .emit(TelemetryEvent::session_started(
            session.provider_id.clone(),
            session.model_id.clone(),
            Some(env_target.clone()),
        ))
        .await;
    let mut ops_event = OpsEvent::new("info", "session_started");
    ops_event.session_id = Some(session.id.0.to_string());
    ops_event.worktree_id = Some(session.worktree_id.0.to_string());
    ops_event.provider_id = Some(session.provider_id.clone());
    ops_event.meta = Some(serde_json::json!({
        "model_id": session.model_id.clone(),
        "env_target": env_target.clone(),
        "parent_session_id": session.parent_session_id.map(|id| id.0.to_string()),
        "relationship": session.relationship.clone(),
    }));
    state.ops_events.emit(ops_event);
    state.remember_session_meta(&session).await;
    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(SessionWithEnv {
        env_target,
        session,
    }))
}

#[derive(Debug, Deserialize, Default)]
struct SessionSnapshotQuery {
    limit: Option<u32>,
    include_events: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionDiffApplyReq {
    action: String, // "accept" | "reject"
    patch: String,
}

#[derive(Debug, Serialize)]
struct SessionDiffResponse {
    diff: String,
}

async fn get_session_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionSnapshotQuery>,
) -> Result<Json<SessionSnapshot>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    let include_events = parse_boolish_flag(q.include_events.as_deref(), "include_events")
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    match state
        .store
        .get_session_snapshot(session_id, limit, include_events)
        .await
    {
        Ok(Some(snapshot)) => Ok(Json(snapshot)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_session_diff(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionDiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    let worktree = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;
    let diff = ctx_fs::worktrees::diff_worktree(&worktree.root_path, &worktree.base_commit_sha)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(SessionDiffResponse { diff }))
}

async fn apply_session_diff_patch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SessionDiffApplyReq>,
) -> Result<Json<SessionDiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
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
    let reverse = match action.as_str() {
        "accept" => false,
        "reject" => true,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "action must be accept or reject".to_string(),
                }),
            ));
        }
    };

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    let worktree = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;

    ctx_fs::git::git_apply_patch(
        &worktree.root_path,
        &req.patch,
        ctx_fs::git::ApplyPatchTarget::Worktree,
        reverse,
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

    let diff = ctx_fs::worktrees::diff_worktree(&worktree.root_path, &worktree.base_commit_sha)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(SessionDiffResponse { diff }))
}

#[derive(Debug, Deserialize, Default)]
struct SessionEventsQuery {
    after_seq: Option<i64>,
    limit: Option<u32>,
    tail: Option<u32>,
    include_transient: Option<String>,
}

async fn get_session_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionEventsQuery>,
) -> Result<Json<ctx_core::models::SessionEventsPage>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 200;
    const MAX_LIMIT: u32 = 1000;

    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let include_transient = parse_boolish_flag(q.include_transient.as_deref(), "include_transient")
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let (events, has_more, next_cursor) = if let Some(tail) = q.tail {
        let tail = tail.clamp(1, MAX_LIMIT);
        let mut rows = state
            .store
            .list_session_events_tail_by_seq(session_id, tail + 1, include_transient)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let has_more = rows.len() as u32 > tail;
        if has_more {
            rows = rows.split_off(rows.len().saturating_sub(tail as usize));
        }
        let next_cursor = rows.last().map(|ev| ev.seq);
        (rows, has_more, next_cursor)
    } else {
        let mut rows = state
            .store
            .list_session_events_page_by_seq(
                session_id,
                q.after_seq,
                Some(limit + 1),
                include_transient,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let has_more = rows.len() as u32 > limit;
        if has_more {
            rows.truncate(limit as usize);
        }
        let next_cursor = rows.last().map(|ev| ev.seq);
        (rows, has_more, next_cursor)
    };

    Ok(Json(ctx_core::models::SessionEventsPage {
        session_id,
        events,
        next_cursor,
        has_more,
    }))
}

#[derive(Debug, Deserialize, Default)]
struct SessionHistoryQuery {
    before_seq: Option<i64>,
    limit: Option<u32>,
}

async fn get_session_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionHistoryQuery>,
) -> Result<Json<SessionHistoryPage>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    match state
        .store
        .get_session_history_page(session_id, q.before_seq, limit)
        .await
    {
        Ok(Some(page)) => Ok(Json(page)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn list_session_turn_tools(
    State(state): State<Arc<AppState>>,
    Path((id, turn_id)): Path<(String, String)>,
) -> Result<Json<Vec<SessionTurnTool>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let turn_id = TurnId(uuid::Uuid::parse_str(&turn_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .list_turn_tools(session_id, turn_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Debug, Deserialize, Default)]
struct FileCompletionsQuery {
    query: Option<String>,
    limit: Option<u32>,
}

async fn session_file_completions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 20;
    const MAX_LIMIT: u32 = 200;
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10);

    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktree = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let query = q.query.unwrap_or_default();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;

    let files = {
        let now = Instant::now();
        let cache = state.file_completions_cache.lock().await;
        if let Some(entry) = cache.get(&worktree.id) {
            if now.duration_since(entry.cached_at) <= CACHE_TTL {
                entry.files.clone()
            } else {
                drop(cache);
                load_and_cache_worktree_files(&state, &worktree, now).await?
            }
        } else {
            drop(cache);
            load_and_cache_worktree_files(&state, &worktree, now).await?
        }
    };

    Ok(Json(completions::filter_and_rank_paths(
        &files, &query, limit,
    )))
}

async fn workspace_file_completions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 20;
    const MAX_LIMIT: u32 = 200;
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10);

    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let ws = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let root = PathBuf::from(&ws.root_path);
    if assert_git_repo(&root).await.is_err() {
        return Ok(Json(Vec::new()));
    }

    let query = q.query.unwrap_or_default();
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;

    let files = {
        let now = Instant::now();
        let cache = state.workspace_file_completions_cache.lock().await;
        if let Some(entry) = cache.get(&ws_id) {
            if now.duration_since(entry.cached_at) <= CACHE_TTL {
                entry.files.clone()
            } else {
                drop(cache);
                load_and_cache_workspace_files(&state, ws_id, &root, now).await?
            }
        } else {
            drop(cache);
            load_and_cache_workspace_files(&state, ws_id, &root, now).await?
        }
    };

    Ok(Json(completions::filter_and_rank_paths(
        &files, &query, limit,
    )))
}

#[derive(Debug, Deserialize)]
struct WorkspaceActiveSnapshotQuery {
    limit: Option<u32>,
}

fn parse_boolish_flag(raw: Option<&str>, label: &str) -> Result<bool, String> {
    match raw {
        Some(value) => {
            let normalized = value.trim();
            if normalized.eq_ignore_ascii_case("true") || normalized == "1" {
                Ok(true)
            } else if normalized.is_empty()
                || normalized.eq_ignore_ascii_case("false")
                || normalized == "0"
            {
                Ok(false)
            } else {
                Err(format!("{label} must be true/false or 1/0"))
            }
        }
        None => Ok(false),
    }
}

async fn get_workspace_active_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<WorkspaceActiveSnapshotQuery>,
) -> Result<Json<WorkspaceActiveSnapshot>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let limit = query.limit.unwrap_or(50) as i64;
    let (tasks, total_count) = state
        .store
        .list_workspace_active_page(workspace_id, limit)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let snapshot_rev = state
        .workspace_active_snapshot
        .current_rev(workspace_id)
        .await;
    let snapshot = WorkspaceActiveSnapshot {
        workspace_id,
        snapshot_rev,
        active: WorkspaceActivePage { tasks, total_count },
    };
    Ok(Json(snapshot))
}

async fn workspace_active_snapshot_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    ws.on_upgrade(move |socket| handle_workspace_active_snapshot_ws(socket, state, workspace_id))
}

const SESSION_REPLAY_MAX_EVENTS: usize = 2000;

async fn send_workspace_active_gap(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
    reason: &str,
) -> bool {
    if crate::fault_injection::maybe_fail("ctx_http.send_workspace_active_gap").is_err() {
        return false;
    }
    let snapshot_rev = state
        .workspace_active_snapshot
        .current_rev(workspace_id)
        .await;
    let event = WorkspaceActiveSnapshotEvent::SessionGap {
        workspace_id,
        snapshot_rev,
        session_id,
        after_seq,
        reason: Some(reason.to_string()),
    };
    if let Ok(text) = serde_json::to_string(&event) {
        socket.send(WsMessage::Text(text)).await.is_ok()
    } else {
        false
    }
}

async fn replay_session_events_active(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
) -> Result<(i64, bool), ()> {
    let snapshot_rev = state
        .workspace_active_snapshot
        .current_rev(workspace_id)
        .await;
    let mut last_sent = after_seq.max(0);
    let limit = u32::try_from(SESSION_REPLAY_MAX_EVENTS + 1).unwrap_or(u32::MAX);
    let events =
        match crate::fault_injection::maybe_fail("ctx_http.replay_session_events_active.list") {
            Ok(()) => match state
                .store
                .list_session_events_page_by_seq(session_id, Some(last_sent), Some(limit), false)
                .await
            {
                Ok(events) => events,
                Err(_) => {
                    let latest = state
                        .store
                        .get_session_last_event_seq(session_id)
                        .await
                        .unwrap_or(last_sent);
                    if !send_workspace_active_gap(
                        socket,
                        state,
                        workspace_id,
                        session_id,
                        latest,
                        "replay_error",
                    )
                    .await
                    {
                        return Err(());
                    }
                    return Ok((latest, true));
                }
            },
            Err(_) => {
                let latest = state
                    .store
                    .get_session_last_event_seq(session_id)
                    .await
                    .unwrap_or(last_sent);
                if !send_workspace_active_gap(
                    socket,
                    state,
                    workspace_id,
                    session_id,
                    latest,
                    "replay_error",
                )
                .await
                {
                    return Err(());
                }
                return Ok((latest, true));
            }
        };

    if events.len() > SESSION_REPLAY_MAX_EVENTS {
        let latest = state
            .store
            .get_session_last_event_seq(session_id)
            .await
            .unwrap_or(last_sent);
        if !send_workspace_active_gap(
            socket,
            state,
            workspace_id,
            session_id,
            latest,
            "replay_too_large",
        )
        .await
        {
            return Err(());
        }
        return Ok((latest, true));
    }

    for event in events {
        let message = if matches!(
            event.event_type,
            SessionEventType::UserMessage | SessionEventType::AssistantMessageInserted
        ) {
            let message_id = event
                .payload_json
                .get("message_id")
                .and_then(|v| v.as_str())
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .map(MessageId);
            match message_id {
                Some(id) => state.store.get_message(id).await.ok().flatten(),
                None => None,
            }
        } else {
            None
        };

        let turn = if matches!(event.event_type, SessionEventType::UserMessage) {
            match event.turn_id {
                Some(turn_id) => state
                    .store
                    .get_session_turn(event.session_id, turn_id)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            }
        } else {
            None
        };

        let delta = SessionHeadDelta {
            session_id: event.session_id,
            last_event_seq: event.seq,
            event: Some(event),
            turn,
            message,
        };
        let next_seq = delta.last_event_seq;
        let wrapped = WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            workspace_id,
            snapshot_rev,
            delta: Box::new(delta),
        };
        let text = serde_json::to_string(&wrapped).map_err(|_| ())?;
        crate::fault_injection::maybe_fail("ctx_http.replay_session_events_active.send")
            .map_err(|_| ())?;
        socket.send(WsMessage::Text(text)).await.map_err(|_| ())?;
        last_sent = next_seq;
    }
    Ok((last_sent, false))
}

async fn handle_workspace_active_snapshot_ws(
    mut socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
) {
    struct SessionCursor {
        last_sent: i64,
    }

    let ready = WorkspaceActiveSnapshotEvent::Ready {
        workspace_id,
        snapshot_rev: state
            .workspace_active_snapshot
            .current_rev(workspace_id)
            .await,
    };
    if let Ok(text) = serde_json::to_string(&ready) {
        if socket.send(WsMessage::Text(text)).await.is_err() {
            return;
        }
    }
    let mut rx = state
        .workspace_active_snapshot
        .subscribe(workspace_id)
        .await;
    let mut subscriptions: HashMap<SessionId, SessionCursor> = HashMap::new();

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        if let Ok(message) = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text) {
                            let (session_ids, sessions) = match message {
                                WorkspaceActiveSnapshotClientMessage::Subscribe { session_ids, sessions } => (session_ids, sessions),
                            };
                            let mut next: Vec<WorkspaceActiveSnapshotSessionSubscription> = if !sessions.is_empty() {
                                sessions
                            } else {
                                session_ids
                                    .into_iter()
                                    .map(|session_id| WorkspaceActiveSnapshotSessionSubscription { session_id, after_seq: None })
                                    .collect()
                            };
                            next.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
                            let mut next_map = HashMap::new();
                            for sub in next {
                                let after_seq = sub.after_seq.unwrap_or(0);
                                let (last_sent, gap) = match replay_session_events_active(
                                    &mut socket,
                                    &state,
                                    workspace_id,
                                    sub.session_id,
                                    after_seq,
                                )
                                .await
                                {
                                    Ok(v) => v,
                                    Err(_) => return,
                                };
                                if gap {
                                    next_map.insert(sub.session_id, SessionCursor { last_sent });
                                    continue;
                                }
                                next_map.insert(sub.session_id, SessionCursor { last_sent });
                            }
                            subscriptions = next_map;
                        }
                    }
                    Some(Ok(WsMessage::Binary(bytes))) => {
                        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                            if let Ok(message) = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text) {
                                let (session_ids, sessions) = match message {
                                    WorkspaceActiveSnapshotClientMessage::Subscribe { session_ids, sessions } => (session_ids, sessions),
                                };
                                let mut next: Vec<WorkspaceActiveSnapshotSessionSubscription> = if !sessions.is_empty() {
                                    sessions
                                } else {
                                    session_ids
                                        .into_iter()
                                        .map(|session_id| WorkspaceActiveSnapshotSessionSubscription { session_id, after_seq: None })
                                        .collect()
                                };
                                next.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
                                let mut next_map = HashMap::new();
                                for sub in next {
                                    let after_seq = sub.after_seq.unwrap_or(0);
                                    let (last_sent, gap) = match replay_session_events_active(
                                        &mut socket,
                                        &state,
                                        workspace_id,
                                        sub.session_id,
                                        after_seq,
                                    )
                                    .await
                                    {
                                        Ok(v) => v,
                                        Err(_) => return,
                                    };
                                    if gap {
                                        next_map.insert(sub.session_id, SessionCursor { last_sent });
                                        continue;
                                    }
                                    next_map.insert(sub.session_id, SessionCursor { last_sent });
                                }
                                subscriptions = next_map;
                            }
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) => break,
                    Some(Ok(_)) => {},
                    Some(Err(_)) => break,
                    None => break,
                }
            }
            event = rx.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let mut failed = false;
                        for (session_id, cursor) in &mut subscriptions {
                            let latest = state
                                .store
                                .get_session_last_event_seq(*session_id)
                                .await
                                .unwrap_or(cursor.last_sent);
                            cursor.last_sent = latest;
                            if !send_workspace_active_gap(&mut socket, &state, workspace_id, *session_id, latest, "stream_lagged").await {
                                failed = true;
                                break;
                            }
                        }
                        if failed {
                            break;
                        }
                        break;
                    }
                    Err(_) => break,
                };

                if let WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } = &event {
                    let Some(cursor) = subscriptions.get_mut(&delta.session_id) else {
                        continue;
                    };
                    if let Some(ev) = &delta.event {
                        if ev.seq <= cursor.last_sent {
                            continue;
                        }
                        cursor.last_sent = ev.seq;
                    } else if delta.last_event_seq <= cursor.last_sent {
                        continue;
                    } else {
                        cursor.last_sent = delta.last_event_seq;
                    }
                }

                if let Ok(text) = serde_json::to_string(&event) {
                    if socket.send(WsMessage::Text(text)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_secure_workspace_gap(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
    reason: &str,
    key: &crate::mobile_e2ee::E2eeKey,
    device_id: &str,
    outbound_seq: &mut i64,
) -> bool {
    if crate::fault_injection::maybe_fail("ctx_http.send_secure_workspace_gap").is_err() {
        return false;
    }
    let snapshot_rev = state
        .workspace_active_snapshot
        .current_rev(workspace_id)
        .await;
    let event = WorkspaceActiveSnapshotEvent::SessionGap {
        workspace_id,
        snapshot_rev,
        session_id,
        after_seq,
        reason: Some(reason.to_string()),
    };
    *outbound_seq += 1;
    send_secure_ws(socket, key, device_id, *outbound_seq, &event)
        .await
        .is_ok()
}

#[allow(clippy::too_many_arguments)]
async fn replay_session_events_secure(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
    key: &crate::mobile_e2ee::E2eeKey,
    device_id: &str,
    outbound_seq: &mut i64,
) -> Result<(i64, bool), ()> {
    let snapshot_rev = state
        .workspace_active_snapshot
        .current_rev(workspace_id)
        .await;
    let mut last_sent = after_seq.max(0);
    let limit = u32::try_from(SESSION_REPLAY_MAX_EVENTS + 1).unwrap_or(u32::MAX);
    let events =
        match crate::fault_injection::maybe_fail("ctx_http.replay_session_events_secure.list") {
            Ok(()) => state
                .store
                .list_session_events_page_by_seq(session_id, Some(last_sent), Some(limit), false)
                .await
                .map_err(|_| ())?,
            Err(_) => {
                let latest = state
                    .store
                    .get_session_last_event_seq(session_id)
                    .await
                    .unwrap_or(last_sent);
                if !send_secure_workspace_gap(
                    socket,
                    state,
                    workspace_id,
                    session_id,
                    latest,
                    "replay_error",
                    key,
                    device_id,
                    outbound_seq,
                )
                .await
                {
                    return Err(());
                }
                return Ok((latest, true));
            }
        };

    if events.len() > SESSION_REPLAY_MAX_EVENTS {
        let latest = state
            .store
            .get_session_last_event_seq(session_id)
            .await
            .unwrap_or(last_sent);
        if !send_secure_workspace_gap(
            socket,
            state,
            workspace_id,
            session_id,
            latest,
            "replay_too_large",
            key,
            device_id,
            outbound_seq,
        )
        .await
        {
            return Err(());
        }
        return Ok((latest, true));
    }

    for event in events {
        let message = if matches!(
            event.event_type,
            SessionEventType::UserMessage | SessionEventType::AssistantMessageInserted
        ) {
            let message_id = event
                .payload_json
                .get("message_id")
                .and_then(|v| v.as_str())
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .map(MessageId);
            match message_id {
                Some(id) => state.store.get_message(id).await.ok().flatten(),
                None => None,
            }
        } else {
            None
        };

        let turn = if matches!(event.event_type, SessionEventType::UserMessage) {
            match event.turn_id {
                Some(turn_id) => state
                    .store
                    .get_session_turn(event.session_id, turn_id)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            }
        } else {
            None
        };

        let delta = SessionHeadDelta {
            session_id: event.session_id,
            last_event_seq: event.seq,
            event: Some(event),
            turn,
            message,
        };
        let next_seq = delta.last_event_seq;
        let wrapped = WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            workspace_id,
            snapshot_rev,
            delta: Box::new(delta),
        };
        *outbound_seq += 1;
        send_secure_ws(socket, key, device_id, *outbound_seq, &wrapped)
            .await
            .map_err(|_| ())?;
        last_sent = next_seq;
    }
    Ok((last_sent, false))
}

async fn load_and_cache_worktree_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
    let started_at = Instant::now();
    let root = PathBuf::from(&worktree.root_path);
    let mut files = list_tracked_files(&root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let untracked = list_untracked_files(&root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !untracked.is_empty() {
        let mut seen: std::collections::HashSet<String> = files.iter().cloned().collect();
        for p in untracked {
            if seen.insert(p.clone()) {
                files.push(p);
            }
        }
    }
    files.sort();
    let files = Arc::new(files);

    let mut cache = state.file_completions_cache.lock().await;
    cache.insert(
        worktree.id,
        crate::daemon::CachedFileCompletions {
            cached_at: now,
            files: files.clone(),
        },
    );
    let mut labels = HashMap::new();
    labels.insert("event".to_string(), "list_files_worktree".to_string());
    labels.insert("source".to_string(), "daemon".to_string());
    let metric = PerfMetric {
        name: "fs.list_files_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: started_at.elapsed().as_millis() as f64,
        labels,
    };
    state
        .perf_telemetry
        .record_metric(metric, None, None, None)
        .await;
    Ok(files)
}

async fn load_and_cache_workspace_files(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
    root: &PathBuf,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
    let started_at = Instant::now();
    let mut files = list_tracked_files(root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let untracked = list_untracked_files(root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !untracked.is_empty() {
        let mut seen: std::collections::HashSet<String> = files.iter().cloned().collect();
        for p in untracked {
            if seen.insert(p.clone()) {
                files.push(p);
            }
        }
    }
    files.sort();
    let files = Arc::new(files);

    let mut cache = state.workspace_file_completions_cache.lock().await;
    cache.insert(
        ws_id,
        crate::daemon::CachedFileCompletions {
            cached_at: now,
            files: files.clone(),
        },
    );
    let mut labels = HashMap::new();
    labels.insert("event".to_string(), "list_files_workspace".to_string());
    labels.insert("source".to_string(), "daemon".to_string());
    let metric = PerfMetric {
        name: "fs.list_files_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: started_at.elapsed().as_millis() as f64,
        labels,
    };
    state
        .perf_telemetry
        .record_metric(metric, None, None, None)
        .await;
    Ok(files)
}

async fn delete_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let msg_id = MessageId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let msg = state
        .store
        .get_message(msg_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if !matches!(msg.delivery, MessageDelivery::Queued) || msg.delivered_at.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    state
        .store
        .delete_message(msg_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(turn_id) = msg.turn_id {
        let _ = state
            .store
            .delete_session_turn(msg.session_id, turn_id)
            .await;
    }

    if let Some(tx) = state.scheduler_sender(msg.session_id).await {
        let _ = tx.send(SchedulerCommand::RemoveQueued(msg_id)).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct PostMessageReq {
    content: String,
    delivery: Option<MessageDelivery>,
    #[serde(default)]
    attachments: Vec<MessageAttachment>,
}

#[derive(Debug, Deserialize)]
struct ArtifactInput {
    absolute_file_path: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetSessionArtifactsReq {
    #[serde(default)]
    artifacts: Vec<ArtifactInput>,
}

async fn normalize_message_attachments(
    state: &Arc<AppState>,
    attachments: Vec<MessageAttachment>,
) -> Result<Vec<MessageAttachment>, StatusCode> {
    let mut out = Vec::with_capacity(attachments.len());
    for att in attachments {
        match att {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_base64.as_bytes())
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
                let saved =
                    persist_blob_bytes(state.as_ref(), &bytes, &mime_type, name.as_deref()).await?;
                out.push(MessageAttachment::ImageRef {
                    blob_id: saved.blob_id,
                    mime_type,
                    name,
                });
            }
            MessageAttachment::ImageRef {
                blob_id,
                mime_type,
                name,
            } => {
                let exists = state
                    .store
                    .get_blob(&blob_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    .is_some();
                if !exists {
                    return Err(StatusCode::BAD_REQUEST);
                }
                out.push(MessageAttachment::ImageRef {
                    blob_id,
                    mime_type,
                    name,
                });
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
enum TitleGenerationSource {
    Llm,
    Fallback,
}

impl TitleGenerationSource {
    fn as_str(self) -> &'static str {
        match self {
            TitleGenerationSource::Llm => "llm",
            TitleGenerationSource::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Clone)]
struct TitleGenerationOutcome {
    title: String,
    source: TitleGenerationSource,
}

async fn configured_title_generation_settings(
    state: &AppState,
) -> Option<user_settings::TitleGenerationSettings> {
    let settings = user_settings::load_settings(&state.data_root).await;
    settings
        .title_generation
        .as_ref()
        .filter(|cfg| title_generation::is_configured(cfg))
        .cloned()
}

async fn generate_title_for_prompt(
    cfg: Option<&user_settings::TitleGenerationSettings>,
    prompt: &str,
) -> anyhow::Result<TitleGenerationOutcome> {
    let fallback = title_generation::fallback_title_from_prompt(prompt);
    if fallback.trim().is_empty() {
        return Err(anyhow::anyhow!("prompt is empty"));
    }

    if let Some(cfg) = cfg.filter(|c| title_generation::is_configured(c)) {
        match title_generation::generate_title(cfg, prompt).await {
            Ok(title) => {
                return Ok(TitleGenerationOutcome {
                    title,
                    source: TitleGenerationSource::Llm,
                })
            }
            Err(err) => {
                tracing::warn!(
                    "title generation failed: {}",
                    logs::redact_sensitive(&err.to_string())
                );
                // TODO: surface a snackbar/toast when title generation fails.
            }
        }
    }

    Ok(TitleGenerationOutcome {
        title: fallback,
        source: TitleGenerationSource::Fallback,
    })
}

async fn apply_session_title_update(
    state: &Arc<AppState>,
    session: &Session,
    outcome: TitleGenerationOutcome,
) -> anyhow::Result<()> {
    let updated = state
        .store
        .update_session_title(session.id, outcome.title.clone())
        .await
        .context("updating session title")?;
    if !updated {
        return Ok(());
    }

    if let Ok(Some(updated_session)) = state.store.get_session(session.id).await {
        state.remember_session_meta(&updated_session).await;
    }

    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    let mut task_updated = false;
    if let Ok(Some(task)) = state.store.get_task(session.task_id).await {
        let title = task.title.trim();
        if (title.is_empty() || title == title_generation::DEFAULT_SESSION_TITLE)
            && state
                .store
                .update_task_title(session.task_id, outcome.title.clone())
                .await
                .unwrap_or(false)
        {
            task_updated = true;
        }
    }

    if task_updated {
        if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
            tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
        }
    }

    let notice = state
        .store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "title_generated",
                "title": outcome.title,
                "source": outcome.source.as_str(),
            }),
        )
        .await;
    if let Ok(event) = notice {
        state.publish_event(event).await;
    }

    Ok(())
}

async fn maybe_generate_session_title(
    state: Arc<AppState>,
    session: Session,
    prompt: String,
    force: bool,
    cfg: Option<user_settings::TitleGenerationSettings>,
) -> anyhow::Result<Option<TitleGenerationOutcome>> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Ok(None);
    }

    let current = session.title.trim();
    if !force && !current.is_empty() && current != title_generation::DEFAULT_SESSION_TITLE {
        return Ok(None);
    }

    let outcome = generate_title_for_prompt(cfg.as_ref(), &prompt).await?;
    apply_session_title_update(&state, &session, outcome.clone()).await?;
    Ok(Some(outcome))
}

async fn schedule_session_title_generation(
    state: Arc<AppState>,
    session: Session,
    prompt: String,
    force: bool,
) -> bool {
    let cfg = configured_title_generation_settings(&state).await;
    if cfg.is_some() {
        tokio::spawn(async move {
            let _ = maybe_generate_session_title(state, session, prompt, force, cfg).await;
        });
        true
    } else {
        let _ = maybe_generate_session_title(state, session, prompt, force, cfg).await;
        false
    }
}

async fn post_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<Message>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let delivery = match req.delivery {
        Some(d) => d,
        None => {
            if state.is_running(session_id).await {
                MessageDelivery::Queued
            } else {
                MessageDelivery::Immediate
            }
        }
    };

    let attachments = normalize_message_attachments(&state, req.attachments).await?;

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let msg = Message {
        id: MessageId::new(),
        session_id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(0),
        role: MessageRole::User,
        content: req.content,
        attachments,
        delivery,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    };
    let saved = state
        .store
        .insert_message(msg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let event = state
        .store
        .append_session_event(
            session_id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::UserMessage,
            serde_json::json!({
                "message_id": saved.id.0,
                "content": saved.content.clone(),
                "delivery": saved.delivery.clone(),
                "attachments": saved.attachments,
            }),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let start_seq = event.seq;

    let turn_status = if matches!(saved.delivery, MessageDelivery::Queued) {
        SessionTurnStatus::Queued
    } else {
        SessionTurnStatus::Running
    };
    let turn = SessionTurn {
        turn_id,
        session_id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: turn_status,
        start_seq: Some(start_seq),
        end_seq: None,
        started_at: saved.created_at,
        updated_at: saved.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    let _ = state.store.insert_session_turn(turn).await;
    state.publish_event(event).await;

    if matches!(saved.delivery, MessageDelivery::Queued) {
        let queued = state
            .store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::InputQueued,
                serde_json::json!({"message_id": saved.id.0}),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state.publish_event(queued).await;
    }

    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = crate::scheduler::QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: run_id_header.clone(),
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    if let Ok(count) = state
        .store
        .count_user_messages_for_session(session_id)
        .await
    {
        if count == 1 {
            let prompt = saved.content.clone();
            let _ =
                schedule_session_title_generation(state.clone(), session.clone(), prompt, false)
                    .await;
        }
    }

    Ok(Json(saved))
}

async fn list_session_subagents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionSummary>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let subs = state
        .store
        .list_subagent_sessions(session.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(subs))
}

#[derive(Debug, Deserialize, Default)]
struct SessionSubagentInvocationsQuery {
    turn_id: Option<String>,
}

async fn list_session_subagent_invocations(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionSubagentInvocationsQuery>,
) -> Result<Json<Vec<SubagentInvocation>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let turn_id = match q.turn_id {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(TurnId(
                    uuid::Uuid::parse_str(trimmed).map_err(|_| StatusCode::BAD_REQUEST)?,
                ))
            }
        }
        None => None,
    };

    let invocations = state
        .store
        .list_subagent_invocations_for_session(session.id, turn_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(invocations))
}

async fn get_subagent_invocation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SubagentInvocation>, StatusCode> {
    let invocation = state
        .store
        .get_subagent_invocation(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(invocation))
}

async fn list_session_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Artifact>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let mut artifacts = state
        .store
        .list_session_artifacts(session.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for artifact in artifacts.iter_mut() {
        if tokio::fs::metadata(&artifact.absolute_path).await.is_err() {
            artifact.missing = Some(true);
        }
    }

    Ok(Json(artifacts))
}

async fn set_session_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionArtifactsReq>,
) -> Result<Json<Vec<Artifact>>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let mut artifacts = Vec::with_capacity(req.artifacts.len());
    for (idx, artifact) in req.artifacts.into_iter().enumerate() {
        let raw = artifact.absolute_file_path.trim();
        if raw.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("artifact {} missing absolute_file_path", idx + 1),
                }),
            ));
        }
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("artifact {} absolute_file_path must be absolute", idx + 1),
                }),
            ));
        }
        let meta = tokio::fs::metadata(&path).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!(
                        "artifact {} path not accessible: {}",
                        idx + 1,
                        logs::redact_sensitive(&e.to_string())
                    ),
                }),
            )
        })?;
        if !meta.is_file() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("artifact {} path is not a file", idx + 1),
                }),
            ));
        }

        let name = normalize_artifact_name(artifact.name, &path);
        let mime_type = infer_artifact_mime_type(&path, artifact.mime_type);
        if artifact_is_quicktime(&path, &mime_type) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!(
                        "artifact {} uses .mov/video/quicktime; mp4 or webm supported, .mov not supported",
                        idx + 1
                    ),
                }),
            ));
        }
        let bytes = meta.len() as i64;
        let created_at = chrono::Utc::now();

        artifacts.push(Artifact {
            id: ArtifactId::new(),
            session_id: session.id,
            task_id: session.task_id,
            workspace_id: session.workspace_id,
            worktree_id: session.worktree_id,
            name,
            absolute_path: path.to_string_lossy().to_string(),
            mime_type,
            bytes,
            created_at,
            missing: None,
        });
    }

    state
        .store
        .replace_session_artifacts(session.id, &artifacts)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let event = state
        .store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::ArtifactsSet,
            serde_json::json!({ "artifacts": artifacts }),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state.publish_event(event).await;

    Ok(Json(artifacts))
}

async fn cancel_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tx = state.ensure_scheduler(session).await;
    let _ = tx.send(SchedulerCommand::Cancel).await;
    Ok(StatusCode::OK)
}

async fn interrupt_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tx = state.ensure_scheduler(session).await;
    let _ = tx.send(SchedulerCommand::Interrupt).await;
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
struct SetSessionModelReq {
    model_id: String,
}

async fn set_session_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModelReq>,
) -> Result<Json<SessionWithEnv>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id).cloned()
    }
    .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    adapter
        .set_session_model(session.id.0.to_string(), req.model_id.clone())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    state
        .store
        .update_session_model(session_id, req.model_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let updated = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let worktree = state
        .store
        .get_worktree(updated.worktree_id)
        .await
        .ok()
        .flatten();
    Ok(Json(SessionWithEnv {
        env_target: env_target_for_worktree(worktree.as_ref()),
        session: updated,
    }))
}

#[derive(Debug, Deserialize)]
struct SetSessionModeReq {
    mode_id: String,
}

async fn set_session_mode(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModeReq>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id).cloned()
    }
    .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    adapter
        .set_session_mode(session.id.0.to_string(), req.mode_id.clone())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let event = state
        .store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Init,
            serde_json::json!({"set_mode": req.mode_id}),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.publish_event(event).await;

    Ok(StatusCode::OK)
}

const DEFAULT_MAX_SUBAGENTS_PER_CALL: usize = 10;

fn resolve_max_subagents_per_call(settings: &user_settings::Settings) -> usize {
    let configured = settings
        .subagents
        .as_ref()
        .and_then(|s| s.max_per_call)
        .filter(|value| *value > 0);
    configured
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_SUBAGENTS_PER_CALL)
}
const DEFAULT_REASONING_EFFORT: &str = "medium";
const KNOWN_EFFORT_IDS: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, Deserialize)]
struct AgentInitReq {
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    response_mode: Option<String>,
    agents: Vec<AgentInitItem>,
}

#[derive(Debug, Deserialize, Clone)]
struct AgentInitItem {
    prompt: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum AgentInitResponseMode {
    Enqueue,
    Await,
}

fn parse_agent_init_response_mode(value: Option<&str>) -> Result<AgentInitResponseMode, String> {
    let trimmed = value.map(|raw| raw.trim()).filter(|raw| !raw.is_empty());
    match trimmed {
        None => Ok(AgentInitResponseMode::Enqueue),
        Some("enqueue") => Ok(AgentInitResponseMode::Enqueue),
        Some("await") => Ok(AgentInitResponseMode::Await),
        Some(_) => Err("response_mode must be 'enqueue' or 'await'".to_string()),
    }
}

fn build_subagent_request_json(agents: &[AgentInitItem]) -> serde_json::Value {
    let mut items = Vec::with_capacity(agents.len());
    for (idx, agent) in agents.iter().enumerate() {
        let prompt = agent.prompt.trim();
        let mut obj = serde_json::Map::new();
        obj.insert(
            "position".to_string(),
            serde_json::Value::Number(serde_json::Number::from(idx as u64)),
        );
        let prompt_length = prompt.chars().count() as u64;
        obj.insert(
            "prompt_length".to_string(),
            serde_json::Value::Number(serde_json::Number::from(prompt_length)),
        );
        if let Some(label) = agent
            .label
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "label".to_string(),
                serde_json::Value::String(label.to_string()),
            );
        }
        if let Some(harness) = agent
            .harness
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "harness".to_string(),
                serde_json::Value::String(harness.to_string()),
            );
        }
        if let Some(model) = agent
            .model
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }
        if let Some(reasoning_effort) = agent
            .reasoning_effort
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let norm = normalize_effort_id(reasoning_effort);
            if !norm.is_empty() {
                obj.insert(
                    "reasoning_effort".to_string(),
                    serde_json::Value::String(norm),
                );
            }
        }
        items.push(serde_json::Value::Object(obj));
    }

    serde_json::json!({
        "agents_total": agents.len(),
        "agents": items,
    })
}

#[derive(Debug, Serialize)]
struct AgentInitResp {
    invocation_id: String,
    status: String,
    results: Vec<AgentInitResult>,
}

#[derive(Debug, Serialize)]
struct AgentInitResult {
    session_id: SessionId,
    label: String,
    provider_id: String,
    model_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SubagentWaitReq {
    invocation_id: String,
}

#[derive(Debug, Serialize)]
struct SubagentWaitResp {
    invocation_id: String,
    status: String,
    results: Vec<AgentInitResult>,
}

#[derive(Debug, Deserialize)]
struct AgentReplyReq {
    session_id: String,
    prompt: String,
}

#[derive(Debug, Serialize)]
struct AgentReplyResp {
    session_id: SessionId,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

#[derive(Debug, Clone)]
struct ModelInfo {
    base: String,
    effort: Option<String>,
}

#[derive(Debug, Clone)]
struct ModelCatalog {
    full_ids: Vec<String>,
    base_ids: Vec<String>,
    efforts_by_base: HashMap<String, Vec<String>>,
    full_id_by_base_effort: HashMap<String, HashMap<String, String>>,
    info_by_full_id: HashMap<String, ModelInfo>,
}

#[derive(Debug, Clone)]
struct ResolvedModel {
    model_id: String,
}

fn normalize_effort_id(value: &str) -> String {
    let raw = value.trim().to_lowercase();
    match raw.as_str() {
        "extra_high" | "extra-high" | "extra high" | "extrahigh" => "xhigh".to_string(),
        _ => raw,
    }
}

fn is_known_effort_id(value: &str) -> bool {
    let norm = normalize_effort_id(value);
    KNOWN_EFFORT_IDS.iter().any(|id| *id == norm)
}

fn split_model_id(full: &str) -> (String, Option<String>) {
    let trimmed = full.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }
    if let Some(idx) = trimmed.rfind('/') {
        if idx > 0 && idx + 1 < trimmed.len() {
            let base = trimmed[..idx].to_string();
            let suffix = trimmed[idx + 1..].trim().to_string();
            if !suffix.is_empty() {
                return (base, Some(suffix));
            }
        }
    }
    (trimmed.to_string(), None)
}

fn has_trailing_paren_suffix(name: &str, suffix: &str) -> bool {
    let trimmed = name.trim_end();
    if !trimmed.ends_with(')') {
        return false;
    }
    let Some(start) = trimmed.rfind('(') else {
        return false;
    };
    let inner = trimmed[start + 1..trimmed.len() - 1].trim();
    normalize_effort_id(inner) == normalize_effort_id(suffix)
}

fn order_effort_ids(list: &mut [String]) {
    let order_index = |value: &str| {
        let norm = normalize_effort_id(value);
        KNOWN_EFFORT_IDS
            .iter()
            .position(|id| *id == norm)
            .unwrap_or(usize::MAX)
    };
    list.sort_by(|a, b| {
        let ia = order_index(a);
        let ib = order_index(b);
        if ia != ib {
            return ia.cmp(&ib);
        }
        a.cmp(b)
    });
}

fn extract_model_entries(models: &serde_json::Value) -> Vec<(String, Option<String>)> {
    let Some(list) = models.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in list {
        if let Some(id) = item.as_str() {
            let id = id.trim();
            if !id.is_empty() {
                out.push((id.to_string(), None));
            }
            continue;
        }
        let Some(obj) = item.as_object() else {
            continue;
        };
        let id = obj
            .get("id")
            .or_else(|| obj.get("model_id"))
            .or_else(|| obj.get("model"))
            .or_else(|| obj.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() {
            continue;
        }
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        out.push((id, name));
    }
    out
}

fn build_model_catalog(models: &serde_json::Value) -> Option<ModelCatalog> {
    let entries = extract_model_entries(models);
    if entries.is_empty() {
        return None;
    }
    let mut full_ids = HashSet::new();
    let mut base_ids = HashSet::new();
    let mut info_by_full_id = HashMap::new();
    let mut raw_efforts_by_base: HashMap<String, HashSet<String>> = HashMap::new();
    let mut full_id_by_base_effort: HashMap<String, HashMap<String, String>> = HashMap::new();

    for (id, name) in entries {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (base_candidate, suffix) = split_model_id(trimmed);
        let effort = suffix.and_then(|s| {
            if is_known_effort_id(&s)
                || name
                    .as_deref()
                    .map(|n| has_trailing_paren_suffix(n, &s))
                    .unwrap_or(false)
            {
                Some(s)
            } else {
                None
            }
        });
        let base = if effort.is_some() {
            base_candidate
        } else {
            trimmed.to_string()
        };
        base_ids.insert(base.clone());
        full_ids.insert(trimmed.to_string());
        info_by_full_id.insert(
            trimmed.to_string(),
            ModelInfo {
                base: base.clone(),
                effort: effort.clone(),
            },
        );
        if let Some(effort) = effort {
            raw_efforts_by_base
                .entry(base.clone())
                .or_default()
                .insert(effort.clone());
            full_id_by_base_effort
                .entry(base)
                .or_default()
                .insert(normalize_effort_id(&effort), trimmed.to_string());
        }
    }

    let mut efforts_by_base = HashMap::new();
    for (base, efforts) in raw_efforts_by_base {
        let mut list = efforts.into_iter().collect::<Vec<_>>();
        order_effort_ids(&mut list);
        efforts_by_base.insert(base, list);
    }

    let mut full_ids = full_ids.into_iter().collect::<Vec<_>>();
    full_ids.sort();
    let mut base_ids = base_ids.into_iter().collect::<Vec<_>>();
    base_ids.sort();

    Some(ModelCatalog {
        full_ids,
        base_ids,
        efforts_by_base,
        full_id_by_base_effort,
        info_by_full_id,
    })
}

fn pick_default_effort(efforts: &[String]) -> Option<String> {
    let medium = efforts
        .iter()
        .find(|e| normalize_effort_id(e) == DEFAULT_REASONING_EFFORT)
        .cloned();
    medium.or_else(|| efforts.first().cloned())
}

fn resolve_model_id(
    requested_model: Option<&str>,
    requested_effort: Option<&str>,
    fallback_model: Option<&str>,
    catalog: Option<&ModelCatalog>,
) -> Result<ResolvedModel, String> {
    let mut model = requested_model
        .or(fallback_model)
        .unwrap_or("")
        .trim()
        .to_string();
    if model.is_empty() {
        return Err("model is required".to_string());
    }
    let effort_input = requested_effort
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .map(|e| e.to_string());

    if let Some(catalog) = catalog {
        let model_known = catalog.full_ids.contains(&model) || catalog.base_ids.contains(&model);
        if !model_known && requested_model.is_some() {
            return Err(format!(
                "unknown model '{model}'; available models: {}",
                catalog.full_ids.join(", ")
            ));
        }

        let info = catalog.info_by_full_id.get(&model);
        let base = info
            .map(|i| i.base.clone())
            .unwrap_or_else(|| model.clone());
        let existing_effort = info.and_then(|i| i.effort.clone());
        let available_efforts = catalog
            .efforts_by_base
            .get(&base)
            .cloned()
            .unwrap_or_default();
        let supports_default_effort = available_efforts.len() >= 2;
        let effort_map = catalog.full_id_by_base_effort.get(&base);

        if let Some(req_effort) = effort_input {
            let req_norm = normalize_effort_id(&req_effort);
            if let Some(existing) = existing_effort.as_ref() {
                if normalize_effort_id(existing) != req_norm {
                    return Err(format!(
                        "model '{model}' already includes effort '{existing}'; requested '{req_effort}'"
                    ));
                }
                return Ok(ResolvedModel { model_id: model });
            }
            if let Some(map) = effort_map {
                if let Some(full_id) = map.get(&req_norm) {
                    return Ok(ResolvedModel {
                        model_id: full_id.clone(),
                    });
                }
            }
            let efforts = if available_efforts.is_empty() {
                effort_map
                    .map(|map| map.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            } else {
                available_efforts.clone()
            };
            if efforts.is_empty() {
                return Err(format!("model '{base}' does not support reasoning_effort"));
            }
            return Err(format!(
                "invalid reasoning_effort '{req_effort}' for model '{base}'; available: {}",
                efforts.join(", ")
            ));
        }

        if existing_effort.is_some() {
            return Ok(ResolvedModel { model_id: model });
        }

        if supports_default_effort {
            if let Some(default_effort) = pick_default_effort(&available_efforts) {
                let default_norm = normalize_effort_id(&default_effort);
                if let Some(map) = effort_map {
                    if let Some(full_id) = map.get(&default_norm) {
                        return Ok(ResolvedModel {
                            model_id: full_id.clone(),
                        });
                    }
                }
            }
        } else if available_efforts.len() == 1 {
            let default_effort = available_efforts[0].clone();
            let default_norm = normalize_effort_id(&default_effort);
            if let Some(map) = effort_map {
                if let Some(full_id) = map.get(&default_norm) {
                    return Ok(ResolvedModel {
                        model_id: full_id.clone(),
                    });
                }
            }
        }

        return Ok(ResolvedModel { model_id: model });
    }

    if let Some(req_effort) = effort_input {
        let (_, suffix) = split_model_id(&model);
        if suffix.is_none() {
            model = format!("{}/{}", model, req_effort);
            return Ok(ResolvedModel { model_id: model });
        }
    }

    Ok(ResolvedModel { model_id: model })
}

async fn load_provider_model_catalog(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<Option<ModelCatalog>, String> {
    let cache_key = format!("{}/{}", workspace.id.0, provider_id);
    if let Some(entry) = state.provider_options_cache.lock().await.get(&cache_key) {
        if let Some(models) = entry.value.get("models") {
            if let Some(catalog) = build_model_catalog(models) {
                return Ok(Some(catalog));
            }
        }
    }

    let cfg = installer::load_agent_server_config(&state.data_root)
        .await
        .unwrap_or_default();
    let matrix =
        crate::provider_matrix::load_matrix_cached(&state.data_root, &state.provider_matrix_cache)
            .await;
    let (command, args) = cfg
        .providers
        .get(provider_id)
        .map(|c| (c.command.clone(), c.args.clone()))
        .or_else(|| default_agent_server_command(&matrix, &state.data_root, provider_id))
        .ok_or_else(|| "unknown provider id".to_string())?;

    let agent = AcpAgentConfig {
        provider_id: provider_id.to_string(),
        command,
        args,
    };
    let client = AcpClientConfig {
        client_name: "ctx".to_string(),
        client_title: "ctx".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        client_capabilities: serde_json::json!({}),
        system_prompt_append: None,
        mcp_servers: vec![],
    };
    let mut env = std::collections::HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.as_ref() {
        env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
    }

    let probe =
        match probe_provider_options(agent, client, PathBuf::from(&workspace.root_path), env).await
        {
            Ok(probe) => probe,
            Err(e) => {
                tracing::warn!(
                    provider_id = provider_id,
                    "provider options probe failed: {}",
                    logs::redact_sensitive(&e.to_string())
                );
                return Ok(None);
            }
        };

    if let Some(models) = probe.models.as_ref().and_then(build_model_catalog) {
        let mut value = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace.id.0,
            "installed": true,
            "probe_ok": true,
            "supports_load": probe.supports_load,
            "auth_required": probe.auth_required,
            "auth_methods": probe.auth_methods,
            "modes": probe.modes,
            "models": probe.models,
            "acp_error": probe.acp_error,
            "probed_at": chrono::Utc::now().to_rfc3339(),
        });
        value = redact_json_value(value);
        state.provider_options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value,
            },
        );
        return Ok(Some(models));
    }

    Ok(None)
}

async fn wait_for_run_terminal_event(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: RunId,
) -> Result<SessionEventType, String> {
    if let Some(event) = state
        .store
        .get_terminal_event_for_run(session_id, run_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?
    {
        return Ok(event.event_type);
    }

    let mut rx = state.get_broadcaster(session_id).await.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                if event.run_id == Some(run_id)
                    && matches!(
                        event.event_type,
                        SessionEventType::Done
                            | SessionEventType::Error
                            | SessionEventType::TurnInterrupted
                    )
                {
                    return Ok(event.event_type);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if let Some(event) = state
                    .store
                    .get_terminal_event_for_run(session_id, run_id)
                    .await
                    .map_err(|e| logs::redact_sensitive(&e.to_string()))?
                {
                    return Ok(event.event_type);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Err("session event stream closed".to_string());
            }
        }
    }
}

async fn emit_subagent_invocation_notice(
    state: &Arc<AppState>,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
    payload: serde_json::Value,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let event = state
        .store
        .append_session_event(
            parent_session_id,
            None,
            parent_turn_id,
            SessionEventType::Notice,
            payload,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state.publish_event(event).await;
    Ok(())
}

fn build_agent_init_result(
    child: &SubagentInvocationChild,
    status: String,
    content: Option<String>,
) -> AgentInitResult {
    let label = child
        .label
        .clone()
        .unwrap_or_else(|| format!("Subagent {}", child.position + 1));
    let provider_id = child.harness.clone().unwrap_or_default();
    let model_id = child.model.clone().unwrap_or_default();
    AgentInitResult {
        session_id: child.child_session_id,
        label,
        provider_id,
        model_id,
        status,
        content,
    }
}

async fn run_subagent_child(
    state: &Arc<AppState>,
    child: SubagentInvocationChild,
) -> Result<AgentInitResult, String> {
    let run_id = child
        .run_id
        .ok_or_else(|| "subagent run_id missing".to_string())?;
    let terminal = wait_for_run_terminal_event(state, child.child_session_id, run_id).await;
    let status = match terminal {
        Ok(SessionEventType::Done) => "completed",
        Ok(SessionEventType::TurnInterrupted) => "interrupted",
        Ok(SessionEventType::Error) => "failed",
        Ok(_) => "completed",
        Err(_) => "unknown",
    }
    .to_string();

    let child_updated_at = chrono::Utc::now();
    let mut updated_child = child.clone();
    updated_child.status = status.clone();
    updated_child.updated_at = child_updated_at;
    state
        .store
        .upsert_subagent_invocation_child(updated_child)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;

    let content = state
        .store
        .get_last_assistant_message_for_run(child.child_session_id, run_id)
        .await
        .ok()
        .flatten()
        .map(|m| m.content);

    Ok(build_agent_init_result(&child, status, content))
}

async fn finalize_subagent_invocation(
    state: &Arc<AppState>,
    invocation_id: &str,
    tool_call_id: &str,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
) -> Result<(), String> {
    let Some(invocation) = state
        .store
        .get_subagent_invocation(invocation_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?
    else {
        return Ok(());
    };

    if invocation.children.is_empty() {
        return Ok(());
    }
    if invocation
        .children
        .iter()
        .any(|child| child.status == "running")
    {
        return Ok(());
    }

    let final_status = if invocation
        .children
        .iter()
        .all(|child| child.status == "completed")
    {
        "completed"
    } else {
        "failed"
    };
    if invocation.status == final_status {
        return Ok(());
    }

    let updated_at = chrono::Utc::now();
    state
        .store
        .update_subagent_invocation_status(invocation_id, final_status, updated_at)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let child_session_ids = invocation
        .children
        .iter()
        .map(|child| child.child_session_id.0.to_string())
        .collect::<Vec<_>>();
    let child_statuses = invocation
        .children
        .iter()
        .map(|child| {
            serde_json::json!({
                "session_id": child.child_session_id.0.to_string(),
                "status": child.status,
            })
        })
        .collect::<Vec<_>>();
    emit_subagent_invocation_notice(
        state,
        parent_session_id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id,
            "tool_call_id": tool_call_id,
            "status": final_status,
            "child_session_ids": child_session_ids,
            "child_statuses": child_statuses,
        }),
    )
    .await
    .map_err(|(_, err)| err.0.error)?;

    Ok(())
}

async fn build_subagent_results_from_invocation(
    state: &Arc<AppState>,
    invocation: &SubagentInvocation,
) -> Result<Vec<AgentInitResult>, String> {
    let mut results = Vec::with_capacity(invocation.children.len());
    for child in &invocation.children {
        let content = match child.run_id {
            Some(run_id) => state
                .store
                .get_last_assistant_message_for_run(child.child_session_id, run_id)
                .await
                .ok()
                .flatten()
                .map(|m| m.content),
            None => None,
        };
        results.push(build_agent_init_result(
            child,
            child.status.clone(),
            content,
        ));
    }
    Ok(results)
}

async fn enqueue_subagent_prompt(
    state: &Arc<AppState>,
    session: &Session,
    prompt: String,
) -> Result<(RunId, Message), (StatusCode, Json<ApiErrorResp>)> {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let msg = Message {
        id: MessageId::new(),
        session_id: session.id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(0),
        role: MessageRole::User,
        content: prompt,
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    };
    let saved = state.store.insert_message(msg).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let event = state
        .store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::UserMessage,
            serde_json::json!({
                "message_id": saved.id.0,
                "content": saved.content.clone(),
                "delivery": saved.delivery.clone(),
                "attachments": saved.attachments,
            }),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let start_seq = event.seq;

    let turn = SessionTurn {
        turn_id,
        session_id: session.id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: SessionTurnStatus::Running,
        start_seq: Some(start_seq),
        end_seq: None,
        started_at: saved.created_at,
        updated_at: saved.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    let _ = state.store.insert_session_turn(turn).await;
    state.publish_event(event).await;

    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = crate::scheduler::QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: None,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    Ok((run_id, saved))
}

async fn mcp_agent_init(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentInitReq>,
) -> Result<Json<AgentInitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    if req.agents.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "agents is required".to_string(),
            }),
        ));
    }
    let settings = user_settings::load_settings(&state.data_root).await;
    let max_subagents = resolve_max_subagents_per_call(&settings);
    if req.agents.len() > max_subagents {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("max {max_subagents} subagents per call"),
            }),
        ));
    }
    let response_mode = parse_agent_init_response_mode(req.response_mode.as_deref())
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;

    let parent = state
        .store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;
    let workspace = state
        .store
        .get_workspace(parent.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let mut provider_ids = HashSet::new();
    for agent in &req.agents {
        let provider_id = agent
            .harness
            .as_deref()
            .unwrap_or(&parent.provider_id)
            .trim();
        if provider_id.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "harness is required".to_string(),
                }),
            ));
        }
        provider_ids.insert(provider_id.to_string());
    }

    let available_providers: Vec<String> = {
        let statuses = state.provider_statuses.lock().await;
        let mut ids = statuses.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    };
    let mut provider_statuses = HashMap::new();
    {
        let statuses = state.provider_statuses.lock().await;
        for provider_id in provider_ids.iter() {
            if let Some(status) = statuses.get(provider_id) {
                provider_statuses.insert(provider_id.clone(), status.clone());
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!(
                            "unknown harness '{provider_id}'; available harnesses: {}",
                            available_providers.join(", ")
                        ),
                    }),
                ));
            }
        }
    }

    for (provider_id, status) in &provider_statuses {
        if !status.installed
            || !matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("harness '{provider_id}' is not installed or unhealthy"),
                }),
            ));
        }
    }

    let mut model_catalogs: HashMap<String, Option<ModelCatalog>> = HashMap::new();
    for provider_id in provider_ids.iter() {
        let catalog = load_provider_model_catalog(&state, &workspace, provider_id).await;
        match catalog {
            Ok(cat) => {
                model_catalogs.insert(provider_id.clone(), cat);
            }
            Err(err) => {
                return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: err })));
            }
        }
    }

    let request_json = Some(build_subagent_request_json(&req.agents));

    let mut requested_tool_call_id = req
        .tool_call_id
        .as_deref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let invocation_id = requested_tool_call_id
        .clone()
        .unwrap_or_else(|| format!("subagent-{}", uuid::Uuid::new_v4()));
    let tool_call_id = requested_tool_call_id
        .take()
        .unwrap_or_else(|| invocation_id.clone());

    let mut parent_turn_id = None;
    if !tool_call_id.trim().is_empty() {
        if let Ok(Some(tool)) = state
            .store
            .get_session_turn_tool(parent.id, &tool_call_id)
            .await
        {
            parent_turn_id = Some(tool.turn_id);
        }
    }
    if parent_turn_id.is_none() {
        if let Ok(turns) = state
            .store
            .list_session_turns_page_by_seq(parent.id, None, Some(5))
            .await
        {
            for turn in turns.iter().rev() {
                if matches!(
                    turn.status,
                    SessionTurnStatus::Running | SessionTurnStatus::Queued
                ) {
                    parent_turn_id = Some(turn.turn_id);
                    break;
                }
            }
        }
    }

    let now = chrono::Utc::now();
    let invocation = SubagentInvocation {
        id: invocation_id.clone(),
        tool_call_id: tool_call_id.clone(),
        parent_session_id: parent.id,
        parent_turn_id,
        requested_count: req.agents.len() as i64,
        request_json,
        status: "requested".to_string(),
        created_at: now,
        updated_at: now,
        children: Vec::new(),
    };
    state
        .store
        .upsert_subagent_invocation(invocation)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    emit_subagent_invocation_notice(
        &state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_created",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "requested",
            "requested_count": req.agents.len(),
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let running_at = chrono::Utc::now();
    state
        .store
        .update_subagent_invocation_status(&invocation_id, "running", running_at)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    emit_subagent_invocation_notice(
        &state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "running",
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let child_ids = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));

    let mut futures = Vec::with_capacity(req.agents.len());
    for (idx, agent) in req.agents.into_iter().enumerate() {
        let state = state.clone();
        let parent = parent.clone();
        let model_catalogs = model_catalogs.clone();
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        let child_ids = child_ids.clone();
        let parent_turn_id = parent_turn_id;
        futures.push(async move {
            let prompt = agent.prompt.trim().to_string();
            if prompt.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("agent {} prompt is required", idx + 1),
                    }),
                ));
            }
            let label = agent
                .label
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Subagent {}", idx + 1));
            let provider_id = agent
                .harness
                .as_deref()
                .unwrap_or(&parent.provider_id)
                .trim()
                .to_string();
            let catalog = model_catalogs.get(&provider_id).and_then(|v| v.as_ref());
            let fallback_model = if agent.model.is_none() {
                if provider_id == parent.provider_id {
                    Some(parent.model_id.as_str())
                } else {
                    catalog.and_then(|c| c.full_ids.first().map(|s| s.as_str()))
                }
            } else {
                None
            };
            if agent.model.is_none() && fallback_model.is_none() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("model is required for harness '{provider_id}'"),
                    }),
                ));
            }
            let resolved = resolve_model_id(
                agent.model.as_deref(),
                agent.reasoning_effort.as_deref(),
                fallback_model,
                catalog,
            )
            .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;

            let prompt_length = prompt.chars().count() as i64;
            let requested_effort = agent
                .reasoning_effort
                .as_deref()
                .map(normalize_effort_id)
                .filter(|value| !value.is_empty());
            let (_, model_effort) = split_model_id(&resolved.model_id);
            let reasoning_effort = requested_effort.or(model_effort);

            let session = state
                .store
                .create_session(
                    parent.task_id,
                    parent.workspace_id,
                    parent.worktree_id,
                    provider_id.clone(),
                    resolved.model_id.clone(),
                    "subagent".into(),
                    Some(parent.id),
                    Some("sub_agent".to_string()),
                    None,
                )
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;

            if state
                .store
                .update_session_title(session.id, label.clone())
                .await
                .is_err()
            {
                tracing::warn!(session_id = %session.id.0, "failed to set subagent label");
            }

            let child_created_at = chrono::Utc::now();
            let (run_id, _message) = enqueue_subagent_prompt(&state, &session, prompt).await?;
            let child_session_id = session.id;
            let child = SubagentInvocationChild {
                invocation_id: invocation_id.clone(),
                child_session_id,
                run_id: Some(run_id),
                position: idx as i64,
                status: "running".to_string(),
                label: Some(label),
                harness: Some(provider_id),
                model: Some(resolved.model_id),
                reasoning_effort,
                prompt_length,
                created_at: child_created_at,
                updated_at: child_created_at,
            };
            state
                .store
                .upsert_subagent_invocation_child(child.clone())
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;

            let child_ids_snapshot = {
                let mut ids = child_ids.lock().await;
                let child_id_string = child_session_id.0.to_string();
                ids.push(child_id_string);
                ids.clone()
            };
            emit_subagent_invocation_notice(
                &state,
                parent.id,
                parent_turn_id,
                serde_json::json!({
                    "kind": "subagent_invocation_updated",
                    "invocation_id": invocation_id.clone(),
                    "tool_call_id": tool_call_id.clone(),
                    "status": "running",
                    "child_session_ids": child_ids_snapshot,
                }),
            )
            .await?;

            Ok(child)
        });
    }

    let children = match futures::future::try_join_all(futures).await {
        Ok(children) => children,
        Err(err) => {
            let updated_at = chrono::Utc::now();
            if let Err(e) = state
                .store
                .update_subagent_invocation_status(&invocation_id, "failed", updated_at)
                .await
            {
                tracing::warn!(error = ?e, "failed to update subagent invocation status");
            }
            let child_session_ids = {
                let ids = child_ids.lock().await;
                ids.clone()
            };
            let _ = emit_subagent_invocation_notice(
                &state,
                parent.id,
                parent_turn_id,
                serde_json::json!({
                    "kind": "subagent_invocation_updated",
                    "invocation_id": invocation_id.clone(),
                    "tool_call_id": tool_call_id.clone(),
                    "status": "failed",
                    "child_session_ids": child_session_ids,
                }),
            )
            .await;
            return Err(err);
        }
    };

    match response_mode {
        AgentInitResponseMode::Await => {
            let mut run_futures = Vec::with_capacity(children.len());
            for child in children.into_iter() {
                let state = state.clone();
                run_futures.push(async move {
                    run_subagent_child(&state, child).await.map_err(|error| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiErrorResp { error }),
                        )
                    })
                });
            }

            let results = match futures::future::try_join_all(run_futures).await {
                Ok(results) => results,
                Err(err) => {
                    let updated_at = chrono::Utc::now();
                    if let Err(e) = state
                        .store
                        .update_subagent_invocation_status(&invocation_id, "failed", updated_at)
                        .await
                    {
                        tracing::warn!(
                            error = ?e,
                            "failed to update subagent invocation status"
                        );
                    }
                    let child_session_ids = {
                        let ids = child_ids.lock().await;
                        ids.clone()
                    };
                    let _ = emit_subagent_invocation_notice(
                        &state,
                        parent.id,
                        parent_turn_id,
                        serde_json::json!({
                            "kind": "subagent_invocation_updated",
                            "invocation_id": invocation_id.clone(),
                            "tool_call_id": tool_call_id.clone(),
                            "status": "failed",
                            "child_session_ids": child_session_ids,
                        }),
                    )
                    .await;
                    return Err(err);
                }
            };

            let final_status = if results.iter().all(|r| r.status == "completed") {
                "completed"
            } else {
                "failed"
            };
            if let Err(error) = finalize_subagent_invocation(
                &state,
                &invocation_id,
                &tool_call_id,
                parent.id,
                parent_turn_id,
            )
            .await
            {
                tracing::warn!(error = %error, "failed to finalize subagent invocation");
            }

            Ok(Json(AgentInitResp {
                invocation_id,
                status: final_status.to_string(),
                results,
            }))
        }
        AgentInitResponseMode::Enqueue => {
            for child in children.iter().cloned() {
                let state = state.clone();
                let invocation_id = invocation_id.clone();
                let tool_call_id = tool_call_id.clone();
                let parent_id = parent.id;
                tokio::spawn(async move {
                    if let Err(error) = run_subagent_child(&state, child).await {
                        tracing::warn!(error = %error, "subagent execution failed");
                    }
                    if let Err(error) = finalize_subagent_invocation(
                        &state,
                        &invocation_id,
                        &tool_call_id,
                        parent_id,
                        parent_turn_id,
                    )
                    .await
                    {
                        tracing::warn!(error = %error, "failed to finalize subagent invocation");
                    }
                });
            }

            let results = children
                .iter()
                .map(|child| build_agent_init_result(child, "running".to_string(), None))
                .collect();

            Ok(Json(AgentInitResp {
                invocation_id,
                status: "running".to_string(),
                results,
            }))
        }
    }
}

async fn mcp_agent_reply(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentReplyReq>,
) -> Result<Json<AgentReplyResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let parent = state
        .store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let child_id = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid child session id".to_string(),
            }),
        )
    })?);
    let child = state
        .store
        .get_session(child_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "subagent session not found".to_string(),
            }),
        ))?;

    if child.parent_session_id != Some(parent.id)
        || child.relationship.as_deref() != Some("sub_agent")
    {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorResp {
                error: "session is not a subagent of the parent".to_string(),
            }),
        ));
    }

    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt is required".to_string(),
            }),
        ));
    }

    let (run_id, _message) = enqueue_subagent_prompt(&state, &child, prompt).await?;
    let terminal = wait_for_run_terminal_event(&state, child.id, run_id).await;
    let status = match terminal {
        Ok(SessionEventType::Done) => "completed",
        Ok(SessionEventType::TurnInterrupted) => "interrupted",
        Ok(SessionEventType::Error) => "failed",
        Ok(_) => "completed",
        Err(_) => "unknown",
    }
    .to_string();

    let content = state
        .store
        .get_last_assistant_message_for_run(child.id, run_id)
        .await
        .ok()
        .flatten()
        .map(|m| m.content);

    Ok(Json(AgentReplyResp {
        session_id: child.id,
        status,
        content,
    }))
}

async fn mcp_subagent_wait(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubagentWaitReq>,
) -> Result<Json<SubagentWaitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let parent = state
        .store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let invocation_id = req.invocation_id.trim();
    if invocation_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invocation_id is required".to_string(),
            }),
        ));
    }

    let invocation = state
        .store
        .get_subagent_invocation(invocation_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "subagent invocation not found".to_string(),
            }),
        ))?;

    if invocation.parent_session_id != parent.id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorResp {
                error: "invocation does not belong to session".to_string(),
            }),
        ));
    }

    if invocation.children.is_empty() {
        return Ok(Json(SubagentWaitResp {
            invocation_id: invocation.id,
            status: invocation.status,
            results: Vec::new(),
        }));
    }

    if matches!(invocation.status.as_str(), "requested" | "running") {
        if invocation
            .children
            .iter()
            .any(|child| child.run_id.is_none())
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "subagent run_id missing; cannot wait".to_string(),
                }),
            ));
        }

        let mut futures = Vec::with_capacity(invocation.children.len());
        for child in invocation.children.clone() {
            let state = state.clone();
            futures.push(async move {
                run_subagent_child(&state, child).await.map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp { error }),
                    )
                })
            });
        }
        let results = futures::future::try_join_all(futures).await?;
        let final_status = if results.iter().all(|r| r.status == "completed") {
            "completed"
        } else {
            "failed"
        };

        if let Err(error) = finalize_subagent_invocation(
            &state,
            invocation_id,
            &invocation.tool_call_id,
            parent.id,
            invocation.parent_turn_id,
        )
        .await
        {
            tracing::warn!(error = %error, "failed to finalize subagent invocation");
        }

        return Ok(Json(SubagentWaitResp {
            invocation_id: invocation_id.to_string(),
            status: final_status.to_string(),
            results,
        }));
    }

    let results = build_subagent_results_from_invocation(&state, &invocation)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp { error }),
            )
        })?;

    Ok(Json(SubagentWaitResp {
        invocation_id: invocation.id,
        status: invocation.status,
        results,
    }))
}

#[derive(Debug, Deserialize)]
struct GenerateSessionTitleReq {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    force: Option<bool>,
}

async fn generate_session_title(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<GenerateSessionTitleReq>,
) -> Result<Json<Session>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let prompt = if let Some(prompt) = req
        .prompt
        .as_ref()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
    {
        prompt
    } else {
        state
            .store
            .get_first_user_message_content(session_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
            .filter(|p| !p.trim().is_empty())
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "prompt required".to_string(),
                }),
            ))?
    };

    let force = req.force.unwrap_or(true);
    let cfg = configured_title_generation_settings(&state).await;
    maybe_generate_session_title(state.clone(), session.clone(), prompt, force, cfg)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title generation skipped".to_string(),
            }),
        ))?;

    let updated = state
        .store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
struct AuthenticateSessionReq {
    #[serde(default)]
    method_id: Option<String>,
}

async fn authenticate_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AuthenticateSessionReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id).cloned()
    }
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "provider adapter not available".to_string(),
            }),
        )
    })?;

    let worktree = state
        .store
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
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;

    let workdir = PathBuf::from(worktree.root_path.clone());

    let mut provider_env = std::collections::HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CTX_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    if let Ok(v) = std::env::var("CTX_MCP_COMMAND") {
        provider_env.insert("CTX_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CTX_MCP_DISABLED") {
        provider_env.insert("CTX_MCP_DISABLED".to_string(), v);
    }

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let state_for_events = state.clone();
    let store = state.store.clone();
    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            let payload = ev.payload_json.clone();
            if matches!(ev.event_type, SessionEventType::Init) {
                if let Some(ps) = payload
                    .get("acp_session_id")
                    .and_then(serde_json::Value::as_str)
                {
                    let _ = store
                        .update_session_provider_session_ref(session_id, Some(ps.to_string()))
                        .await;
                }
            }
            let appended = store
                .append_session_event(session_id, None, None, ev.event_type.clone(), payload)
                .await;
            if let Ok(event) = appended {
                state_for_events.publish_event(event).await;
            }
        }
    });

    let started = state
        .store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "auth_started",
                "provider": session.provider_id,
                "method_id": req.method_id,
            }),
        )
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to append auth event".to_string(),
                }),
            )
        })?;
    state.publish_event(started).await;

    let session_key = session.id.0.to_string();
    let result = adapter
        .authenticate_session(
            session_key,
            workdir,
            provider_env,
            req.method_id.clone(),
            ev_tx,
        )
        .await;

    match result {
        Ok(()) => {
            let done = state
                .store
                .append_session_event(
                    session_id,
                    None,
                    None,
                    SessionEventType::Notice,
                    serde_json::json!({
                        "kind": "auth_finished",
                        "provider": session.provider_id,
                    }),
                )
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "failed to append auth event".to_string(),
                        }),
                    )
                })?;
            state.publish_event(done).await;
            Ok(StatusCode::OK)
        }
        Err(e) => {
            let msg = logs::redact_sensitive(&e.to_string());
            let failed = state
                .store
                .append_session_event(
                    session_id,
                    None,
                    None,
                    SessionEventType::Notice,
                    serde_json::json!({
                        "kind": "auth_failed",
                        "provider": session.provider_id,
                        "message": msg,
                    }),
                )
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "failed to append auth event".to_string(),
                        }),
                    )
                })?;
            state.publish_event(failed).await;
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "authentication failed".to_string(),
                }),
            ))
        }
    }
}

#[derive(Debug, Deserialize)]
struct SubmitAskUserQuestionReq {
    tool_call_id: String,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    answers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct SubmitAskUserQuestionResp {
    ok: bool,
}

async fn submit_ask_user_question(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubmitAskUserQuestionReq>,
) -> Result<Json<SubmitAskUserQuestionResp>, (StatusCode, Json<ApiErrorResp>)> {
    let session_uuid = uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?;
    let session_id = SessionId(session_uuid);

    // Validate the session exists (prevents accidentally fulfilling a prompt for a deleted session).
    let exists = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .is_some();
    if !exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ));
    }

    let tool_call_id = req.tool_call_id.trim().to_string();
    if tool_call_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "missing tool_call_id".to_string(),
            }),
        ));
    }

    let outcome = match req.outcome.as_deref() {
        Some("cancelled") => AskUserQuestionOutcome::Cancelled,
        Some("submitted") | None => AskUserQuestionOutcome::Submitted,
        Some(other) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("invalid outcome: {other}"),
                }),
            ))
        }
    };
    let answers = req.answers.unwrap_or_default();
    let answers_for_event = answers.clone();

    let ok = state
        .ask_user_question
        .submit(
            &session_uuid.to_string(),
            &tool_call_id,
            AskUserQuestionAnswer { outcome, answers },
        )
        .await;

    if !ok {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "no pending AskUserQuestion for this tool_call_id".to_string(),
            }),
        ));
    }

    if let Ok(event) = state
        .store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "ask_user_question_answered",
                "tool_call_id": tool_call_id,
                "outcome": outcome.as_str(),
                "answers": answers_for_event,
            }),
        )
        .await
    {
        state.publish_event(event).await;
    }

    Ok(Json(SubmitAskUserQuestionResp { ok: true }))
}

async fn dictation_livekit_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| dictation_livekit::dictation_livekit_stream(socket, state))
}

async fn verify_mobile_api_token(
    state: &Arc<AppState>,
    token: &str,
) -> Result<Option<ConnectionProfileId>, StatusCode> {
    let hash = hash_api_token(token);
    let profile = state
        .store
        .get_mobile_connection_profile_by_token_hash(&hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to query mobile connection profile: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if let Some(profile) = profile {
        if let Err(err) = state
            .store
            .mark_mobile_connection_profile_used(profile.id)
            .await
        {
            tracing::warn!("failed to update profile usage: {err:?}");
        }
        Ok(Some(profile.id))
    } else {
        Ok(None)
    }
}

fn hash_api_token(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn hash_pairing_token(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn generate_mobile_api_token() -> String {
    format!("ctxm_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
}

fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use ctx_providers::fake::FakeProviderAdapter;
    use ctx_store::Store;

    async fn setup_state() -> (tempfile::TempDir, Arc<AppState>, Session) {
        let data_dir = tempfile::tempdir().unwrap();
        let db_dir = data_dir.path().join("db");
        tokio::fs::create_dir_all(&db_dir).await.unwrap();
        let db_path = db_dir.join("db.sqlite");
        let store = Store::open(&db_path).await.unwrap();

        let workspace = store
            .create_workspace(
                "ws".to_string(),
                data_dir.path().to_string_lossy().to_string(),
            )
            .await
            .unwrap();
        let worktree = store
            .create_worktree(
                workspace.id,
                data_dir.path().to_string_lossy().to_string(),
                "base".to_string(),
                None,
            )
            .await
            .unwrap();
        let task = store
            .create_task(
                workspace.id,
                title_generation::DEFAULT_SESSION_TITLE.to_string(),
                None,
            )
            .await
            .unwrap();
        let session = store
            .create_session(
                task.id,
                workspace.id,
                worktree.id,
                "fake".to_string(),
                "fake-model".to_string(),
                "implementer".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
            HashMap::new();
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

        let state = Arc::new(AppState::new(
            data_dir.path().to_path_buf(),
            store,
            providers,
            "http://127.0.0.1:0".to_string(),
            None,
        ));

        (data_dir, state, session)
    }

    #[tokio::test]
    async fn schedule_title_generation_falls_back_without_config() {
        let (_data_dir, state, session) = setup_state().await;
        let prompt = "make the title this: hello world";
        let spawned = schedule_session_title_generation(
            state.clone(),
            session.clone(),
            prompt.to_string(),
            false,
        )
        .await;

        assert!(!spawned);

        let updated = state.store.get_session(session.id).await.unwrap().unwrap();
        let expected = title_generation::fallback_title_from_prompt(prompt);
        assert_eq!(updated.title, expected);
    }
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
    let summary =
        state
            .perf_telemetry
            .summary(q.metric.as_deref(), q.run_id.as_deref(), q.window_ms, limit);
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
    let path = crate::perf_telemetry::perf_log_path_for_date(&state.data_root, &date);
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
            .perf_telemetry
            .record_metric(metric, event.run_id, None, None)
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
