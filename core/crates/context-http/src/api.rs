use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::Json;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tower_http::services::{ServeDir, ServeFile};
use std::time::Instant;

use context_core::ids::*;
use context_core::models::*;
use context_fs::git::{assert_git_repo, rev_parse_head};
use context_fs::worktrees::{create_worktree, managed_worktree_path};

use crate::daemon::AppState;
use crate::installs::{InstallId, InstallInfo, InstallProgressEvent};
use crate::installer;
use crate::logs;
use crate::scheduler::SchedulerCommand;
use crate::updates;
use context_providers::adapters::ProviderStatus;
use context_providers::events::NormalizedEvent;
use context_providers::{acp::probe_provider_options, acp::AcpAgentConfig, acp::AcpClientConfig};

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

pub fn router(state: Arc<AppState>) -> axum::Router {
    let auth_state = state.clone();
    let api = axum::Router::new()
        .route("/api/health", get(health))
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/logs/open", post(open_logs_folder))
        .route("/api/desktop/log", post(append_desktop_log))
        .route("/api/updates/check", get(check_updates))
        .route("/api/updates/appimage/download", post(download_appimage_update))
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
        .route("/api/lsp/diagnostics", post(lsp_diagnostics))
        .route("/api/workspaces", get(list_workspaces).post(create_workspace))
        .route("/api/workspaces/:id", delete(delete_workspace).get(get_workspace))
        .route(
            "/api/workspaces/:id/providers/:provider_id/options",
            get(get_provider_options),
        )
        .route(
            "/api/workspaces/:id/tasks",
            get(list_tasks).post(create_task),
        )
        .route("/api/tasks/:id", get(get_task))
        .route("/api/tasks/:id/tracks", get(list_tracks).post(create_track))
        .route(
            "/api/tracks/:id/sessions",
            get(list_sessions_for_track).post(create_session_for_track),
        )
        .route("/api/sessions/:id", get(get_session))
        .route("/api/sessions/:id/messages", get(list_messages).post(post_message))
        .route("/api/sessions/:id/model", post(set_session_model))
        .route("/api/sessions/:id/mode", post(set_session_mode))
        .route("/api/sessions/:id/events", get(list_session_events))
        .route("/api/sessions/:id/queue", get(list_queue))
        .route("/api/messages/:id", delete(delete_message))
        .route("/api/sessions/:id/cancel", post(cancel_session))
        .route("/api/sessions/:id/interrupt", post(interrupt_session))
        .route("/api/sessions/:id/authenticate", post(authenticate_session))
        .route("/api/tracks/:id/diff", get(track_diff))
        .route("/api/tracks/:id/diff/apply", post(track_diff_apply))
        .route("/api/stream", get(global_stream_ws))
        .route("/api/sessions/:id/stream", get(session_stream_ws))
        .layer(middleware::from_fn_with_state(auth_state, auth_middleware))
        .with_state(state)
        ;

    let dist_dir = std::env::var("CONTEXT_WEB_DIST").unwrap_or_else(|_| "apps/web/dist".into());
    let index_path = format!("{}/index.html", dist_dir);
    api.fallback_service(
        ServeDir::new(dist_dir).not_found_service(ServeFile::new(index_path)),
    )
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

async fn health(State(state): State<Arc<AppState>>) -> Result<Json<HealthResp>, StatusCode> {
    Ok(Json(HealthResp {
        version: env!("CARGO_PKG_VERSION").to_string(),
        pid: std::process::id(),
        data_root: state.data_root.to_string_lossy().to_string(),
        daemon_url: state.daemon_url.clone(),
        auth_required: state.auth_token.is_some(),
    }))
}

#[derive(Debug, Serialize)]
struct DiagnosticsResp {
    daemon: HealthResp,
    platform: serde_json::Value,
    logs: serde_json::Value,
    providers: Vec<ProviderStatus>,
    managed_installs: serde_json::Value,
}

async fn diagnostics(State(state): State<Arc<AppState>>) -> Result<Json<DiagnosticsResp>, StatusCode> {
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
struct LspDiagnosticsReq {
    /// Optional session scope; when present, `path` is resolved within the session worktree.
    session_id: Option<String>,
    /// Optional explicit root path; used only when `session_id` is absent.
    root_path: Option<String>,
    /// File path to analyze (absolute or relative to resolved root).
    path: String,
}

async fn lsp_diagnostics(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LspDiagnosticsReq>,
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
    req: LspDiagnosticsReq,
) -> Result<(PathBuf, PathBuf), StatusCode> {
    let root = if let Some(session_id) = req.session_id.as_deref() {
        let sid = SessionId(
            uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?,
        );
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
    let file = candidate.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?;
    if !file.starts_with(&root) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((root, file))
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

    let manifest = updates::fetch_latest_manifest_with_params(&base_url, &channel, query.as_deref())
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
    let dest = updates::updates_dir(&state.data_root).join("context.AppImage.new");
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
                error: "CONTEXT_APPIMAGE_PATH not set; cannot apply in place".to_string(),
            }),
        ));
    };
    let downloaded = updates::updates_dir(&state.data_root).join("context.AppImage.new");
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
        message: "Update applied in place. Quit and relaunch the desktop app to run the new version.".to_string(),
    }))
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    let Some(expected) = state.auth_token.clone() else {
        return Ok(next.run(req).await);
    };

    let path = req.uri().path();
    if !path.starts_with("/api/") || path == "/api/health" {
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

    if token.as_deref() != Some(expected.as_str()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
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

    for status in out.iter_mut() {
        installer::apply_managed_install_details(status, &managed);
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
    installer::apply_managed_install_details(&mut status, &managed);
    if let Some(install_id) = state.find_running_install(&id).await {
        status
            .details
            .insert("install_running".into(), "true".into());
        status.details.insert("install_id".into(), install_id.to_string());
    }
    Ok(Json(status))
}

#[derive(Debug, Serialize)]
struct InstallStartResponse {
    provider_id: String,
    install_id: InstallId,
}

fn default_agent_server_command(provider_id: &str) -> Option<(String, Vec<String>)> {
    match provider_id {
        "codex" => Some(("codex-acp".to_string(), vec![])),
        "claude" => Some(("claude-code-acp".to_string(), vec![])),
        "gemini" => Some(("gemini".to_string(), vec!["--experimental-acp".to_string()])),
        _ => None,
    }
}

async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiErrorResp>)> {
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let cache_key = format!("{}/{}", ws_id.0, provider_id);
    if let Some(cached) = state.provider_options_cache.lock().await.get(&cache_key) {
        if cached.cached_at.elapsed() < CACHE_TTL {
            return Ok(Json(cached.value.clone()));
        }
    }

    let provider_status = state
        .provider_statuses
        .lock()
        .await
        .get(&provider_id)
        .cloned();

    if let Some(st) = provider_status.as_ref() {
        if !st.installed || !matches!(st.health, context_providers::adapters::ProviderHealth::Ok) {
            let resp = redact_json_value(serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": st.installed,
                "health": st.health,
                "diagnostics": st.diagnostics,
                "probe_ok": false,
                "probe_error": "provider not installed or unhealthy",
                "probed_at": chrono::Utc::now().to_rfc3339(),
            }));
            state
                .provider_options_cache
                .lock()
                .await
                .insert(
                    cache_key,
                    crate::daemon::CachedProviderOptions {
                        cached_at: std::time::Instant::now(),
                        value: resp.clone(),
                    },
                );
            return Ok(Json(resp));
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

    let (command, args) = cfg
        .providers
        .get(&provider_id)
        .map(|c| (c.command.clone(), c.args.clone()))
        .or_else(|| default_agent_server_command(&provider_id))
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
        client_name: "context".to_string(),
        client_title: "Context".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        client_capabilities: serde_json::json!({}),
        mcp_servers: vec![],
    };

    let mut env = std::collections::HashMap::new();
    env.insert("CONTEXT_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.as_ref() {
        env.insert("CONTEXT_AUTH_TOKEN".to_string(), token.clone());
    }

    let probe = probe_provider_options(agent, client, PathBuf::from(&ws.root_path), env).await;

    let resp = match probe {
        Ok(probe) => redact_json_value(serde_json::json!({
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
        })),
        Err(e) => redact_json_value(serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.as_ref().map(|s| s.installed).unwrap_or(false),
            "probe_ok": false,
            "probe_error": logs::redact_sensitive(&e.to_string()),
            "probed_at": chrono::Utc::now().to_rfc3339(),
        })),
    };

    state
        .provider_options_cache
        .lock()
        .await
        .insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
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
    if !installer::is_supported_managed_provider(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let (install_id, started_new) = state.start_install(id.clone()).await;
    if started_new {
        let state2 = state.clone();
        let provider_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_provider_with_progress(state2.clone(), install_id, provider_id.clone()).await {
                tracing::error!("provider install failed ({provider_id}): {e:#}");
            }
        });
    }

    Ok(Json(InstallStartResponse {
        provider_id: id,
        install_id,
    }))
}

