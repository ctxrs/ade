use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Multipart, Path, Query, State};
use axum::body::{Body, Bytes};
use axum::http::header;
use axum::http::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::Json;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use base64::Engine;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::io::ReaderStream;
use tower_http::services::{ServeDir, ServeFile};
use std::time::Instant;

use context_core::ids::*;
use context_core::models::*;
use context_fs::git::{assert_git_repo, list_tracked_files, list_untracked_files, rev_parse_head};
use context_fs::worktrees::{create_worktree, managed_worktree_path};

use crate::completions;
use crate::daemon::AppState;
use crate::installs::{InstallId, InstallInfo, InstallProgressEvent};
use crate::installer;
use crate::logs;
use crate::scheduler::SchedulerCommand;
use crate::updates;
use crate::buffers::{BufferCloseReq, BufferConflictResp, BufferId, BufferOpenReq, BufferOpenResp, BufferUpdateReq, BufferUpdateResp};
use context_providers::adapters::ProviderStatus;
use context_providers::events::NormalizedEvent;
use context_providers::{
    acp::{
        authenticate_provider, probe_provider_options, verify_provider_connection, AcpAgentConfig,
        AcpClientConfig,
    },
    ask_user_question::{AskUserQuestionAnswer, AskUserQuestionOutcome},
};
use crate::settings as user_settings;
use crate::dictation_livekit;

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
        .route("/api/settings", get(get_settings).post(update_settings))
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/blobs", post(upload_blob))
        .route("/api/blobs/:id", get(get_blob))
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
        .route("/api/lsp/status", get(lsp_status))
        .route("/api/lsp/catalog", get(lsp_catalog_list))
        .route("/api/lsp/catalog/:id/install", post(install_lsp_catalog_server))
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
        .route("/api/lsp/code_action/resolve", post(lsp_code_action_resolve))
        .route("/api/lsp/inlay_hints", post(lsp_inlay_hints))
        .route("/api/lsp/document_highlight", post(lsp_document_highlight))
        .route("/api/lsp/selection_ranges", post(lsp_selection_ranges))
        .route("/api/lsp/call_hierarchy/prepare", post(lsp_call_hierarchy_prepare))
        .route("/api/lsp/call_hierarchy/incoming", post(lsp_call_hierarchy_incoming))
        .route("/api/lsp/call_hierarchy/outgoing", post(lsp_call_hierarchy_outgoing))
        .route("/api/lsp/code_lens", post(lsp_code_lens))
        .route("/api/lsp/code_lens/resolve", post(lsp_code_lens_resolve))
        .route("/api/lsp/prepare_rename", post(lsp_prepare_rename))
        .route("/api/lsp/document_links", post(lsp_document_links))
        .route("/api/lsp/document_links/resolve", post(lsp_document_link_resolve))
        .route("/api/lsp/semantic_tokens/full", post(lsp_semantic_tokens_full))
        .route("/api/lsp/semantic_tokens/delta", post(lsp_semantic_tokens_delta))
        .route("/api/lsp/folding_ranges", post(lsp_folding_ranges))
        .route("/api/lsp/linked_editing_range", post(lsp_linked_editing_range))
        .route("/api/lsp/type_hierarchy/prepare", post(lsp_type_hierarchy_prepare))
        .route("/api/lsp/type_hierarchy/supertypes", post(lsp_type_hierarchy_supertypes))
        .route("/api/lsp/type_hierarchy/subtypes", post(lsp_type_hierarchy_subtypes))
        .route("/api/lsp/execute_command", post(lsp_execute_command))
        .route("/api/lsp/execute_command/plan", post(lsp_execute_command_plan))
        .route("/api/lsp/document_symbols", post(lsp_document_symbols))
        .route("/api/lsp/workspace_symbols", post(lsp_workspace_symbols))
        .route("/api/lsp/workspace_symbols/resolve", post(lsp_workspace_symbol_resolve))
        .route("/api/lsp/code_actions", post(lsp_code_actions))
        .route("/api/lsp/code_actions/by_diagnostic/plan", post(lsp_code_actions_by_diagnostic_plan))
        .route("/api/lsp/rename/plan", post(lsp_rename_plan))
        .route("/api/lsp/format/plan", post(lsp_format_plan))
        .route("/api/lsp/organize_imports/plan", post(lsp_organize_imports_plan))
        .route("/api/lsp/code_actions/plan", post(lsp_code_actions_plan))
        .route("/api/tracks/:id/edit_plans", get(list_edit_plans_for_track))
        .route("/api/edit_plans/:id", get(get_edit_plan))
        .route("/api/edit_plans/:id/apply", post(apply_edit_plan_patch))
        .route("/api/edit_plans/:id/discard", post(discard_edit_plan))
        .route("/api/buffers/open", post(open_buffer))
        .route("/api/buffers/update", post(update_buffer))
        .route("/api/buffers/close", post(close_buffer))
        .route("/api/workspaces", get(list_workspaces).post(create_workspace))
        .route("/api/workspaces/:id", delete(delete_workspace).get(get_workspace))
        .route(
            "/api/workspaces/:id/completions/files",
            get(workspace_file_completions),
        )
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
            "/api/workspaces/:id/tasks",
            get(list_tasks).post(create_task),
        )
        .route("/api/tasks/:id", get(get_task).delete(delete_task))
        .route("/api/tasks/:id/title", post(update_task_title))
        .route("/api/tasks/:id/archive", post(archive_task))
        .route("/api/tasks/:id/unarchive", post(unarchive_task))
        .route("/api/tasks/:id/mark_read", post(mark_task_read))
        .route("/api/tasks/:id/mark_unread", post(mark_task_unread))
        .route("/api/tasks/:id/tracks", get(list_tracks).post(create_track))
        .route("/api/worktrees/:id", get(get_worktree))
        .route(
            "/api/tracks/:id/sessions",
            get(list_sessions_for_track).post(create_session_for_track),
        )
        .route("/api/sessions/:id", get(get_session))
        .route("/api/sessions/:id/messages", get(list_messages).post(post_message))
        .route("/api/sessions/:id/model", post(set_session_model))
        .route("/api/sessions/:id/mode", post(set_session_mode))
        .route("/api/sessions/:id/turns", get(list_session_turns))
        .route("/api/sessions/:id/turns/:turn_id/tools", get(list_session_turn_tools))
        .route("/api/sessions/:id/events", get(list_session_events))
        .route("/api/sessions/:id/completions/files", get(session_file_completions))
        .route("/api/sessions/:id/queue", get(list_queue))
        .route("/api/messages/:id", delete(delete_message))
        .route("/api/sessions/:id/cancel", post(cancel_session))
        .route("/api/sessions/:id/interrupt", post(interrupt_session))
        .route("/api/sessions/:id/authenticate", post(authenticate_session))
        .route(
            "/api/sessions/:id/ask_user_question",
            post(submit_ask_user_question),
        )
        .route("/api/tracks/:id/diff", get(track_diff))
        .route("/api/tracks/:id/diff/apply", post(track_diff_apply))
        .route("/api/dictation/livekit/stream", get(dictation_livekit_stream_ws))
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
struct UpdateTaskTitleReq {
    title: String,
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
    let resp =
        persist_blob_bytes(&state, &bytes, &mime_type, file_name.as_deref()).await?;
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

async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let settings = user_settings::load_settings(&state.data_root).await;
    Ok(Json(user_settings::to_public(&settings)))
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
    Ok(Json(user_settings::to_public(&next)))
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
        ("dockerfile", cfg.dockerfile_command.clone(), cfg.dockerfile_args.clone()),
        ("cpp", cfg.clangd_command.clone(), cfg.clangd_args.clone()),
        ("lua", cfg.lua_command.clone(), cfg.lua_args.clone()),
        ("toml", cfg.toml_command.clone(), cfg.toml_args.clone()),
        ("markdown", cfg.markdown_command.clone(), cfg.markdown_args.clone()),
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
                "Configured via data_root/lsp/user_servers.json (restart daemon after edits).".to_string(),
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
    (a_old == b_old && a_new == b_new)
        || (a_new == b_new && !a_new.is_empty())
        || (a_old == b_old && !a_old.is_empty())
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
    let file =
        crate::buffers::BufferStore::resolve_path(&root, path).await.map_err(|_| StatusCode::BAD_REQUEST)?;
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
        .open_or_reuse(sid, worktree_id, root.clone(), file.clone(), text.clone(), disk_sha.clone())
        .await;
    if state.lsp.enabled() {
        if let Some(lang) = context_lsp::Language::detect(&file, &state.lsp_cfg) {
            state.ensure_lsp_diagnostics_forwarder(root.clone(), lang).await;
        }
        let _ = state.lsp.sync_document_text(&root, &file, st.text.clone()).await;
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
        let disk_text = tokio::fs::read_to_string(&current.path).await.map_err(|_| {
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
        if let Some(lang) = context_lsp::Language::detect(&st.path, &state.lsp_cfg) {
            state.ensure_lsp_diagnostics_forwarder(st.root.clone(), lang).await;
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
    let sid = SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
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
    let track_id = session.track_id;
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

    let diag: lsp_types::Diagnostic = serde_json::from_value(req.diagnostic.clone()).map_err(|e| {
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
                let edit = ca
                    .edit
                    .or_else(|| ca.command.as_ref().and_then(extract_workspace_edit_from_command));
                (ca.title, edit, ca.is_preferred.unwrap_or(false))
            }
            lsp_types::CodeActionOrCommand::Command(cmd) => {
                let edit = extract_workspace_edit_from_command(&cmd);
                (cmd.title, edit, false)
            }
        };
        let Some(edit) = edit else { continue };
        let plan = crate::edit_plans::workspace_edit_to_plan(
            &root,
            &root,
            sid,
            track_id,
            title,
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
    let track_id = session.track_id;

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
        .execute_command_for_file(&root, &file, req.command.clone(), req.arguments.unwrap_or_default())
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
                error: format!("execute_command returned no WorkspaceEdit (result={})", result),
            }),
        ));
    };

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        track_id,
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
    let track_id = session.track_id;
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
    let root = PathBuf::from(wt.root_path)
        .canonicalize()
        .map_err(|_| {
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
        track_id,
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
    let track_id = session.track_id;
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
    let root = PathBuf::from(wt.root_path)
        .canonicalize()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid worktree root".to_string(),
                }),
            )
        })?;
    let file = root.join(&req.path);
    let edits = state
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
        track_id,
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

