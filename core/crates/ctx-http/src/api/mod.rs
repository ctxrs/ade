use std::collections::HashMap;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, MatchedPath, Path, Query, State};
use axum::http::header;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::Json;
use base64::Engine;
use opentelemetry::trace::SpanKind;
use opentelemetry::KeyValue;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio::process::Command;
use tower::util::ServiceExt;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use url::Url;

mod artifacts;
mod auth;
mod demo;
pub(crate) mod errors;
mod execution;
mod extractors;
mod lsp;
mod merge_queue_api;
mod mobile_access;
mod provider_catalog;
mod provider_launch;
pub(crate) mod provider_probe_auth;
pub(crate) mod providers;
mod repo;
pub(crate) mod sessions;
mod settings;
pub(crate) mod shared;
pub(crate) mod tasks;
mod telemetry;
mod terminals;
mod types;
mod updates;
mod web_sessions;
mod workspaces;
mod ws;
use artifacts::*;
use execution::*;
use lsp::*;
use merge_queue_api::*;
use mobile_access::*;
use providers::*;
use repo::*;
use sessions::*;
use settings::*;
use tasks::*;
use telemetry::*;
use terminals::*;
use types::*;
use updates::*;
use web_sessions::*;
use workspaces::*;

use auth::{
    auth_middleware, generate_mobile_api_token, generate_pairing_token, hash_api_token,
    hash_pairing_token, MobileAuthContext,
};
use demo::*;
use errors::ApiErrorResp;
use extractors::extract_workspace_edit_from_command;
use ws::{
    dictation_livekit_stream_ws, mobile_secure_workspace_stream_ws, terminal_stream_ws,
    web_session_signal, workspace_active_snapshot_stream_ws,
};

use ctx_core::{ids::*, models::*};
use ctx_store::store::MobileDeviceUpsert;

use crate::buffers::{
    BufferCloseReq, BufferConflictResp, BufferId, BufferOpenReq, BufferOpenResp, BufferUpdateReq,
    BufferUpdateResp,
};
use crate::daemon::AppState;
use crate::installer;
use crate::logs;
use crate::merge_queue;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::resource_utilization;
use crate::title_generation_local;
use crate::web_sessions::{
    render_web_session_view, WebSessionInfo, WebSessionRunRequest, WebSessionRunResponse,
    WebSessionViewport,
};
use ctx_provider_install::install_state::InstallId;
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
        .unwrap_or_else(|| {
            fallback_url
                .as_ref()
                .map(|url| url.scheme().to_string())
                .unwrap_or_else(|| "http".to_string())
        })
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase();
    let host = forwarded_host
        .or_else(|| header_first_value(headers, "x-forwarded-host"))
        .or_else(|| header_first_value(headers, header::HOST.as_str()))
        .or(fallback_host);

    match host {
        Some(host)
            if is_safe_request_base_host(&host) && matches!(proto.as_str(), "http" | "https") =>
        {
            format!("{}://{}", proto, host.trim_end_matches('/'))
        }
        None => fallback.to_string(),
        Some(_) => fallback.to_string(),
    }
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim().trim_matches('[').trim_matches(']');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized.eq_ignore_ascii_case("tauri.localhost")
        || normalized == "127.0.0.1"
        || normalized == "::1"
}

fn is_safe_request_base_host(host: &str) -> bool {
    let trimmed = host.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    let parsed = match Url::parse(&format!("http://{trimmed}")) {
        Ok(url) => url,
        Err(_) => return false,
    };
    parsed.host_str().map(is_loopback_host).unwrap_or(false)
}

fn is_allowed_desktop_worker_origin(origin: &HeaderValue) -> bool {
    let Ok(raw) = origin.to_str() else {
        return false;
    };
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("tauri://localhost") {
        return true;
    }
    let Ok(url) = Url::parse(trimmed) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    match url.scheme() {
        "http" | "https" => is_loopback_host(host),
        "tauri" => host.eq_ignore_ascii_case("localhost"),
        _ => false,
    }
}

fn daemon_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            is_allowed_desktop_worker_origin(origin)
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::HeaderName::from_static("traceparent"),
            header::HeaderName::from_static("x-ctx-run-id"),
        ])
}

pub fn router(state: Arc<AppState>) -> axum::Router {
    let auth_state = state.clone();
    let perf_state = state.clone();
    let api = axum::Router::new()
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
        .route(
            "/api/sessions/:id/artifacts/:artifact_id",
            get(get_session_artifact),
        )
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
        .route("/api/dev/providers/restart", post(dev_restart_providers))
        .route(
            "/api/dev/sessions/:id/seed_transcript",
            post(dev_seed_session_transcript),
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
        .route(
            "/api/dictation/livekit/stream",
            get(dictation_livekit_stream_ws),
        )
        .route_layer(middleware::from_fn_with_state(perf_state, perf_middleware))
        .layer(middleware::from_fn_with_state(auth_state, auth_middleware))
        .layer(daemon_cors_layer())
        .with_state(state);

    let dist_dir = std::env::var("CTX_WEB_DIST").unwrap_or_else(|_| "apps/web/dist".into());
    let index_path = format!("{dist_dir}/index.html");
    api.fallback_service(ServeDir::new(dist_dir).not_found_service(ServeFile::new(index_path)))
}

const MOBILE_API_MIN_VERSION: i64 = 1;
const MOBILE_API_MAX_VERSION: i64 = 1;

#[derive(Debug, Serialize)]
struct HealthCompatibility {
    desktop_exact_version: String,
    desktop_build_id: String,
    desktop_dev_instance_id: String,
    protocol_compatibility_token: String,
    mobile_api_min: i64,
    mobile_api_max: i64,
}

#[derive(Debug, Serialize)]
struct HealthResp {
    version: String,
    daemon_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    daemon_url: Option<String>,
    auth_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_file_limit: Option<crate::process_limits::OpenFileLimitSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage: Option<crate::storage_guard::StorageGuardStatus>,
    compatibility: HealthCompatibility,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_request_base_url_accepts_loopback_host_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:4455"));
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321"),
            "http://127.0.0.1:4455"
        );
    }

    #[test]
    fn resolve_request_base_url_rejects_non_loopback_host_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("evil.example"));
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321"),
            "http://127.0.0.1:4321"
        );
    }

    #[test]
    fn resolve_request_base_url_rejects_non_http_forwarded_proto() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("proto=javascript;host=127.0.0.1:4455"),
        );
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321"),
            "http://127.0.0.1:4321"
        );
    }

    #[test]
    fn resolve_request_base_url_accepts_forwarded_loopback_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("proto=https;host=tauri.localhost:3000"),
        );
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321"),
            "https://tauri.localhost:3000"
        );
    }
}