async fn install_all_providers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<InstallStartResponse>>, StatusCode> {
    let mut out = Vec::new();
    for id in ["codex", "claude", "gemini"] {
        let (install_id, started_new) = state.start_install(id.to_string()).await;
        if started_new {
            let state2 = state.clone();
            let provider_id = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = installer::install_provider_with_progress(state2.clone(), install_id, provider_id.clone()).await {
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
    let install_id: InstallId = uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
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
    let install_id: InstallId = uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
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
    let install_id: InstallId = uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let Some(sender) = state.get_install_sender(install_id).await else {
        return Err(StatusCode::NOT_FOUND);
    };

    let history = state.get_install_events(install_id).await.unwrap_or_default();
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

async fn list_workspaces(State(state): State<Arc<AppState>>) -> Result<Json<Vec<Workspace>>, StatusCode> {
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
    match state.store.get_workspace(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? {
        Some(ws) => Ok(Json(ws)),
        None => Err(StatusCode::NOT_FOUND),
    }
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
                error: format!(
                    "invalid root_path '{}': {}",
                    expanded.to_string_lossy(),
                    e
                ),
            }),
        )
    })?;

    assert_git_repo(&root_path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp { error: e.to_string() }),
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
    state
        .store
        .create_workspace(name, root_path_str)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })
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
struct CreateTaskReq {
    title: String,
    description: Option<String>,
    #[serde(default = "default_true")]
    create_default_track: bool,
    #[serde(default)]
    default_track_label: Option<String>,
}

