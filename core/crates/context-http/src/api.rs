use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
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
        .route("/api/workspaces", get(list_workspaces).post(create_workspace))
        .route("/api/workspaces/:id", delete(delete_workspace).get(get_workspace))
        .route(
            "/api/workspaces/:id/tasks",
            get(list_tasks).post(create_task),
        )
        .route("/api/tasks/:id", get(get_task))
        .route("/api/tasks/:id/tracks", get(list_tracks))
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
) -> Result<Json<Task>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let ws = state
        .store
        .get_workspace(ws_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    assert_git_repo(&ws.root_path)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let base_commit_sha = rev_parse_head(&ws.root_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let task = state
        .store
        .create_task(ws_id, req.title, req.description)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let worktree_id = WorktreeId::new();
    let wt_path = managed_worktree_path(&state.data_root, ws_id, worktree_id);
    if let Some(parent) = wt_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    let branch_name = format!("context/{}/{}", task.id.0, worktree_id.0);
    create_worktree(&ws.root_path, &wt_path, &base_commit_sha, &branch_name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let _track = state
        .store
        .create_track(task.id, ws_id, worktree_id, "default".into())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
        let broadcaster = state.get_broadcaster(session.id).await;
        let _ = broadcaster.send(event);

        let tx = state.ensure_scheduler(session.clone()).await;
        let _ = tx.send(SchedulerCommand::Enqueue(saved)).await;
    }

    Ok(Json(session))
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Session>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match state.store.get_session(session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? {
        Some(session) => Ok(Json(session)),
        None => Err(StatusCode::NOT_FOUND),
    }
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
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .list_queued_messages_for_session(session_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn list_session_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionEvent>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .store
        .list_session_events(session_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
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

    let broadcaster = state.get_broadcaster(session_id).await;
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
    let _ = broadcaster.send(event);

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
        let _ = broadcaster.send(queued);
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
    let broadcaster = state.get_broadcaster(session_id).await;
    let _ = broadcaster.send(event);

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
    let store = state.store.clone();
    let broadcaster = state.get_broadcaster(session_id).await;
    let drain_broadcaster = broadcaster.clone();
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
                let _ = drain_broadcaster.send(event);
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
    let _ = broadcaster.send(started);

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
            let _ = broadcaster.send(done);
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
            let _ = broadcaster.send(failed);
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