fn extract_workspace_edit_from_command(cmd: &lsp_types::Command) -> Option<lsp_types::WorkspaceEdit> {
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
                    if let Ok(edit) = serde_json::from_value::<lsp_types::WorkspaceEdit>(val.clone()) {
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
    let track_id = session.track_id;
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
    let root = PathBuf::from(wt.root_path)
        .canonicalize()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid worktree root".to_string(),
                }),
            )
        })?;

    let action: lsp_types::CodeActionOrCommand = serde_json::from_value(req.action.clone()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let (title, edit) = match action {
        lsp_types::CodeActionOrCommand::CodeAction(ca) => {
            let edit = ca
                .edit
                .or_else(|| ca.command.as_ref().and_then(extract_workspace_edit_from_command));
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

    let plan = crate::edit_plans::workspace_edit_to_plan(
        &root,
        &root,
        sid,
        track_id,
        title,
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
    let track_id = session.track_id;
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
    let root = PathBuf::from(wt.root_path)
        .canonicalize()
        .map_err(|_| {
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
                start: lsp_types::Position { line: 0, character: 0 },
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
        lsp_types::CodeActionOrCommand::CodeAction(ca) => ca
            .edit
            .or_else(|| ca.command.as_ref().and_then(extract_workspace_edit_from_command)),
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
        track_id,
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

async fn list_edit_plans_for_track(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::edit_plans::EditPlanSummary>>, StatusCode> {
    let track_id = TrackId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let map = state.edit_plans.lock().await;
    let mut out = map
        .values()
        .filter(|p| p.track_id == track_id)
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
    let Some(plan) = map.get(&pid) else { return Err(StatusCode::NOT_FOUND) };
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
                let rel = if !pf.new_path.is_empty() { &pf.new_path } else { &pf.old_path };
                let Some(base) = plan_files.iter().find(|f| {
                    plan_paths_match(&pf.old_path, &pf.new_path, &f.old_path, &f.new_path)
                }).map(|f| f.base_sha256.clone()) else {
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
                            error: format!(
                                "edit plan is stale for {}; regenerate the plan",
                                rel
                            ),
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

            context_fs::git::git_apply_patch(
                worktree_root.to_string_lossy().as_ref(),
                &patch,
                context_fs::git::ApplyPatchTarget::Worktree,
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
                let rel = if !pf.new_path.is_empty() { pf.new_path.clone() } else { pf.old_path.clone() };
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
                        let f_rel = if !f.new_path.is_empty() { &f.new_path } else { &f.old_path };
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
            return Ok(Json(summary));
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
            return Ok(Json(summary));
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
}

async fn discard_edit_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let pid = crate::edit_plans::EditPlanId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
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

    let show_fake = std::env::var("CONTEXT_SHOW_FAKE_PROVIDER").ok().as_deref() == Some("1");
    for status in out.iter_mut() {
        installer::apply_managed_install_details(status, &managed);
        if status.provider_id == "fake" {
            status
                .details
                .insert("ui_hidden".into(), if show_fake { "false" } else { "true" }.into());
        }
        status.details.insert(
            "install_supported".into(),
            if installer::is_supported_managed_provider(&status.provider_id) {
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
    installer::apply_managed_install_details(&mut status, &managed);
    status.details.insert(
        "install_supported".into(),
        if installer::is_supported_managed_provider(&status.provider_id) {
            "true".into()
        } else {
            "false".into()
        },
    );
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
        "qwen" => Some(("qwen".to_string(), vec!["--experimental-acp".to_string()])),
        "auggie" => Some(("auggie".to_string(), vec!["--acp".to_string()])),
        "cagent" => Some(("cagent".to_string(), vec!["acp".to_string()])),
        "opencode" => Some(("opencode".to_string(), vec!["acp".to_string()])),
        "openhands" => Some(("openhands".to_string(), vec!["acp".to_string()])),
        "mistral" => Some(("vibe-acp".to_string(), vec![])),
        "goose" => Some(("goose".to_string(), vec!["acp".to_string()])),
        "kimi" => Some(("kimi".to_string(), vec!["--acp".to_string()])),
        _ => None,
    }
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
        if !st.installed || !matches!(st.health, context_providers::adapters::ProviderHealth::Ok) {
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
            state
                .provider_options_cache
                .lock()
                .await
                .insert(
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
    env.insert("CONTEXT_MCP_DISABLED".to_string(), "1".to_string());

    let probe = authenticate_provider(agent, client, PathBuf::from(&ws.root_path), env, req.method_id).await;
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
    env.insert("CONTEXT_MCP_DISABLED".to_string(), "1".to_string());

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
    state
        .provider_verify_cache
        .lock()
        .await
        .insert(
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
            if let Err(e) =
                installer::install_lsp_server_with_progress(state2.clone(), install_id, server_id.clone()).await
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
    for id in [
        "codex",
        "claude",
        "gemini",
        "qwen",
        "auggie",
        "cagent",
        "opencode",
        "openhands",
        "mistral",
        "goose",
        "kimi",
    ] {
        if let Some(install_id) = state.find_running_install(id).await {
            out.push(InstallStartResponse {
                provider_id: id.to_string(),
                install_id,
            });
            continue;
        }

        let status = state.provider_statuses.lock().await.get(id).cloned();
        if let Some(st) = status {
            if st.installed && matches!(st.health, context_providers::adapters::ProviderHealth::Ok) {
                continue;
            }
        }

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
    match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => Ok(Json(task)),
        None => Err(StatusCode::NOT_FOUND),
    }
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

    match state
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
        Some(task) => Ok(Json(task)),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "task not found".to_string(),
            }),
        )),
    }
}

async fn delete_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let deleted = state
        .store
        .delete_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn archive_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let updated = state
        .store
        .archive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => Ok(Json(task)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn unarchive_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let updated = state
        .store
        .unarchive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => Ok(Json(task)),
        None => Err(StatusCode::NOT_FOUND),
    }
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
    match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => Ok(Json(task)),
        None => Err(StatusCode::NOT_FOUND),
    }
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
    match state
        .store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
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
    #[serde(default)]
    env_target: Option<String>, // "worktree" | "local"
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

    let env_target = req.env_target.as_deref().unwrap_or("worktree").trim().to_lowercase();
    let worktree_id = match env_target.as_str() {
        "worktree" => {
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
            worktree_id
        }
        "local" => {
            if let Some(existing) = state
                .store
                .get_local_worktree_for_root(task.workspace_id, &ws.root_path)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?
            {
                existing.id
            } else {
                let worktree_id = WorktreeId::new();
                let worktree = Worktree {
                    id: worktree_id,
                    workspace_id: task.workspace_id,
                    root_path: ws.root_path.clone(),
                    base_commit_sha,
                    git_branch: None,
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
                worktree_id
            }
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "env_target must be worktree or local".to_string(),
                }),
            ));
        }
    };

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
    initial_prompt: Option<String>,
}

async fn create_session_for_track(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<SessionWithEnv>, StatusCode> {
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
        let start_seq = event.seq;
        state.publish_event(event).await;

        let turn = SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: Some(saved.id),
            assistant_message_id: None,
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

        let tx = state.ensure_scheduler(session.clone()).await;
        let _ = tx.send(SchedulerCommand::Enqueue(saved)).await;
    }

    let worktree = state
        .store
        .get_worktree(session.worktree_id)
        .await
        .ok()
        .flatten();
    Ok(Json(SessionWithEnv {
        env_target: env_target_for_worktree(worktree.as_ref()),
        session,
    }))
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionWithEnv>, StatusCode> {
    let perf = std::env::var_os("CONTEXT_PERF").is_some();
    let t0 = Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let out = match state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(session) => {
            let worktree = state
                .store
                .get_worktree(session.worktree_id)
                .await
                .ok()
                .flatten();
            Ok(Json(SessionWithEnv {
                env_target: env_target_for_worktree(worktree.as_ref()),
                session,
            }))
        }
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
) -> Result<Json<Vec<SessionWithEnv>>, StatusCode> {
    let track_id = TrackId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let sessions = state
        .store
        .list_sessions_for_track(track_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut out = Vec::with_capacity(sessions.len());
    for session in sessions {
        let worktree = state
            .store
            .get_worktree(session.worktree_id)
            .await
            .ok()
            .flatten();
        out.push(SessionWithEnv {
            env_target: env_target_for_worktree(worktree.as_ref()),
            session,
        });
    }
    Ok(Json(out))
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

#[derive(Debug, Deserialize, Default)]
struct ListSessionTurnsQuery {
    before_seq: Option<i64>,
    limit: Option<u32>,
}

async fn list_session_turns(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<ListSessionTurnsQuery>,
) -> Result<Json<Vec<SessionTurn>>, StatusCode> {
    let perf = std::env::var_os("CONTEXT_PERF").is_some();
    let t0 = Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit;
    let out = state
        .store
        .list_session_turns_page_by_seq(session_id, q.before_seq, limit)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    if perf {
        tracing::info!(
            target: "context_perf",
            endpoint = "list_session_turns",
            session_id = %session_id.0,
            ms = %t0.elapsed().as_millis(),
        );
    }
    out
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

async fn list_session_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<ListSessionEventsQuery>,
) -> Result<Json<Vec<SessionEvent>>, StatusCode> {
    let perf = std::env::var_os("CONTEXT_PERF").is_some();
    let t0 = Instant::now();
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit;
    let out = if let Some(tail) = q.tail {
        match state.store.list_session_events_tail_by_seq(session_id, tail).await {
            Ok(events) => Ok(Json(events)),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    } else {
        match state
            .store
            .list_session_events_page_by_seq(session_id, q.after_seq, limit)
            .await
        {
            Ok(events) => Ok(Json(events)),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
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
    after_seq: Option<i64>,
    tail: Option<u32>,
    limit: Option<u32>,
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
    let limit = q
        .limit
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT) as usize;

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

    Ok(Json(completions::filter_and_rank_paths(&files, &query, limit)))
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
    let limit = q
        .limit
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT) as usize;

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

    Ok(Json(completions::filter_and_rank_paths(&files, &query, limit)))
}

async fn load_and_cache_worktree_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
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
    Ok(files)
}

async fn load_and_cache_workspace_files(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
    root: &PathBuf,
    now: Instant,
) -> Result<Arc<Vec<String>>, StatusCode> {
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
                let saved = persist_blob_bytes(state.as_ref(), &bytes, &mime_type, name.as_deref())
                    .await?;
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

    let attachments = normalize_message_attachments(&state, req.attachments).await?;

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
    state.publish_event(event).await;

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
        assistant_message_id: None,
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

    let worktree = state.store.get_worktree(updated.worktree_id).await.ok().flatten();
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
            }),
        )
        .await
    {
        state.publish_event(event).await;
    }

    Ok(Json(SubmitAskUserQuestionResp { ok: true }))
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
struct StreamSessionSpec {
    session_id: String,
    #[serde(default)]
    after_seq: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GlobalStreamClientMsg {
    Set { sessions: Vec<StreamSessionSpec> },
}

async fn global_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_global_ws(socket, state))
}

async fn handle_global_ws(socket: WebSocket, state: Arc<AppState>) {
    use futures::{SinkExt, StreamExt};
    use std::collections::{HashMap, HashSet};
    use tokio::select;
    use tokio::sync::broadcast::error::RecvError;
    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};
    use tokio_util::sync::CancellationToken;

    let (ws_tx, mut ws_rx) = socket.split();
    let ws_tx = Arc::new(Mutex::new(ws_tx));

    let mut diag_rx = state.lsp_diag_broadcaster().subscribe();

    let conn_cancel = CancellationToken::new();
    let mut subscribed: HashMap<SessionId, i64> = HashMap::new();
    let mut tasks: HashMap<SessionId, (CancellationToken, tokio::task::JoinHandle<()>)> = HashMap::new();

    let stop_all = |tasks: &mut HashMap<SessionId, (CancellationToken, tokio::task::JoinHandle<()>)>| {
        for (_sid, (tok, handle)) in tasks.drain() {
            tok.cancel();
            handle.abort();
        }
    };

    let spawn_session_task = |session_id: SessionId,
                             mut after_seq: i64,
                             ws_tx: Arc<Mutex<futures::stream::SplitSink<WebSocket, WsMessage>>>,
                             state: Arc<AppState>,
                             conn_cancel: CancellationToken| {
        let task_cancel = CancellationToken::new();
        let task_cancel_child = task_cancel.clone();
        let conn_cancel_child = conn_cancel.clone();
        let join = tokio::spawn(async move {
            let mut head_rx = state.subscribe_session_event_head(session_id).await;
            let page_limit: u32 = 500;

            loop {
                if task_cancel_child.is_cancelled() || conn_cancel_child.is_cancelled() {
                    break;
                }

                // Catch up to current DB head.
                loop {
                    if task_cancel_child.is_cancelled() || conn_cancel_child.is_cancelled() {
                        return;
                    }
                    let page = match state
                        .store
                        .list_session_events_page_by_seq(session_id, Some(after_seq), Some(page_limit))
                        .await
                    {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(session_id = %session_id.0, "stream replay failed: {e}");
                            conn_cancel_child.cancel();
                            return;
                        }
                    };
                    if page.is_empty() {
                        break;
                    }

                    for ev in page {
                        after_seq = ev.seq;
                        let text = match serde_json::to_string(&ev) {
                            Ok(t) => t,
                            Err(_) => continue,
                        };
                        let send_res = timeout(Duration::from_secs(2), async {
                            let mut locked = ws_tx.lock().await;
                            locked.send(WsMessage::Text(text)).await
                        })
                        .await;

                        match send_res {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => {
                                conn_cancel_child.cancel();
                                return;
                            }
                            Err(_) => {
                                // Slow consumer: force reconnect/resume.
                                conn_cancel_child.cancel();
                                return;
                            }
                        }
                    }
                }

                // Wait for the session head to advance, then loop to fetch from DB.
                // `watch` is level-triggered (stores latest), so we can't miss a signal.
                let head = *head_rx.borrow();
                if after_seq >= head {
                    select! {
                        _ = task_cancel_child.cancelled() => break,
                        _ = conn_cancel_child.cancelled() => break,
                        _ = head_rx.changed() => {}
                    }
                }
            }
        });

        (task_cancel, join)
    };

    loop {
        select! {
            _ = conn_cancel.cancelled() => break,
            msg = ws_rx.next() => {
                let Some(Ok(msg)) = msg else { break };
                let WsMessage::Text(text) = msg else { continue };
                let Ok(parsed) = serde_json::from_str::<GlobalStreamClientMsg>(&text) else { continue };

                let GlobalStreamClientMsg::Set { sessions } = parsed;
                let mut next: HashMap<SessionId, i64> = HashMap::new();
                for s in sessions {
                    let Ok(u) = uuid::Uuid::parse_str(&s.session_id) else { continue };
                    next.insert(SessionId(u), s.after_seq.unwrap_or(0));
                }

                // Remove old sessions.
                let next_ids: HashSet<SessionId> = next.keys().cloned().collect();
                for sid in subscribed.keys().cloned().collect::<Vec<_>>() {
                    if next_ids.contains(&sid) { continue; }
                    if let Some((tok, handle)) = tasks.remove(&sid) {
                        tok.cancel();
                        handle.abort();
                    }
                }

                // Start/update tasks.
                for (sid, after_seq) in next.iter() {
                    let current = subscribed.get(sid).copied();
                    if current == Some(*after_seq) && tasks.contains_key(sid) {
                        continue;
                    }
                    if let Some((tok, handle)) = tasks.remove(sid) {
                        tok.cancel();
                        handle.abort();
                    }
                    let (tok, handle) = spawn_session_task(
                        *sid,
                        *after_seq,
                        ws_tx.clone(),
                        state.clone(),
                        conn_cancel.clone(),
                    );
                    tasks.insert(*sid, (tok, handle));
                }

                subscribed = next;
            }
            dev = diag_rx.recv() => {
                let msg = match dev {
                    Ok(msg) => msg,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                };
                if subscribed.is_empty() { continue; }
                let sid = msg.get("session_id").and_then(|v| v.as_str()).and_then(|s| uuid::Uuid::parse_str(s).ok()).map(SessionId);
                let Some(sid) = sid else { continue };
                if !subscribed.contains_key(&sid) { continue; }
                let Ok(text) = serde_json::to_string(&msg) else { continue };
                let send_res = timeout(Duration::from_secs(2), async {
                    let mut locked = ws_tx.lock().await;
                    locked.send(WsMessage::Text(text)).await
                }).await;
                match send_res {
                    Ok(Ok(())) => {}
                    _ => conn_cancel.cancel(),
                }
            }
        }
    }

    stop_all(&mut tasks);
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

async fn dictation_livekit_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| dictation_livekit::dictation_livekit_stream(socket, state))
}