fn default_true() -> bool {
    true
}

async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Task>>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .list_tasks(ws_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match state.store.get_task(task_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? {
        Some(task) => Ok(Json(task)),
        None => Err(StatusCode::NOT_FOUND),
    }
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

    let want_default_track = req.create_default_track;
    if want_default_track {
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

    if !req.create_default_track {
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
                    error: "git repo has no commits; create an initial commit before creating a worktree track".to_string(),
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
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
    }
    let branch_name = format!("context/{}/{}", task.id.0, worktree_id.0);
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
    };
    state
        .store
        .insert_worktree(worktree)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let label = req
        .default_track_label
        .unwrap_or_else(|| "default".to_string());
    let _track = state
        .store
        .create_track(task.id, ws_id, worktree_id, label)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    Ok(Json(task))
}

async fn list_tracks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Track>>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .list_tracks_for_task(task_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Debug, Deserialize)]
struct CreateTrackReq {
    #[serde(default)]
    label: Option<String>,
}

async fn create_track(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateTrackReq>,
) -> Result<Json<Track>, (StatusCode, Json<ApiErrorResp>)> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid task id".to_string(),
            }),
        )
    })?);

    let task = state
        .store
        .get_task(task_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load task".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "task not found".to_string(),
            }),
        ))?;

    let ws = state
        .store
        .get_workspace(task.workspace_id)
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

    assert_git_repo(&ws.root_path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let base_commit_sha = rev_parse_head(&ws.root_path).await.map_err(|e| {
        let msg = e.to_string().to_lowercase();
        if msg.contains("ambiguous argument 'head'")
            || msg.contains("unknown revision or path not in the working tree")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "git repo has no commits; create an initial commit before creating a worktree track".to_string(),
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
    let wt_path = managed_worktree_path(&state.data_root, task.workspace_id, worktree_id);
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
    let branch_name = format!("context/{}/{}", task.id.0, worktree_id.0);
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
        workspace_id: task.workspace_id,
        root_path: wt_path.to_string_lossy().to_string(),
        base_commit_sha,
        git_branch: Some(branch_name),
        created_at: chrono::Utc::now(),
    };
    state
        .store
        .insert_worktree(worktree)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    let label = req.label.unwrap_or_else(|| "track".to_string());
    let track = state
        .store
        .create_track(task_id, task.workspace_id, worktree_id, label)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    Ok(Json(track))
}

#[derive(Debug, Deserialize)]
struct CreateSessionReq {
    provider_id: String,
    model_id: String,
    initial_prompt: Option<String>,
}

async fn create_session_for_track(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<Session>, StatusCode> {
    let track_id = TrackId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let track = state
        .store
        .get_track(track_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let session = state
        .store
        .create_session(&track, req.provider_id, req.model_id, "implementer".into(), None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(prompt) = req.initial_prompt {
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let msg = Message {
            id: MessageId::new(),
            session_id: session.id,
            task_id: session.task_id,
            track_id: session.track_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
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
        state.publish_event(event).await;

        let tx = state.ensure_scheduler(session.clone()).await;
        let _ = tx.send(SchedulerCommand::Enqueue(saved)).await;
    }

    Ok(Json(session))
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Session>, StatusCode> {
    let perf = std::env::var_os("CONTEXT_PERF").is_some();
    let t0 = Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let out =
        match state
            .store
            .get_session(session_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            Some(session) => Ok(Json(session)),
            None => Err(StatusCode::NOT_FOUND),
        };
    if perf {
        tracing::info!(
            target: "context_perf",
            endpoint = "get_session",
            session_id = %session_id.0,
            ms = %t0.elapsed().as_millis(),
        );
    }
    out
}

async fn list_sessions_for_track(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Session>>, StatusCode> {
    let track_id = TrackId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .list_sessions_for_track(track_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn list_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Message>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .list_messages_for_session(session_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn list_queue(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Message>>, StatusCode> {
    let perf = std::env::var_os("CONTEXT_PERF").is_some();
    let t0 = Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let out = state
        .store
        .list_queued_messages_for_session(session_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    if perf {
        tracing::info!(
            target: "context_perf",
            endpoint = "list_queue",
            session_id = %session_id.0,
            ms = %t0.elapsed().as_millis(),
        );
    }
    out
}

async fn list_session_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<ListSessionEventsQuery>,
) -> Result<Json<Vec<SessionEvent>>, StatusCode> {
    let perf = std::env::var_os("CONTEXT_PERF").is_some();
    let t0 = Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let after = q
        .after
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(SessionEventId);
    let out = match state
        .store
        .list_session_events_page(session_id, after, q.limit)
        .await
    {
        Ok(events) => Ok(Json(events)),
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("after event not found") {
                Err(StatusCode::BAD_REQUEST)
            } else {
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    };
    if perf {
        tracing::info!(
            target: "context_perf",
            endpoint = "list_session_events",
            session_id = %session_id.0,
            ms = %t0.elapsed().as_millis(),
        );
    }
    out
}

#[derive(Debug, Deserialize, Default)]
struct ListSessionEventsQuery {
    after: Option<String>,
    limit: Option<u32>,
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

async fn post_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<Message>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
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

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let msg = Message {
        id: MessageId::new(),
        session_id,
        task_id: session.task_id,
        track_id: session.track_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        role: MessageRole::User,
        content: req.content,
        attachments: req.attachments,
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

    let tx = state.ensure_scheduler(session).await;
    let _ = tx.send(SchedulerCommand::Enqueue(saved.clone())).await;

    Ok(Json(saved))
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
) -> Result<Json<Session>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id)
            .cloned()
            .or_else(|| map.get("fake").cloned())
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

    Ok(Json(updated))
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
        map.get(&session.provider_id)
            .cloned()
            .or_else(|| map.get("fake").cloned())
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
        map.get(&session.provider_id)
            .cloned()
            .or_else(|| map.get("fake").cloned())
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
    provider_env.insert("CONTEXT_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.clone() {
        provider_env.insert("CONTEXT_AUTH_TOKEN".to_string(), token);
    }
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CONTEXT_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CONTEXT_SESSION_ID".to_string(), session.id.0.to_string());
    provider_env.insert("CONTEXT_MCP_TOKEN".to_string(), uuid::Uuid::new_v4().to_string());
    if let Ok(v) = std::env::var("CONTEXT_MCP_COMMAND") {
        provider_env.insert("CONTEXT_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CONTEXT_MCP_DISABLED") {
    provider_env.insert("CONTEXT_MCP_DISABLED".to_string(), v);
    }

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let state_for_events = state.clone();
    let store = state.store.clone();
    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            let payload = ev.payload_json.clone();
            if matches!(ev.event_type, SessionEventType::Init) {
                if let Some(ps) = payload.get("acp_session_id").and_then(serde_json::Value::as_str)
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
        .authenticate_session(session_key, workdir, provider_env, req.method_id.clone(), ev_tx)
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

#[derive(Debug, Serialize)]
struct DiffResponse {
    diff: String,
}

async fn track_diff(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DiffResponse>, StatusCode> {
    let perf = std::env::var_os("CONTEXT_PERF").is_some();
    let t0 = Instant::now();
    let track_id = TrackId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let track = state
        .store
        .get_track(track_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktree = state
        .store
        .get_worktree(track.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let diff = context_fs::worktrees::diff_worktree(&worktree.root_path, &worktree.base_commit_sha)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if perf {
        tracing::info!(
            target: "context_perf",
            endpoint = "track_diff",
            track_id = %track_id.0,
            ms = %t0.elapsed().as_millis(),
            bytes = diff.len(),
        );
    }
    Ok(Json(DiffResponse { diff }))
}

#[derive(Debug, Deserialize)]
struct TrackDiffApplyReq {
    action: String, // "accept" | "reject"
    patch: String,
}

async fn track_diff_apply(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<TrackDiffApplyReq>,
) -> Result<Json<DiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let perf = std::env::var_os("CONTEXT_PERF").is_some();
    let t0 = Instant::now();
    let track_id = TrackId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid track id".to_string(),
            }),
        )
    })?);
    let track = state
        .store
        .get_track(track_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "track not found".to_string(),
            }),
        ))?;
    let worktree = state
        .store
        .get_worktree(track.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;

    let action = req.action.trim().to_lowercase();
    let patch = req.patch;
    if patch.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "patch is empty".to_string(),
            }),
        ));
    }

    match action.as_str() {
        "accept" => {
            context_fs::git::git_apply_patch(
                &worktree.root_path,
                &patch,
                context_fs::git::ApplyPatchTarget::Index,
                false,
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
        }
        "reject" => {
            // First revert in worktree, then best-effort unstage.
            context_fs::git::git_apply_patch(
                &worktree.root_path,
                &patch,
                context_fs::git::ApplyPatchTarget::Worktree,
                true,
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
            context_fs::git::git_apply_patch_allow_noop(
                &worktree.root_path,
                &patch,
                context_fs::git::ApplyPatchTarget::Index,
                true,
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "action must be accept or reject".to_string(),
                }),
            ));
        }
    }

    let diff =
        context_fs::worktrees::diff_worktree(&worktree.root_path, &worktree.base_commit_sha)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
    if perf {
        tracing::info!(
            target: "context_perf",
            endpoint = "track_diff_apply",
            action = %action,
            track_id = %track_id.0,
            ms = %t0.elapsed().as_millis(),
            patch_bytes = patch.len(),
            diff_bytes = diff.len(),
        );
    }

    Ok(Json(DiffResponse { diff }))
}

async fn session_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match uuid::Uuid::parse_str(&id) {
        Ok(u) => SessionId(u),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    ws.on_upgrade(move |socket| handle_ws(socket, state, session_id))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GlobalStreamClientMsg {
    Subscribe { session_ids: Vec<String> },
    Unsubscribe { session_ids: Vec<String> },
    Set { session_ids: Vec<String> },
}

async fn global_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_global_ws(socket, state))
}

async fn handle_global_ws(mut socket: WebSocket, state: Arc<AppState>) {
    use tokio::select;

    let mut rx = state.global_broadcaster().subscribe();
    let mut subscribed: std::collections::HashSet<SessionId> = std::collections::HashSet::new();

    loop {
        select! {
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { break };
                let WsMessage::Text(text) = msg else { continue };
                let parsed: Result<GlobalStreamClientMsg, _> = serde_json::from_str(&text);
                let Ok(parsed) = parsed else { continue };

                let ids = match parsed {
                    GlobalStreamClientMsg::Subscribe { session_ids } => {
                        for s in session_ids {
                            if let Ok(u) = uuid::Uuid::parse_str(&s) {
                                subscribed.insert(SessionId(u));
                            }
                        }
                        continue;
                    }
                    GlobalStreamClientMsg::Unsubscribe { session_ids } => {
                        for s in session_ids {
                            if let Ok(u) = uuid::Uuid::parse_str(&s) {
                                subscribed.remove(&SessionId(u));
                            }
                        }
                        continue;
                    }
                    GlobalStreamClientMsg::Set { session_ids } => session_ids,
                };

                subscribed.clear();
                for s in ids {
                    if let Ok(u) = uuid::Uuid::parse_str(&s) {
                        subscribed.insert(SessionId(u));
                    }
                }
            }
            ev = rx.recv() => {
                let Ok(event) = ev else { continue };
                if subscribed.is_empty() { continue; }
                if !subscribed.contains(&event.session_id) { continue; }
                if let Ok(text) = serde_json::to_string(&event) {
                    if socket.send(WsMessage::Text(text)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>, session_id: SessionId) {
    let tx = state.get_broadcaster(session_id).await;
    let mut rx = tx.subscribe();
    while let Ok(event) = rx.recv().await {
        if let Ok(text) = serde_json::to_string(&event) {
            if socket.send(WsMessage::Text(text)).await.is_err() {
                break;
            }
        }
    }
}
